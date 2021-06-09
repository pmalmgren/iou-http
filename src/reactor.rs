use http;
use httparse::{Error as HttpParseError, Request, EMPTY_HEADER};
use io_uring::{
    opcode,
    squeue::{Entry, PushError},
    types::Fd,
    CompletionQueue, IoUring, SubmissionQueue, Submitter,
};
use log::{debug, error, info};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::os::unix::io::{AsRawFd, RawFd};
use std::rc::Rc;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::{io, mem, net};

use libc;
use nix::sys::socket::{InetAddr, SockAddr};
use thiserror::Error;
// https://github.com/dtolnay/thiserror

const AF_INET: u16 = libc::AF_INET as u16;
const AF_INET6: u16 = libc::AF_INET6 as u16;
const BUF_SIZE: usize = 512;
const CHANNEL_BUFFER: usize = 10;

pub type ReactorSender = SyncSender<(Entry, Callback)>;

#[derive(Error, Debug)]
pub enum IouError {
    #[error("Got invalid address family {0}")]
    InvalidAddressFamily(u16),
    #[error("Error submitting event to submission queue {0}")]
    Push(#[from] PushError),
    #[error("IO Error: {0}")]
    Io(#[from] io::Error),
}

type Callback = Box<dyn FnOnce(i32) + Send + 'static>;
// TODO rename this

// TODO switch this to an UnsafeCell instead of a Mutex
pub struct Reactor(Rc<RefCell<Inner>>);

struct Inner {
    iouring: IoUring,
    events: HashMap<u64, Callback>,
    user_data: u64,
    receiver: Receiver<(Entry, Callback)>,
}

impl Reactor {
    pub fn new() -> Result<(Reactor, ReactorSender), IouError> {
        // TODO make the io uring larger
        let iouring = IoUring::new(8)?;
        let events: HashMap<u64, Callback> = HashMap::new();
        let (tx, receiver) = sync_channel(10);

        let reactor = Reactor(Rc::new(RefCell::new(Inner {
            receiver,
            iouring,
            events,
            user_data: 0,
        })));

        Ok((reactor, tx))
    }

    pub fn tick(&mut self) -> Result<(), IouError> {
        let mut inner = self.0.borrow_mut();
        // TODO do we need to manually drop these?

        while let Ok((mut entry, callback)) = inner.receiver.try_recv() {
            let user_data = inner.user_data;
            debug!("Submitting entry {} to io uring", user_data);
            entry = entry.user_data(user_data);
            unsafe {
                inner.iouring.submission().push(&entry)?;
            }
            inner.events.insert(user_data, callback);
            inner.user_data += 1;
            inner.iouring.submit()?;
        }

        // inner.iouring.submit_and_wait(1);

        inner.iouring.completion().sync();

        let completed_entries: Vec<(u64, i32)> = inner
            .iouring
            .completion()
            .filter_map(|cqe| {
                let user_data = cqe.user_data();

                if user_data == u64::MAX {
                    debug!("Skipped cancelled completion entry.");
                    return None;
                }

                Some((user_data, cqe.result()))
            })
            .collect();

        if completed_entries.len() > 0 {
            debug!("Consumed {} entries in 1 tick", completed_entries.len());
        }

        for (user_data, ret) in completed_entries {
            debug!("Got completion for entry {}: {}", user_data, ret);

            if let Some(callback) = inner.events.remove(&user_data) {
                (callback)(ret)
            } else {
                error!(
                    "got completion event from unknown submission: {}",
                    user_data
                );
            }
        }

        Ok(())
    }
}
