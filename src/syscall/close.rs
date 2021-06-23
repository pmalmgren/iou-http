use io_uring::{opcode, types::Fd};
use std::net::TcpStream;
use std::os::unix::io::IntoRawFd;
use crate::syscall::SysCall;

pub struct Close{ }

impl Close{
    pub fn submit(socket: TcpStream) -> SysCall<Close> {
		// TODO do we need to make sure the socket isn't dropped
		// (because Rust will close the socket with a normal syscall
		// if the value is dropped)
        let entry =
            opcode::Close::new(Fd(socket.into_raw_fd()))
                .build();
        SysCall::from_entry(entry, Close{})
    }
}
