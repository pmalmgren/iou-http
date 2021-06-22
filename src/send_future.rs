use io_uring::{opcode, types::Fd};
use std::net::TcpStream;
use std::os::unix::io::AsRawFd;
use std::marker::PhantomData;

use crate::syscall::SysCall;

pub struct SendFuture<'a> {
    stream: PhantomData<&'a TcpStream>,
}

impl<'a> SendFuture<'a> {
    pub fn submit(buf: &'a mut [u8], stream: &'a mut TcpStream) -> SysCall<SendFuture<'a>> {
        let raw_fd = stream.as_raw_fd();
        let entry =
            opcode::Send::new(Fd(raw_fd), buf.as_mut_ptr(), buf.len() as u32)
            .build();
        let future = SendFuture {
            stream: PhantomData
        };
        SysCall::from_entry(entry, future)
    }
}
