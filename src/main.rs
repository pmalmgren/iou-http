use io_uring::{opcode, types, IoUring};
use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
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
}

enum EventType {
    Accept(AcceptParams),
    Recv,
    Send,
}

struct AcceptParams {
    address: libc::sockaddr,
    address_length: libc::socklen_t,
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

struct Socket {
    addr: SockAddr,
    fd: i32,
}

fn main() {
    let mut ring = IoUring::new(8).unwrap();
    let mut user_data = 1u64;
    let mut events: HashMap<u64, EventType> = HashMap::new();

    let sock = net::TcpListener::bind("0:8888").unwrap();
    let raw_sock = sock.as_raw_fd();

    loop {
        let address = libc::sockaddr {
            sa_family: 0,
            sa_data: [0 as libc::c_char; 14],
        };
        let address_length: libc::socklen_t = mem::size_of::<libc::sockaddr>() as _;
        let event = EventType::Accept(AcceptParams {
            address,
            address_length,
        });
        events.insert(user_data, event);
        let accept = match events.get_mut(&user_data).unwrap() {
            EventType::Accept(ref mut params) => {
                opcode::Accept::new(
                    types::Fd(raw_sock),
                    &mut params.address,
                    &mut params.address_length,
                )
                .build()
                .user_data(user_data)
            },
            _ => panic!("This should not happen")
        };
        user_data += 1;

        unsafe {
            match ring.submission().push(&accept) {
                Err(e) => {
                    eprintln!("Error submitting accept: {:?}", e);
                    continue;
                }
                Ok(_) => {}
            };
        }

        match ring.submitter().submit_and_wait(1) {
            Err(e) => {
                eprintln!("Error submitting accept: {:?}", e);
                continue;
            }
            Ok(_) => {}
        };

        for cqe in ring.completion() {
            let ret = cqe.result();
            let user_data = cqe.user_data();

            if let Some(event) = events.remove(&user_data) {
                match event {
                    EventType::Accept(accept) => {
                        // create socket struct
                        if ret < 0 {
                            eprintln!(
                                "user_data {:?} error {:?}",
                                user_data,
                                io::Error::from_raw_os_error(-ret)
                            );
                            continue;
                        }
                        let socket = SocketParams::new_from_accept_params(accept, ret);
                        match socket {
                            Ok(ref sock) => {
                                println!("Received connection from {:?}", sock.address);
                            }
                            Err(e) => {
                                println!("Error accepting connection {:?}", e);
                            }
                        };
                    }
                    EventType::Recv => {}
                    EventType::Send => {}
                }
            } else {
                eprintln!(
                    "got completion event from unknown submission: {}",
                    user_data
                );
            }
        }
    }
}
