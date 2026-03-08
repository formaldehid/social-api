use crate::infra::metrics::Metrics;
use reqwest::StatusCode;
use social_core::{
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
}

impl HttpContentCatalog {
    pub fn new(
        registry: Arc<HashMap<String, Url>>,
        client: reqwest::Client,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            registry,
            client,
            metrics,
        }
    }
}

#[async_trait::async_trait]
impl ContentCatalog for HttpContentCatalog {
    async fn exists(&self, key: &ContentKey) -> Result<bool, ContentError> {
        let base = self
            .registry
            .get(&key.content_type)
            .ok_or_else(|| ContentError::UnknownContentType(key.content_type.clone()))?
            .clone();

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
                return Err(ContentError::DependencyUnavailable(e.to_string()));
            }
        };

        self.metrics
            .external_calls_total
            .with_label_values(&["content_api", "get", resp.status().as_str()])
            .inc();

        match resp.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            status => Err(ContentError::DependencyUnavailable(format!(
                "content api returned {status}"
            ))),
        }
    }
}
