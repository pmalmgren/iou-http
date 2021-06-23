mod executor;
mod reactor;
mod runtime;
mod server;
mod syscall;

use pretty_env_logger;
use runtime::{spawn, Runtime};
use std::net::{TcpListener, TcpStream};
use std::os::unix::io::FromRawFd;
use syscall::{Accept, Close, Recv, Send};

fn main() {
    pretty_env_logger::init();
    let mut runtime = Runtime::new();

    // Accept loop
    runtime.block_on(async {
        let socket = TcpListener::bind("0.0.0.0:8889").expect("bind");
        loop {
            let fd = Accept::submit(&socket).await.unwrap();
            let mut stream = unsafe { TcpStream::from_raw_fd(fd as i32) };

            spawn(async move {
                let mut buf = [0u8; 512];

                loop {
                    let bytes_received = Recv::submit(&mut buf, &mut stream).await.unwrap();

                    if bytes_received == 0 {
                        Close::submit(stream).await.unwrap();
                        break;
                    }

                    Send::submit(&mut buf, &mut stream).await.unwrap();
                }
            })
        }
    });
}
