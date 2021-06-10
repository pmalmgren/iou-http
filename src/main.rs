mod accept_future;
mod executor;
mod reactor;
mod server;
mod runtime;

use pretty_env_logger;
use runtime::Runtime;

use accept_future::AcceptFuture;

fn main() {
    pretty_env_logger::init();
    let mut runtime = Runtime::new();

    runtime.spawn(async {
        println!("Creating accept future");
        let result = AcceptFuture::new( "0.0.0.0:8888").await.unwrap();
        println!("got result {}", result);
    });

    runtime.block_on(async {
        let result2 = AcceptFuture::new("0.0.0.0:8889").await.unwrap();
        println!("got result {}", result2);
    });
}
