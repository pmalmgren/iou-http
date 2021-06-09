use io_uring::{opcode, squeue::{PushError, Entry}, types::Fd, IoUring, SubmissionQueue, Submitter, CompletionQueue};
use log::{debug, error, info};
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::os::unix::io::{AsRawFd, RawFd};
use std::{io, mem, net};
use std::sync::mpsc::{Receiver, sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use httparse::{Request, EMPTY_HEADER, Error as HttpParseError};
use http;

use libc;
use nix::sys::socket::{InetAddr, SockAddr};
use thiserror::Error;
// https://github.com/dtolnay/thiserror

const AF_INET: u16 = libc::AF_INET as u16;
const AF_INET6: u16 = libc::AF_INET6 as u16;
const BUF_SIZE: usize = 512;
const CHANNEL_BUFFER: usize = 10;

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
#[derive(Clone)]
pub struct Reactor(Arc<Mutex<ReactorInner>>);

struct ReactorInner {
    completion: CompletionQueue<'static>,
    events: Arc<Mutex<HashMap<u64, Callback>>>,
}

#[derive(Clone)]
pub struct ReactorSender(Arc<Mutex<ReactorSenderInner>>);
struct ReactorSenderInner {
    submission_queue: SubmissionQueue<'static>,
    submitter: Submitter<'static>,
    user_data: AtomicU64,
    events: Arc<Mutex<HashMap<u64, Callback>>>,
}

impl ReactorSender {
    pub fn submit(&self, mut entry: Entry, callback: Callback) -> Result<(), IouError> {
        let inner = self.0.lock().unwrap();
        let user_data = inner.user_data.fetch_add(1, Ordering::Relaxed);
        debug!("Submitting entry {} to io uring", user_data);
        entry = entry.user_data(user_data);
        unsafe {
            inner.submission_queue.push(&entry)?;
        }
        inner.events.lock().unwrap().insert(user_data, callback);
        inner.submitter.submit()?;

        Ok(())
    }
}

impl Reactor {
    pub fn new() -> Result<(Reactor, ReactorSender), IouError> {
        // TODO make the io uring larger
        let ring = IoUring::new(8)?;
        let events: Arc<Mutex<HashMap<u64, Callback>>> = Arc::new(Mutex::new(HashMap::new()));
        let (submitter, submission_queue, completion) = Arc::new(ring).split();

        let reactor = Reactor(Arc::new(Mutex::new(ReactorInner {
            completion,
            events: events.clone(),
        })));
        let sender = ReactorSender(Arc::new(Mutex::new(ReactorSenderInner {
            submission_queue,
            submitter,
            user_data: AtomicU64::new(0),
            events,
        })));

        Ok((reactor, sender))
    }

    pub fn run(&mut self) -> Result<(), IouError> {
        let inner = self.0.lock().unwrap();
        loop {
            for cqe in inner.completion {
                let ret = cqe.result();
                let user_data = cqe.user_data();
                debug!("Got completion for entry {}: {}", user_data, ret);

                if let Some(callback) = inner.events.lock().unwrap().remove(&user_data) {
                    (callback)(ret)
                } else {
                    error!(
                        "got completion event from unknown submission: {}",
                        user_data
                    );
                }
            }
        }
    }
}
