mod server;

use server::Server;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: server host:port");
    }
    // loop through open sockets
    // allocate memory for reading
    // for each one, submit a receive SQE
    // during loop through completion queue

    let mut server = Server::bind(args[1].clone()).unwrap();
    server.run().unwrap();
}
