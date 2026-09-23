//! Inbox contract: the one way background threads reach the UI thread.
//!
//! The UI thread must never wait for anything.  So background work never calls
//! back into the UI directly — it *posts* a message into an [`Inbox`], and the
//! host drains it.  Both operations on the UI side are non-blocking
//! (`try_pop`), which is what makes "nothing blocks the UI thread" structural
//! rather than a promise.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use light_note::inbox::Inbox;

#[test]
fn messages_come_out_in_the_order_they_went_in() {
    let inbox = Inbox::<u32>::new();

    for value in 1..=5 {
        inbox.push(value);
    }

    assert_eq!(inbox.len(), 5);
    let drained: Vec<u32> = std::iter::from_fn(|| inbox.try_pop()).collect();
    assert_eq!(drained, vec![1, 2, 3, 4, 5]);
    assert!(inbox.is_empty());
}

#[test]
fn popping_an_empty_inbox_never_blocks() {
    let inbox = Inbox::<u32>::new();

    let started = Instant::now();
    assert_eq!(inbox.try_pop(), None);
    assert!(started.elapsed() < Duration::from_millis(1), "try_pop is non-blocking");
}

#[test]
fn a_producer_thread_can_post_without_the_consumer_waiting() {
    let inbox = Inbox::<String>::new();
    let producer = {
        let inbox = Arc::clone(&inbox);
        std::thread::spawn(move || {
            for index in 0..1000 {
                inbox.push(format!("sample-{index}"));
            }
        })
    };

    // The UI thread keeps doing its own work; it just picks up whatever is
    // ready.  Nothing here blocks on the producer finishing.
    let mut seen = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while seen < 1000 && Instant::now() < deadline {
        if inbox.try_pop().is_some() {
            seen += 1;
        }
    }
    producer.join().expect("producer");
    assert_eq!(seen, 1000, "no message is lost");
}

#[test]
fn many_producers_are_serialized_by_the_queue() {
    let inbox = Inbox::<usize>::new();
    let pushed = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let inbox = Arc::clone(&inbox);
            let pushed = Arc::clone(&pushed);
            std::thread::spawn(move || {
                for _ in 0..250 {
                    inbox.push(pushed.fetch_add(1, Ordering::SeqCst));
                }
            })
        })
        .collect();

    for thread in threads {
        thread.join().expect("producer");
    }

    let drained: Vec<usize> = std::iter::from_fn(|| inbox.try_pop()).collect();
    assert_eq!(drained.len(), 1000);
    let mut sorted = drained.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..1000).collect::<Vec<_>>(), "every message exactly once");
}

#[test]
fn a_waiter_wakes_up_when_a_message_arrives() {
    let inbox = Inbox::<u32>::new();

    // Nothing to wait for: the timeout expires and the caller is told so.
    let started = Instant::now();
    assert_eq!(inbox.pop_timeout(Duration::from_millis(30)), None);
    assert!(started.elapsed() >= Duration::from_millis(25));

    // A message arrives while somebody waits: the wait ends immediately.
    let poster = {
        let inbox = Arc::clone(&inbox);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            inbox.push(7);
        })
    };
    let started = Instant::now();
    assert_eq!(inbox.pop_timeout(Duration::from_secs(5)), Some(7));
    assert!(started.elapsed() < Duration::from_secs(1), "woken by the push");
    poster.join().expect("poster");
}

#[test]
fn a_pump_can_wait_for_a_message_without_taking_it() {
    // The UI is woken by a background task that parks on the inbox.  That task
    // must not *consume* what it was woken for: the message has to still be
    // there for the UI thread to drain.  (A pump that eats its own wakeup loses
    // one pen sample per frame — which is exactly what the drawing looked like.)
    let inbox = Inbox::<u32>::new();
    let waiter = {
        let inbox = Arc::clone(&inbox);
        std::thread::spawn(move || {
            let started = Instant::now();
            inbox.wait_ready();
            started.elapsed()
        })
    };

    std::thread::sleep(Duration::from_millis(30));
    inbox.push(11);

    let waited = waiter.join().expect("waiter");
    assert!(waited >= Duration::from_millis(20), "woken by the push, not by a poll");
    assert_eq!(inbox.try_pop(), Some(11), "waiting does not consume the message");
}

#[test]
fn waiting_for_a_message_that_is_already_there_returns_at_once() {
    let inbox = Inbox::<u32>::new();
    inbox.push(1);

    let started = Instant::now();
    inbox.wait_ready();

    assert!(started.elapsed() < Duration::from_millis(20), "no wait when work is queued");
    assert_eq!(inbox.try_pop(), Some(1));
}

#[test]
fn an_inbox_is_shared_between_threads() {
    // The handle is `Arc<Inbox<T>>`: cheap to clone, `Send + Sync`, and the
    // payload only has to be `Send`.
    fn assert_shareable<T: Send + 'static>() {
        fn assert_send_sync<U: Send + Sync>() {}
        assert_send_sync::<Inbox<T>>();
    }
    assert_shareable::<Vec<u8>>();
}
