use io_uring::{opcode, squeue::PushError, types::Fd, IoUring};
use log::{debug, error, info};
use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::os::unix::io::{AsRawFd, RawFd};
use std::{io, mem, net};

use libc;
use nix::sys::socket::{InetAddr, SockAddr};
use thiserror::Error;
// https://github.com/dtolnay/thiserror

const AF_INET: u16 = libc::AF_INET as u16;
const AF_INET6: u16 = libc::AF_INET6 as u16;

#[derive(Error, Debug)]
pub enum IouError {
    #[error("Got invalid address family {0}")]
    InvalidAddressFamily(u16),
    #[error("Error submitting event to submission queue {0}")]
    Push(#[from] PushError),
    #[error("IO Error: {0}")]
    Io(#[from] io::Error),
}

enum EventType {
    Accept(AcceptParams),
    Recv(ReceiveParams),
    Close(Fd),
}

struct AcceptParams {
    address: libc::sockaddr,
    address_length: libc::socklen_t,
}

struct ReceiveParams {
    buf: [u8; 512],
    fd: Fd,
}

struct Peer {
    fd: Fd,
    address: SockAddr,
}

impl Peer {
    fn new_from_accept_params(accept_params: AcceptParams, fd: Fd) -> Result<Self, IouError> {
        let address = match accept_params.address.sa_family {
            AF_INET => unsafe {
                Ok(SockAddr::Inet(InetAddr::V4(
                    *((&accept_params.address as *const libc::sockaddr)
                        as *const libc::sockaddr_in),
                )))
            },
            AF_INET6 => unsafe {
                Ok(SockAddr::Inet(InetAddr::V6(
                    *((&accept_params.address as *const libc::sockaddr)
                        as *const libc::sockaddr_in6),
                )))
            },
            _ => Err(IouError::InvalidAddressFamily(
                accept_params.address.sa_family,
            )),
        }?;

        Ok(Peer { fd, address })
    }
}

#[allow(dead_code)]
pub struct Server {
    ring: IoUring,
    user_data: u64,
    events: HashMap<u64, EventType>,
    socket: net::TcpListener,
    raw_fd: RawFd,
    peers: HashMap<i32, Peer>,
}

impl Server {
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Server, IouError> {
        let socket = net::TcpListener::bind(addr)?;
        info!("listening on: {}", socket.local_addr().unwrap());
        let raw_fd = socket.as_raw_fd();
        let ring = IoUring::new(8)?;
        let user_data = 1u64;
        let events: HashMap<u64, EventType> = HashMap::new();
        let peers: HashMap<i32, Peer> = HashMap::new();

        Ok(Server {
            socket,
            raw_fd,
            events,
            ring,
            user_data,
            peers,
        })
    }

    pub fn run(&mut self) -> Result<(), IouError> {
        self.accept()?;
        loop {
            self.ring.submitter().submit_and_wait(1)?;

            let mut should_accept = false;
            let mut read_fds: Vec<Fd> = Vec::new();
            let mut close_fds: Vec<Fd> = Vec::new();
            for cqe in self.ring.completion() {
                let ret = cqe.result();
                let user_data = cqe.user_data();

                if let Some(event) = self.events.remove(&user_data) {
                    match event {
                        EventType::Accept(accept) => {
                            // create socket struct
                            let fd = if ret >= 0 {
                                Fd(ret)
                            } else {
                                error!("accept error: {}", ret);
                                return Err(io::Error::from_raw_os_error(-ret).into());
                            };
                            let peer = Peer::new_from_accept_params(accept, fd)?;
                            debug!("Received connection from {:?}", peer.address);
                            self.peers.insert(ret, peer);
                            read_fds.push(fd);
                            should_accept = true;
                        }
                        EventType::Recv(receive) => {
                            if ret < 0 {
                                let err = io::Error::from_raw_os_error(-ret);
                                error!("recv error on fd {:?}: {:?} ", receive.fd, err);
                            }
                            if ret == 0 {
                                info!("client {:?} disconnected", receive.fd);
                                close_fds.push(receive.fd);
                                self.peers.remove(&(receive.fd.0 as i32));
                                continue;
                            }

                            debug!("socket {:?} received data {:?}", receive.fd, receive.buf);
                            read_fds.push(receive.fd);
                        }
                        EventType::Close(fd) => {
                            if ret < 0 {
                                let err = io::Error::from_raw_os_error(-ret);
                                error!("close error {:?}: {:?} ", fd, err);
                            }
                            debug!("closed socket {:?}", fd);
                            self.peers.remove(&fd.0);
                        }
                    }
                } else {
                    error!(
                        "got completion event from unknown submission: {}",
                        user_data
                    );
                }
            }
            if should_accept {
                self.accept()?;
            }
            for fd in read_fds {
                self.receive(fd)?;
            }
            for fd in close_fds {
                self.close(fd)?;
            }
        }
    }

    fn close(&mut self, fd: Fd) -> Result<(), IouError> {
        let event = EventType::Close(fd);
        let close = opcode::Close::new(fd).build().user_data(self.user_data);
        self.events.insert(self.user_data, event);
        self.user_data += 1;
        unsafe {
            self.ring.submission().push(&close)?;
        }
        Ok(())
    }

    fn accept(&mut self) -> Result<(), IouError> {
        let address = libc::sockaddr {
            sa_family: 0,
            sa_data: [0 as libc::c_char; 14],
        };
        let address_length: libc::socklen_t = mem::size_of::<libc::sockaddr>() as _;
        let event = EventType::Accept(AcceptParams {
            address,
            address_length,
        });
        self.events.insert(self.user_data, event);
        let accept = match self.events.get_mut(&self.user_data).unwrap() {
            EventType::Accept(ref mut ap) => {
                opcode::Accept::new(Fd(self.raw_fd), &mut ap.address, &mut ap.address_length)
                            .build()
                            .user_data(self.user_data)
            },
            _ => panic!("unreachable code"),
        };
        self.user_data += 1;
        unsafe {
            self.ring.submission().push(&accept)?;
        }
        Ok(())
    }

    fn receive(&mut self, fd: Fd) -> Result<(), IouError> {
        let buf = [0; 512];
        self.events
            .insert(self.user_data, EventType::Recv(ReceiveParams { buf, fd }));

        let receive = match self.events.get_mut(&self.user_data).unwrap() {
            EventType::Recv(ref mut params) => {
                opcode::Recv::new(fd, params.buf.as_mut_ptr(), buf.len() as u32)
                    .flags(libc::MSG_WAITALL)
                    .build()
                    .user_data(self.user_data)
            },
            _ => panic!("Unreachable code"),
        };
        self.user_data += 1;
        unsafe { self.ring.submission().push(&receive)? }
        Ok(())
    }
}
