use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

#[derive(Clone, Debug)]
pub struct ReadinessBarrier {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    ready: AtomicBool,
    notify: Notify,
}

impl Default for ReadinessBarrier {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadinessBarrier {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner::default()),
        }
    }

    pub fn mark_ready(&self) {
        if !self.inner.ready.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    pub async fn wait_ready(&self) {
        if self.inner.ready.load(Ordering::SeqCst) {
            return;
        }

        loop {
            self.inner.notify.notified().await;
            if self.inner.ready.load(Ordering::SeqCst) {
                return;
            }
        }
    }
}
