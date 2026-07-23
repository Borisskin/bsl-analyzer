use std::{net::SocketAddr, sync::Arc, time::Instant};

use axum::{extract::State, routing::get, Json, Router};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;

use crate::{McpProfile, McpServer};

/// Maximum accepted HTTP request body. MCP requests are small JSON-RPC
/// envelopes; a finite limit prevents unbounded buffering before dispatch.
pub const MAX_HTTP_REQUEST_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
struct HealthState {
    profile: McpProfile,
    address: SocketAddr,
    started_at: Instant,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    profile: &'static str,
    mode: &'static str,
    host: String,
    port: u16,
    pid: u32,
    uptime_seconds: u64,
}

async fn health(State(state): State<HealthState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        profile: state.profile.as_str(),
        mode: "http",
        host: state.address.ip().to_string(),
        port: state.address.port(),
        pid: std::process::id(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
    })
}

/// Serve stateful MCP sessions and a readiness endpoint on an already-bound
/// listener. Binding remains the caller's responsibility so startup can fail
/// before expensive shared state is constructed.
pub async fn serve_http(
    listener: TcpListener,
    server: McpServer,
    profile: McpProfile,
    address: SocketAddr,
    allowed_hosts: Vec<String>,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let config =
        StreamableHttpServerConfig::default().with_cancellation_token(cancellation.clone());
    let config =
        if allowed_hosts.is_empty() { config } else { config.with_allowed_hosts(allowed_hosts) };
    let mcp = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let health_state = HealthState { profile, address, started_at: Instant::now() };

    let app = Router::new()
        .route_service("/mcp", mcp)
        .route("/health", get(health))
        .with_state(health_state)
        .layer(RequestBodyLimitLayer::new(MAX_HTTP_REQUEST_BODY_BYTES));

    axum::serve(listener, app).with_graceful_shutdown(cancellation.cancelled_owned()).await?;
    Ok(())
}
