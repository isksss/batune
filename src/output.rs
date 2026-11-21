//! Hands session output from the reader thread to the UI thread with
//! backpressure: the reader waits while the UI has a full buffer to get
//! through, so `yes` or a huge `cat` stalls the program instead of growing
//! memory without bound.
//!
//! Events posted to the winit event loop cannot be bounded or waited on, so
//! output goes into a shared buffer and only the first push after a take
//! posts a wake.

use std::mem;
use std::sync::{Arc, Condvar, Mutex};

/// Bytes the reader may queue before it waits for the UI. Large enough that
/// a burst is taken in one wake, small enough that one take is quick to
/// parse.
pub const LIMIT: usize = 256 * 1024;

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    /// The session's output has ended.
    closed: bool,
    /// A wake was posted and not yet answered by a take.
    woken: bool,
    /// The UI is gone; pushes no longer wait or keep data.
    dropped: bool,
}

struct Shared {
    state: Mutex<State>,
    /// Signalled when a take frees room or the UI is gone.
    room: Condvar,
}

/// The reader thread's end.
pub struct Sender {
    shared: Arc<Shared>,
    limit: usize,
    wake: Box<dyn Fn() + Send>,
}

/// The UI thread's end. Dropping it releases a waiting reader.
pub struct Receiver {
    shared: Arc<Shared>,
}

/// What one take found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Taken {
    pub bytes: Vec<u8>,
    pub closed: bool,
}

/// `wake` is called on the reader thread when the UI should call
/// [`Receiver::take`].
pub fn channel(limit: usize, wake: impl Fn() + Send + 'static) -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State::default()),
        room: Condvar::new(),
    });
    (
        Sender {
            shared: shared.clone(),
            limit,
            wake: Box::new(wake),
        },
        Receiver { shared },
    )
}

impl Sender {
    /// Queues `bytes`, first waiting while `limit` bytes are already queued.
    pub fn push(&self, bytes: &[u8]) {
        let mut state = self.shared.state.lock().unwrap();
        while state.bytes.len() >= self.limit && !state.dropped {
            state = self.shared.room.wait(state).unwrap();
        }
        if state.dropped {
            return;
        }
        state.bytes.extend_from_slice(bytes);
        self.wake_once(state);
    }

    /// Marks the end of output; the UI sees it after the bytes before it.
    pub fn close(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.closed = true;
        self.wake_once(state);
    }

    fn wake_once(&self, mut state: std::sync::MutexGuard<'_, State>) {
        if !state.woken && !state.dropped {
            state.woken = true;
            drop(state);
            (self.wake)();
        }
    }
}

impl Receiver {
    /// Takes everything queued and lets a waiting reader continue.
    pub fn take(&self) -> Taken {
        let mut state = self.shared.state.lock().unwrap();
        state.woken = false;
        let taken = Taken {
            bytes: mem::take(&mut state.bytes),
            closed: state.closed,
        };
        self.shared.room.notify_all();
        taken
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.dropped = true;
        state.bytes = Vec::new();
        self.shared.room.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    fn counting(limit: usize) -> (Sender, Receiver, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        let (tx, rx) = channel(limit, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        (tx, rx, wakes)
    }

    #[test]
    fn pushes_before_a_take_share_one_wake() {
        let (tx, rx, wakes) = counting(1024);
        tx.push(b"ab");
        tx.push(b"cd");
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(rx.take().bytes, b"abcd");
        tx.push(b"e");
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn close_is_seen_with_the_output_before_it() {
        let (tx, rx, wakes) = counting(1024);
        tx.push(b"bye");
        tx.close();
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(
            rx.take(),
            Taken {
                bytes: b"bye".to_vec(),
                closed: true
            }
        );
    }

    #[test]
    fn push_waits_while_the_buffer_is_full_until_a_take() {
        let (tx, rx, _) = counting(4);
        tx.push(b"full");
        let (done_tx, done_rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            tx.push(b"more");
            done_tx.send(()).unwrap();
        });
        assert!(
            done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "push did not wait"
        );
        assert_eq!(rx.take().bytes, b"full");
        done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        reader.join().unwrap();
        assert_eq!(rx.take().bytes, b"more");
    }

    #[test]
    fn dropping_the_receiver_releases_a_waiting_push() {
        let (tx, rx, _) = counting(4);
        tx.push(b"full");
        let reader = thread::spawn(move || tx.push(b"more"));
        thread::sleep(Duration::from_millis(50));
        drop(rx);
        reader.join().unwrap();
    }
}
