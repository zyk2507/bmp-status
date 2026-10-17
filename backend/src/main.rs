mod admin;
mod api;
mod assets;
mod bmp;
mod config;
mod demo;
mod model;
mod runtime_config;
mod state;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Arc::new(config::Config::parse());

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bmp_status=info,tower_http=warn".into()),
        )
        .with_target(false)
        .init();

    let admin = Arc::new(admin::AdminStore::load(
        &config.data_dir,
        config.admin_password.as_deref(),
        config.cookie_secure,
    )?);
    let runtime = Arc::new(runtime_config::RuntimeConfigStore::load(&config.data_dir)?);
    let state = Arc::new(state::AppState::new(config.clone(), admin, runtime));

    let bmp_listener = tokio::net::TcpListener::bind(config.bmp_bind)
        .await
        .with_context(|| format!("failed to bind BMP listener on {}", config.bmp_bind))?;
    let http_listener = tokio::net::TcpListener::bind(config.http_bind)
        .await
        .with_context(|| format!("failed to bind HTTP listener on {}", config.http_bind))?;

    info!("BMP listener bound to {}", config.bmp_bind);
    info!("Dashboard & API on http://{}", config.http_bind);
    match config.frontend_dir.as_ref() {
        Some(dir) => info!("Frontend: disk override at {}", dir.display()),
        None => info!("Frontend: embedded in binary"),
    }
    info!("Admin data directory: {}", config.data_dir.display());

    let bmp_task = tokio::spawn(bmp::serve(bmp_listener, state.clone(), config.clone()));
    let api_task = tokio::spawn(api::serve(http_listener, state.clone()));

    if config.demo {
        match demo::start(&state) {
            Ok(_) => info!("synthetic BMP demo feed started (--demo)"),
            Err(e) => warn!("failed to start synthetic BMP demo feed: {e}"),
        }
    }

    wait_for_signal().await;
    info!("shutdown signal received, draining connections...");
    demo::stop(&state);
    state.shutdown();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let drained = tokio::time::timeout(Duration::from_secs(6), async {
        let _ = bmp_task.await;
        let _ = api_task.await;
    })
    .await;
    if drained.is_err() {
        warn!("timed out waiting for tasks to shut down");
    }
    info!("bmp-status stopped");
    Ok(())
}

async fn wait_for_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!("failed to install Ctrl-C handler: {e}");
        }
    };

    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(e) => {
                    warn!("failed to install SIGTERM handler: {e}");
                    ctrl_c.await;
                    return;
                }
            };
        tokio::select! {
            _ = ctrl_c => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    ctrl_c.await;
}
