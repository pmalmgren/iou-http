mod executor;
mod reactor;
mod runtime;
mod server;
mod syscall;

use http::{Request, Response, StatusCode};
use pretty_env_logger;
use runtime::Runtime;
use server::HttpServer;

fn main() {
    pretty_env_logger::init();
    let mut runtime = Runtime::new();

    let handler = |_request: Request<Vec<u8>>| async {
        Response::builder()
            .status(StatusCode::OK)
            .body("hello world".as_bytes().to_vec())
            .unwrap()
    };
    let server = HttpServer::bind("0.0.0.0:8889").expect("bind");
    runtime.block_on(server.serve(handler));
}
