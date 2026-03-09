use crate::{
    adapters::storage::redis_content_validation::RedisContentValidationCache,
    infra::metrics::Metrics,
};
use reqwest::StatusCode;
use social_core::{
    circuit_breaker::{CircuitBreaker, CircuitState},
    domain::ContentKey,
    ports::{ContentCatalog, ContentError},
};
use std::{collections::HashMap, sync::Arc, time::Instant};
use url::Url;

#[derive(Clone)]
pub struct HttpContentCatalog {
    registry: Arc<HashMap<String, Url>>,
    client: reqwest::Client,
    metrics: Arc<Metrics>,
    cache: RedisContentValidationCache,
    breakers: Arc<HashMap<String, CircuitBreaker>>,
}

impl HttpContentCatalog {
    pub fn new(
        registry: Arc<HashMap<String, Url>>,
        client: reqwest::Client,
        metrics: Arc<Metrics>,
        cache: RedisContentValidationCache,
        breakers: Arc<HashMap<String, CircuitBreaker>>,
    ) -> Self {
        Self {
            registry,
            client,
            metrics,
            cache,
            breakers,
        }
    }

    fn breaker_label(content_type: &str) -> String {
        format!("content_api_{content_type}")
    }

    fn set_cb_metric(&self, label: &str, state: CircuitState) {
        self.metrics
            .circuit_breaker_state
            .with_label_values(&[label])
            .set(state.as_i64());
    }

    fn on_cb_transition(&self, label: &str, t: social_core::circuit_breaker::Transition) {
        tracing::warn!(
            service = "social-api",
            external_service = %label,
            transition = %t,
            "circuit breaker state transition"
        );
        self.set_cb_metric(label, t.to);
    }
}

#[async_trait::async_trait]
impl ContentCatalog for HttpContentCatalog {
    async fn exists(&self, key: &ContentKey) -> Result<bool, ContentError> {
        // Cache-first: content existence changes rarely.
        match self.cache.get(key).await {
            Ok(Some(v)) => return Ok(v),
            Ok(None) => {}
            Err(e) => {
                // Degraded mode if Redis is unavailable.
                tracing::warn!(
                    service = "social-api",
                    error_type = "cache",
                    error_message = %e,
                    "content validation cache unavailable; falling back to external call"
                );
            }
        }

        let base = self
            .registry
            .get(&key.content_type)
            .ok_or_else(|| ContentError::UnknownContentType(key.content_type.clone()))?
            .clone();

        let breaker = self
            .breakers
            .get(&key.content_type)
            .cloned()
            .ok_or_else(|| ContentError::UnknownContentType(key.content_type.clone()))?;
        let breaker_label = Self::breaker_label(&key.content_type);

        let allow = breaker.allow_request();
        if let Some(t) = allow.transition {
            self.on_cb_transition(&breaker_label, t);
        }
        self.set_cb_metric(&breaker_label, allow.state);
        if !allow.allowed {
            return Err(ContentError::DependencyUnavailable(
                "content api circuit breaker open".to_string(),
            ));
        }

        let url = base
            .join(&format!("/v1/{}/{}", key.content_type, key.content_id))
            .map_err(|e| ContentError::DependencyUnavailable(e.to_string()))?;

        let start = Instant::now();
        let resp = self.client.get(url).send().await;
        let elapsed = start.elapsed().as_secs_f64();

        self.metrics
            .external_call_duration_seconds
            .with_label_values(&["content_api", "get"])
            .observe(elapsed);

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                self.metrics
                    .external_calls_total
                    .with_label_values(&["content_api", "get", "error"])
                    .inc();

                let d = breaker.record_failure();
                if let Some(t) = d.transition {
                    self.on_cb_transition(&breaker_label, t);
                } else {
                    self.set_cb_metric(&breaker_label, d.state);
                }

                tracing::warn!(
                    service = "social-api",
                    external_service = %breaker_label,
                    method = "get",
                    latency_ms = (elapsed * 1000.0) as u64,
                    success = false,
                    error_message = %e,
                    "external call failed"
                );
                return Err(ContentError::DependencyUnavailable(e.to_string()));
            }
        };

        self.metrics
            .external_calls_total
            .with_label_values(&["content_api", "get", resp.status().as_str()])
            .inc();

        // Log external call (required structured fields).
        tracing::info!(
            service = "social-api",
            external_service = %breaker_label,
            method = "get",
            latency_ms = (elapsed * 1000.0) as u64,
            success = resp.status() == StatusCode::OK || resp.status() == StatusCode::NOT_FOUND,
            status = resp.status().as_u16(),
            "external call completed"
        );

        match resp.status() {
            StatusCode::OK => {
                let _ = self.cache.set(key, true).await;

                let d = breaker.record_success();
                if let Some(t) = d.transition {
                    self.on_cb_transition(&breaker_label, t);
                } else {
                    self.set_cb_metric(&breaker_label, d.state);
                }

                Ok(true)
            }
            StatusCode::NOT_FOUND => {
                let _ = self.cache.set(key, false).await;

                let d = breaker.record_success();
                if let Some(t) = d.transition {
                    self.on_cb_transition(&breaker_label, t);
                } else {
                    self.set_cb_metric(&breaker_label, d.state);
                }

                Ok(false)
            }
            status => {
                let d = breaker.record_failure();
                if let Some(t) = d.transition {
                    self.on_cb_transition(&breaker_label, t);
                } else {
                    self.set_cb_metric(&breaker_label, d.state);
                }

                Err(ContentError::DependencyUnavailable(format!(
                    "content api returned {status}"
                )))
            }
        }
    }
}
