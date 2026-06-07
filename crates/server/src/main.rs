//! Thin native entry point. Reads config from the environment, initialises
//! tracing, and hands off to the library's `serve`. All behaviour lives in the
//! lib so integration tests can exercise it without the binary.

use shape_server::{serve, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    serve(Config::from_env()).await
}
