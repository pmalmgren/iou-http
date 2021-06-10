mod accept_future;
mod executor;
mod reactor;
mod server;

use clap::{App, Arg};
use executor::new_executor_and_spawner;
use pretty_env_logger;
use std::sync::{Arc, Mutex};
use std::thread;
use log::trace;

use accept_future::AcceptFuture;

fn main() {
    pretty_env_logger::init();

    let (mut reactor, mut sender) = reactor::Reactor::new().unwrap();

    let (mut executor, spawner) = new_executor_and_spawner();

    let sender_clone = sender.clone();
    spawner.spawn(async {
        println!("Creating accept future");
        let result = AcceptFuture::new(sender, "0.0.0.0:8888").await.unwrap();
        println!("got result {}", result);
    });

    spawner.spawn(async {
        let result2 = AcceptFuture::new(sender_clone, "0.0.0.0:8889").await.unwrap();
        println!("got result {}", result2);
    });

    drop(spawner);

    while executor.tick() {
        trace!("tick");
        reactor.tick().unwrap();
    }
}
