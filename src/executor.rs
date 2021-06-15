use log::debug;
use {
    futures::{
        future::{BoxFuture, FutureExt},
        task::{waker_ref, ArcWake},
    },
    std::{
        future::Future,
        sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError},
        sync::{Arc, Mutex},
        task::{Context, Poll},
        time::Duration,
    },
    // The timer we wrote in the previous section:
};

/// Task executor that receives tasks off of a channel and runs them.
pub(crate) struct Executor {
    ready_queue: Receiver<Arc<Task>>,
}

impl Executor {
    // Returns true if the executor has more work to do
    pub fn tick(&self) -> bool {
        loop {
            let result = self.ready_queue.try_recv();
            match result {
                Ok(task) => {
                    // Take the future, and if it has not yet completed (is still Some),
                    // poll it in an attempt to complete it.
                    let mut future_slot = task.future.lock().unwrap();
                    if let Some(mut future) = future_slot.take() {
                        // Create a `LocalWaker` from the task itself
                        let waker = waker_ref(&task);
                        let context = &mut Context::from_waker(&*waker);
                        // `BoxFuture<T>` is a type alias for
                        // `Pin<Box<dyn Future<Output = T> + Send + 'static>>`.
                        // We can get a `Pin<&mut dyn Future + Send + 'static>`
                        // from it by calling the `Pin::as_mut` method.
                        debug!("Polling future");
                        if let Poll::Pending = future.as_mut().poll(context) {
                            // We're not done processing the future, so put it
                            // back in its task to be run again in the future.
                            *future_slot = Some(future);
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => return true,
            }
        }
    }
}

/// `Spawner` spawns new futures onto the task channel.
#[derive(Clone)]
pub struct Spawner {
    task_sender: SyncSender<Arc<Task>>,
}

impl Spawner {
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static + Send) {
        debug!("Spawning future");
        let future = future.boxed();
        let task = Arc::new(Task {
            future: Mutex::new(Some(future)),
            task_sender: self.task_sender.clone(),
        });
        self.task_sender.send(task).expect("too many tasks queued");
    }
}

/// A future that can reschedule itself to be polled by an `Executor`.
struct Task {
    /// In-progress future that should be pushed to completion.
    ///
    /// The `Mutex` is not necessary for correctness, since we only have
    /// one thread executing tasks at once. However, Rust isn't smart
    /// enough to know that `future` is only mutated from one thread,
    /// so we need to use the `Mutex` to prove thread-safety. A production
    /// executor would not need this, and could use `UnsafeCell` instead.
    future: Mutex<Option<BoxFuture<'static, ()>>>,

    /// Handle to place the task itself back onto the task queue.
    task_sender: SyncSender<Arc<Task>>,
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        // Implement `wake` by sending this task back onto the task channel
        // so that it will be polled again by the executor.
        let cloned = arc_self.clone();
        arc_self
            .task_sender
            .send(cloned)
            .expect("too many tasks queued");
    }
}

pub(crate) fn new_executor_and_spawner() -> (Executor, Spawner) {
    // Maximum number of tasks to allow queueing in the channel at once.
    // This is just to make `sync_channel` happy, and wouldn't be present in
    // a real executor.
    const MAX_QUEUED_TASKS: usize = 10_000;
    let (task_sender, ready_queue) = sync_channel(MAX_QUEUED_TASKS);
    (Executor { ready_queue }, Spawner { task_sender })
}

/*
   Finished dev [unoptimized + debuginfo] target(s) in 0.04s
     Running `target/debug/iou-httpserver`
 DEBUG iou_httpserver::executor > Spawning future
 DEBUG iou_httpserver::executor > Spawning future
 DEBUG iou_httpserver::executor > Polling future
Creating accept future
4
 DEBUG iou_httpserver::accept_future > Submitting accept
 DEBUG iou_httpserver::executor      > Polling future
5
 DEBUG iou_httpserver::accept_future > Submitting accept
 TRACE iou_httpserver::runtime       > tick
 DEBUG iou_httpserver::reactor       > Submitting entry 0 to io uring
 DEBUG iou_httpserver::reactor       > Submitting entry 1 to io uring
 TRACE iou_httpserver::reactor       > Reactor has 2 events in flight
 DEBUG iou_httpserver::reactor       > Consumed 1 entries in 1 tick
 DEBUG iou_httpserver::reactor       > Got completion for entry 0: 6
 DEBUG iou_httpserver::accept_future > Accept result: 6
 DEBUG iou_httpserver::executor      > Polling future
got result 6
 TRACE iou_httpserver::runtime       > tick
 TRACE iou_httpserver::reactor       > Reactor has 1 events in flight
 DEBUG iou_httpserver::reactor       > Consumed 1 entries in 1 tick
 DEBUG iou_httpserver::reactor       > Got completion for entry 1: -22
 DEBUG iou_httpserver::accept_future > Accept result: -22
 DEBUG iou_httpserver::executor      > Polling future
thread 'main' panicked at 'called `Result::unwrap()` on an `Err` value: Os { code: 22, kind: InvalidInput, message: "Invalid argument" }', src/main.rs:23:63
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
*/