//! Inbox: the one way a worker thread reaches the UI thread.
//!
//! The rule is that the UI thread never waits.  A worker therefore cannot call
//! back into the UI — it **posts** a message, and the UI picks it up when it is
//! convenient:
//!
//! ```text
//! worker thread                      UI thread
//! -------------                      ---------
//! inbox.push(message)   ─────────►   inbox.try_pop()   // never blocks
//! inbox.pop_timeout(d)  ◄─ wakeup ─  (the pump task)
//! ```
//!
//! [`Inbox::try_pop`] takes the lock for the time it takes to move one value out
//! of a `VecDeque` — there is no condition wait, no channel rendezvous, no
//! allocation on the way out.  [`Inbox::pop_timeout`] exists for the *background*
//! side (a worker that parks until work appears); the UI never calls it.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A many-producer, one-consumer message queue.
#[derive(Debug)]
pub struct Inbox<T> {
    queue: Mutex<VecDeque<T>>,
    ready: Condvar,
}

impl<T> Default for Inbox<T> {
    /// An empty inbox.  Written by hand (not derived) so that `T` does not have
    /// to be `Default`: the queue is empty whatever it holds.
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        }
    }
}

impl<T> Inbox<T> {
    /// A shared inbox handle.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Posts a message.  Takes the lock only long enough to push.
    pub fn push(&self, value: T) {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        queue.push_back(value);
        drop(queue);
        self.ready.notify_one();
    }

    /// Takes the oldest message, or `None` if there is none.
    ///
    /// This is the UI thread's only way in, and it never blocks.
    pub fn try_pop(&self) -> Option<T> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.pop_front()
    }

    /// Waits up to `timeout` for a message (for worker threads, never the UI).
    pub fn pop_timeout(&self, timeout: Duration) -> Option<T> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(value) = queue.pop_front() {
            return Some(value);
        }
        let (mut queue, _) = self
            .ready
            .wait_timeout(queue, timeout)
            .unwrap_or_else(|error| error.into_inner());
        queue.pop_front()
    }

    /// Waits until a message arrives.
    pub fn pop_blocking(&self) -> T {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(value) = queue.pop_front() {
                return value;
            }
            queue = self
                .ready
                .wait(queue)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// How many messages are waiting (approximate — it is a snapshot).
    pub fn len(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
