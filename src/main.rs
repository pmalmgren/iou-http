mod executor;
mod reactor;
mod runtime;
mod server;
mod syscall;

use http::{Response, StatusCode};
use httparse::Request;
use pretty_env_logger;
use runtime::Runtime;
use server::HttpServer;

fn main() {
    pretty_env_logger::init();
    let mut runtime = Runtime::new();

    let handler = |_request: Request, _body: Option<&[u8]>| -> Response<String> {
        Response::builder()
            .status(StatusCode::OK)
            .body("hello world".to_string())
            .unwrap()
    };
    let server = HttpServer::bind("0.0.0.0:8889").expect("bind");
    runtime.block_on(
        server.serve(handler)
    );
}
