use io_uring::{opcode, types::Fd};
use std::mem;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::marker::PhantomData;
use std::os::unix::io::AsRawFd;

use crate::syscall::{SysCall, Lifecycle};
pub struct Recv<'a> {
    stream: PhantomData<&'a TcpStream>,
}

impl<'a> Recv<'a> {
    pub fn submit(buf: &'a mut [u8], stream: &'a mut TcpStream) -> SysCall<Recv<'a>> {
        let raw_fd = stream.as_raw_fd();
        let entry =
            opcode::Recv::new(Fd(raw_fd), buf.as_mut_ptr(), buf.len() as u32)
            .build();
        let future = Recv {
            stream: PhantomData
        };
        SysCall::from_entry(entry, future)
    }
}