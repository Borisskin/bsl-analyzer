//! End-to-end contract for the Streamable HTTP transport.
//!
//! The server is deliberately built before the transport starts: the future CLI
//! owns validation, project locking, listener binding, and the one final
//! `McpServer::shutdown` call. This suite verifies only the transport boundary.

use std::{net::SocketAddr, time::Duration};

use mcp_server::{serve_http, McpProfile, McpServer, SharedState, MAX_HTTP_REQUEST_BODY_BYTES};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HOST};
use rmcp::{
    model::CallToolRequestParams, service::RunningService,
    transport::StreamableHttpClientTransport, RoleClient, ServiceExt,
};
use serde_json::Value;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;

const TEST_TIMEOUT: Duration = Duration::from_secs(15);
const MCP_ACCEPT: &str = "application/json, text/event-stream";

type Client = RunningService<RoleClient, ()>;

struct TestServer {
    address: SocketAddr,
    cancellation: CancellationToken,
    task: JoinHandle<anyhow::Result<()>>,
}

impl TestServer {
    async fn start(allowed_hosts: Vec<String>) -> Self {
        Self::start_with_state(McpProfile::Reference, SharedState::reference(None), allowed_hosts)
            .await
    }

    async fn start_with_state(
        profile: McpProfile,
        state: SharedState,
        allowed_hosts: Vec<String>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("test listener should bind");
        let address = listener.local_addr().expect("bound listener has an address");
        let cancellation = CancellationToken::new();
        let server = McpServer::new(profile, state);

        let task_cancellation = cancellation.clone();
        let task = tokio::spawn(async move {
            serve_http(listener, server, profile, address, allowed_hosts, task_cancellation).await
        });

        Self { address, cancellation, task }
    }

    fn mcp_url(&self) -> String {
        format!("http://{}/mcp", self.address)
    }

    fn health_url(&self) -> String {
        format!("http://{}/health", self.address)
    }

    async fn connect(&self) -> Client {
        let transport = StreamableHttpClientTransport::from_uri(self.mcp_url());
        tokio::time::timeout(TEST_TIMEOUT, ().serve(transport))
            .await
            .expect("HTTP initialize should not hang")
            .expect("HTTP client should initialize")
    }

    async fn stop(self) {
        self.cancellation.cancel();
        tokio::time::timeout(TEST_TIMEOUT, self.task)
            .await
            .expect("HTTP graceful shutdown should not hang")
            .expect("HTTP task should not panic")
            .expect("HTTP server should stop cleanly");
    }
}

fn loopback_allowed_hosts() -> Vec<String> {
    vec!["127.0.0.1".to_owned(), "localhost".to_owned()]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_initializes_lists_tools_and_calls_a_safe_tool() {
    let server = TestServer::start(Vec::new()).await;
    let client = server.connect().await;

    assert!(client.peer_info().is_some(), "initialize should return server information");

    let tools = tokio::time::timeout(TEST_TIMEOUT, client.list_tools(Default::default()))
        .await
        .expect("tools/list should not hang")
        .expect("tools/list should succeed");
    assert!(
        tools.tools.iter().any(|tool| tool.name == "search"),
        "reference profile should publish the search tool"
    );

    let mut arguments = serde_json::Map::new();
    arguments.insert("action".to_owned(), Value::String("status".to_owned()));
    tokio::time::timeout(
        TEST_TIMEOUT,
        client.call_tool(CallToolRequestParams::new("search").with_arguments(arguments)),
    )
    .await
    .expect("safe tools/call should not hang")
    .expect("safe tools/call should succeed");

    client.cancel().await.expect("client session should close");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn workspace_http_preserves_the_metadata_meta_type_contract() {
    let project = tempfile::tempdir().expect("temporary workspace should be created");
    let state = SharedState::workspace(project.path().to_path_buf())
        .expect("temporary workspace should initialize");
    let server =
        TestServer::start_with_state(McpProfile::Workspace, state, loopback_allowed_hosts()).await;
    let client = server.connect().await;

    let tools = tokio::time::timeout(TEST_TIMEOUT, client.list_tools(Default::default()))
        .await
        .expect("tools/list should not hang")
        .expect("tools/list should succeed");
    let metadata = tools
        .tools
        .iter()
        .find(|tool| tool.name == "metadata")
        .expect("workspace profile should publish the metadata tool");
    assert!(
        metadata
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.contains_key("meta_type")),
        "HTTP tools/list must preserve the metadata.meta_type input contract"
    );

    client.cancel().await.expect("client session should close");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_clients_use_the_same_ready_server() {
    let server = TestServer::start(loopback_allowed_hosts()).await;
    let first = server.connect().await;
    let second = server.connect().await;

    let (first_tools, second_tools) =
        tokio::join!(first.list_tools(Default::default()), second.list_tools(Default::default()));
    assert!(first_tools.expect("first client should list tools").tools.len() >= 3);
    assert!(second_tools.expect("second client should list tools").tools.len() >= 3);

    first.cancel().await.expect("first session should close independently");
    assert!(
        second.list_tools(Default::default()).await.is_ok(),
        "closing one session must not stop the shared server"
    );
    second.cancel().await.expect("second session should close");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_reports_actual_listener_without_sensitive_state() {
    let server = TestServer::start(loopback_allowed_hosts()).await;

    let response =
        reqwest::get(server.health_url()).await.expect("health request should reach server");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.expect("health should be JSON");

    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["profile"], "reference");
    assert_eq!(body["mode"], "http");
    assert_eq!(body["host"], server.address.ip().to_string());
    assert_eq!(body["port"], u64::from(server.address.port()));
    assert_eq!(body["pid"], u64::from(std::process::id()));
    assert!(body["uptime_seconds"].is_number());
    for forbidden in
        ["source_dir", "workspace_root", "onec_url", "onec_user", "onec_password", "password"]
    {
        assert!(body.get(forbidden).is_none(), "health leaked forbidden field {forbidden}");
    }

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disallowed_host_is_rejected_before_mcp_dispatch() {
    let server = TestServer::start(loopback_allowed_hosts()).await;
    let response = reqwest::Client::new()
        .post(server.mcp_url())
        .header(HOST, "attacker.example")
        .header(ACCEPT, MCP_ACCEPT)
        .header(CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
        .expect("request should receive an HTTP response");

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_request_body_is_rejected() {
    let server = TestServer::start(loopback_allowed_hosts()).await;
    let response = reqwest::Client::new()
        .post(server.mcp_url())
        .header(ACCEPT, MCP_ACCEPT)
        .header(CONTENT_TYPE, "application/json")
        .body(vec![b'x'; MAX_HTTP_REQUEST_BODY_BYTES + 1])
        .send()
        .await
        .expect("oversized request should receive an HTTP response");

    assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_json_does_not_stop_the_server() {
    let server = TestServer::start(loopback_allowed_hosts()).await;
    let client = reqwest::Client::new();
    let malformed = client
        .post(server.mcp_url())
        .header(ACCEPT, MCP_ACCEPT)
        .header(CONTENT_TYPE, "application/json")
        .body("{")
        .send()
        .await
        .expect("malformed request should receive an HTTP response");
    assert!(malformed.status().is_client_error());

    let health =
        client.get(server.health_url()).send().await.expect("server should remain reachable");
    assert_eq!(health.status(), reqwest::StatusCode::OK);

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_stops_an_active_session_and_releases_the_listener() {
    let server = TestServer::start(loopback_allowed_hosts()).await;
    let address = server.address;
    let client = server.connect().await;

    TcpListener::bind(address)
        .await
        .expect_err("the running HTTP server must keep its listener exclusive");

    server.stop().await;

    let rebound = TcpListener::bind(address)
        .await
        .expect("graceful shutdown should release the port even with an active client");
    drop(rebound);
    drop(client);
}
