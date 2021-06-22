mod accept_future;
mod executor;
mod reactor;
mod server;
mod runtime;
mod syscall;
mod recv_future;
mod send_future;

use pretty_env_logger;
use runtime::Runtime; 
use std::net::{TcpListener, TcpStream};
use std::os::unix::io::FromRawFd;

use accept_future::AcceptFuture;
use recv_future::RecvFuture;
use send_future::SendFuture;

fn main() {
    pretty_env_logger::init();
    let mut runtime = Runtime::new();

    runtime.spawn(async {
        println!("Creating accept future");
        let socket = TcpListener::bind("0.0.0.0:8888").expect("bind");
        let fd = AcceptFuture::submit(&socket).await.unwrap();
        let mut stream = unsafe {TcpStream::from_raw_fd(fd as i32) };
        // TODO send it a vec instead of a slice
        let mut buf = [0u8; 512];
        println!("but capacity in main is {}", buf.len());
        let bytes_received = RecvFuture::submit(&mut buf, &mut stream).await.unwrap();
        println!("received {} bytes: {:?}", bytes_received, std::str::from_utf8(&buf).unwrap());
        let bytes_sent = SendFuture::submit(&mut buf, &mut stream).await.unwrap();
        println!("sent {} bytes: {:?}", bytes_sent, std::str::from_utf8(&buf).unwrap());

        // TODO use lifetimes to make sure the socket lives long enough
        let _l = socket;
    });

    runtime.block_on(async {
        let socket = TcpListener::bind("0.0.0.0:8889").expect("bind");
        let fd = AcceptFuture::submit(&socket).await.unwrap();
        let mut stream = unsafe {TcpStream::from_raw_fd(fd as i32) };
        let mut buf = [0u8; 512];
        let bytes_received = RecvFuture::submit(&mut buf, &mut stream).await.unwrap();
        println!("received {} bytes: {:?}", bytes_received, std::str::from_utf8(&buf).unwrap());
        let bytes_sent = SendFuture::submit(&mut buf, &mut stream).await.unwrap();
        println!("sent {} bytes: {:?}", bytes_sent, std::str::from_utf8(&buf).unwrap());

        let _l = socket;
    });
}
