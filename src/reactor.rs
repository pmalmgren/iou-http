use io_uring::{
    squeue::{Entry, PushError},
    IoUring
};
use log::{debug, error, trace};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::io;

use thiserror::Error;
// https://github.com/dtolnay/thiserror

pub(crate) type ReactorSender = SyncSender<(Entry, Callback)>;

#[derive(Error, Debug)]
pub enum IouError {
    #[error("Error submitting event to submission queue {0}")]
    Push(#[from] PushError),
    #[error("IO Error: {0}")]
    Io(#[from] io::Error),
}

pub(crate) type Callback = Box<dyn FnOnce(i32) + Send + 'static>;
// TODO rename this

// TODO switch this to an UnsafeCell instead of a Mutex
pub(crate) struct Reactor(Rc<RefCell<Inner>>);

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

    pub fn tick(&mut self) -> Result<bool, IouError> {
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

        trace!("Reactor has {} events in flight", inner.events.len());

        // If there is at least one entry that's been submitted to io-uring
        // block this thread until there's a completion queue event
        // (events is how we track requests that are in-flight)
        if !inner.events.is_empty() {
            inner.iouring.submit_and_wait(1)?;
        }

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

        Ok(inner.events.len() > 0)
    }
}
