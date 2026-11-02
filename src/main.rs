use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use naps::config::Config;
use naps::connect::connect;
use naps::http::{HttpState, serve};
use naps::metrics::Metrics;
use naps::pipeline::run_route;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "/etc/naps/naps.yaml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("install rustls crypto provider"))?;

    info!(config = %cli.config.display(), "starting naps");

    let config = match Config::load(&cli.config) {
        Ok(config) => config,
        Err(err) => {
            error!(error = format!("{err:#}"), "invalid config");
            std::process::exit(1);
        }
    };
    info!(routes = config.routes.len(), "config loaded");

    let source = match connect(&config.source, "naps-source").await {
        Ok(client) => client,
        Err(err) => {
            error!(error = format!("{err:#}"), "connect source");
            std::process::exit(1);
        }
    };
    let destination = match connect(&config.destination, "naps-destination").await {
        Ok(client) => client,
        Err(err) => {
            error!(error = format!("{err:#}"), "connect destination");
            std::process::exit(1);
        }
    };

    let cancel = CancellationToken::new();
    let http_cancel = CancellationToken::new();
    let metrics = Arc::new(Metrics::new());
    let draining = Arc::new(AtomicBool::new(false));

    let listener = match tokio::net::TcpListener::bind(config.http.listen).await {
        Ok(listener) => listener,
        Err(err) => {
            error!(error = format!("{err:#}"), listen = %config.http.listen, "bind http");
            std::process::exit(1);
        }
    };
    let http = tokio::spawn(serve(
        listener,
        HttpState {
            source: source.clone(),
            destination: destination.clone(),
            metrics: metrics.clone(),
            draining: draining.clone(),
        },
        http_cancel.clone(),
    ));

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let signal_cancel = cancel.clone();
    let signal_draining = draining.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = sigint.recv() => {}
        }
        info!("shutdown signal received");
        signal_draining.store(true, Ordering::SeqCst);
        signal_cancel.cancel();
    });

    let mut routes = JoinSet::new();
    for route in config.routes {
        let (ready, _) = oneshot::channel();
        routes.spawn(run_route(
            route,
            source.clone(),
            destination.clone(),
            cancel.clone(),
            ready,
            metrics.clone(),
        ));
    }

    let mut failed = false;
    while let Some(joined) = routes.join_next().await {
        let result = joined
            .map_err(anyhow::Error::from)
            .and_then(|result| result);
        if let Err(err) = result {
            error!(error = format!("{err:#}"), "route failed");
            failed = true;
            cancel.cancel();
        }
    }

    if let Err(err) = destination.flush().await {
        error!(error = format!("{err:#}"), "flush destination");
    }
    if let Err(err) = source.drain().await {
        error!(error = format!("{err:#}"), "drain source");
    }
    if let Err(err) = destination.drain().await {
        error!(error = format!("{err:#}"), "drain destination");
    }

    http_cancel.cancel();
    match http.await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => error!(error = format!("{err:#}"), "http server"),
        Err(err) => error!(error = format!("{err:#}"), "http task"),
    }

    if failed {
        std::process::exit(1);
    }
    Ok(())
}
