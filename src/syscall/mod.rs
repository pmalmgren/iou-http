use io_uring::squeue::Entry;
use std::marker::PhantomData;
use std::io::Error;
use std::sync::{Arc, Mutex};
use std::future::Future;
use std::pin::Pin;
use std::mem;
use std::task::{Context, Poll, Waker};

mod accept;
mod close;
mod recv;
mod send;

pub use accept::Accept;
pub use close::Close;
pub use recv::Recv;
pub use send::Send;

use crate::runtime::register;

pub(crate) enum Lifecycle {
    Submitted,
    Waiting(Waker),
    // TODO what do we do with this?
    // Ignored,
    Completed(i32),
}

pub struct SysCall<T> {
    state: Arc<Mutex<Lifecycle>>,
	kind: PhantomData<T>,
}

impl<T> SysCall<T> {
	pub fn from_entry(entry: Entry, _kind: T) -> SysCall<T> {
		let state = Arc::new(Mutex::new(Lifecycle::Submitted));
		let state_clone = state.clone();
        register(
            entry,
            Box::new(move |n: i32| {
                let previous_state = mem::replace(
                    &mut *(state_clone).lock().unwrap(),
                    Lifecycle::Completed(n),
                );
                if let Lifecycle::Waiting(waker) = previous_state {
                    waker.wake();
                }
            }));
		SysCall {
			state,
			kind: PhantomData
		}
	}
}

impl<T> Future for SysCall<T> {
    type Output = Result<u32, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let previous_state = mem::replace(&mut *(self.state).lock().unwrap(), Lifecycle::Submitted);
        match previous_state {
            Lifecycle::Submitted => {
                // need to submit to the queue
                *self.state.lock().unwrap() = Lifecycle::Waiting(cx.waker().clone());

                Poll::Pending
            }
            Lifecycle::Waiting(waker) => {
                if !waker.will_wake(cx.waker()) {
                    *self.state.lock().unwrap() = Lifecycle::Waiting(cx.waker().clone());
                }
                Poll::Pending
            }
            // Lifecycle::Ignored => {
            //     unimplemented!("ignored futures aren't implemented yet");
            // },
            Lifecycle::Completed(ret) => {
                // todo, replace state with completed
                if ret >= 0 {
                    Poll::Ready(Ok(ret as u32))
                } else {
                    Poll::Ready(Err(Error::from_raw_os_error(-ret)))
                }
            }
        }
    }
}
