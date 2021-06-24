use tracing::trace;
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
        let mut processed_events = false;
        loop {
            let result = self.ready_queue.try_recv();
            match result {
                Ok(task) => {
                    processed_events = true;
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
                        trace!("polling future");
                        if let Poll::Pending = future.as_mut().poll(context) {
                            // We're not done processing the future, so put it
                            // back in its task to be run again in the future.
                            *future_slot = Some(future);
                            trace!("future is still pending");
                        } else {
                            trace!("future finished");
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    trace!("channel has no more connected senders");
                    return false;
                }
                Err(TryRecvError::Empty) => {
                    if processed_events {
                        trace!("executor polled futures");
                    } else {
                        trace!("executor did not poll any futures");
                    }
                    return processed_events;
                }
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
        trace!("spawning future");
        let future = future.boxed();
        let task = Arc::new(Task {
            future: Mutex::new(Some(future)),
            task_sender: self.task_sender.clone(),
        });
        self.task_sender
            .send(task)
            .expect("cannot spawn future because there is no receiver on the channel");
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
        trace!("waking task");
        arc_self
            .task_sender
            .send(cloned)
            .expect("cannot wake task because there is no receiver on the channel");
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
