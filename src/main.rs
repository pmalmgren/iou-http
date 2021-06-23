mod executor;
mod reactor;
mod runtime;
mod server;
mod syscall;

use http::{Response, StatusCode};
use pretty_env_logger;
use log::debug;
use runtime::{spawn, Runtime};
use std::net::{TcpListener, TcpStream};
use std::os::unix::io::FromRawFd;
use syscall::{Accept, Close, Recv, Send};
use httparse::{Error as HttpParseError, Request, EMPTY_HEADER};

fn complete_http_request(buf: &[u8]) -> bool {
    // a complete HTTP request is of the form
    // VERB /PATH HTTP/VERSION\r\nHeader: value...\r\n
    let iter = buf.windows(4);

    for slice in iter {
        if slice == b"\r\n\r\n" {
            return true;
        }
    }
    false
}

fn serialize_response(response: http::Response<String>) -> Vec<u8> {
    let (parts, body) = response.into_parts();
    // TODO serialize the HTTP response with less copying
    let mut response = format!("HTTP/1.1 {}\r\n", parts.status);
    let mut has_content_length: bool = false;
    for (name, value) in parts.headers.iter() {
        if let Ok(value) = value.to_str() {
            has_content_length =
                value.to_ascii_lowercase() == "content-length";
            response
                .push_str(format!("{}: {}\r\n", name, value).as_str());
        }
    }
    if !has_content_length {
        response.push_str(
            format!("Content-Length: {}\r\n", body.len()).as_str(),
        );
    }
    response.push_str("\r\n");
    response.push_str(body.as_str());

    response.into_bytes()
}

fn main() {
    pretty_env_logger::init();
    let mut runtime = Runtime::new();
    
    let handler = |request: Request| -> Response<String> {
        Response::builder()
            .status(StatusCode::OK)
            .body("hello world".to_string())
            .unwrap()
    };

    // Accept loop
    runtime.block_on(async move {
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

                    let mut headers = [EMPTY_HEADER; 64];
                    let mut request = Request::new(&mut headers);
                    if complete_http_request(&buf) {
                        request.parse(&buf).unwrap();
                        // TODO pass an http::Request to the handler instead of httparse::Request
                        let response = handler(request);
                        let mut response_buf = serialize_response(response);
                        Send::submit(&mut response_buf, &mut stream).await.unwrap();
                    } else {
                        panic!("request is too long");
                    }
                    // TODO handle requests that are larger than 512 bytes
                }
            })
        }
    });
}
