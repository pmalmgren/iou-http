TODO

- [ ] Parse HTTP headers
- [ ] Parse HTTP body (optional for hello world)
- [ ] Call handler, write response back to the socket
- [ ] Add tests for receiving data
- [ ] Implement an HTTP request handler, using [httparse](https://docs.rs/httparse/1.4.1/httparse/)

Later
- [ ] Implement an executor

```rust

// std::futures::Future<Output: HttpResponse>
async fn handler(request: HttpRequest) -> HttpResponse {
    HttpResponse {
        body: "Hello world".to_string()
    }
}

fn main() {
    let server = iou_http::Server::new();
    // fn(HttpRequest) -> HttpResponse
    server.handler = handler;

    server.run();
}

impl Server {
    fn run() {
        // poll completion queue
            // on a new request:
                // read the full request
                // parse the request
                // call handler, put the handler future into the futures vec
        // poll pending futures
            // when the response is ready, send it on the socket and remove future from the future vec

        
    }
}

```
