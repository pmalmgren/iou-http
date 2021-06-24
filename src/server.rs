use crate::runtime::{spawn, Runtime};
use crate::syscall::{Accept, Close, Recv, Send};
use futures::{channel::mpsc::unbounded, future::Future, SinkExt, StreamExt};
use http::{Request, Response, Version};
use httparse::{Request as ParseRequest, Status, EMPTY_HEADER};
use log::{debug, error, trace};
use std::io::Error;
use std::net::{TcpListener, TcpStream};
use std::os::unix::io::FromRawFd;
use std::str;
use std::sync::Arc;
use std::thread::spawn as spawn_thread;

const BUF_SIZE: usize = 512;

fn serialize_response(response: http::Response<Vec<u8>>) -> Vec<u8> {
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

    let mut response = response.into_bytes();
    response.extend_from_slice(&body);

    response
}

fn convert_http_request<T>(request: ParseRequest, body: T) -> Request<T> {
    let builder = Request::builder()
        .method(request.method.unwrap())
        .uri(request.path.unwrap())
        .version(Version::HTTP_11);
    let builder = request.headers.iter().fold(builder, |builder, header| {
        builder.header(header.name, header.value)
    });
    builder.body(body).unwrap()
}

pub struct HttpServer {
    socket: TcpListener,
}

impl HttpServer {
    pub fn bind(addr: &str) -> Result<HttpServer, Error> {
        let socket = TcpListener::bind(addr)?;
        Ok(HttpServer { socket })
    }

    pub async fn serve<H, R>(self, handler: H)
    where
        H: (Fn(Request<&[u8]>) -> R) + 'static + std::marker::Send + Sync,
        R: Future<Output = Response<Vec<u8>>> + 'static + std::marker::Send,
    {
        // Handler is wrapped in an Arc so it can be cloned each time a task is spawned
        let handler = Arc::new(handler);

        // Accept loop
        loop {
            // TODO have more than 1 accept call in flight
            let fd = Accept::submit(&self.socket).await.unwrap();
            let stream = unsafe { TcpStream::from_raw_fd(fd as i32) };

            let handler_clone = handler.clone();

            spawn(HttpServer::handle_http_requests(stream, handler_clone));
        }
    }

    pub fn run_on_threads<H, R>(self, threads: usize, handler: H)
    where
        H: (Fn(Request<&[u8]>) -> R) + 'static + std::marker::Send + Sync,
        R: Future<Output = Response<Vec<u8>>> + 'static + std::marker::Send,
    {
        // Handler is wrapped in an Arc so it can be cloned each time a task is spawned
        let handler = Arc::new(handler);

        let num_workers = threads - 1;
        let mut channels = Vec::new();
        debug!("spawning {} worker threads", num_workers);

        // Spawn worker threads
        for worker in 0..num_workers {
            let handler = handler.clone();
            let (sender, mut receiver) = unbounded::<TcpStream>();
            channels.push(sender);

            // TODO should we do something with the JoinHandle returned by this spawn call?
            spawn_thread(move || {
                let mut runtime = Runtime::new();
                runtime.block_on(async move {
                    // The thread should shutdown when the channel
                    // from the main thread is closed
                    while let Some(stream) = receiver.next().await {
                        trace!("worker {} got stream", worker);
                        spawn(HttpServer::handle_http_requests(stream, handler.clone()));
                    }
                });
                let _runtime = runtime;
                trace!("worker {} shutting down", worker);
            });
        }

        // Set up accept loop
        let mut runtime = Runtime::new();

        runtime.block_on(async move {
            let mut next_worker: usize = 0;

            // Accept loop
            loop {
                // TODO have more than 1 accept call in flight
                let fd = Accept::submit(&self.socket).await.unwrap();
                let stream = unsafe { TcpStream::from_raw_fd(fd as i32) };

                // Send streams to workers in round-robin fashion
                // TODO determine which workers are busy before sending
                channels[next_worker]
                    .send(stream)
                    .await
                    .expect(&format!("error sending stream to worker {}", next_worker));
                next_worker = (next_worker + 1) % num_workers;
            }
        });
    }

    async fn handle_http_requests<H, R>(mut stream: TcpStream, handler: Arc<H>)
    where
        H: (Fn(Request<&[u8]>) -> R) + 'static + std::marker::Send + Sync,
        R: Future<Output = Response<Vec<u8>>> + 'static + std::marker::Send,
    {
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

            // TODO what if there are more than this number of headers?
            let mut headers = [EMPTY_HEADER; 24];
            let mut request = ParseRequest::new(&mut headers);

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
                        Some(content_length) if body_start + content_length <= buf.len() => {
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
                            trace!(
                                "Got request with no content-length header. Ignoring request body"
                            );
                            (request, None)
                        }
                    };

                    let request = convert_http_request(request, body.unwrap_or(&[]));
                    let response = (handler)(request).await;
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
                        .body("Invalid HTTP Request".as_bytes().to_vec())
                        .unwrap();
                    Send::submit(&mut serialize_response(response), &mut stream)
                        .await
                        .unwrap();
                    break;
                }
            }
        }
    }
}
