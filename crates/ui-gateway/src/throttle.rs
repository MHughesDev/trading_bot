//! Intentionally-lossy frame throttle.
//!
//! Producers may push updates at any rate.  The consumer receives at most
//! `max_fps` frames per second — intermediate frames are silently dropped.
//! This is correct for human visualization; it is **not** acceptable for
//! strategy execution, which must use the raw NATS streams directly.

use std::time::Duration;

use tokio::sync::watch;

/// Lossy throttle backed by a `watch` channel.
///
/// `push()` overwrites the latest frame atomically.  A background consumer
/// (created via `throttled_stream`) polls at the configured fps interval,
/// forwarding only the most-recent frame at each tick.
pub struct FrameThrottle<T: Clone + Send + Sync + 'static> {
    tx: watch::Sender<Option<T>>,
}

impl<T: Clone + Send + Sync + 'static> FrameThrottle<T> {
    /// Create a throttle and its paired receiver.
    pub fn new() -> (Self, watch::Receiver<Option<T>>) {
        let (tx, rx) = watch::channel(None);
        (Self { tx }, rx)
    }

    /// Push a new frame.  If the consumer hasn't read the previous frame yet,
    /// that frame is silently dropped (intentional).
    pub fn push(&self, frame: T) {
        let _ = self.tx.send(Some(frame));
    }
}

/// Spawn a background task that polls `rx` at `max_fps` and calls `on_frame` for each tick
/// that has a non-`None` value.
///
/// Returns the `JoinHandle`; abort it to stop the loop.
pub fn throttled_stream<T: Clone + Send + Sync + 'static>(
    mut rx: watch::Receiver<Option<T>>,
    max_fps: u32,
    on_frame: impl Fn(T) + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let interval_ms = 1000u64 / u64::from(max_fps.max(1));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            ticker.tick().await;
            let frame = rx.borrow_and_update().clone();
            if let Some(f) = frame {
                on_frame(f);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn burst_yields_bounded_frame_count() {
        let (throttle, rx) = FrameThrottle::<u32>::new();
        let count = Arc::new(Mutex::new(0u32));
        let count_clone = Arc::clone(&count);

        let handle = throttled_stream(rx, 20, move |_| {
            *count_clone.lock().unwrap() += 1;
        });

        // Push 500 frames as fast as possible.
        for i in 0u32..500 {
            throttle.push(i);
        }

        // Wait slightly more than 1 second.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        handle.abort();

        let n = *count.lock().unwrap();
        // At 20 fps for 1.1 s, expect ≤ 25 frames — nowhere near 500.
        assert!(
            n <= 25,
            "rate not bounded: {n} frames (expected ≤ 25 at 20 fps)"
        );
        // Should have at least a few frames.
        assert!(n >= 10, "too few frames: {n}");
    }

    #[tokio::test]
    async fn no_frames_sent_when_no_updates() {
        let (_throttle, rx) = FrameThrottle::<u32>::new();
        let count = Arc::new(Mutex::new(0u32));
        let count_clone = Arc::clone(&count);

        let handle = throttled_stream(rx, 20, move |_| {
            *count_clone.lock().unwrap() += 1;
        });

        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();

        assert_eq!(*count.lock().unwrap(), 0);
    }
}
