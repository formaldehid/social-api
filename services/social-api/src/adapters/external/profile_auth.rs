use crate::infra::metrics::Metrics;
use reqwest::StatusCode;
use serde::Deserialize;
use social_core::{
    circuit_breaker::{CircuitBreaker, CircuitState},
    domain::UserIdentity,
    ports::{AuthError, AuthProvider},
};
use std::{sync::Arc, time::Instant};
use url::Url;

#[derive(Clone)]
pub struct ProfileHttpAuth {
    base_url: Url,
    client: reqwest::Client,
    metrics: Arc<Metrics>,
    breaker: CircuitBreaker,
    breaker_label: String,
}

impl ProfileHttpAuth {
    pub fn new(
        base_url: Url,
        client: reqwest::Client,
        metrics: Arc<Metrics>,
        breaker: CircuitBreaker,
        breaker_label: impl Into<String>,
    ) -> Self {
        Self {
            base_url,
            client,
            metrics,
            breaker,
            breaker_label: breaker_label.into(),
        }
    }

    fn set_cb_metric(&self, state: CircuitState) {
        self.metrics
            .circuit_breaker_state
            .with_label_values(&[self.breaker_label.as_str()])
            .set(state.as_i64());
    }

    fn on_cb_transition(&self, t: social_core::circuit_breaker::Transition) {
        tracing::warn!(
            service = "social-api",
            external_service = %self.breaker_label,
            transition = %t,
            "circuit breaker state transition"
        );
        self.set_cb_metric(t.to);
    }
}

#[derive(Debug, Deserialize)]
struct ValidateOk {
    valid: bool,
    user_id: Option<String>,
    display_name: Option<String>,
}

#[async_trait::async_trait]
impl AuthProvider for ProfileHttpAuth {
    async fn validate_token(&self, bearer_token: &str) -> Result<UserIdentity, AuthError> {
        let allow = self.breaker.allow_request();
        if let Some(t) = allow.transition {
            self.on_cb_transition(t);
        }
        self.set_cb_metric(allow.state);

        if !allow.allowed {
            return Err(AuthError::DependencyUnavailable(
                "profile api circuit breaker open".to_string(),
            ));
        }

        let url = self.base_url.join("/v1/auth/validate").map_err(|e| {
            AuthError::DependencyUnavailable(format!("invalid PROFILE_API_URL: {e}"))
        })?;

        let start = Instant::now();
        let resp = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {bearer_token}"))
            .send()
            .await;

        let elapsed = start.elapsed().as_secs_f64();
        self.metrics
            .external_call_duration_seconds
            .with_label_values(&["profile_api", "validate"])
            .observe(elapsed);

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                self.metrics
                    .external_calls_total
                    .with_label_values(&["profile_api", "validate", "error"])
                    .inc();

                let d = self.breaker.record_failure();
                if let Some(t) = d.transition {
                    self.on_cb_transition(t);
                } else {
                    self.set_cb_metric(d.state);
                }

                tracing::warn!(
                    service = "social-api",
                    external_service = %self.breaker_label,
                    method = "validate",
                    latency_ms = (elapsed * 1000.0) as u64,
                    success = false,
                    error_message = %e,
                    "external call failed"
                );
                return Err(AuthError::DependencyUnavailable(e.to_string()));
            }
        };

        self.metrics
            .external_calls_total
            .with_label_values(&["profile_api", "validate", resp.status().as_str()])
            .inc();

        // Log external call (required structured fields).
        tracing::info!(
            service = "social-api",
            external_service = %self.breaker_label,
            method = "validate",
            latency_ms = (elapsed * 1000.0) as u64,
            success = resp.status() == StatusCode::OK || resp.status() == StatusCode::UNAUTHORIZED,
            status = resp.status().as_u16(),
            "external call completed"
        );

        match resp.status() {
            StatusCode::OK => {
                let body: ValidateOk = match resp.json().await {
                    Ok(b) => b,
                    Err(e) => {
                        let d = self.breaker.record_failure();
                        if let Some(t) = d.transition {
                            self.on_cb_transition(t);
                        } else {
                            self.set_cb_metric(d.state);
                        }
                        return Err(AuthError::DependencyUnavailable(e.to_string()));
                    }
                };

                // 200 is a success from the circuit-breaker perspective.
                let d = self.breaker.record_success();
                if let Some(t) = d.transition {
                    self.on_cb_transition(t);
                } else {
                    self.set_cb_metric(d.state);
                }

                if !body.valid {
                    return Err(AuthError::Unauthorized);
                }

                let user_id = body.user_id.ok_or(AuthError::Unauthorized)?;
                Ok(UserIdentity {
                    user_id,
                    display_name: body.display_name,
                })
            }
            StatusCode::UNAUTHORIZED => {
                // 401 is still a "successful" call, just an auth rejection.
                let d = self.breaker.record_success();
                if let Some(t) = d.transition {
                    self.on_cb_transition(t);
                } else {
                    self.set_cb_metric(d.state);
                }
                Err(AuthError::Unauthorized)
            }
            status => {
                // Unexpected status = failure.
                let d = self.breaker.record_failure();
                if let Some(t) = d.transition {
                    self.on_cb_transition(t);
                } else {
                    self.set_cb_metric(d.state);
                }

                Err(AuthError::DependencyUnavailable(format!(
                    "profile api returned {status}"
                )))
            }
        }
    }
}
