use io_uring::{opcode, types::Fd};
use log::debug;
use std::io::Error;
use std::marker::PhantomData;
use std::mem;
use std::net::{TcpListener, TcpStream};
use std::os::unix::io::{AsRawFd, RawFd, FromRawFd};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::{future::Future, pin::Pin};

use crate::runtime::register;

use crate::lifecycle::Lifecycle;

type AcceptResult = Result<TcpStream, Error>;

// TODO (v2, after adding in other operations): Operation state

pub struct AcceptFuture<'a> {
    state: Arc<Mutex<Lifecycle>>,
    // If this is dropped, the file descriptor will be freed
    raw_fd: RawFd,
    socket: PhantomData<&'a TcpListener>
}

impl<'a> AcceptFuture<'a> {
    // TODO does this lifetime need to be static?
    pub fn new(socket: &'a TcpListener) -> AcceptFuture<'a> {
        AcceptFuture {
            raw_fd: socket.as_raw_fd(),
            state: Arc::new(Mutex::new(Lifecycle::Submitted)),
            socket: PhantomData
        }
    }
}

impl<'a> Future for AcceptFuture<'a> {
    type Output = AcceptResult;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let previous_state = mem::replace(&mut *(self.state).lock().unwrap(), Lifecycle::Submitted);
        match previous_state {
            Lifecycle::Submitted => {
                println!("{}", self.raw_fd);
                // need to submit to the queue
                *self.state.lock().unwrap() = Lifecycle::Waiting(cx.waker().clone());

                debug!("Submitting accept");
                let entry =
                    opcode::Accept::new(Fd(self.raw_fd), std::ptr::null_mut(), std::ptr::null_mut())
                        .build();
                let state_clone = self.state.clone();
                register(
                    entry,
                    Box::new(move |n: i32| {
                        debug!("Accept result: {}", n);
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
                    let stream = unsafe {
                        TcpStream::from_raw_fd(ret as RawFd)
                    };
                    Poll::Ready(Ok(stream))
                } else {
                    Poll::Ready(Err(Error::from_raw_os_error(-ret)))
                }
            }
        }
    }
}
