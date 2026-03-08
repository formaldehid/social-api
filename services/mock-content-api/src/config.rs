use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    #[serde(default = "default_content_type")]
    pub content_type: String,

    #[serde(default = "default_http_port")]
    pub http_port: u16,
}

impl Settings {
    pub fn from_env() -> Result<Self> {
        Ok(envy::from_env::<Settings>()?)
    }
}

fn default_content_type() -> String {
    "post".to_string()
}

fn default_http_port() -> u16 {
    8080
}
