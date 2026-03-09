use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// High-level circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    HalfOpen,
    Open,
}

impl CircuitState {
    /// Spec mapping: 0=closed, 1=half-open, 2=open.
    pub fn as_i64(self) -> i64 {
        match self {
            CircuitState::Closed => 0,
            CircuitState::HalfOpen => 1,
            CircuitState::Open => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: CircuitState,
    pub to: CircuitState,
}

impl fmt::Display for Transition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} -> {:?}", self.from, self.to)
    }
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Closed→Open after N consecutive failures.
    pub failure_threshold: u32,
    /// Open→HalfOpen after this timeout.
    pub recovery_timeout: Duration,
    /// HalfOpen→Closed after N consecutive successes.
    pub success_threshold: u32,
    /// Rolling window used for the failure-rate criterion.
    pub failure_rate_window: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            success_threshold: 3,
            failure_rate_window: Duration::from_secs(30),
        }
    }
}

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug, Clone)]
struct Event {
    at: Instant,
    success: bool,
}

#[derive(Debug)]
enum Inner {
    Closed {
        consecutive_failures: u32,
        events: VecDeque<Event>,
    },
    Open {
        opened_at: Instant,
    },
    HalfOpen {
        consecutive_successes: u32,
    },
}

impl Inner {
    fn state(&self) -> CircuitState {
        match self {
            Inner::Closed { .. } => CircuitState::Closed,
            Inner::Open { .. } => CircuitState::Open,
            Inner::HalfOpen { .. } => CircuitState::HalfOpen,
        }
    }
}

/// A lightweight, in-memory circuit breaker state machine.
///
/// This is intentionally transport-agnostic: it can wrap HTTP today, and gRPC tomorrow.
///
/// Spec alignment:
/// - Closed→Open: after N consecutive failures OR >50% failure rate in a 30s window.
/// - Open→HalfOpen: after 30s.
/// - HalfOpen→Closed: after 3 consecutive successes.
#[derive(Clone)]
pub struct CircuitBreaker {
    cfg: CircuitBreakerConfig,
    clock: Arc<dyn Clock>,
    inner: Arc<Mutex<Inner>>,
}

pub struct AllowDecision {
    pub allowed: bool,
    pub state: CircuitState,
    pub transition: Option<Transition>,
}

pub struct RecordDecision {
    pub state: CircuitState,
    pub transition: Option<Transition>,
}

impl CircuitBreaker {
    pub fn new(cfg: CircuitBreakerConfig) -> Self {
        Self::with_clock(cfg, Arc::new(SystemClock))
    }

    pub fn with_clock(cfg: CircuitBreakerConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            cfg,
            clock,
            inner: Arc::new(Mutex::new(Inner::Closed {
                consecutive_failures: 0,
                events: VecDeque::new(),
            })),
        }
    }

    pub fn state(&self) -> CircuitState {
        self.inner.lock().expect("cb mutex").state()
    }

    /// Returns whether the caller is allowed to attempt a request.
    ///
    /// If the breaker is `Open` and the recovery timeout has elapsed, this transitions to
    /// `HalfOpen` and allows the request.
    pub fn allow_request(&self) -> AllowDecision {
        let now = self.clock.now();
        let mut inner = self.inner.lock().expect("cb mutex");

        let mut transition = None;
        let allowed = match &*inner {
            Inner::Closed { .. } => true,
            Inner::HalfOpen { .. } => true,
            Inner::Open { opened_at } => {
                if now.duration_since(*opened_at) >= self.cfg.recovery_timeout {
                    // Open -> HalfOpen
                    let from = inner.state();
                    *inner = Inner::HalfOpen {
                        consecutive_successes: 0,
                    };
                    transition = Some(Transition {
                        from,
                        to: inner.state(),
                    });
                    true
                } else {
                    false
                }
            }
        };

        AllowDecision {
            allowed,
            state: inner.state(),
            transition,
        }
    }

    pub fn record_success(&self) -> RecordDecision {
        self.record_result(true)
    }

    pub fn record_failure(&self) -> RecordDecision {
        self.record_result(false)
    }

    pub fn record_result(&self, success: bool) -> RecordDecision {
        let now = self.clock.now();
        let mut inner = self.inner.lock().expect("cb mutex");
        let from = inner.state();
        let mut transition = None;

        match &mut *inner {
            Inner::Closed {
                consecutive_failures,
                events,
            } => {
                // Rolling window maintenance.
                while let Some(front) = events.front() {
                    if now.duration_since(front.at) > self.cfg.failure_rate_window {
                        events.pop_front();
                    } else {
                        break;
                    }
                }
                events.push_back(Event { at: now, success });

                if success {
                    *consecutive_failures = 0;
                } else {
                    *consecutive_failures = consecutive_failures.saturating_add(1);
                }

                // Open conditions.
                let mut should_open = false;

                if *consecutive_failures >= self.cfg.failure_threshold {
                    should_open = true;
                } else {
                    // Failure-rate criterion.
                    let total = events.len() as u32;
                    if total >= self.cfg.failure_threshold {
                        let failures = events.iter().filter(|e| !e.success).count() as u32;
                        // Strictly greater than 50%.
                        if failures * 2 > total {
                            should_open = true;
                        }
                    }
                }

                if should_open {
                    *inner = Inner::Open { opened_at: now };
                }
            }
            Inner::Open { .. } => {
                // While open we don't learn from results because requests are normally blocked.
                // If a caller still records, ignore.
            }
            Inner::HalfOpen {
                consecutive_successes,
            } => {
                if success {
                    *consecutive_successes = consecutive_successes.saturating_add(1);
                    if *consecutive_successes >= self.cfg.success_threshold {
                        // HalfOpen -> Closed
                        *inner = Inner::Closed {
                            consecutive_failures: 0,
                            events: VecDeque::new(),
                        };
                    }
                } else {
                    // HalfOpen -> Open immediately on a failure.
                    *inner = Inner::Open { opened_at: now };
                }
            }
        }

        let to = inner.state();
        if from != to {
            transition = Some(Transition { from, to });
        }

        RecordDecision {
            state: to,
            transition,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Debug)]
    struct ManualClock {
        base: Instant,
        offset_ms: AtomicU64,
    }

    impl ManualClock {
        fn new() -> Self {
            Self {
                base: Instant::now(),
                offset_ms: AtomicU64::new(0),
            }
        }

        fn advance(&self, d: Duration) {
            self.offset_ms
                .fetch_add(d.as_millis() as u64, Ordering::SeqCst);
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            self.base + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
        }
    }

    fn cfg() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            success_threshold: 3,
            failure_rate_window: Duration::from_secs(30),
        }
    }

    #[test]
    fn closed_to_open_after_consecutive_failures() {
        let clock = Arc::new(ManualClock::new());
        let cb = CircuitBreaker::with_clock(cfg(), clock);

        assert_eq!(cb.state(), CircuitState::Closed);
        for _ in 0..4 {
            let d = cb.record_failure();
            assert!(d.transition.is_none());
            assert_eq!(d.state, CircuitState::Closed);
        }
        let last = cb.record_failure();
        assert_eq!(last.state, CircuitState::Open);
        assert!(last.transition.is_some());
    }

    #[test]
    fn open_blocks_until_timeout_then_half_open_allows() {
        let clock = Arc::new(ManualClock::new());
        let cb = CircuitBreaker::with_clock(cfg(), clock.clone());

        // Trip to open.
        for _ in 0..5 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);

        let d1 = cb.allow_request();
        assert!(!d1.allowed);
        assert_eq!(d1.state, CircuitState::Open);

        clock.advance(Duration::from_secs(31));

        let d2 = cb.allow_request();
        assert!(d2.allowed);
        assert_eq!(d2.state, CircuitState::HalfOpen);
        assert_eq!(d2.transition.unwrap().from, CircuitState::Open);
    }

    #[test]
    fn half_open_to_closed_after_success_threshold() {
        let clock = Arc::new(ManualClock::new());
        let cb = CircuitBreaker::with_clock(cfg(), clock.clone());

        for _ in 0..5 {
            cb.record_failure();
        }
        clock.advance(Duration::from_secs(31));
        let _ = cb.allow_request();
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success();
        cb.record_success();
        let d3 = cb.record_success();
        assert_eq!(d3.state, CircuitState::Closed);
        assert_eq!(d3.transition.unwrap().from, CircuitState::HalfOpen);
    }

    #[test]
    fn half_open_failure_returns_to_open() {
        let clock = Arc::new(ManualClock::new());
        let cb = CircuitBreaker::with_clock(cfg(), clock.clone());

        for _ in 0..5 {
            cb.record_failure();
        }
        clock.advance(Duration::from_secs(31));
        let _ = cb.allow_request();
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        let d = cb.record_failure();
        assert_eq!(d.state, CircuitState::Open);
        assert_eq!(d.transition.unwrap().from, CircuitState::HalfOpen);
    }

    #[test]
    fn closed_to_open_on_failure_rate_over_half_in_window() {
        let clock = Arc::new(ManualClock::new());
        let cb = CircuitBreaker::with_clock(cfg(), clock);

        // 5 events in window, 3 failures => 60% failures => should open.
        cb.record_success();
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        let d = cb.record_failure();
        assert_eq!(d.state, CircuitState::Open);
        assert!(d.transition.is_some());
    }
}
