mod server;

use clap::{App, Arg};
use pretty_env_logger;
use server::Server;

use http::{Response, StatusCode};

fn main() {
    pretty_env_logger::init();

    let matches = App::new("iou-http")
        .arg(
            Arg::with_name("address")
                .help("Address and port to bind to")
                .default_value("localhost:8888")
                .index(1)
                .takes_value(true),
        )
        .get_matches();
    let address = matches
        .value_of("address")
        .expect("Bind address is required");

    let mut server = Server::new(address, |_request| {
        let body = "hello world".to_string();
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Length", body.len())
            .body(body)
            .unwrap()
    }).unwrap();
    server.run().unwrap();
}
