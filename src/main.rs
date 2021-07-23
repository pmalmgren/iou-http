mod executor;
mod http_server;
mod reactor;
mod runtime;
mod syscall;

use http::{Request, Response, StatusCode};
use http_server::HttpServer;
#[allow(unused_imports)]
use runtime::Runtime;

fn main() {
    tracing_subscriber::fmt::init();

    let handler = |_request: Request<&[u8]>| async {
        Response::builder()
            .status(StatusCode::OK)
            .body("hello world".as_bytes().to_vec())
            .unwrap()
    };

    let server = HttpServer::bind("0.0.0.0:3000").expect("bind");

    // This runs the server on multiple threads
    server.run_on_threads(8, handler);

    // This runs the server on the current thread only
    // let mut runtime = Runtime::new();
    // runtime.run(server.serve(handler));
}
