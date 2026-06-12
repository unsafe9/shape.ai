//! Server configuration sourced from the environment.

use std::net::SocketAddr;
use std::path::PathBuf;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8787;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub client_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let host = std::env::var("SHAPE_AI_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
        let port = std::env::var("SHAPE_AI_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        let client_dir = std::env::var("SHAPE_AI_CLIENT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_client_dir());
        Config {
            host,
            port,
            client_dir,
        }
    }

    pub fn socket_addr(&self) -> anyhow::Result<SocketAddr> {
        let addr = format!("{}:{}", self.host, self.port);
        addr.parse()
            .map_err(|e| anyhow::anyhow!("invalid bind address {addr:?}: {e}"))
    }
}

/// `<repo>/dist/client`, resolved relative to this crate at compile time so it
/// works regardless of cwd.
fn default_client_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("dist")
        .join("client")
}
