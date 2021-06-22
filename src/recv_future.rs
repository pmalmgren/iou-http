
use io_uring::{opcode, types::Fd};
use log::debug;
use std::io::Error;
use std::mem;
use std::net::TcpStream;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::{future::Future, pin::Pin};
use std::convert::TryInto;
use std::marker::PhantomData;

use crate::runtime::register;

type RecvResult = Result<usize, Error>;

use crate::lifecycle::Lifecycle;

pub struct RecvFuture<'a> {
    stream: PhantomData<&'a TcpStream>,
    raw_fd: RawFd,
	// TODO use a more efficient way to store this
	state: Arc<Mutex<Lifecycle>>,
}

impl<'a> RecvFuture<'a> {
    // TODO should this buffer be pinned? (because we're giving the pointer to io-uring)
    pub fn submit(buf: &'a mut [u8], stream: &'a mut TcpStream) -> RecvFuture<'a> {
        let raw_fd = stream.as_raw_fd();
        let state = Arc::new(Mutex::new(Lifecycle::Submitted));
        // TODO how do we figure out the length?
        debug!("Submitting Recv (buf length: {})", buf.len());
        let entry =
            opcode::Recv::new(Fd(raw_fd), buf.as_mut_ptr(), buf.len() as u32)
            .build();
        let state_clone = state.clone();
        register(
            entry,
            Box::new(move |n: i32| {
                debug!("Recv result: {}", n);
                let previous_state = mem::replace(
                    &mut *(state_clone).lock().unwrap(),
                    Lifecycle::Completed(n),
                );
                if let Lifecycle::Waiting(waker) = previous_state {
                    waker.wake();
                }
            }));
        RecvFuture {
            stream: PhantomData,
            raw_fd,
            state,
        }
    }
}

// https://docs.rs/bytes/1.0.1/bytes/

impl<'a> Future for RecvFuture<'a> {
    type Output = RecvResult;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let previous_state = mem::replace(&mut *(self.state).lock().unwrap(), Lifecycle::Submitted);
        match previous_state {
            Lifecycle::Submitted => {
                println!("{}", self.raw_fd);
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
                    Poll::Ready(Ok(ret.try_into().unwrap()))
                } else {
                    Poll::Ready(Err(Error::from_raw_os_error(-ret)))
                }
            }
        }
    }
}
