use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::watch;

/// Process-wide shutdown signal.
///
/// We use a `watch` channel because:
/// - we only care about the latest state (shutting down or not)
/// - late subscribers should immediately observe the shutdown state
///
/// This signal is used to:
/// - close long-lived SSE connections with a final `shutdown` event
/// - stop background jobs (leaderboard refresher)
/// - fail readiness quickly so upstream stops routing traffic
#[derive(Clone)]
pub struct Shutdown {
    tx: watch::Sender<bool>,
    triggered: Arc<AtomicBool>,
}

impl Shutdown {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self {
            tx,
            triggered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Triggers shutdown. This is idempotent.
    pub fn trigger(&self) {
        self.triggered.store(true, Ordering::SeqCst);
        // Best-effort: ignore send errors (they only happen when there are no receivers).
        let _ = self.tx.send(true);
    }

    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }
}
