use crate::runtime::spawn;
use crate::syscall::{Accept, Close, Recv, Send};
use http::Response;
use httparse::{Request, Status, EMPTY_HEADER};
use log::{error, trace};
use std::io::Error;
use std::net::{TcpListener, TcpStream};
use std::os::unix::io::FromRawFd;
use std::str;
use std::sync::Arc;

fn serialize_response(response: http::Response<String>) -> Vec<u8> {
    let (parts, body) = response.into_parts();
    // TODO serialize the HTTP response with less copying
    let mut response = format!("HTTP/1.1 {}\r\n", parts.status);
    let mut has_content_length: bool = false;
    for (name, value) in parts.headers.iter() {
        if let Ok(value) = value.to_str() {
            has_content_length = value.to_ascii_lowercase() == "content-length";
            response.push_str(format!("{}: {}\r\n", name, value).as_str());
        }
    }
    if !has_content_length {
        response.push_str(format!("Content-Length: {}\r\n", body.len()).as_str());
    }
    response.push_str("\r\n");
    response.push_str(body.as_str());

    response.into_bytes()
}

const BUF_SIZE: usize = 512;

pub struct HttpServer {
    socket: TcpListener,
}

impl HttpServer {
    pub fn bind(addr: &str) -> Result<HttpServer, Error> {
        let socket = TcpListener::bind(addr)?;
        Ok(HttpServer { socket })
    }

    pub async fn serve<H>(self, handler: H)
    where
        H: (Fn(Request, Option<&[u8]>) -> Response<String>) + 'static + std::marker::Send + Sync,
    {
        // Handler is wrapped in an Arc so it can be cloned each time a task is spawned
        let handler = Arc::new(handler);

        // Accept loop
        loop {
            let fd = Accept::submit(&self.socket).await.unwrap();
            let mut stream = unsafe { TcpStream::from_raw_fd(fd as i32) };

            let handler_clone = handler.clone();

            spawn(async move {
                let mut buf: Vec<u8> = Vec::with_capacity(512);
                let mut curr_chunk = 0;

                loop {
                    // Make sure the buf has enough space for the next chunk to be read
                    buf.extend_from_slice(&[0; BUF_SIZE]);

                    // Receive the next chunk
                    let start = curr_chunk * BUF_SIZE;
                    let end = start + BUF_SIZE;
                    // TODO read more than 512 bytes at a time
                    let bytes_received = Recv::submit(&mut buf[start..end], &mut stream)
                        .await
                        .unwrap();
                    curr_chunk += 1;

                    if bytes_received == 0 {
                        Close::submit(stream).await.unwrap();
                        break;
                    }

                    let mut headers = [EMPTY_HEADER; 24];
                    let mut request = Request::new(&mut headers);

                    // If we've gotten a complete request, call the handler with it
                    // and return the response. If not, call recv again
                    match request.parse(&buf) {
                        Ok(Status::Complete(body_start)) => {
                            // Find and parse the Content-Length header
                            let content_length: Option<usize> = request
                                .headers
                                .iter()
                                .find(|header| header.name.eq_ignore_ascii_case("content-length"))
                                .and_then(|header| str::from_utf8(header.value).ok())
                                .and_then(|s| s.parse::<usize>().ok());

                            // Use the Content-Length header to determine whether we've received the full request body
                            let (request, body) = match content_length {
                                Some(content_length)
                                    if body_start + content_length <= buf.len() =>
                                {
                                    trace!("Got request with content-length: {}", content_length);
                                    (request, Some(&buf[body_start..body_start + content_length]))
                                }
                                Some(content_length) => {
                                    // We haven't read the whole body yet
                                    trace!("Request has content-length ({}) but we have only read {} bytes from the body so far", content_length, buf.len() - body_start);
                                    continue;
                                }
                                None => {
                                    // TODO should we error or figure out the length of the body
                                    // some other way if no Content-Length header is found
                                    trace!("Got request with no content-length header. Ignoring request body");
                                    (request, None)
                                }
                            };

                            // TODO pass http::Request to handler instead of httparse::Request
                            let response = (handler_clone)(request, body);
                            let mut response_buf = serialize_response(response);
                            Send::submit(&mut response_buf, &mut stream).await.unwrap();
                            break;
                        }
                        Ok(Status::Partial) => {
                            trace!("Got partial HTTP request");
                            continue;
                        }
                        Err(err) => {
                            error!("Invalid HTTP request: {:?}", err);
                            let response = Response::builder()
                                .status(400)
                                .body("Invalid HTTP Request".to_string())
                                .unwrap();
                            Send::submit(&mut serialize_response(response), &mut stream)
                                .await
                                .unwrap();
                            break;
                        }
                    }
                }
            })
        }
    }
}
