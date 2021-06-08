mod server;
mod executor;
mod reactor;
mod accept_future;

use clap::{App, Arg};
use pretty_env_logger;
use executor::new_executor_and_spawner;
use std::thread;
use std::sync::{Mutex, Arc};

use accept_future::AcceptFuture;

fn main() {
    pretty_env_logger::init();

    let (mut reactor, sender) = reactor::Reactor::new().unwrap();

    thread::spawn(move || {
        reactor.run().unwrap();
    });

    let (executor, spawner) = new_executor_and_spawner();
    thread::spawn(move || {
        executor.run();
    });

    spawner.spawn(async {
        println!("Creating accept future");
        let result = AcceptFuture::new(sender, "0.0.0.0:8888").await.unwrap();
        println!("got result {}", result);
    });

    let done: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let done_clone = done.clone();

    loop {
        if *done.lock().unwrap() {
            break;
        }
    }
    // println!("Address: {:p}", &address);
}
