use futures::FutureExt;
use io_uring::{opcode, squeue::Entry, types::Fd};
use libc::sockaddr;
use log::{debug, trace};
use std::io::Error;
use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::{future::Future, pin::Pin};

use crate::reactor::ReactorRegistrator;

type AcceptResult = Result<RawFd, Error>;

struct SharedState {
    result: Option<AcceptResult>,
    waker: Option<Waker>,
}

pub struct AcceptFuture {
    submitted: bool,
    sender: ReactorRegistrator,
    address: Pin<Box<libc::sockaddr>>,
    state: Arc<Mutex<SharedState>>,
    socket: TcpListener,
    raw_fd: RawFd,
}

impl AcceptFuture {
    pub fn new(sender: ReactorRegistrator, addr: &str) -> AcceptFuture {
        let mut address = libc::sockaddr {
            sa_family: 0,
            sa_data: [0 as libc::c_char; 14],
        };
        println!("Address: {:p}", &address);
        let socket = TcpListener::bind(addr).expect("bind");
        let raw_fd = socket.as_raw_fd();
        AcceptFuture {
            submitted: false,
            sender,
            socket,
            raw_fd,
            address: Box::pin(address),
            state: Arc::new(Mutex::new(SharedState {
                result: None,
                waker: None,
            })),
        }
    }
}

impl Future for AcceptFuture {
    type Output = AcceptResult;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        trace!("Poll");
        println!("Address: {:p}", &self.address);
        // Store the most recent waker in the future
        {
            let mut shared_state = self.state.lock().unwrap();
            shared_state.waker = Some(cx.waker().clone());
        }

        if !self.submitted {
            debug!("Submitting accept");
            self.submitted = true;
            let mut address_length: libc::socklen_t = std::mem::size_of::<libc::sockaddr>() as _;
            let entry =
                opcode::Accept::new(Fd(self.raw_fd), &mut *self.address, &mut address_length)
                    .build();
            let shared_state_clone = self.state.clone();
            self.sender
                .send((
                    entry,
                    Box::new(move |n: i32| {
                        debug!("Accept result: {}", n);
                        let mut shared_state = shared_state_clone.lock().unwrap();
                        if n < 0 {
                            shared_state.result = Some(Err(Error::from_raw_os_error(-n)))
                        } else {
                            shared_state.result = Some(Ok(n as RawFd));
                        }
                        let waker = shared_state.waker.take().expect("Expected waker");
                        waker.wake();
                    }),
                ))
                .unwrap();
            return Poll::Pending;
        }

        // TODO panic if polled after finished
        if let Some(result) = self.state.lock().unwrap().result.take() {
            Poll::Ready(result)
        } else {
            Poll::Pending
        }
    }
}
