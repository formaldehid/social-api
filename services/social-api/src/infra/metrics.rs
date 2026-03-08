use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
    TextEncoder,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct Metrics {
    registry: Registry,

    pub http_requests_total: IntCounterVec,
    pub http_request_duration_seconds: HistogramVec,

    pub cache_operations_total: IntCounterVec,
    pub external_calls_total: IntCounterVec,
    pub external_call_duration_seconds: HistogramVec,

    pub circuit_breaker_state: IntGaugeVec,
    pub db_pool_connections: IntGaugeVec,
    pub sse_connections_active: IntGauge,
    pub likes_total: IntCounterVec,
}

impl Metrics {
    pub fn new() -> anyhow::Result<Arc<Self>> {
        let registry = Registry::new();

        let http_requests_total = IntCounterVec::new(
            Opts::new("social_api_http_requests_total", "Total HTTP requests."),
            &["method", "path", "status"],
        )?;

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "social_api_http_request_duration_seconds",
                "HTTP request duration in seconds.",
            )
            .buckets(vec![
                0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "path"],
        )?;

        let cache_operations_total = IntCounterVec::new(
            Opts::new("social_api_cache_operations_total", "Cache operations."),
            &["operation", "result"],
        )?;

        let external_calls_total = IntCounterVec::new(
            Opts::new("social_api_external_calls_total", "External HTTP calls."),
            &["service", "method", "status"],
        )?;

        let external_call_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "social_api_external_call_duration_seconds",
                "External call duration in seconds.",
            )
            .buckets(vec![
                0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
            ]),
            &["service", "method"],
        )?;

        let circuit_breaker_state = IntGaugeVec::new(
            Opts::new(
                "social_api_circuit_breaker_state",
                "Circuit breaker state (0=closed, 1=half-open, 2=open).",
            ),
            &["service"],
        )?;

        let db_pool_connections = IntGaugeVec::new(
            Opts::new(
                "social_api_db_pool_connections",
                "Database pool connections (active/idle/max).",
            ),
            &["state"],
        )?;

        let sse_connections_active = IntGauge::new(
            "social_api_sse_connections_active",
            "Active SSE connections.",
        )?;

        let likes_total = IntCounterVec::new(
            Opts::new("social_api_likes_total", "Like/unlike operations."),
            &["content_type", "operation"],
        )?;

        registry.register(Box::new(http_requests_total.clone()))?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;
        registry.register(Box::new(cache_operations_total.clone()))?;
        registry.register(Box::new(external_calls_total.clone()))?;
        registry.register(Box::new(external_call_duration_seconds.clone()))?;
        registry.register(Box::new(circuit_breaker_state.clone()))?;
        registry.register(Box::new(db_pool_connections.clone()))?;
        registry.register(Box::new(sse_connections_active.clone()))?;
        registry.register(Box::new(likes_total.clone()))?;

        let m = Self {
            registry,
            http_requests_total,
            http_request_duration_seconds,
            cache_operations_total,
            external_calls_total,
            external_call_duration_seconds,
            circuit_breaker_state,
            db_pool_connections,
            sse_connections_active,
            likes_total,
        };

        // Pre-initialize metric families/series so the required names appear on /metrics
        // even before any traffic.
        m.http_requests_total
            .with_label_values(&["GET", "/health/live", "200"]);
        m.http_request_duration_seconds
            .with_label_values(&["GET", "/health/live"])
            .observe(0.001);

        m.cache_operations_total.with_label_values(&["get", "hit"]);
        m.external_calls_total
            .with_label_values(&["profile_api", "validate", "200"]);
        m.external_call_duration_seconds
            .with_label_values(&["profile_api", "validate"])
            .observe(0.001);

        m.circuit_breaker_state
            .with_label_values(&["profile_api"])
            .set(0);
        m.db_pool_connections.with_label_values(&["active"]).set(0);
        m.db_pool_connections.with_label_values(&["idle"]).set(0);
        m.db_pool_connections.with_label_values(&["max"]).set(0);
        m.sse_connections_active.set(0);
        m.likes_total.with_label_values(&["post", "like"]);

        Ok(Arc::new(m))
    }

    pub fn render(&self) -> anyhow::Result<String> {
        let metric_families = self.registry.gather();
        let encoder = TextEncoder::new();
        let mut buf = Vec::new();
        encoder.encode(&metric_families, &mut buf)?;
        Ok(String::from_utf8(buf)?)
    }

    pub fn cache_hit(&self, op: &str) {
        self.cache_operations_total
            .with_label_values(&[op, "hit"])
            .inc();
    }
    pub fn cache_miss(&self, op: &str) {
        self.cache_operations_total
            .with_label_values(&[op, "miss"])
            .inc();
    }
    pub fn cache_error(&self, op: &str) {
        self.cache_operations_total
            .with_label_values(&[op, "error"])
            .inc();
    }
}
