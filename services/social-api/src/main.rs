#![forbid(unsafe_code)]

use anyhow::Result;

mod adapters;
mod app;
mod infra;
mod middleware;
mod state;

#[tokio::main]
async fn main() -> Result<()> {
    app::run().await
}
