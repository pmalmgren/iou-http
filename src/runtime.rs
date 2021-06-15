use std::cell::RefCell;
use io_uring::squeue::Entry;
use std::future::Future;
use crate::reactor::{Reactor, ReactorSender, Callback};
use crate::executor::{Executor, Spawner, new_executor_and_spawner};
use log::trace;
use std::thread_local;

// TODO should this be scoped thread local storage?
thread_local!(static RUNTIME: RefCell<Option<(Spawner, ReactorSender)>> = RefCell::new(None));

pub fn spawn(future: impl Future<Output = ()> + 'static + Send) {
	RUNTIME.with(move |handle| {
		match &*handle.borrow() {
			Some((spawner, _)) => spawner.spawn(future),
			None => panic!("cannot call spawn before creating a runtime")
		}
	})
}

pub(crate) fn register(entry: Entry, callback: Callback) {
	RUNTIME.with(move |handle| {
		match &*handle.borrow() {
			Some((_, reactor_sender)) => reactor_sender.send((entry, callback)).unwrap(),
			None => panic!("cannot call spawn before creating a runtime")
		}
	})
}

pub struct Runtime {
	executor: Executor,
	reactor: Reactor,
	spawner: Option<Spawner>,
}

impl Runtime {
	pub fn new() -> Runtime {
		let (reactor, reactor_sender) = Reactor::new().unwrap();
		let (executor, spawner) = new_executor_and_spawner();

		// TODO should the thread local be set when the Runtime is created or when a future is spawned?
		let spawner_clone = spawner.clone();
		let reactor_sender_clone = reactor_sender.clone();
		RUNTIME.with(move |handle| {
			handle.replace(Some((spawner_clone, reactor_sender_clone)));
		});

		Runtime {
			reactor,
			executor,
			spawner: Some(spawner)
		}
	}

	pub fn run(&mut self) {
		// Drop the spawner so that the executor exits when there are no more
		// references to the spawner
		drop(self.spawner.take());

		let mut processing = true;
		while processing {
			trace!("tick");
			// Check whether the executor or reactor did anything on this tick
			// If either one had work, that means we might still be going
			// but if neither did anything, that means there are no pending futures
			// and no IO tasks in flight so we should exit.
			let executor_processing = self.executor.tick();
			let reactor_processing = self.reactor.tick().unwrap();
			processing = executor_processing || reactor_processing;
		}
	}

	pub fn spawn(&mut self, future: impl Future<Output = ()> + 'static + Send) {
		spawn(future);
	}

	pub fn block_on(&mut self, future: impl Future<Output = ()> + 'static + Send) {
		self.spawn(future);
		self.run();
	}
}
