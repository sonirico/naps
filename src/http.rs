use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use tokio_util::sync::CancellationToken;

use crate::metrics::Metrics;

const OPENMETRICS_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

#[derive(Clone)]
pub struct HttpState {
    pub source: async_nats::Client,
    pub destination: async_nats::Client,
    pub metrics: Arc<Metrics>,
    pub draining: Arc<AtomicBool>,
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    state: HttpState,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(state);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .context("serve http")
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn readyz(State(state): State<HttpState>) -> impl IntoResponse {
    let connected = state.source.connection_state() == async_nats::connection::State::Connected
        && state.destination.connection_state() == async_nats::connection::State::Connected;
    if connected && !state.draining.load(Ordering::SeqCst) {
        (StatusCode::OK, "ready")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready")
    }
}

async fn metrics(State(state): State<HttpState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, OPENMETRICS_CONTENT_TYPE)],
        state.metrics.encode(),
    )
}
