
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
use libc;
use std::marker::PhantomData;
use bytes::BytesMut;

use crate::runtime::register;

type RecvResult = Result<usize, Error>;

use crate::lifecycle::Lifecycle;

pub struct RecvFuture<'a> {
    stream: PhantomData<&'a TcpStream>,
    raw_fd: RawFd,
	// TODO use a more efficient way to store this
    buf: BytesMut,
	state: Arc<Mutex<Lifecycle>>,
}

impl<'a> RecvFuture<'a> {
    pub fn new(buf: BytesMut, stream: &'a mut TcpStream) -> RecvFuture<'a> {
        let raw_fd = stream.as_raw_fd();
        RecvFuture {
            stream: PhantomData,
            raw_fd,
            buf,
            state: Arc::new(Mutex::new(Lifecycle::Submitted))
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
                // need to submit to the queue
                *self.state.lock().unwrap() = Lifecycle::Waiting(cx.waker().clone());

                debug!("Submitting Recv (buf capacity: {})", self.buf.capacity());
                let entry =
                    opcode::Recv::new(Fd(self.raw_fd), self.buf.as_mut_ptr(), self.buf.capacity() as u32)
                        /*
						// TODO change this flag to not wait until socket is closed (probably)
                        .flags(libc::MSG_WAITALL)
                        */
                        .build();
                let state_clone = self.state.clone();
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
                    }),
                );
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
