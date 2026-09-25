//! End-to-end `metadata` over the real MCP transport, covering how a consumer tells
//! "not ready yet" from an answer.
//!
//! The distinction has to be readable from a field. A retry envelope a consumer fails to
//! recognize is handed to a model as prose: it spends a turn on "повторите запрос" or, worse,
//! reads it as the answer and concludes the object does not exist. So this suite classifies
//! responses exactly the way a client must — by `structuredContent.status` — and never by
//! matching the sentence, which can also occur inside a module body and reach a tool's output
//! as a genuine result.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use mcp_server::{serve_stream, McpProfile, McpServer, OnecConnection, SharedState};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

type Client = RunningService<RoleClient, ()>;

/// A minimal but valid configuration in a scratch dir, so the resident's on-disk caches never
/// land in the repo tree and every run starts cold.
fn stage_workspace() -> TempDir {
    let dir = TempDir::new().expect("scratch workspace");
    let root = dir.path();
    std::fs::write(
        root.join("Configuration.xml"),
        "<Configuration><Name>ТестоваяКонфигурация</Name></Configuration>",
    )
    .expect("write Configuration.xml");
    let module = root.join("CommonModules").join("Сервер").join("Ext");
    std::fs::create_dir_all(&module).expect("mkdir module");
    std::fs::write(
        root.join("CommonModules").join("Сервер.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\">\n\
         \t<CommonModule uuid=\"00000000-0000-0000-0000-000000000001\">\n\
         \t\t<Properties><Name>Сервер</Name><Server>true</Server></Properties>\n\
         \t</CommonModule>\n\
         </MetaDataObject>\n",
    )
    .expect("write module descriptor");
    std::fs::write(
        module.join("Module.bsl"),
        "&НаСервере\nФункция Считать() Экспорт Возврат 1; КонецФункции\n",
    )
    .expect("write module body");
    std::fs::create_dir_all(root.join("CommonForms").join("Настройки")).expect("mkdir common form");
    dir
}

async fn workspace_client(root: &Path) -> Client {
    let state = SharedState::workspace(root.to_path_buf()).expect("valid workspace project");
    workspace_client_with_state(state).await
}

async fn workspace_client_with_state(state: SharedState) -> Client {
    let server = McpServer::new(McpProfile::Workspace, state);
    let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
    tokio::spawn(serve_stream(server, server_io));
    ().serve(client_io).await.expect("session initialized")
}

struct RejectingLiveService {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl RejectingLiveService {
    async fn start() -> Self {
        async fn reject(
            State(requests): State<Arc<Mutex<Vec<Value>>>>,
            Json(body): Json<Value>,
        ) -> (StatusCode, &'static str) {
            requests.lock().expect("request log").push(body);
            (StatusCode::BAD_REQUEST, "unsupported metadata collection")
        }

        let requests = Arc::new(Mutex::new(Vec::new()));
        let app =
            Router::new().route("/metadata-structure", post(reject)).with_state(requests.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind live fixture");
        let url = format!("http://{}", listener.local_addr().expect("fixture address"));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve live fixture");
        });
        Self { url, requests, task }
    }
}

impl Drop for RejectingLiveService {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn call(client: &Client, args: &[(&str, Value)]) -> CallToolResult {
    let args: Map<String, Value> =
        args.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect();
    client
        .call_tool(CallToolRequestParams::new("metadata").with_arguments(args))
        .await
        .expect("transport ok")
}

fn text_of(result: &CallToolResult) -> &str {
    result.content[0].as_text().expect("text content").text.as_str()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_list_publishes_the_mode_dependent_object_type_contract() {
    let ws = stage_workspace();
    let client = workspace_client(ws.path()).await;
    let tools = client.list_tools(Default::default()).await.expect("tools/list");
    let metadata = tools.tools.iter().find(|tool| tool.name == "metadata").expect("metadata tool");

    let description = metadata
        .description
        .as_deref()
        .expect("metadata description")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(description.contains("source mode or auto without `connection`"), "{description}");
    assert!(description.contains("infobase mode or auto with `connection`"), "{description}");
    assert!(description.contains("source-only managed form"), "{description}");

    let properties = metadata.input_schema["properties"].as_object().expect("input properties");
    let object_type = properties["object_type"]["description"]
        .as_str()
        .expect("object_type doc")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(object_type.contains("`mode=source`"), "{object_type}");
    assert!(object_type.contains("`mode=infobase`"), "{object_type}");
    assert!(object_type.contains("without singular/plural conversion"), "{object_type}");

    let mut fields: Vec<&str> = properties.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        [
            "action",
            "connection",
            "filter",
            "form_name",
            "max_items",
            "max_output_tokens",
            "meta_type",
            "mode",
            "name_mask",
            "object_name",
            "object_type",
        ],
        "the documentation-only change must not alter the input schema",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_and_auto_without_connection_keep_form_on_the_source_path() {
    let ws = stage_workspace();
    let live = RejectingLiveService::start().await;
    let mut state = SharedState::workspace(ws.path().to_path_buf()).expect("valid workspace");
    state.add_onec_connection(
        "live".into(),
        OnecConnection::new(onec_client::Client::new(&live.url, "", ""), false),
    );
    let client = workspace_client_with_state(state).await;

    let source = call(
        &client,
        &[
            ("action", json!("form")),
            ("mode", json!("source")),
            ("connection", json!("live")),
            ("object_type", json!("CommonForm")),
        ],
    )
    .await;
    assert!(text_of(&source).contains("Настройки"), "explicit source mode must read XML");

    let auto = call(
        &client,
        &[("action", json!("form")), ("mode", json!("auto")), ("object_type", json!("CommonForm"))],
    )
    .await;
    assert!(text_of(&auto).contains("Настройки"), "auto without connection must read XML");
    assert!(live.requests.lock().expect("request log").is_empty(), "source paths touched live 1C");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn infobase_and_auto_with_connection_pass_object_type_through_once() {
    let ws = stage_workspace();
    let live = RejectingLiveService::start().await;
    let mut state = SharedState::workspace(ws.path().to_path_buf()).expect("valid workspace");
    state.add_onec_connection(
        "live".into(),
        OnecConnection::new(onec_client::Client::new(&live.url, "", ""), false),
    );
    let client = workspace_client_with_state(state).await;

    for (mode, object_type) in [("infobase", "Справочники"), ("auto", "Справочник")]
    {
        let arguments = Map::from_iter([
            ("action".into(), json!("object")),
            ("mode".into(), json!(mode)),
            ("connection".into(), json!("live")),
            ("object_type".into(), json!(object_type)),
            ("object_name".into(), json!("Партнеры")),
        ]);
        let error = client
            .call_tool(CallToolRequestParams::new("metadata").with_arguments(arguments))
            .await
            .expect_err("the fixture rejects the live request");
        assert!(error.to_string().contains("unsupported metadata collection"), "{error}");
    }

    assert_eq!(
        *live.requests.lock().expect("request log"),
        [
            json!({"meta_type": "Справочники", "name": "Партнеры"}),
            json!({"meta_type": "Справочник", "name": "Партнеры"}),
        ],
        "plural and invalid singular forms must each reach the live service unchanged, once",
    );
}

/// A call issued while the resident builds answers with the retry envelope, and one issued
/// after it is ready answers with the configuration summary — both distinguishable by
/// `structuredContent.status` alone. The small fixture may finish loading before the first
/// response; the loading envelope itself is also covered by the metadata unit test.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cold_info_call_is_classifiable_by_status_alone() {
    let ws = stage_workspace();
    let client = workspace_client(ws.path()).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let result = call(&client, &[("action", json!("info"))]).await;
        let loading =
            result.structured_content.as_ref().is_some_and(|body| body["status"] == "loading");
        if !loading {
            let text = text_of(&result);
            assert!(
                text.starts_with("# Конфигурация:") && text.contains("Общие модули: 1"),
                "a ready `info` answers with the configuration summary: {text}",
            );
            break;
        }

        let body = result.structured_content.as_ref().expect("the envelope is structured");
        assert!(
            body["detail"].as_str().is_some_and(|detail| !detail.is_empty()),
            "the retry envelope says why it is not ready: {body}",
        );
        assert!(
            text_of(&result).starts_with("Метаданные загружаются"),
            "the human sentence stays alongside the envelope: {}",
            text_of(&result),
        );
        assert!(tokio::time::Instant::now() < deadline, "the resident never became ready: {body}",);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `status` answers off the lifecycle, not off a built resident, so it must report a state
/// immediately — that is what makes polling it cheaper than firing `info` and reading the
/// envelope. Its shape is the resident's, shared with `diagnostics status`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_reports_the_resident_lifecycle_right_away() {
    let ws = stage_workspace();
    let client = workspace_client(ws.path()).await;

    let result = call(&client, &[("action", json!("status"))]).await;
    let body = result.structured_content.as_ref().expect("status is structured");

    let state = body["state"].as_str().expect("status reports a state");
    assert!(
        matches!(state, "idle" | "loading" | "ready"),
        "an unexpected lifecycle state for a live workspace: {body}",
    );
    assert!(body["generation"].is_number(), "status carries the resident generation: {body}");
    assert!(body["reload"].is_string(), "status carries the reload state: {body}");
}

/// Readiness is a property of this server, not of the requested mode. A client with named 1C
/// connections passes `connection` on every metadata call; if `status` were routed behind the
/// infobase branch it would answer "action unavailable in infobase mode" for that client only,
/// which reads as "this build has no status action" rather than "wrong mode".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_answers_even_when_a_connection_is_passed() {
    let ws = stage_workspace();
    let client = workspace_client(ws.path()).await;

    let result =
        call(&client, &[("action", json!("status")), ("connection", json!("производственная"))])
            .await;
    let body = result.structured_content.as_ref().expect("status is structured");

    assert!(body["state"].is_string(), "status still reports the resident lifecycle: {body}");
}
