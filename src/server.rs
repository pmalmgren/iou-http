use http;
use httparse::{Error as HttpParseError, Request, EMPTY_HEADER};
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
const BUF_SIZE: usize = 512;

#[derive(Error, Debug)]
pub enum IouError {
    #[error("Got invalid address family {0}")]
    InvalidAddressFamily(u16),
    #[error("Error submitting event to submission queue {0}")]
    Push(#[from] PushError),
    #[error("IO Error: {0}")]
    Io(#[from] io::Error),
    #[error("Error parsing HTTP headers: {0}")]
    HttpParse(#[from] HttpParseError),
}

enum EventType {
    Accept(AcceptParams),
    Recv(ReceiveParams),
    Close(Fd),
    Send(SendParams),
}

struct AcceptParams {
    address: libc::sockaddr,
    address_length: libc::socklen_t,
}

struct ReceiveParams {
    buf: Vec<u8>,
    fd: Fd,
    curr_chunk: usize,
}

#[derive(Debug)]
struct SendParams {
    buf: Vec<u8>,
    fd: Fd,
}

impl ReceiveParams {
    fn new(fd: Fd) -> ReceiveParams {
        ReceiveParams {
            fd,
            buf: Vec::new(),
            curr_chunk: 0,
        }
    }
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

pub type HttpHandler = fn(Request) -> http::Response<String>;

#[allow(dead_code)]
pub struct Server {
    ring: IoUring,
    user_data: u64,
    events: HashMap<u64, EventType>,
    socket: net::TcpListener,
    raw_fd: RawFd,
    peers: HashMap<i32, Peer>,
    handler: HttpHandler,
}

fn complete_http_request(buf: &[u8]) -> bool {
    // a complete HTTP request is of the form
    // VERB /PATH HTTP/VERSION\r\nHeader: value...\r\n
    let iter = buf.windows(4);

    for slice in iter {
        if slice == b"\r\n\r\n" {
            return true;
        }
    }
    false
}

impl Server {
    pub fn new<A: ToSocketAddrs>(addr: A, handler: HttpHandler) -> Result<Server, IouError> {
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
            handler,
        })
    }

    pub fn run(&mut self) -> Result<(), IouError> {
        self.accept()?;
        loop {
            self.ring.submitter().submit_and_wait(1)?;

            let mut should_accept = false;
            let mut read_fds: Vec<ReceiveParams> = Vec::new();
            let mut close_fds: Vec<Fd> = Vec::new();
            let mut send_fds: Vec<SendParams> = Vec::new();
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
                            read_fds.push(ReceiveParams::new(fd));
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
                                continue;
                            }

                            let mut headers = [EMPTY_HEADER; 64];
                            let mut request = Request::new(&mut headers);
                            if complete_http_request(&receive.buf) {
                                request.parse(&receive.buf)?;
                                debug!("{:?}", request);

                                let response = (self.handler)(request);
                                let (parts, body) = response.into_parts();
                                // TODO serialize the HTTP response with less copying
                                let mut response = format!("HTTP/1.1 {}\r\n", parts.status);
                                let mut has_content_length: bool = false;
                                for (name, value) in parts.headers.iter() {
                                    if let Ok(value) = value.to_str() {
                                        has_content_length =
                                            value.to_ascii_lowercase() == "content-length";
                                        response
                                            .push_str(format!("{}: {}\r\n", name, value).as_str());
                                    }
                                }
                                if !has_content_length {
                                    response.push_str(
                                        format!("Content-Length: {}\r\n", body.len()).as_str(),
                                    );
                                }
                                response.push_str("\r\n");
                                response.push_str(body.as_str());
                                debug!("response: {}", response);
                                send_fds.push(SendParams {
                                    buf: response.into_bytes(),
                                    fd: receive.fd,
                                });
                            }

                            // TODO handle HTTP Keep Alive
                            if (ret as usize) < BUF_SIZE {
                                read_fds.push(ReceiveParams::new(receive.fd));
                            } else {
                                read_fds.push(receive);
                            }
                        }
                        EventType::Close(fd) => {
                            if ret < 0 {
                                let err = io::Error::from_raw_os_error(-ret);
                                error!("close error {:?}: {:?} ", fd, err);
                            }
                            debug!("closed socket {:?}", fd);
                            self.peers.remove(&fd.0);
                        }
                        EventType::Send(send_params) => {
                            debug!("Wrote {} bytes", ret);
                            // TODO handle partial write
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
            for receive_param in read_fds {
                self.receive(receive_param)?;
            }
            for send_param in send_fds {
                self.send(send_param)?;
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
            }
            _ => panic!("unreachable code"),
        };
        self.user_data += 1;
        unsafe {
            self.ring.submission().push(&accept)?;
        }
        Ok(())
    }

    fn receive(&mut self, mut receive_params: ReceiveParams) -> Result<(), IouError> {
        receive_params.buf.extend_from_slice(&[0; BUF_SIZE]);
        receive_params.curr_chunk += 1;
        self.events
            .insert(self.user_data, EventType::Recv(receive_params));

        let receive = match self.events.get_mut(&self.user_data).unwrap() {
            EventType::Recv(ref mut params) => {
                let start = (params.curr_chunk - 1) * BUF_SIZE;
                let end = start + BUF_SIZE;
                let data_buf = params.buf[start..end].as_mut_ptr();
                opcode::Recv::new(params.fd, data_buf, BUF_SIZE as u32)
                    .flags(libc::MSG_WAITALL)
                    .build()
                    .user_data(self.user_data)
            }
            _ => panic!("Unreachable code"),
        };
        self.user_data += 1;
        unsafe { self.ring.submission().push(&receive)? }
        Ok(())
    }

    fn send(&mut self, mut send_params: SendParams) -> Result<(), IouError> {
        self.events
            .insert(self.user_data, EventType::Send(send_params));

        let send = match self.events.get_mut(&self.user_data).unwrap() {
            EventType::Send(ref mut params) => {
                opcode::Send::new(params.fd, params.buf.as_mut_ptr(), params.buf.len() as u32)
                    .build()
                    .user_data(self.user_data)
            }
            _ => panic!("unreachable code"),
        };

        self.user_data += 1;

        unsafe { self.ring.submission().push(&send)? }
        Ok(())
    }
}
