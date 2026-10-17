//! Tokio-based BMP TCP listener with framed reads, backpressure-safe ingest and
//! graceful connection management.

use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use bgpkit_parser::parse_bmp_msg;
use bytes::Bytes;
use socket2::{SockRef, TcpKeepalive};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::state::AppState;

/// Maximum number of raw bytes kept per message for the detail drawer.
const MAX_RAW_PREVIEW: usize = 1_024;

pub async fn serve(listener: TcpListener, state: Arc<AppState>, config: Arc<Config>) {
    let semaphore = Arc::new(Semaphore::new(config.max_connections.max(1)));
    let mut shutdown = state.subscribe_shutdown();
    let mut tasks: JoinSet<()> = JoinSet::new();

    info!(
        bind = %config.bmp_bind,
        max_connections = config.max_connections,
        max_msg_size = config.max_msg_size,
        "BMP listener ready"
    );

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, addr)) => {
                        if state.is_shutting_down() {
                            break;
                        }
                        if !state.admin.is_source_allowed(addr.ip()) {
                            let reason = format!(
                                "source {} is not in the enabled instance allowlist",
                                addr.ip()
                            );
                            warn!(%addr, "{reason}");
                            state.record_rejected_connection(addr, reason);
                            drop(stream);
                            continue;
                        }
                        let permit = match semaphore.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(%addr, "BMP connection limit reached, rejecting connection");
                                state.record_rejected_connection(
                                    addr,
                                    "connection limit reached".to_string(),
                                );
                                drop(stream);
                                continue;
                            }
                        };
                        let conn_id = state.connection_open(addr);
                        info!(%addr, conn_id, "BMP connection accepted");
                        let state2 = state.clone();
                        let config2 = config.clone();
                        tasks.spawn(async move {
                            let _permit = permit;
                            let reason =
                                handle_connection(stream, addr, conn_id, state2.clone(), config2)
                                    .await;
                            state2.connection_close(conn_id, &reason);
                        });
                    }
                    Err(e) => {
                        warn!("BMP accept error: {e}");
                        sleep(Duration::from_millis(200)).await;
                    }
                }
            }
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(e) = result {
                    warn!("BMP connection task failed: {e}");
                }
            }
        }
    }

    info!("BMP listener shutting down");
    let deadline = sleep(Duration::from_secs(5));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => {
                tasks.abort_all();
                break;
            }
            result = tasks.join_next() => match result {
                Some(Err(e)) => warn!("BMP connection task failed: {e}"),
                Some(Ok(())) => {}
                None => break,
            }
        }
    }
    info!("BMP listener stopped");
}

async fn handle_connection(
    stream: TcpStream,
    addr: std::net::SocketAddr,
    conn_id: u64,
    state: Arc<AppState>,
    config: Arc<Config>,
) -> String {
    if let Err(e) = stream.set_nodelay(true) {
        debug!(%addr, "failed to set TCP_NODELAY: {e}");
    }
    configure_keepalive(&stream);

    let idle = Duration::from_secs(config.idle_timeout_secs.max(5));
    let mut reader = BufReader::with_capacity(64 * 1024, stream);
    let mut shutdown = state.subscribe_shutdown();
    let mut header = [0u8; 6];

    loop {
        let read = tokio::select! {
            _ = shutdown.changed() => return "server shutdown".to_string(),
            result = timeout(idle, reader.read_exact(&mut header)) => result,
        };
        match read {
            Ok(Ok(_)) => {}
            Ok(Err(e)) if e.kind() == ErrorKind::UnexpectedEof => {
                return "peer closed connection".to_string();
            }
            Ok(Err(e)) => return format!("read error: {e}"),
            Err(_) => return format!("idle timeout ({}s)", idle.as_secs()),
        }

        let version = header[0];
        let msg_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
        let msg_type = header[5];

        if version != 3 {
            let (hex, truncated) = hex_preview(&header, MAX_RAW_PREVIEW);
            state.ingest_error(
                conn_id,
                addr.ip(),
                "protocol_error",
                msg_type,
                format!("invalid BMP version {version}; expected 3"),
                6,
                hex,
                truncated,
            );
            return format!("invalid BMP version {version}");
        }
        if msg_len < 6 || msg_len > config.max_msg_size {
            let (hex, truncated) = hex_preview(&header, MAX_RAW_PREVIEW);
            state.ingest_error(
                conn_id,
                addr.ip(),
                "protocol_error",
                msg_type,
                format!(
                    "invalid BMP message length {msg_len}; allowed range 6..={}",
                    config.max_msg_size
                ),
                6,
                hex,
                truncated,
            );
            return format!("invalid BMP message length {msg_len}");
        }

        let mut body = vec![0u8; (msg_len - 6) as usize];
        let read = tokio::select! {
            _ = shutdown.changed() => return "server shutdown".to_string(),
            result = timeout(idle, reader.read_exact(&mut body)) => result,
        };
        match read {
            Ok(Ok(_)) => {}
            Ok(Err(e)) if e.kind() == ErrorKind::UnexpectedEof => {
                return "connection closed in the middle of a BMP message".to_string();
            }
            Ok(Err(e)) => return format!("read error: {e}"),
            Err(_) => return format!("idle timeout ({}s)", idle.as_secs()),
        }

        let mut raw = Vec::with_capacity(msg_len as usize);
        raw.extend_from_slice(&header);
        raw.extend_from_slice(&body);
        let raw_len = raw.len();
        let (hex, truncated) = hex_preview(&raw, MAX_RAW_PREVIEW);
        let mut data = Bytes::from(raw);

        match parse_bmp_msg(&mut data) {
            Ok(msg) => state.ingest(conn_id, addr, msg, raw_len, hex, truncated),
            Err(e) => state.ingest_error(
                conn_id,
                addr.ip(),
                "parse_error",
                msg_type,
                format!("bgpkit-parser: {e}"),
                raw_len,
                hex,
                truncated,
            ),
        }
    }
}

fn configure_keepalive(stream: &TcpStream) {
    let socket = SockRef::from(stream);
    if let Err(e) = socket.set_keepalive(true) {
        debug!("failed to enable TCP keepalive: {e}");
        return;
    }
    let keepalive = TcpKeepalive::new()
        .with_time(Duration::from_secs(60))
        .with_interval(Duration::from_secs(20))
        .with_retries(3);
    if let Err(e) = socket.set_tcp_keepalive(&keepalive) {
        debug!("failed to configure TCP keepalive: {e}");
    }
}

fn hex_preview(bytes: &[u8], limit: usize) -> (String, bool) {
    let n = bytes.len().min(limit);
    let mut out = String::with_capacity(n * 2);
    for b in &bytes[..n] {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    (out, bytes.len() > limit)
}
