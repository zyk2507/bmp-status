//! Command line / environment configuration.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

const DEFAULT_MAX_MSG_SIZE: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "bmp-status",
    version,
    about = "BMP (BGP Monitoring Protocol) collector and live dashboard"
)]
pub struct Config {
    /// TCP address to listen for BMP connections from routers (BIRD, FRR, ...).
    #[arg(long, env = "BMP_BIND", default_value = "0.0.0.0:11019")]
    pub bmp_bind: SocketAddr,

    /// HTTP address for the dashboard, JSON API and WebSocket.
    #[arg(long, env = "HTTP_BIND", default_value = "0.0.0.0:8787")]
    pub http_bind: SocketAddr,

    /// Optional on-disk frontend directory used instead of the embedded assets.
    /// Useful for live frontend development; leave unset in production.
    #[arg(long, env = "FRONTEND_DIR")]
    pub frontend_dir: Option<PathBuf>,

    /// Directory used to persist the page password hash, instance registry and runtime settings.
    #[arg(long, env = "DATA_DIR", default_value = "data")]
    pub data_dir: PathBuf,

    /// Maximum accepted BMP message size in bytes.
    #[arg(long, env = "MAX_MSG_SIZE", default_value_t = DEFAULT_MAX_MSG_SIZE)]
    pub max_msg_size: u32,

    /// Close BMP connections that stay silent for this many seconds.
    #[arg(long, env = "IDLE_TIMEOUT", default_value_t = 600)]
    pub idle_timeout_secs: u64,

    /// Maximum number of concurrent BMP connections.
    #[arg(long, env = "MAX_CONNECTIONS", default_value_t = 256)]
    pub max_connections: usize,

    /// Start the synthetic BMP demo feed on startup.
    #[arg(long, env = "DEMO", default_value_t = false)]
    pub demo: bool,

    /// Delay between synthetic BMP messages (milliseconds).
    #[arg(long, env = "DEMO_INTERVAL_MS", default_value_t = 350)]
    pub demo_interval_ms: u64,

    /// Set or reset the page password (hashed on startup). Otherwise a generated
    /// password is written to `<data-dir>/initial-admin-password.txt` on first run.
    #[arg(long, env = "ADMIN_PASSWORD", hide_env_values = true)]
    pub admin_password: Option<String>,

    /// Add the Secure flag to the page session cookie (enable behind HTTPS).
    #[arg(long, env = "COOKIE_SECURE", default_value_t = false)]
    pub cookie_secure: bool,
}
