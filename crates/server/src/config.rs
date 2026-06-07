//! Server configuration sourced from the environment.
//!
//! Kept deliberately small: host/port for the listener and the directory of
//! pre-built client assets to serve. Later phases (canvas actor, MCP) add their
//! own knobs here rather than threading raw env reads through the app.

use std::net::SocketAddr;
use std::path::PathBuf;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8787;

/// Resolved server settings. Built once at startup from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    /// Directory of pre-built client assets to serve statically, if present.
    pub client_dir: PathBuf,
}

impl Config {
    /// Read configuration from the process environment, applying defaults.
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

    /// The socket address to bind the listener to.
    pub fn socket_addr(&self) -> anyhow::Result<SocketAddr> {
        let addr = format!("{}:{}", self.host, self.port);
        addr.parse()
            .map_err(|e| anyhow::anyhow!("invalid bind address {addr:?}: {e}"))
    }
}

/// The default location of built client assets: `<repo>/dist/client`, resolved
/// relative to this crate at compile time so it works regardless of cwd.
fn default_client_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("dist")
        .join("client")
}
