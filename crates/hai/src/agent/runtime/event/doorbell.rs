use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::Notify;

/// A doorbell for one chat: the changes themselves live in the database.
#[derive(Clone)]
pub struct Doorbell {
    notify: Arc<Notify>,
    pending: Arc<AtomicBool>,
}

impl Doorbell {
    pub fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            pending: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn notify(&self) {
        self.pending.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    pub fn notified(&self) -> impl Future<Output = ()> + '_ {
        self.notify.notified()
    }

    /// Reads and clears the arrival flag: from here the accumulation a caller holds accounts for
    /// every arrival so far.
    pub fn take_signal(&self) -> bool {
        self.pending.swap(false, Ordering::SeqCst)
    }
}
