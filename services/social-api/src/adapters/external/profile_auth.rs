use crate::infra::metrics::Metrics;
use reqwest::StatusCode;
use serde::Deserialize;
use social_core::{
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
}

impl ProfileHttpAuth {
    pub fn new(base_url: Url, client: reqwest::Client, metrics: Arc<Metrics>) -> Self {
        Self {
            base_url,
            client,
            metrics,
        }
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
                return Err(AuthError::DependencyUnavailable(e.to_string()));
            }
        };

        self.metrics
            .external_calls_total
            .with_label_values(&["profile_api", "validate", resp.status().as_str()])
            .inc();

        match resp.status() {
            StatusCode::OK => {
                let body: ValidateOk = resp
                    .json()
                    .await
                    .map_err(|e| AuthError::DependencyUnavailable(e.to_string()))?;

                if !body.valid {
                    return Err(AuthError::Unauthorized);
                }

                let user_id = body.user_id.ok_or(AuthError::Unauthorized)?;
                Ok(UserIdentity {
                    user_id,
                    display_name: body.display_name,
                })
            }
            StatusCode::UNAUTHORIZED => Err(AuthError::Unauthorized),
            status => Err(AuthError::DependencyUnavailable(format!(
                "profile api returned {status}"
            ))),
        }
    }
}
