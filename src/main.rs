use io_uring::{opcode, squeue::{PushError, Entry}, types, IoUring};
use std::collections::HashMap;
use std::env;
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
    Send,
}

struct AcceptParams {
    address: libc::sockaddr,
    address_length: libc::socklen_t,
}

struct ReceiveParams {
    buf: [u8; 512],
    fd: i32,
}

struct SocketParams {
    fd: i32,
    address: SockAddr,
}

impl SocketParams {
    fn new_from_accept_params(accept_params: AcceptParams, fd: i32) -> Result<Self, IouError> {
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

        Ok(SocketParams { fd, address })
    }
}

struct Server {
    ring: IoUring,
    user_data: u64,
    events: HashMap<u64, EventType>,
    socket: net::TcpListener,
    raw_fd: RawFd,
}

impl Server {
    fn bind<A: ToSocketAddrs>(addr: A) -> Result<Server, IouError> {
        let socket = net::TcpListener::bind(addr)?;
        let raw_fd = socket.as_raw_fd();
        let ring = IoUring::new(8)?;
        let user_data = 1u64;
        let events: HashMap<u64, EventType> = HashMap::new();

        Ok(Server {
            socket,
            raw_fd,
            events,
            ring,
            user_data,
        })
    }

    fn run(&mut self) -> Result<(), IouError> {
        self.accept()?;
        loop {
            match self.ring.submitter().submit_and_wait(1) {
                Err(e) => {
                    eprintln!("Error submitting accept: {:?}", e);
                    continue;
                }
                Ok(_) => {}
            };

            let mut should_accept = false;
            let mut read_fds: Vec<i32> = Vec::new();
            for cqe in self.ring.completion() {
                let ret = cqe.result();
                let user_data = cqe.user_data();

                if let Some(event) = self.events.remove(&user_data) {
                    match event {
                        EventType::Accept(accept) => {
                            // create socket struct
                            if ret < 0 {
                                return Err(io::Error::from_raw_os_error(-ret).into());
                            }
                            let socket = SocketParams::new_from_accept_params(accept, ret);
                            match socket {
                                Ok(ref sock) => {
                                    println!("Received connection from {:?}", sock.address);
                                    read_fds.push(sock.fd);
                                }
                                Err(e) => {
                                    println!("Error accepting connection {:?}", e);
                                }
                            };

                            should_accept = true;
                        }
                        EventType::Recv(receive) => {
                            if ret > 0 {
                                println!("received data {:?}", receive.buf);
                                read_fds.push(receive.fd);
                            } else {
                                // TODO handle this error
                                eprintln!("receive error {}", io::Error::from_raw_os_error(-ret));;
                            }
                        }
                        EventType::Send => {}
                    }
                } else {
                    eprintln!(
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
        }
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
            EventType::Accept(ref mut params) => opcode::Accept::new(
                types::Fd(self.raw_fd),
                &mut params.address,
                &mut params.address_length,
            )
            .build()
            .user_data(self.user_data),
            _ => panic!("This should not happen"),
        };
        self.user_data += 1;
        unsafe { self.ring.submission().push(&accept)?; }
        Ok(())
    }

    fn receive(&mut self, fd: i32) -> Result<(), IouError> {
        let mut buf = [0; 512];
        let receive = opcode::Recv::new(types::Fd(fd), buf.as_mut_ptr(), buf.len() as u32)
            .flags(libc::MSG_WAITALL)
            .build()
            .user_data(self.user_data);
        self.events.insert(self.user_data, EventType::Recv(ReceiveParams {
            buf,
            fd
        }));
        self.user_data += 1;
        unsafe {
            self.ring.submission().push(&receive)?
        }
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: server host:port");
    }
    // loop through open sockets
    // allocate memory for reading
    // for each one, submit a receive SQE
    // during loop through completion queue

    let mut server = Server::bind(args[1].clone()).unwrap();
    server.run().unwrap();
}
