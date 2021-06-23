use io_uring::{opcode, types::Fd};
use std::marker::PhantomData;
use std::net::TcpListener;
use std::os::unix::io::AsRawFd;
use crate::syscall::SysCall;

// TODO (v2, after adding in other operations): Operation state

pub struct Accept<'a> {
    // If this is dropped, the file descriptor will be freed
    socket: PhantomData<&'a TcpListener>
}

impl<'a> Accept<'a> {
    // TODO does this lifetime need to be static?
    pub fn submit(socket: &'a TcpListener) -> SysCall<Accept<'a>> {
        let entry =
            opcode::Accept::new(Fd(socket.as_raw_fd()), std::ptr::null_mut(), std::ptr::null_mut())
                .build();
        let future = Accept{
            socket: PhantomData
        };
        SysCall::from_entry(entry, future)
    }
}
