use std::task::Waker;

pub(crate) enum Lifecycle {
    Submitted,
    Waiting(Waker),
    // TODO what do we do with this?
    // Ignored,
    Completed(i32),
}