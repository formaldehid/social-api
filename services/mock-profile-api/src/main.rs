#![forbid(unsafe_code)]

use anyhow::Result;

mod app;
mod config;
mod routes;
mod seed;
mod state;

#[tokio::main]
async fn main() -> Result<()> {
    app::run().await
}
