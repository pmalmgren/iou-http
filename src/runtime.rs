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
	// TODO do these need to be stored on the Runtime?
	// will the executor exit properly if they're not?
	spawner: Option<Spawner>,
	reactor_sender: ReactorSender,
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
			reactor_sender,
			spawner: Some(spawner)
		}
	}

	pub fn run(&mut self) {
		// Drop the spawner so that the executor exits when there are no more
		// references to the spawner
		self.spawner.take();

		while self.executor.tick() {
			trace!("tick");
			self.reactor.tick().unwrap();
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
