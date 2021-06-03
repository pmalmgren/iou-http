mod server;
mod executor;
mod reactor;

use clap::{App, Arg};
use pretty_env_logger;
use reactor::Reactor;
use std::thread;
use std::sync::{Mutex, Arc};
use std::net::TcpListener;
use io_uring::{squeue::Entry, opcode, types::Fd};
use std::os::unix::io::{AsRawFd, RawFd};
use libc;

use http::{Response, StatusCode};

fn main() {
    pretty_env_logger::init();

    let (mut reactor, sender) = reactor::Reactor::new().unwrap();

    thread::spawn(move || {
        reactor.run().unwrap();
    });

    let mut address = libc::sockaddr {
        sa_family: 0,
        sa_data: [0 as libc::c_char; 14],
    };
    let mut address_length: libc::socklen_t = std::mem::size_of::<libc::sockaddr>() as _;
    println!("Address: {:p}", &address);
    let socket = TcpListener::bind("0.0.0.0:8888").expect("bind");
    let raw_fd = socket.as_raw_fd();
    let entry = opcode::Accept::new(Fd(raw_fd), &mut address, &mut address_length)
        .build();

    let done: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let done_clone = done.clone();
    sender.send((entry, Box::new(move |n| {
        println!("Received return {}", n);
        let mut done = done_clone.lock().unwrap();
        *done = true;
    }))).unwrap();

    loop {
        if *done.lock().unwrap() {
            break;
        }
    }
    println!("Address: {:p}", &address);
}
