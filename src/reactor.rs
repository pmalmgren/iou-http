use io_uring::{opcode, squeue::{PushError, Entry}, types::Fd, IoUring};
use log::{debug, error, info};
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::os::unix::io::{AsRawFd, RawFd};
use std::{io, mem, net};
use std::sync::mpsc::{Receiver, sync_channel, SyncSender};
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
pub type ReactorRegistrator = SyncSender<(Entry, Callback)>;

pub struct Reactor {
    ring: IoUring,
    user_data: u64,
    events: HashMap<u64, Callback>,
    receiver: Receiver<(Entry, Callback)>,
}

impl Reactor {
    pub fn new() -> Result<(Reactor, ReactorRegistrator), IouError> {
        let ring = IoUring::new(8)?;
        let user_data = 1u64;
        let events: HashMap<u64, Callback> = HashMap::new();
        let (sender, receiver) = sync_channel(CHANNEL_BUFFER);
        Ok((Reactor {
            ring,
            user_data,
            events,
            receiver
        }, sender))
    }

    pub fn run(&mut self) -> Result<(), IouError> {
        // TODO split submission and completion queues so they're owned
        // by different threads
        loop {
            // TODO break out of the loop when the senders disconnect
            while let Ok((mut entry, callback)) = self.receiver.try_recv() {
                debug!("Submitting entry {} to io uring", self.user_data);
                entry = entry.user_data(self.user_data);
                unsafe {
                    self.ring.submission().push(&entry)?;
                }
                self.events.insert(self.user_data, callback);
                self.user_data += 1;
            }

            self.ring.submitter().submit_and_wait(1)?;

            for cqe in self.ring.completion() {
                let ret = cqe.result();
                let user_data = cqe.user_data();
                debug!("Got completion for entry {}: {}", user_data, ret);

                if let Some(callback) = self.events.remove(&user_data) {
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
