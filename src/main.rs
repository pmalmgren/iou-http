mod accept_future;
mod executor;
mod reactor;
mod server;

use clap::{App, Arg};
use executor::new_executor_and_spawner;
use pretty_env_logger;
use std::sync::{Arc, Mutex};
use std::thread;

use accept_future::AcceptFuture;

fn main() {
    pretty_env_logger::init();

    let (mut reactor, mut sender) = reactor::Reactor::new().unwrap();

    let (mut executor, spawner) = new_executor_and_spawner();

    spawner.spawn(async {
        println!("Creating accept future");
        let result = AcceptFuture::new(sender, "0.0.0.0:8888").await.unwrap();
        println!("got result {}", result);
    });

    drop(spawner);

    // TODO how do we know when to finish?
    while executor.tick() {
        reactor.tick().unwrap();
    }
    // println!("Address: {:p}", &address);
}
