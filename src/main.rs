mod executor;
mod reactor;
mod runtime;
mod server;
mod syscall;

use http::{Request, Response, StatusCode};
use pretty_env_logger;
// use runtime::Runtime;
use server::HttpServer;

fn main() {
    pretty_env_logger::init();

    let handler = |_request: Request<&[u8]>| async {
        Response::builder()
            .status(StatusCode::OK)
            .body("hello world".as_bytes().to_vec())
            .unwrap()
    };
    let server = HttpServer::bind("0.0.0.0:8889").expect("bind");
    server.run_on_threads(8, handler);

    // let mut runtime = Runtime::new();
    // runtime.run(server.serve(handler));
}
