
use io_uring::{opcode, types::Fd};
use std::mem;
use std::net::TcpStream;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex};
use std::marker::PhantomData;

use crate::syscall::{SysCall, Lifecycle};

pub struct RecvFuture<'a> {
    stream: PhantomData<&'a TcpStream>,
}

impl<'a> RecvFuture<'a> {
    pub fn submit(buf: &'a mut [u8], stream: &'a mut TcpStream) -> SysCall<RecvFuture<'a>> {
        let raw_fd = stream.as_raw_fd();
        let entry =
            opcode::Recv::new(Fd(raw_fd), buf.as_mut_ptr(), buf.len() as u32)
            .build();
        let future = RecvFuture {
            stream: PhantomData
        };
        SysCall::from_entry(entry, future)
    }
}

