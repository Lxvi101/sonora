//! A minimal waker-driven channel for handing results from worker threads (and
//! the platform's open-URL callback) to tasks on GPUI's main thread, so the UI
//! wakes exactly when something arrives instead of polling.

use std::collections::VecDeque;
use std::future::poll_fn;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

struct State<T> {
    queue: VecDeque<T>,
    waker: Option<Waker>,
    senders: usize,
}

pub struct Sender<T>(Arc<Mutex<State<T>>>);
pub struct Receiver<T>(Arc<Mutex<State<T>>>);

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let state = Arc::new(Mutex::new(State {
        queue: VecDeque::new(),
        waker: None,
        senders: 1,
    }));
    (Sender(state.clone()), Receiver(state))
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        let waker = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.queue.push_back(value);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).senders += 1;
        Sender(self.0.clone())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let waker = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.senders -= 1;
            if state.senders == 0 {
                state.waker.take()
            } else {
                None
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Receiver<T> {
    /// Waits for the next value; `None` once every sender is gone and the
    /// queue is drained.
    pub async fn recv(&mut self) -> Option<T> {
        poll_fn(|cx| {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(value) = state.queue.pop_front() {
                Poll::Ready(Some(value))
            } else if state.senders == 0 {
                Poll::Ready(None)
            } else {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }

    /// Waits for a value, then skips ahead to the newest one queued. Used for
    /// progressive snapshots where only the latest matters.
    pub async fn recv_latest(&mut self) -> Option<T> {
        let first = self.recv().await?;
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Some(state.queue.drain(..).last().unwrap_or(first))
    }
}
