use io_uring::{opcode, types::Fd};
use log::debug;
use std::io::Error;
use std::mem;
use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::{future::Future, pin::Pin};

use crate::reactor::ReactorSender;

type AcceptResult = Result<RawFd, Error>;

// TODO (v2, after adding in other operations): Operation state

enum Lifecycle {
    Submitted,
    Waiting(Waker),
    // TODO what do we do with this?
    // Ignored,
    Completed(i32),
}

pub struct AcceptFuture {
    sender: ReactorSender,
    address: Pin<Box<libc::sockaddr>>,
    state: Arc<Mutex<Lifecycle>>,
    // If this is dropped, the file descriptor will be freed
    socket: TcpListener,
    raw_fd: RawFd,
}

impl AcceptFuture {
    pub fn new(sender: ReactorSender, addr: &str) -> AcceptFuture {
        let address = libc::sockaddr {
            sa_family: 0,
            sa_data: [0 as libc::c_char; 14],
        };
        let socket = TcpListener::bind(addr).expect("bind");
        let raw_fd = socket.as_raw_fd();
        AcceptFuture {
            sender,
            socket,
            raw_fd,
            address: Box::pin(address),
            state: Arc::new(Mutex::new(Lifecycle::Submitted)),
        }
    }
}

impl Future for AcceptFuture {
    type Output = AcceptResult;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let previous_state = mem::replace(&mut *(self.state).lock().unwrap(), Lifecycle::Submitted);
        match previous_state {
            Lifecycle::Submitted => {
                println!("{}", self.raw_fd);
                // need to submit to the queue
                *self.state.lock().unwrap() = Lifecycle::Waiting(cx.waker().clone());

                debug!("Submitting accept");
                let mut address_length: libc::socklen_t =
                    std::mem::size_of::<libc::sockaddr>() as _;
                let entry =
                    opcode::Accept::new(Fd(self.raw_fd), &mut *self.address, &mut address_length)
                        .build();
                let state_clone = self.state.clone();
                self.sender
                    .send((
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
                    ))
                    .unwrap();
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
                    Poll::Ready(Ok(ret as RawFd))
                } else {
                    Poll::Ready(Err(Error::from_raw_os_error(-ret)))
                }
            }
        }
    }
}
