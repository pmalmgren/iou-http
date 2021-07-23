# iou-http

This is a toy async runtime and minimal HTTP server written in Rust as a project to learn about async runtimes and [io-uring](https://unixism.net/loti/what_is_io_uring.html).

The project was written by Peter Malmgren and Evan Schwartz while participating in the [Recurse Center's](https://recurse.com) programmer retreat.

## Design

The [Runtime](./src/runtime.rs) consists of the:
- [Executor](./src/executor.rs), which runs Futures from a queue of pending Tasks
- [Reactor](./src/reactor.rs), which submits I/O operations to the kernel using io-uring and reacts to completion events

io-uring uses two ring buffers shared between the userspace code and the kernel in order to submit I/O calls and handle the results. The Runtime is single-threaded because the ring buffers cannot be safely modified by multiple threads without wrapping them in a mutex, which would degrade performance.

While the runtime is single-threaded, the [HTTP Server](./src/http_server.rs) is multi-threaded. It uses one thread (with its own runtime and io-uring buffers) to accept incoming TCP connections and it uses the other threads (also with their own runtimes) to handle requests on those connections.

## Acknowledgements

This project takes inspiration from [`tokio-uring`'s design document](https://github.com/tokio-rs/tokio-uring/blob/design-doc/DESIGN.md) and the [Rust Async Book's](https://rust-lang.github.io/async-book/02_execution/01_chapter.html) chapter on building an executor. The project also uses the [`io-uring`](https://github.com/tokio-rs/io-uring) crate's Rust bindings for io-uring.
