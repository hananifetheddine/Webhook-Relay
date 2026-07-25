#![forbid(unsafe_code)]

use std::time::Duration;

use anyhow::Context;
use sqlx::mysql::MySqlPoolOptions;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

use webhook_relay::{AppState, Config, migrate, router, worker};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("webhook_relay=info,tower_http=info")),
        )
        .init();

    let config = Config::from_env().context("invalid configuration")?;
    let pool = MySqlPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .context("database connection failed")?;
    migrate(&pool).await.context("database migration failed")?;

    let bind_addr = config.bind_addr.clone();
    let state = AppState::new(pool, config).context("failed to build application state")?;
    let app = router(state.clone());

    let listener = TcpListener::bind(&bind_addr)
        .await
        .context("failed to bind listener")?;
    tracing::info!(%bind_addr, "webhook relay listening");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker_task = tokio::spawn(worker::run(state, shutdown_rx));
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_signal().await;
        let _ = shutdown_tx.send(true);
    });

    let result = server.await.context("http server failed");
    let _ = worker_task.await;
    result
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(err) => {
                    tracing::error!(error = %err, "failed to install SIGTERM handler");
                    if let Err(err) = ctrl_c.await {
                        tracing::error!(error = %err, "ctrl-c listener failed");
                    }
                    return;
                }
            };
        tokio::select! {
            result = ctrl_c => {
                if let Err(err) = result {
                    tracing::error!(error = %err, "ctrl-c listener failed");
                }
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        if let Err(err) = ctrl_c.await {
            tracing::error!(error = %err, "ctrl-c listener failed");
        }
    }

    tracing::info!("shutdown signal received");
}
