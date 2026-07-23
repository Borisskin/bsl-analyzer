mod baseline;
pub mod broker;
mod cache;
mod change_hub;
mod diagnostics_state;
mod drift_classify;
mod graph;
mod graph_db;
mod graph_query;
mod http;
mod state;
mod tools;

pub use baseline::{
    resolve_project_baseline_diagnostics, BaselineConfigDiagnostics, BaselineResolutionSummary,
};
pub use cache::graph_db_path;
pub use graph_db::{
    read_source_root_scoped_sqlite_method_call_digest, read_sqlite_method_call_digest,
};
pub use graph_query::{GraphDb, GraphDbContextProvider};
pub use http::{serve_http, MAX_HTTP_REQUEST_BODY_BYTES};
use state::WorkspaceSearchMode;
pub use state::{OnecConnection, SharedState};

pub async fn serve_stdio(server: McpServer) -> anyhow::Result<()> {
    serve_stream(server, rmcp::transport::stdio()).await
}

/// Serve one MCP session over an arbitrary bidirectional transport. The transport
/// carries framed JSON-RPC; MCP handling is identical whether the bytes come from
/// stdio (`serve_stdio`) or a local socket (the broker). This is the single seam
/// stdio and socket serving share.
pub async fn serve_stream<T, A>(server: McpServer, transport: T) -> anyhow::Result<()>
where
    T: rmcp::transport::IntoTransport<rmcp::RoleServer, std::io::Error, A>,
{
    use rmcp::ServiceExt;
    let session = server.serve(transport).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    session.waiting().await.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

use crate::graph::GraphStatus;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpProfile {
    Workspace,
    Reference,
}

impl McpProfile {
    /// Stable lowercase tag for the profile. Used as part of the broker backend
    /// identity, so it must stay byte-stable across releases.
    pub fn as_str(self) -> &'static str {
        match self {
            McpProfile::Workspace => "workspace",
            McpProfile::Reference => "reference",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct MetadataParams {
    /// info | tree | object | form.
    action: String,
    /// `tree`: case-insensitive substring to narrow the returned tree (optional).
    filter: Option<String>,
    /// `tree` in infobase mode: metadata collection, e.g. `Справочники`/`Documents`.
    meta_type: Option<String>,
    /// `tree` in infobase mode: case-insensitive object name/synonym substring.
    name_mask: Option<String>,
    /// `tree` in infobase mode: maximum returned objects (default 100, max 1000).
    max_items: Option<u32>,
    /// Metadata object type, e.g. `Документ`/`Справочник`/`ОбщийМодуль`. Required for
    /// `object` and `form`.
    object_type: Option<String>,
    /// Metadata object name, e.g. `ЗаказКлиента`. Required for `object`; for `form` it
    /// selects the owner object (omit for a configuration-level common form).
    object_name: Option<String>,
    /// `form`: managed-form name (optional; omit for the object's default form).
    form_name: Option<String>,
    /// `tree` (filtered listing): output budget in tokens (~4 chars each); an over-budget
    /// listing is truncated at a line boundary with a continuation note (default 6000).
    max_output_tokens: Option<usize>,
    /// Named live 1C connection for `mode=infobase` (optional).
    connection: Option<String>,
    /// auto | source | infobase (default auto).
    mode: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct SearchParams {
    /// Workspace profile: `search_code` | `status`. Reference profile: `find_docs` |
    /// `search_docs` | `status`.
    action: String,
    /// Free-text query. Required for `search_code`/`find_docs`/`search_docs`.
    query: Option<String>,
    /// Cap on returned hits (default 10, max 50).
    limit: Option<usize>,
    /// Output budget in tokens (~4 chars each); over-budget results are truncated at a hit
    /// boundary with a note telling you to raise `limit` or narrow the query (default 6000).
    max_output_tokens: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct SyntaxHelpParams {
    /// Platform member name to look up, e.g. `СтрНайти` or a type method.
    name: String,
    /// Owning platform type when `name` is a member of a specific type (optional).
    type_name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    /// validate | execute | schema.
    action: String,
    /// SDBL text — required for `validate`/`execute`, omitted for `schema`.
    query: Option<String>,
    /// `execute`: cap on returned rows (optional).
    limit: Option<u32>,
    /// `execute`: named SDBL query parameters (`&Param` → value) (optional).
    parameters: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Named live 1C connection (optional when only one/default connection exists).
    connection: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ExecuteParams {
    /// check | run | eval.
    action: String,
    /// BSL source to `check`/`run`, or the single expression to `eval`.
    code: String,
    /// Named live 1C connection (optional when only one/default connection exists).
    connection: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct EventLogParams {
    /// Lower time bound (inclusive), ISO-8601, e.g. `2026-07-05T00:00:00` or `2026-07-05`.
    date_from: Option<String>,
    /// Upper time bound (inclusive), ISO-8601.
    date_to: Option<String>,
    /// Severity: Информация/Предупреждение/Ошибка/Примечание or Information/Warning/Error/Note.
    level: Option<String>,
    /// Infobase user name (deleted users can only be matched by name).
    user: Option<String>,
    /// Event name, e.g. `_$Session$_.Authentication` or a metadata event like `_$Data$_.Post`.
    event: Option<String>,
    /// Full metadata name to filter by, e.g. `Документ.ЗаказКлиента`.
    metadata: Option<String>,
    /// Case-insensitive substring filter over the comment/data columns, applied AFTER the
    /// platform read — so it narrows the already-`limit`-capped newest window, it does not
    /// scan the whole log. Widen `limit` if a match may lie deeper.
    contains: Option<String>,
    /// Max records (newest first), default 100, capped at 1000.
    limit: Option<u32>,
    /// Named live 1C connection (optional when only one/default connection exists).
    connection: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct GraphParams {
    /// overview | schema | status | node | source | neighbors | callers | callees | resolve
    action: String,
    /// Durable node id (required for node/neighbors/callers/callees).
    id: Option<String>,
    /// Imprecise lookup string (required for `resolve`): wrong casing, a bare method/object
    /// name, or a partial id.
    query: Option<String>,
    /// Durable node ids (required for `source`).
    #[serde(default)]
    ids: Vec<String>,
    /// Output budget in tokens (~4 chars each) for source-bearing actions: `source`
    /// (default 4000) and `node`/`neighbors` at `detail=bodies` (default 6000). When the
    /// body output is truncated the response carries `budget_exhausted: true`.
    max_output_tokens: Option<usize>,
    /// names | signatures | bodies (default: signatures).
    detail: Option<String>,
    /// in | out | both — only for `neighbors` (default: in).
    dir: Option<String>,
    /// Traversal depth for neighbors (default: 1).
    depth: Option<usize>,
    /// Server-side cap on returned neighbour nodes (default: 50).
    max_nodes: Option<usize>,
    /// Keep only edges with these provenances (resolved/inferred/visibility_blocked/unresolved).
    #[serde(default)]
    provenance: Vec<String>,
    /// Keep only edges of these kinds (call/manager_creates/manager_access/query_ref/
    /// contains/data_binding) — lets metadata-impact queries isolate e.g. only `query_ref`.
    #[serde(default)]
    edge_kinds: Vec<String>,
    /// How many top-centrality methods to include in `overview` (default: 20).
    top: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct SymbolInfoParams {
    /// Qualified name of the symbol (primary input): a common-module method
    /// (`ОбщегоНазначения.ЗначениеРеквизитаОбъекта`), a metadata object (`Справочник.Товары`)
    /// or its attribute (`Справочник.Товары.Артикул`), an object/manager module method
    /// (`Документ.ЗаказКлиента.Провести`), or a platform member (`СтрНайти`, `Массив.Добавить`).
    /// Case-insensitive; the MdoType keyword accepts singular or plural, RU or EN.
    symbol: Option<String>,
    /// Positional fallback for locals/parameters that have no qualified name: absolute or
    /// workspace-relative `.bsl` path. Requires `line`.
    path: Option<String>,
    /// `path`: 0-based line of the symbol occurrence.
    line: Option<u32>,
    /// `path`: 0-based character offset within the line of the symbol occurrence (default 0).
    column: Option<u32>,
    /// Card sections to include: any of `definition` | `type` | `doc`. Empty = all. `usages`
    /// is always a summary and is added when the call graph is ready.
    #[serde(default)]
    include: Vec<String>,
    /// Type/label language: `ru` (default) or `en`.
    locale: Option<String>,
    /// Output budget in tokens (~4 chars each); an over-budget member list is trimmed and the
    /// response carries `truncated: true` with a `budget_hint` (default 6000).
    max_output_tokens: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct DiagnosticsParams {
    /// catalog | schema | status | file | workspace.
    action: String,
    /// `file`: absolute or workspace-relative `.bsl` path.
    path: Option<String>,
    /// `catalog`: narrow to these codes. `file`: keep only these codes.
    #[serde(default)]
    codes: Vec<String>,
    /// `catalog`: ru | en (default ru) — title language.
    locale: Option<String>,
    /// `file`: inclusive severity floor error|warning|info|hint (default warning).
    min_severity: Option<String>,
    /// `file`: 0-based first line to include (optional).
    range_start: Option<usize>,
    /// `file`: 0-based last line to include (optional).
    range_end: Option<usize>,
    /// `file`: concise | detailed (default concise).
    detail: Option<String>,
    /// `file`: cap on returned findings (default 200).
    max_findings: Option<usize>,
    /// `workspace`: cap on files swept (default 1000).
    max_files: Option<usize>,
    /// `catalog`/`file`/`workspace`: output budget in tokens (~4 chars each); a truncated
    /// response carries `budget_exhausted: true` and a `budget_hint` on how to narrow it
    /// (tighten `codes`/`min_severity`/range or raise the budget). When omitted, no token
    /// budget applies — only the action's own count caps (`max_findings`/`max_files`).
    max_output_tokens: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct ItsHelpParams {
    /// Natural-language question for the ITS expert help.
    question: String,
}

#[derive(Deserialize, JsonSchema)]
struct DebugParams {
    /// attach | disconnect | set_breakpoint | remove_breakpoint | continue | step |
    /// wait_stop | stack_trace | locals | eval.
    action: String,
    /// `attach`: debugger host (required).
    host: Option<String>,
    /// `attach`: debugger port (default 1550).
    port: Option<u16>,
    /// `attach`: infobase name (required).
    infobase: Option<String>,
    /// `attach`: configuration source root (optional).
    config_root: Option<String>,
    /// `attach`: `[name, root]` pairs for loaded extensions (optional).
    #[serde(default)]
    extensions: Vec<[String; 2]>,
    /// `attach`: object-name patterns to auto-attach on connect (optional).
    #[serde(default)]
    auto_attach: Vec<String>,
    /// `set_breakpoint`/`remove_breakpoint`: target module id (required).
    module: Option<String>,
    /// `set_breakpoint`/`remove_breakpoint`: 1-based line (required).
    line: Option<u32>,
    /// `set_breakpoint`: conditional-breakpoint expression (optional).
    condition: Option<String>,
    /// `step`: `over`/`next`, `in`/`into`, or `out` (required).
    direction: Option<String>,
    /// `wait_stop`: max seconds to wait for a stop event (optional).
    timeout_secs: Option<u64>,
    /// `locals`/`eval`: stack frame level to evaluate in (optional, default top frame).
    stack_level: Option<u32>,
    /// `eval`: BSL expression to evaluate in the current stop (required).
    expression: Option<String>,
}

fn default_debug_port() -> u16 {
    1550
}

fn require<T>(val: Option<T>, field: &str, action: &str) -> Result<T, McpError> {
    val.ok_or_else(|| {
        McpError::invalid_params(format!("'{field}' is required for action '{action}'"), None)
    })
}

#[derive(Clone)]
pub struct McpServer {
    profile: McpProfile,
    state: SharedState,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = workspace_tool_router)]
impl McpServer {
    pub fn new(profile: McpProfile, state: SharedState) -> Self {
        let tool_router = match profile {
            McpProfile::Workspace => Self::workspace_tool_router(),
            McpProfile::Reference => Self::reference_tool_router(),
        };
        Self { profile, state, tool_router }
    }

    pub fn shutdown(&self) {
        self.state.shutdown();
    }

    /// Whether long-running background work is still in flight — a search/embedding
    /// pass OR a diagnostics resident build/reload. The broker backend polls this to
    /// avoid idle-exiting (and killing the work) while it runs, so an expensive
    /// embedding run or a cold diagnostics build finishes instead of restarting on the
    /// next request. See [`SharedState::background_indexing_active`].
    pub fn background_work_active(&self) -> bool {
        self.state.background_indexing_active() || self.state.diagnostics().is_busy()
    }

    /// Browse the configuration's metadata: objects, their structure, and managed forms.
    /// Use to answer "what objects exist / what does object X contain / what is on form Y" —
    /// attributes, tabular sections, forms, types — straight from the metadata substrate. Not
    /// for call relationships (use `graph`) and not for finding code by meaning (use `search`).
    /// Actions: `info` — configuration summary; `tree` — the metadata object tree (filterable);
    /// `object` — one object's structure (needs `object_type` + `object_name`); `form` — a
    /// managed form's layout (needs `object_type`). Reads the resident analysis host; while it
    /// builds it returns a retry envelope, not an error.
    #[tool(name = "metadata", annotations(read_only_hint = true))]
    async fn metadata(
        &self,
        params: Parameters<MetadataParams>,
    ) -> Result<CallToolResult, McpError> {
        use crate::diagnostics_state::{DiagnosticsStatus, ResidentOutcome};

        let p = params.0;
        let mode = p.mode.as_deref().unwrap_or("auto");
        if !matches!(mode, "auto" | "source" | "infobase") {
            return Err(McpError::invalid_params(
                format!("Unknown metadata mode '{mode}'. Expected: auto, source, infobase"),
                None,
            ));
        }
        let live = mode == "infobase" || (mode == "auto" && p.connection.is_some());
        if live {
            return match p.action.as_str() {
                "tree" => {
                    let meta_type = require(p.meta_type, "meta_type", "tree in infobase mode")?;
                    tools::metadata::get_live_metadata_tree(
                        &self.state,
                        p.connection.as_deref(),
                        &meta_type,
                        p.name_mask,
                        p.max_items.unwrap_or(100),
                    )
                    .await
                }
                "object" => {
                    let object_type = require(p.object_type, "object_type", "object")?;
                    let object_name = require(p.object_name, "object_name", "object")?;
                    tools::metadata::get_live_metadata_object(
                        &self.state,
                        p.connection.as_deref(),
                        &object_type,
                        &object_name,
                    )
                    .await
                }
                other => Err(McpError::invalid_params(
                    format!(
                        "Metadata action '{other}' is unavailable in infobase mode. Expected: tree, object"
                    ),
                    None,
                )),
            };
        }

        // `form` reads managed-form XML straight off the configuration source root — data
        // the metadata substrate does not carry — so it needs neither the resident db nor
        // a loaded configuration, and stays available while the resident is building or
        // evicted. `source_root` survives the `MetadataCache` retirement for exactly this.
        if p.action == "form" {
            let object_type = require(p.object_type, "object_type", "form")?;
            return tools::metadata::get_form_structure(
                self.state.source_root().map(|p| p.as_path()),
                &object_type,
                p.object_name.as_deref(),
                p.form_name.as_deref(),
            );
        }

        // `info`/`tree`/`object` read the resident analysis host. Trigger the build if idle
        // or idle-evicted and, while it is not ready, return a "loading, retry" envelope —
        // never a hard "not loaded" error, so an evicted resident degrades to slow, not
        // wrong. Reference/shared profiles have no resident and stay "not configured".
        let diag = self.state.diagnostics().clone();
        diag.ensure_loading();
        match diag.status() {
            DiagnosticsStatus::Disabled => {
                return Err(McpError::invalid_params(
                    "metadata is only available in the workspace profile",
                    None,
                ))
            }
            DiagnosticsStatus::Failed(msg) => {
                return Err(McpError::internal_error(
                    format!("metadata database load failed: {msg}"),
                    None,
                ))
            }
            DiagnosticsStatus::Idle | DiagnosticsStatus::Loading => {
                return Ok(tools::metadata::loading(&diag.status_report()))
            }
            DiagnosticsStatus::Ready { .. } => {}
        }

        let action = p.action.clone();
        let filter = p.filter.clone();
        let object_type = p.object_type.clone();
        let object_name = p.object_name.clone();
        let max_output_tokens =
            p.max_output_tokens.unwrap_or(tools::response::DEFAULT_OUTPUT_BUDGET_TOKENS);

        tokio::task::spawn_blocking(move || {
            let read = |diag: &crate::diagnostics_state::DiagnosticsState| {
                diag.read(|resident, _generation| {
                    let db = resident.db();
                    match action.as_str() {
                        "info" => {
                            let (config, extensions) = tools::metadata::configs_from_db(db);
                            tools::metadata::get_configuration_info(&config, &extensions)
                        }
                        "tree" => {
                            let (config, extensions) = tools::metadata::configs_from_db(db);
                            tools::metadata::get_metadata_tree(
                                &config,
                                &extensions,
                                filter.clone(),
                                max_output_tokens,
                            )
                        }
                        "object" => {
                            let object_type =
                                require(object_type.clone(), "object_type", "object")?;
                            let object_name =
                                require(object_name.clone(), "object_name", "object")?;
                            tools::metadata::object_from_db(db, &object_type, &object_name)
                        }
                        other => Err(McpError::invalid_params(
                            format!("Unknown action '{other}'. Expected: info, tree, object, form"),
                            None,
                        )),
                    }
                })
            };

            let mut outcome = read(&diag);
            // A miss for a VALID object type may be an object added since the last throttled
            // drift scan: force ONE storm-guarded re-scan and retry. A bad object type is
            // returned as-is (reloading cannot fix it, and it must not force a scan).
            if action == "object" {
                if let ResidentOutcome::Ready(Err(_), _) = &outcome {
                    let valid_type = object_type
                        .as_deref()
                        .is_some_and(tools::metadata::is_resolvable_object_type);
                    if valid_type {
                        diag.force_rescan();
                        outcome = read(&diag);
                    }
                }
            }

            match outcome {
                ResidentOutcome::Ready(result, _freshness) => result,
                ResidentOutcome::Loading => Ok(tools::metadata::loading(&diag.status_report())),
                ResidentOutcome::Disabled => Err(McpError::invalid_params(
                    "metadata is only available in the workspace profile",
                    None,
                )),
                ResidentOutcome::Failed(msg) => {
                    Err(McpError::internal_error(format!("metadata database: {msg}"), None))
                }
            }
        })
        .await
        .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
    }

    /// Hybrid lexical + semantic code search across the project source. Use when you need to
    /// find code by meaning or a free-form phrase ("where the reserve for an order is built")
    /// or when the exact symbol name is unknown. Not for walking call relationships — that is
    /// `graph` (callers/callees by durable id) — and not for analyzer findings — that is
    /// `diagnostics`. Actions: `search_code` — the search (`query` required; `limit` default
    /// 10, max 50); `status` — index readiness. While the index warms up it returns a retry
    /// envelope; retry shortly.
    #[tool(name = "search", annotations(read_only_hint = true))]
    async fn workspace_search(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match p.action.as_str() {
            "status" => {
                let engine = self.state.search_engine().clone();
                let progress = self.state.index_progress().clone();
                let semantic_runtime = self.state.semantic_runtime();
                let workspace_search_mode = self.state.workspace_search_mode();
                let overlay_warmup = self
                    .state
                    .overlay_warmup()
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or(crate::state::OverlayWarmupState::Pending);
                let baseline = self.state.baseline_view();
                tokio::task::spawn_blocking(move || {
                    tools::search::search_status(
                        &engine,
                        &progress,
                        &semantic_runtime,
                        workspace_search_mode,
                        overlay_warmup,
                        baseline.configured,
                        baseline.external,
                        baseline.pending,
                    )
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            // `search_code` is the unified lexical+semantic code search (smart-fused: exact-symbol
            // tier then semantic tail).
            "search_code" => {
                let query = require(p.query, "query", &p.action)?;
                let limit = p.limit.unwrap_or(10).min(50);
                let max_output_tokens =
                    p.max_output_tokens.unwrap_or(tools::response::DEFAULT_OUTPUT_BUDGET_TOKENS);
                let engine = self.state.search_engine().clone();
                let semantic_runtime = self.state.semantic_runtime();
                let workspace_search_mode = self.state.workspace_search_mode();
                // A query landing during the deferred baseline connect gets the retry
                // envelope, not the gates' "fix config / restart MCP" errors — those are
                // for resolved-and-broken, this is merely not-resolved-yet. One snapshot
                // feeds both the pending check and the gates, so a publish in between
                // cannot produce a torn pending/configured/external mix.
                let baseline = self.state.baseline_view();
                if matches!(
                    workspace_search_mode,
                    crate::state::WorkspaceSearchMode::PostgresRemoteOverlay
                ) && baseline.pending
                {
                    return Ok(tools::search::baseline_warming_not_ready(
                        self.state.index_progress(),
                    ));
                }
                let configured_baseline = baseline.configured;
                let external_baseline = baseline.external;
                // The graph keys file ids against the repo (workspace) root; pass it so search
                // can mint form/file `graph_id`s with the same `src/cf/…` prefix the graph uses.
                let graph_root = self.state.workspace_root().cloned();
                let index_progress = self.state.index_progress().clone();
                tokio::task::spawn_blocking(move || {
                    tools::search::hybrid_code(
                        &engine,
                        &semantic_runtime,
                        workspace_search_mode,
                        configured_baseline.as_ref(),
                        external_baseline,
                        graph_root.as_deref(),
                        &index_progress,
                        &query,
                        limit,
                        max_output_tokens,
                    )
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            other => Err(McpError::invalid_params(
                format!("Unknown action '{other}'. Expected: search_code, status"),
                None,
            )),
        }
    }

    /// Validate or execute SDBL (the 1C query language) against the configuration schema. Use
    /// to check a query for errors before shipping it, to run a read-only query, or to fetch
    /// the query-language schema. Not for browsing metadata structure (use `metadata`) and not
    /// for BSL code (use `execute`). Actions: `validate` — parse and type-check a query (`query`
    /// required); `execute` — run it (`query` required; optional `limit`, `parameters`);
    /// `schema` — the SDBL schema reference.
    #[tool(name = "query", annotations(read_only_hint = true))]
    async fn query(&self, params: Parameters<QueryParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match p.action.as_str() {
            "schema" => Ok(tools::query::schema()),
            "validate" => {
                let query = require(p.query, "query", "validate")?;
                tools::query::validate_query(&self.state, &query, p.connection.as_deref()).await
            }
            "execute" => {
                let query = require(p.query, "query", "execute")?;
                tools::query::execute_query(
                    &self.state,
                    &query,
                    p.limit,
                    p.parameters,
                    p.connection.as_deref(),
                )
                .await
            }
            other => Err(McpError::invalid_params(
                format!("Unknown action '{other}'. Expected: validate, execute, schema"),
                None,
            )),
        }
    }

    /// Run or syntax-check BSL code in an embedded interpreter. Use to confirm a snippet
    /// compiles, run a small script, or evaluate a single expression. Not for querying the
    /// database (use `query` for SDBL) and not for analyzer findings (use `diagnostics`).
    /// Actions: `check` — syntax-check `code`; `run` — execute `code`; `eval` — evaluate the
    /// single expression in `code`. `run`/`eval` execute code, so this tool is not read-only.
    #[tool(name = "execute")]
    async fn execute(&self, params: Parameters<ExecuteParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match p.action.as_str() {
            "check" => {
                tools::execution::check_syntax(&self.state, &p.code, p.connection.as_deref()).await
            }
            "run" => {
                tools::execution::execute_code(&self.state, &p.code, p.connection.as_deref()).await
            }
            "eval" => {
                tools::execution::eval_expression(&self.state, &p.code, p.connection.as_deref())
                    .await
            }
            other => Err(McpError::invalid_params(
                format!("Unknown action '{other}'. Expected: check, run, eval"),
                None,
            )),
        }
    }

    /// Read the 1C infobase event log (журнал регистрации) through the deployed BSL_Analyzer
    /// extension. Use to inspect runtime events — errors, authentications, data changes —
    /// filtered by time, user, event, metadata object, or severity. Not for static analysis of
    /// source (use `diagnostics`): this reads live runtime records from a running infobase.
    /// Filters: `date_from`/`date_to`, `level`, `user`, `event`, `metadata`, and `contains`
    /// (post-read substring over the newest `limit` window). `limit` is newest-first (default
    /// 100, max 1000). Requires the extension deployed with event-log read rights.
    #[tool(name = "event_log", annotations(read_only_hint = true))]
    async fn event_log(
        &self,
        params: Parameters<EventLogParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        tools::event_log::event_log(
            &self.state,
            tools::event_log::EventLogQuery {
                date_from: p.date_from,
                date_to: p.date_to,
                level: p.level,
                user: p.user,
                event: p.event,
                metadata: p.metadata,
                contains: p.contains,
                limit: p.limit,
                connection: p.connection,
            },
        )
        .await
    }

    /// Drive a live 1C debugger session: attach, set breakpoints, step, and inspect state. Use
    /// to debug a running infobase — attach, break, then step and read locals/eval. Not for
    /// static analysis (use `diagnostics`) and not for running standalone code (use `execute`).
    /// Actions: `attach`/`disconnect`; `set_breakpoint`/`remove_breakpoint`; `continue`/`step`;
    /// `wait_stop`; `stack_trace`; `locals`; `eval`. Requires a reachable debug endpoint
    /// (`host` + `infobase`, default port 1550).
    #[tool(name = "debug")]
    async fn debug(&self, params: Parameters<DebugParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let session = self.state.debug_session().clone();

        match p.action.as_str() {
            "attach" => {
                let host = require(p.host, "host", "attach")?;
                let infobase = require(p.infobase, "infobase", "attach")?;
                let port = p.port.unwrap_or_else(default_debug_port);
                let workspace_root = self.state.workspace_root().cloned();
                let config_root = p.config_root;
                let extensions = p.extensions;
                let auto_attach = p.auto_attach;
                tokio::task::spawn_blocking(move || {
                    tools::debug::debug_attach(
                        &session,
                        tools::debug::AttachParams {
                            host: &host,
                            port,
                            infobase: &infobase,
                            config_root: config_root.as_deref(),
                            workspace_root: workspace_root.as_deref(),
                            extensions: &extensions,
                            auto_attach: &auto_attach,
                        },
                    )
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "disconnect" => {
                tokio::task::spawn_blocking(move || tools::debug::debug_disconnect(&session))
                    .await
                    .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "set_breakpoint" => {
                let module = require(p.module, "module", "set_breakpoint")?;
                let line = require(p.line, "line", "set_breakpoint")?;
                let condition = p.condition;
                tokio::task::spawn_blocking(move || {
                    tools::debug::debug_set_breakpoint(
                        &session,
                        &module,
                        line,
                        condition.as_deref(),
                    )
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "remove_breakpoint" => {
                let module = require(p.module, "module", "remove_breakpoint")?;
                let line = require(p.line, "line", "remove_breakpoint")?;
                tokio::task::spawn_blocking(move || {
                    tools::debug::debug_remove_breakpoint(&session, &module, line)
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "continue" => {
                tokio::task::spawn_blocking(move || tools::debug::debug_continue(&session))
                    .await
                    .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "step" => {
                let direction = require(p.direction, "direction", "step")?;
                tokio::task::spawn_blocking(move || tools::debug::debug_step(&session, &direction))
                    .await
                    .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "wait_stop" => {
                let timeout_secs = p.timeout_secs;
                tokio::task::spawn_blocking(move || {
                    tools::debug::debug_wait_stop(&session, timeout_secs)
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "stack_trace" => {
                tokio::task::spawn_blocking(move || tools::debug::debug_stack_trace(&session))
                    .await
                    .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "locals" => {
                let stack_level = p.stack_level;
                tokio::task::spawn_blocking(move || {
                    tools::debug::debug_locals(&session, stack_level)
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "eval" => {
                let expression = require(p.expression, "expression", "eval")?;
                let stack_level = p.stack_level;
                tokio::task::spawn_blocking(move || {
                    tools::debug::debug_eval(&session, &expression, stack_level)
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            other => Err(McpError::invalid_params(
                format!(
                    "Unknown action '{other}'. Expected: attach, disconnect, set_breakpoint, \
                     remove_breakpoint, continue, step, wait_stop, stack_trace, locals, eval"
                ),
                None,
            )),
        }
    }

    /// Whole-config semantic call graph: traverse who-calls-whom and object/metadata usage by
    /// durable node id. Use to understand call relationships and change impact — start with
    /// `overview` on an unfamiliar project, then `node`/`callers`/`callees`/`neighbors` on the
    /// ids it returns. Not for finding code by meaning (use `search`) and not for analyzer
    /// findings (use `diagnostics`). Actions: `overview`, `schema`, `status`, `resolve`
    /// (imprecise name → node id), `node`, `source`, `neighbors`, `callers`, `callees`.
    /// Source-bearing actions honour `max_output_tokens` and flag `budget_exhausted` on
    /// truncation. Lazily indexes on first use; while it builds it returns a retry envelope.
    #[tool(name = "graph", annotations(read_only_hint = true))]
    async fn graph(&self, params: Parameters<GraphParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let graph = self.state.graph().clone();

        // `schema` is static and needs no loaded graph.
        if p.action == "schema" {
            return Ok(tools::graph::schema());
        }

        // `status` reports the graph lifecycle (and kicks the lazy build) so an agent can start
        // it and poll progress instead of reading a flat `loading` envelope from a data action.
        if p.action == "status" {
            graph.ensure_loading();
            let report = tokio::task::spawn_blocking(move || graph.status_report())
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?;
            return Ok(tools::graph::status(&report));
        }

        // Lazily trigger the background load on first use.
        graph.ensure_loading();

        match graph.status() {
            GraphStatus::Disabled => {
                return Err(McpError::invalid_params(
                    "graph is only available in the workspace profile",
                    None,
                ))
            }
            GraphStatus::Idle | GraphStatus::Loading => {
                return Ok(tools::graph::loading(Some(
                    "call graph is still indexing; retry shortly",
                )))
            }
            GraphStatus::Failed(msg) => {
                return Err(McpError::internal_error(format!("graph load failed: {msg}"), None))
            }
            GraphStatus::Ready { .. } => {}
        }

        let Some(snapshot) = graph.snapshot() else {
            return Ok(tools::graph::loading(None));
        };

        tokio::task::spawn_blocking(move || {
            let gdb = &snapshot.graph;
            let value = match p.action.as_str() {
                "overview" => tools::graph::overview(gdb, p.top.unwrap_or(20)),
                "resolve" => {
                    let query = require(p.query, "query", "resolve")?;
                    let limit = p.top.unwrap_or(tools::graph::DEFAULT_RESOLVE_LIMIT);
                    tools::graph::resolve(gdb, &query, limit)
                }
                "node" => {
                    let id = require(p.id, "id", "node")?;
                    let detail = tools::graph::detail_from(p.detail.as_deref())
                        .map_err(|e| McpError::invalid_params(e, None))?;
                    let budget =
                        p.max_output_tokens.unwrap_or(tools::graph::DEFAULT_BODY_BUDGET_TOKENS);
                    tools::graph::node(gdb, &id, detail, budget)
                }
                "source" => {
                    if p.ids.is_empty() {
                        return Err(McpError::invalid_params(
                            "'ids' is required (non-empty) for action 'source'",
                            None,
                        ));
                    }
                    let budget = p.max_output_tokens.unwrap_or(4000);
                    tools::graph::source(gdb, &p.ids, budget)
                }
                action @ ("neighbors" | "callers" | "callees") => {
                    let id = require(p.id, "id", action)?;
                    let dir = match action {
                        "callers" => ide::Direction::In,
                        "callees" => ide::Direction::Out,
                        _ => tools::graph::direction_from(p.dir.as_deref())
                            .map_err(|e| McpError::invalid_params(e, None))?,
                    };
                    let detail = tools::graph::detail_from(p.detail.as_deref())
                        .map_err(|e| McpError::invalid_params(e, None))?;
                    tools::graph::validate_edge_kinds(&p.edge_kinds)
                        .map_err(|e| McpError::invalid_params(e, None))?;
                    let neighbors = ide::NeighborsParams {
                        id: &id,
                        dir,
                        depth: p.depth.unwrap_or(1),
                        max_nodes: p.max_nodes.unwrap_or(50),
                        detail,
                        provenance_filter: p.provenance.clone(),
                        edge_kind_filter: p.edge_kinds.clone(),
                    };
                    let budget =
                        p.max_output_tokens.unwrap_or(tools::graph::DEFAULT_BODY_BUDGET_TOKENS);
                    tools::graph::neighbors(gdb, &neighbors, budget)
                }
                other => {
                    return Err(McpError::invalid_params(
                        format!(
                            "Unknown action '{other}'. Expected: overview, schema, status, node, \
                             source, neighbors, callers, callees, resolve"
                        ),
                        None,
                    ))
                }
            };
            // Stamp freshness relative to the snapshot that served this answer: the
            // scan may detect drift and kick a background reload, but `revision`
            // and `stale` describe the data actually returned above.
            let freshness = graph.freshness(&snapshot);
            Ok(tools::graph::envelope(freshness, value))
        })
        .await
        .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
    }

    /// One symbol's consolidated card: kind, signature, type, doc, definition site, and a
    /// usages summary — by qualified name. Use to answer "what is X / where is it defined / what
    /// does it return / who calls it" for a single symbol in ONE call, instead of chaining
    /// hover + definition + references. Pass `symbol` (a common-module method
    /// `ОбщегоНазначения.ЗначениеРеквизитаОбъекта`, a metadata object `Справочник.Товары` or its
    /// attribute `Справочник.Товары.Артикул`, an object/manager method
    /// `Документ.ЗаказКлиента.Провести`, or a platform member `СтрНайти`); for a local/parameter
    /// with no qualified name pass `path`+`line` instead. An imprecise `symbol` returns candidate
    /// ids (not an error) — resolve one, or open it in `graph`. Not for finding code by meaning
    /// (use `search`), whole-object browsing (use `metadata`), or the full caller list (use
    /// `graph` with the returned `graph_id`). Reads the resident host; while it builds it returns
    /// a retry envelope. The `usages` summary needs the call graph; if it is still indexing the
    /// core card is still served with `usages_unavailable`.
    #[tool(name = "symbol_info", annotations(read_only_hint = true))]
    async fn symbol_info(
        &self,
        params: Parameters<SymbolInfoParams>,
    ) -> Result<CallToolResult, McpError> {
        use crate::diagnostics_state::{DiagnosticsStatus, ResidentOutcome};

        let p = params.0;
        if p.symbol.is_none() && p.path.is_none() {
            return Err(McpError::invalid_params(
                "one of 'symbol' or 'path'+'line' is required",
                None,
            ));
        }

        // The core card resolves on the resident host — mirror `metadata`'s lazy-load lifecycle
        // so an evicted/building resident degrades to a retry envelope, never a hard error.
        let diag = self.state.diagnostics().clone();
        diag.ensure_loading();
        match diag.status() {
            DiagnosticsStatus::Disabled => {
                return Err(McpError::invalid_params(
                    "symbol_info is only available in the workspace profile",
                    None,
                ))
            }
            DiagnosticsStatus::Failed(msg) => {
                return Err(McpError::internal_error(
                    format!("symbol_info database load failed: {msg}"),
                    None,
                ))
            }
            DiagnosticsStatus::Idle | DiagnosticsStatus::Loading => {
                return Ok(tools::metadata::loading(&diag.status_report()))
            }
            DiagnosticsStatus::Ready { .. } => {}
        }

        let sections = tools::symbol_info::sections_from(&p.include);
        let locale = tools::symbol_info::locale_from(p.locale.as_deref())?;
        let max_output_tokens =
            p.max_output_tokens.unwrap_or(tools::response::DEFAULT_OUTPUT_BUDGET_TOKENS);
        let symbol = p.symbol.clone();
        let path = p.path.clone();
        let line = p.line;
        let column = p.column;

        // The graph is enrichment only (usages + fuzzy candidates on a resident miss). Take a
        // best-effort snapshot: `None` when it is not `Ready`, in which case the core card is
        // still served (with `usages_unavailable`).
        let graph = self.state.graph().clone();
        graph.ensure_loading();

        tokio::task::spawn_blocking(move || {
            let read = |diag: &crate::diagnostics_state::DiagnosticsState| {
                diag.read(|resident, _generation| {
                    tools::symbol_info::resolve_card(
                        resident,
                        symbol.as_deref(),
                        path.as_deref(),
                        line,
                        column,
                        sections,
                        locale,
                    )
                })
            };

            let mut outcome = read(&diag);
            // A resident miss on a well-formed qualified name may be a symbol added since the
            // last throttled drift scan: force ONE storm-guarded re-scan and retry, matching the
            // `metadata object` miss path.
            if let ResidentOutcome::Ready(Ok(None), _) = &outcome {
                if symbol.is_some() {
                    diag.force_rescan();
                    outcome = read(&diag);
                }
            }

            let card = match outcome {
                ResidentOutcome::Ready(result, _freshness) => result?,
                ResidentOutcome::Loading => {
                    return Ok(tools::metadata::loading(&diag.status_report()))
                }
                ResidentOutcome::Disabled => {
                    return Err(McpError::invalid_params(
                        "symbol_info is only available in the workspace profile",
                        None,
                    ))
                }
                ResidentOutcome::Failed(msg) => {
                    return Err(McpError::internal_error(
                        format!("symbol_info database: {msg}"),
                        None,
                    ))
                }
            };

            let snapshot = graph.snapshot();
            let gdb = snapshot.as_ref().map(|s| &*s.graph);

            match card {
                Some(card) => Ok(tools::symbol_info::render_card(
                    &card,
                    gdb,
                    tools::symbol_info::DEFAULT_TOP_MODULES,
                    max_output_tokens,
                )),
                None => {
                    // Resident miss: offer graph candidates for an imprecise qualified name.
                    let symbol = symbol.as_deref().unwrap_or_default();
                    Ok(tools::symbol_info::render_not_found(
                        symbol,
                        gdb,
                        tools::symbol_info::DEFAULT_CANDIDATE_LIMIT,
                    ))
                }
            }
        })
        .await
        .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
    }

    /// Semantic analyzer findings the compiler and grep cannot give you — unreachable code,
    /// type mismatch, unresolved calls, and 180+ other rules. Use to check a file or the whole
    /// config for issues, or to discover which rules exist. Not for finding code (use `search`)
    /// and not for call relationships (use `graph`). Actions: `catalog` — list rules (start here
    /// to learn the codes); `schema` — response shape; `status` — analysis readiness; `file` —
    /// per-finding results for one `.bsl` path; `workspace` — a bounded per-code aggregate sweep
    /// of the whole config. Honours `max_output_tokens`/`max_findings` and flags truncation.
    /// Reads the resident host; while it builds it returns a retry envelope.
    #[tool(name = "diagnostics", annotations(read_only_hint = true))]
    async fn diagnostics(
        &self,
        params: Parameters<DiagnosticsParams>,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match p.action.as_str() {
            // `catalog` and `schema` are static (compile-time metadata), so they need
            // no resident analysis database and answer in either profile.
            "schema" => Ok(tools::diagnostics::schema()),
            "catalog" => {
                let locale = match p.locale.as_deref() {
                    Some(s) => ide::Locale::from_config_str(s)
                        .map_err(|e| McpError::invalid_params(e.to_string(), None))?,
                    None => ide::Locale::default(),
                };
                Ok(tools::diagnostics::catalog(locale, &p.codes, p.max_output_tokens))
            }
            // `status` reports the resident lifecycle (and kicks the lazy build) so an
            // agent can start it and poll progress instead of a flat `loading`.
            "status" => {
                let diag = self.state.diagnostics();
                diag.ensure_loading();
                Ok(tools::diagnostics::status(&diag.status_report()))
            }
            "file" => self.diagnostics_file(p).await,
            "workspace" => self.diagnostics_workspace(p, ct).await,
            other => Err(McpError::invalid_params(
                format!(
                    "Unknown action '{other}'. Expected: catalog, schema, status, file, workspace"
                ),
                None,
            )),
        }
    }

    /// The `diagnostics file` action: build/serve per-file findings from the resident
    /// analysis database, behind the lazy-load lifecycle and freshness envelope.
    async fn diagnostics_file(&self, p: DiagnosticsParams) -> Result<CallToolResult, McpError> {
        use crate::diagnostics_state::DiagnosticsStatus;
        use tools::diagnostics::{parse_detail, parse_min_severity, FileFilters};

        let diag = self.state.diagnostics().clone();
        let path = require(p.path, "path", "file")?;
        let path = std::path::PathBuf::from(path);

        diag.ensure_loading();
        match diag.status() {
            DiagnosticsStatus::Disabled => {
                return Err(McpError::invalid_params(
                    "diagnostics 'file' is only available in the workspace profile",
                    None,
                ))
            }
            DiagnosticsStatus::Failed(msg) => {
                return Err(McpError::internal_error(
                    format!("diagnostics database load failed: {msg}"),
                    None,
                ))
            }
            DiagnosticsStatus::Idle | DiagnosticsStatus::Loading => {
                return Ok(tools::diagnostics::loading(&diag.status_report()))
            }
            DiagnosticsStatus::Ready { .. } => {}
        }

        let min_severity = parse_min_severity(p.min_severity.as_deref())
            .map_err(|e| McpError::invalid_params(e, None))?;
        let detailed =
            parse_detail(p.detail.as_deref()).map_err(|e| McpError::invalid_params(e, None))?;
        let range = match (p.range_start, p.range_end) {
            (Some(s), Some(e)) => Some((s, e)),
            (Some(s), None) => Some((s, usize::MAX)),
            (None, Some(e)) => Some((0, e)),
            (None, None) => None,
        };
        let filters = FileFilters {
            min_severity,
            codes: p.codes,
            range,
            max_findings: p.max_findings.unwrap_or(tools::diagnostics::DEFAULT_MAX_FINDINGS),
            max_output_tokens: p.max_output_tokens,
            detailed,
        };

        tokio::task::spawn_blocking(move || {
            // `generation` is supplied by `read` under the lock (so `result_id` describes
            // the exact resident state queried), and the freshness verdict is computed
            // under that same lock and returned alongside — the envelope is atomic.
            let outcome = diag.read(|resident, generation| {
                tools::diagnostics::file_findings(resident, &path, &filters, generation)
            });
            use crate::diagnostics_state::ResidentOutcome;
            match outcome {
                ResidentOutcome::Ready(result, freshness) => {
                    Ok(tools::diagnostics::envelope(freshness, result))
                }
                ResidentOutcome::Loading => Ok(tools::diagnostics::loading(&diag.status_report())),
                ResidentOutcome::Disabled => Err(McpError::invalid_params(
                    "diagnostics 'file' is only available in the workspace profile",
                    None,
                )),
                ResidentOutcome::Failed(msg) => {
                    Err(McpError::internal_error(format!("diagnostics database: {msg}"), None))
                }
            }
        })
        .await
        .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
    }

    /// The `diagnostics workspace` action: an opt-in, bounded whole-config sweep that
    /// returns per-code aggregates only (no per-finding detail). The rayon sweep runs
    /// under the resident lock (so no reload mutates the db mid-sweep), which serialises
    /// other diagnostics calls for its duration — acceptable for a capped, opt-in pass.
    ///
    /// `ct` is the rmcp per-request token, cancelled on MCP `notifications/cancelled`
    /// and on transport shutdown; on cancel the call answers immediately with an
    /// error while the sweep's per-worker salsa tokens unwind it early, releasing
    /// the resident lock instead of silently running to completion for minutes.
    async fn diagnostics_workspace(
        &self,
        p: DiagnosticsParams,
        ct: tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        use crate::diagnostics_state::{
            DiagnosticsStatus, ResidentOutcome, SweepCancel, SweepOptions,
        };
        use tools::diagnostics::{
            parse_min_severity, DEFAULT_MAX_SWEEP_FILES, MAX_SWEEP_FILES_CEILING,
        };

        let diag = self.state.diagnostics().clone();
        diag.ensure_loading();
        match diag.status() {
            DiagnosticsStatus::Disabled => {
                return Err(McpError::invalid_params(
                    "diagnostics 'workspace' is only available in the workspace profile",
                    None,
                ))
            }
            DiagnosticsStatus::Failed(msg) => {
                return Err(McpError::internal_error(
                    format!("diagnostics database load failed: {msg}"),
                    None,
                ))
            }
            DiagnosticsStatus::Idle | DiagnosticsStatus::Loading => {
                return Ok(tools::diagnostics::loading(&diag.status_report()))
            }
            DiagnosticsStatus::Ready { .. } => {}
        }

        let min_severity = parse_min_severity(p.min_severity.as_deref())
            .map_err(|e| McpError::invalid_params(e, None))?;
        let max_output_tokens = p.max_output_tokens;
        let opts = SweepOptions {
            min_severity,
            codes: p.codes,
            // Clamp to the ceiling: the sweep holds the resident lock throughout, so an
            // unbounded request would stall every other diagnostics call. A larger
            // config surfaces as `truncated` with the true `files_total`.
            max_files: p.max_files.unwrap_or(DEFAULT_MAX_SWEEP_FILES).min(MAX_SWEEP_FILES_CEILING),
        };

        // Bridge MCP cancellation into the sweep: rmcp cancels `ct` on
        // `notifications/cancelled`; `join_unless_cancelled` observes it, fans the
        // cancel out to every per-worker salsa token the sweep has registered, and
        // answers immediately instead of waiting out the resident queue. Only
        // worker-clone tokens are cancelled — the master db and concurrent
        // diagnostics calls stay untouched.
        let cancel = std::sync::Arc::new(SweepCancel::default());

        let started = std::time::Instant::now();
        let sweep_cancel = std::sync::Arc::clone(&cancel);
        let join = tokio::task::spawn_blocking(move || {
            let outcome = diag.read(|resident, generation| {
                let sweep = resident.workspace_aggregates(resident.config(), &opts, &sweep_cancel);
                if sweep.cancelled {
                    tracing::info!(
                        tool = "diagnostics",
                        action = "workspace",
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        files_processed = sweep.files_swept,
                        files_total = sweep.files_total,
                        "MCP call cancelled, sweep unwound early"
                    );
                }
                tools::diagnostics::workspace_findings(&sweep, generation, max_output_tokens)
            });
            match outcome {
                ResidentOutcome::Ready(result, freshness) => {
                    Ok(tools::diagnostics::envelope(freshness, result))
                }
                ResidentOutcome::Loading => Ok(tools::diagnostics::loading(&diag.status_report())),
                ResidentOutcome::Disabled => Err(McpError::invalid_params(
                    "diagnostics 'workspace' is only available in the workspace profile",
                    None,
                )),
                ResidentOutcome::Failed(msg) => {
                    Err(McpError::internal_error(format!("diagnostics database: {msg}"), None))
                }
            }
        });
        match join_unless_cancelled(ct, cancel, join).await {
            // Per the MCP cancellation spec the client ignores any response after
            // its `notifications/cancelled`, so answer with a plain error instead
            // of inventing a partial-success shape; the detached sweep logs the
            // partial coverage on its own.
            None => Err(McpError::internal_error("request cancelled", None)),
            Some(joined) => {
                joined.map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
        }
    }
}

/// Await the sweep's blocking task under the rmcp per-request token. Cancellation
/// wins: when `ct` fires (MCP `notifications/cancelled` or transport shutdown) —
/// including a token already cancelled before the first poll — the sweep's salsa
/// tokens are cancelled via `cancel.cancel_all()` and `None` is returned right
/// away, WITHOUT waiting for the blocking task: it may still be queued behind
/// another sweep on the resident mutex, and once it runs it exits early and logs
/// on its own. `Some(join result)` when the task finishes first; a completed call
/// never cancels anything.
async fn join_unless_cancelled<T>(
    ct: tokio_util::sync::CancellationToken,
    cancel: std::sync::Arc<crate::diagnostics_state::SweepCancel>,
    mut join: tokio::task::JoinHandle<T>,
) -> Option<Result<T, tokio::task::JoinError>> {
    tokio::select! {
        // Biased so an already-cancelled token deterministically beats a completed
        // join — a cancelled request must never race into a normal response.
        biased;
        _ = ct.cancelled() => {
            cancel.cancel_all();
            None
        }
        joined = &mut join => Some(joined),
    }
}

#[tool_router(router = reference_tool_router)]
impl McpServer {
    /// Search the platform reference documentation index (not project code). Use to find
    /// platform API documentation by keyword or meaning. For project code search use the
    /// workspace profile's `search`; for one platform member's signature use `syntax_help`.
    /// Actions: `find_docs` / `search_docs` — doc search (`query` required; `limit` default 10,
    /// max 50); `status` — index readiness. While the index warms up it returns a retry
    /// envelope.
    #[tool(name = "search", annotations(read_only_hint = true))]
    async fn reference_search(
        &self,
        params: Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match p.action.as_str() {
            "status" => {
                let engine = self.state.search_engine().clone();
                let progress = self.state.index_progress().clone();
                let semantic_runtime = self.state.semantic_runtime();
                let baseline = self.state.baseline_view();
                // The reference/shared path runs no overlay warmup, so its state is always
                // `Pending`; the Summary block words this profile as a reference docs index.
                let overlay_warmup = crate::state::OverlayWarmupState::Pending;
                tokio::task::spawn_blocking(move || {
                    tools::search::search_status(
                        &engine,
                        &progress,
                        &semantic_runtime,
                        WorkspaceSearchMode::SqliteLocal,
                        overlay_warmup,
                        baseline.configured,
                        baseline.external,
                        baseline.pending,
                    )
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            "find_docs" | "search_docs" => {
                let query = require(p.query, "query", &p.action)?;
                let limit = p.limit.unwrap_or(10).min(50);
                let max_output_tokens =
                    p.max_output_tokens.unwrap_or(tools::response::DEFAULT_OUTPUT_BUDGET_TOKENS);
                let engine = self.state.search_engine().clone();
                let baseline = self.state.baseline_view();
                let configured_baseline = baseline.configured;
                let external_baseline = baseline.external;
                let action = p.action.clone();
                tokio::task::spawn_blocking(move || match action.as_str() {
                    "find_docs" => tools::search::find_docs(
                        &engine,
                        configured_baseline.as_ref(),
                        external_baseline.clone(),
                        &query,
                        limit,
                        max_output_tokens,
                    ),
                    "search_docs" => tools::search::search_docs(
                        &engine,
                        configured_baseline.as_ref(),
                        external_baseline,
                        &query,
                        limit,
                        max_output_tokens,
                    ),
                    _ => unreachable!(),
                })
                .await
                .map_err(|e| McpError::internal_error(format!("Task error: {e}"), None))?
            }
            other => Err(McpError::invalid_params(
                format!("Unknown action '{other}'. Expected: find_docs, search_docs, status"),
                None,
            )),
        }
    }

    /// Look up one platform member's reference card — signature, parameters, and description —
    /// from the built-in platform data. Use when you know the member name (e.g. `СтрНайти`) and
    /// want its exact signature. For free-text doc discovery use `search`; for broader
    /// conceptual guidance use `its_help`. Params: `name` (required), optional `type_name` when
    /// the member belongs to a specific platform type.
    #[tool(name = "syntax_help", annotations(read_only_hint = true))]
    async fn syntax_help(
        &self,
        params: Parameters<SyntaxHelpParams>,
    ) -> Result<CallToolResult, McpError> {
        tools::platform::bsl_syntax_help(&params.0.name, params.0.type_name.as_deref())
    }

    /// Ask the ITS expert-help knowledge base a natural-language question about the 1C platform
    /// and development standards. Use for conceptual "how / why" questions. For one member's
    /// signature use `syntax_help`; for doc keyword search use `search`. Param: `question`
    /// (required).
    #[tool(name = "its_help", annotations(read_only_hint = true))]
    async fn its_help(
        &self,
        params: Parameters<ItsHelpParams>,
    ) -> Result<CallToolResult, McpError> {
        tools::its_help::its_help(&params.0.question).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(match self.profile {
            McpProfile::Workspace => {
                "BSL Analyzer workspace MCP server for a 1C:Enterprise (BSL) configuration. \
                 Route by task (each tool's own description carries the full contract):\n\
                 - find code by meaning or unknown name → `search`;\n\
                 - who-calls-whom / change impact → `graph` (durable ids; start at overview);\n\
                 - one symbol's kind/signature/type/doc/definition/usages by name → `symbol_info`;\n\
                 - analyzer findings (unreachable, type mismatch, unresolved) → `diagnostics` \
                 (start at catalog to learn the codes);\n\
                 - metadata objects / structure / forms → `metadata`;\n\
                 - SDBL query validate/run → `query`; run/check BSL code → `execute`;\n\
                 - live infobase runtime events → `event_log`; live debugger session → `debug`.\n\
                 Tools whose data is built lazily (metadata, graph, diagnostics, search) return a \
                 retry envelope while indexing rather than an error; large responses are bounded \
                 by `max_output_tokens`/limits — JSON tools (graph, diagnostics) flag \
                 `budget_exhausted`, text tools (search, metadata) append a truncation note."
                    .into()
            }
            McpProfile::Reference => {
                "BSL Analyzer reference MCP server for the 1C platform (no project code). \
                 Route by task (each tool's own description carries the full contract):\n\
                 - one platform member's signature by name → `syntax_help`;\n\
                 - platform docs by keyword or meaning → `search`;\n\
                 - conceptual how/why question on the platform or standards → `its_help`.\n\
                 Tools: search, syntax_help, its_help."
                    .into()
            }
        });
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = rmcp::model::Implementation::from_build_env();
        info
    }
}

#[cfg(test)]
mod contract {
    use super::*;
    use expect_test::expect;
    use std::fmt::Write;

    /// Render a profile's `tools/list` into a stable text contract: every tool (sorted by
    /// name) with its description and each parameter field's description. This is the #25
    /// guardrail — a refactor that drops a tool description or a field doc changes this
    /// snapshot loudly, instead of silently shipping an empty contract to agents. Rebase with
    /// `UPDATE_EXPECT=1 cargo test -p mcp-server contract`.
    fn render(tools: &[rmcp::model::Tool]) -> String {
        let mut tools: Vec<&rmcp::model::Tool> = tools.iter().collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        let mut out = String::new();
        for tool in tools {
            let _ = writeln!(out, "## {}", tool.name);
            let _ =
                writeln!(out, "{}", tool.description.as_deref().unwrap_or("<MISSING DESCRIPTION>"));
            if let Some(props) = tool.input_schema.get("properties").and_then(|v| v.as_object()) {
                let mut keys: Vec<&String> = props.keys().collect();
                keys.sort();
                for key in keys {
                    let desc = props[key]
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("<no doc>");
                    let _ = writeln!(out, "  - {key}: {desc}");
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn workspace_tools_contract() {
        let rendered = render(&McpServer::workspace_tool_router().list_all());
        expect![[r###"
            ## debug
            Drive a live 1C debugger session: attach, set breakpoints, step, and inspect state. Use
            to debug a running infobase — attach, break, then step and read locals/eval. Not for
            static analysis (use `diagnostics`) and not for running standalone code (use `execute`).
            Actions: `attach`/`disconnect`; `set_breakpoint`/`remove_breakpoint`; `continue`/`step`;
            `wait_stop`; `stack_trace`; `locals`; `eval`. Requires a reachable debug endpoint
            (`host` + `infobase`, default port 1550).
              - action: attach | disconnect | set_breakpoint | remove_breakpoint | continue | step |
            wait_stop | stack_trace | locals | eval.
              - auto_attach: `attach`: object-name patterns to auto-attach on connect (optional).
              - condition: `set_breakpoint`: conditional-breakpoint expression (optional).
              - config_root: `attach`: configuration source root (optional).
              - direction: `step`: `over`/`next`, `in`/`into`, or `out` (required).
              - expression: `eval`: BSL expression to evaluate in the current stop (required).
              - extensions: `attach`: `[name, root]` pairs for loaded extensions (optional).
              - host: `attach`: debugger host (required).
              - infobase: `attach`: infobase name (required).
              - line: `set_breakpoint`/`remove_breakpoint`: 1-based line (required).
              - module: `set_breakpoint`/`remove_breakpoint`: target module id (required).
              - port: `attach`: debugger port (default 1550).
              - stack_level: `locals`/`eval`: stack frame level to evaluate in (optional, default top frame).
              - timeout_secs: `wait_stop`: max seconds to wait for a stop event (optional).

            ## diagnostics
            Semantic analyzer findings the compiler and grep cannot give you — unreachable code,
            type mismatch, unresolved calls, and 180+ other rules. Use to check a file or the whole
            config for issues, or to discover which rules exist. Not for finding code (use `search`)
            and not for call relationships (use `graph`). Actions: `catalog` — list rules (start here
            to learn the codes); `schema` — response shape; `status` — analysis readiness; `file` —
            per-finding results for one `.bsl` path; `workspace` — a bounded per-code aggregate sweep
            of the whole config. Honours `max_output_tokens`/`max_findings` and flags truncation.
            Reads the resident host; while it builds it returns a retry envelope.
              - action: catalog | schema | status | file | workspace.
              - codes: `catalog`: narrow to these codes. `file`: keep only these codes.
              - detail: `file`: concise | detailed (default concise).
              - locale: `catalog`: ru | en (default ru) — title language.
              - max_files: `workspace`: cap on files swept (default 1000).
              - max_findings: `file`: cap on returned findings (default 200).
              - max_output_tokens: `catalog`/`file`/`workspace`: output budget in tokens (~4 chars each); a truncated
            response carries `budget_exhausted: true` and a `budget_hint` on how to narrow it
            (tighten `codes`/`min_severity`/range or raise the budget). When omitted, no token
            budget applies — only the action's own count caps (`max_findings`/`max_files`).
              - min_severity: `file`: inclusive severity floor error|warning|info|hint (default warning).
              - path: `file`: absolute or workspace-relative `.bsl` path.
              - range_end: `file`: 0-based last line to include (optional).
              - range_start: `file`: 0-based first line to include (optional).

            ## event_log
            Read the 1C infobase event log (журнал регистрации) through the deployed BSL_Analyzer
            extension. Use to inspect runtime events — errors, authentications, data changes —
            filtered by time, user, event, metadata object, or severity. Not for static analysis of
            source (use `diagnostics`): this reads live runtime records from a running infobase.
            Filters: `date_from`/`date_to`, `level`, `user`, `event`, `metadata`, and `contains`
            (post-read substring over the newest `limit` window). `limit` is newest-first (default
            100, max 1000). Requires the extension deployed with event-log read rights.
              - connection: Named live 1C connection (optional when only one/default connection exists).
              - contains: Case-insensitive substring filter over the comment/data columns, applied AFTER the
            platform read — so it narrows the already-`limit`-capped newest window, it does not
            scan the whole log. Widen `limit` if a match may lie deeper.
              - date_from: Lower time bound (inclusive), ISO-8601, e.g. `2026-07-05T00:00:00` or `2026-07-05`.
              - date_to: Upper time bound (inclusive), ISO-8601.
              - event: Event name, e.g. `_$Session$_.Authentication` or a metadata event like `_$Data$_.Post`.
              - level: Severity: Информация/Предупреждение/Ошибка/Примечание or Information/Warning/Error/Note.
              - limit: Max records (newest first), default 100, capped at 1000.
              - metadata: Full metadata name to filter by, e.g. `Документ.ЗаказКлиента`.
              - user: Infobase user name (deleted users can only be matched by name).

            ## execute
            Run or syntax-check BSL code in an embedded interpreter. Use to confirm a snippet
            compiles, run a small script, or evaluate a single expression. Not for querying the
            database (use `query` for SDBL) and not for analyzer findings (use `diagnostics`).
            Actions: `check` — syntax-check `code`; `run` — execute `code`; `eval` — evaluate the
            single expression in `code`. `run`/`eval` execute code, so this tool is not read-only.
              - action: check | run | eval.
              - code: BSL source to `check`/`run`, or the single expression to `eval`.
              - connection: Named live 1C connection (optional when only one/default connection exists).

            ## graph
            Whole-config semantic call graph: traverse who-calls-whom and object/metadata usage by
            durable node id. Use to understand call relationships and change impact — start with
            `overview` on an unfamiliar project, then `node`/`callers`/`callees`/`neighbors` on the
            ids it returns. Not for finding code by meaning (use `search`) and not for analyzer
            findings (use `diagnostics`). Actions: `overview`, `schema`, `status`, `resolve`
            (imprecise name → node id), `node`, `source`, `neighbors`, `callers`, `callees`.
            Source-bearing actions honour `max_output_tokens` and flag `budget_exhausted` on
            truncation. Lazily indexes on first use; while it builds it returns a retry envelope.
              - action: overview | schema | status | node | source | neighbors | callers | callees | resolve
              - depth: Traversal depth for neighbors (default: 1).
              - detail: names | signatures | bodies (default: signatures).
              - dir: in | out | both — only for `neighbors` (default: in).
              - edge_kinds: Keep only edges of these kinds (call/manager_creates/manager_access/query_ref/
            contains/data_binding) — lets metadata-impact queries isolate e.g. only `query_ref`.
              - id: Durable node id (required for node/neighbors/callers/callees).
              - ids: Durable node ids (required for `source`).
              - max_nodes: Server-side cap on returned neighbour nodes (default: 50).
              - max_output_tokens: Output budget in tokens (~4 chars each) for source-bearing actions: `source`
            (default 4000) and `node`/`neighbors` at `detail=bodies` (default 6000). When the
            body output is truncated the response carries `budget_exhausted: true`.
              - provenance: Keep only edges with these provenances (resolved/inferred/visibility_blocked/unresolved).
              - query: Imprecise lookup string (required for `resolve`): wrong casing, a bare method/object
            name, or a partial id.
              - top: How many top-centrality methods to include in `overview` (default: 20).

            ## metadata
            Browse the configuration's metadata: objects, their structure, and managed forms.
            Use to answer "what objects exist / what does object X contain / what is on form Y" —
            attributes, tabular sections, forms, types — straight from the metadata substrate. Not
            for call relationships (use `graph`) and not for finding code by meaning (use `search`).
            Actions: `info` — configuration summary; `tree` — the metadata object tree (filterable);
            `object` — one object's structure (needs `object_type` + `object_name`); `form` — a
            managed form's layout (needs `object_type`). Reads the resident analysis host; while it
            builds it returns a retry envelope, not an error.
              - action: info | tree | object | form.
              - connection: Named live 1C connection for `mode=infobase` (optional).
              - filter: `tree`: case-insensitive substring to narrow the returned tree (optional).
              - form_name: `form`: managed-form name (optional; omit for the object's default form).
              - max_items: `tree` in infobase mode: maximum returned objects (default 100, max 1000).
              - max_output_tokens: `tree` (filtered listing): output budget in tokens (~4 chars each); an over-budget
            listing is truncated at a line boundary with a continuation note (default 6000).
              - meta_type: `tree` in infobase mode: metadata collection, e.g. `Справочники`/`Documents`.
              - mode: auto | source | infobase (default auto).
              - name_mask: `tree` in infobase mode: case-insensitive object name/synonym substring.
              - object_name: Metadata object name, e.g. `ЗаказКлиента`. Required for `object`; for `form` it
            selects the owner object (omit for a configuration-level common form).
              - object_type: Metadata object type, e.g. `Документ`/`Справочник`/`ОбщийМодуль`. Required for
            `object` and `form`.

            ## query
            Validate or execute SDBL (the 1C query language) against the configuration schema. Use
            to check a query for errors before shipping it, to run a read-only query, or to fetch
            the query-language schema. Not for browsing metadata structure (use `metadata`) and not
            for BSL code (use `execute`). Actions: `validate` — parse and type-check a query (`query`
            required); `execute` — run it (`query` required; optional `limit`, `parameters`);
            `schema` — the SDBL schema reference.
              - action: validate | execute | schema.
              - connection: Named live 1C connection (optional when only one/default connection exists).
              - limit: `execute`: cap on returned rows (optional).
              - parameters: `execute`: named SDBL query parameters (`&Param` → value) (optional).
              - query: SDBL text — required for `validate`/`execute`, omitted for `schema`.

            ## search
            Hybrid lexical + semantic code search across the project source. Use when you need to
            find code by meaning or a free-form phrase ("where the reserve for an order is built")
            or when the exact symbol name is unknown. Not for walking call relationships — that is
            `graph` (callers/callees by durable id) — and not for analyzer findings — that is
            `diagnostics`. Actions: `search_code` — the search (`query` required; `limit` default
            10, max 50); `status` — index readiness. While the index warms up it returns a retry
            envelope; retry shortly.
              - action: Workspace profile: `search_code` | `status`. Reference profile: `find_docs` |
            `search_docs` | `status`.
              - limit: Cap on returned hits (default 10, max 50).
              - max_output_tokens: Output budget in tokens (~4 chars each); over-budget results are truncated at a hit
            boundary with a note telling you to raise `limit` or narrow the query (default 6000).
              - query: Free-text query. Required for `search_code`/`find_docs`/`search_docs`.

            ## symbol_info
            One symbol's consolidated card: kind, signature, type, doc, definition site, and a
            usages summary — by qualified name. Use to answer "what is X / where is it defined / what
            does it return / who calls it" for a single symbol in ONE call, instead of chaining
            hover + definition + references. Pass `symbol` (a common-module method
            `ОбщегоНазначения.ЗначениеРеквизитаОбъекта`, a metadata object `Справочник.Товары` or its
            attribute `Справочник.Товары.Артикул`, an object/manager method
            `Документ.ЗаказКлиента.Провести`, or a platform member `СтрНайти`); for a local/parameter
            with no qualified name pass `path`+`line` instead. An imprecise `symbol` returns candidate
            ids (not an error) — resolve one, or open it in `graph`. Not for finding code by meaning
            (use `search`), whole-object browsing (use `metadata`), or the full caller list (use
            `graph` with the returned `graph_id`). Reads the resident host; while it builds it returns
            a retry envelope. The `usages` summary needs the call graph; if it is still indexing the
            core card is still served with `usages_unavailable`.
              - column: `path`: 0-based character offset within the line of the symbol occurrence (default 0).
              - include: Card sections to include: any of `definition` | `type` | `doc`. Empty = all. `usages`
            is always a summary and is added when the call graph is ready.
              - line: `path`: 0-based line of the symbol occurrence.
              - locale: Type/label language: `ru` (default) or `en`.
              - max_output_tokens: Output budget in tokens (~4 chars each); an over-budget member list is trimmed and the
            response carries `truncated: true` with a `budget_hint` (default 6000).
              - path: Positional fallback for locals/parameters that have no qualified name: absolute or
            workspace-relative `.bsl` path. Requires `line`.
              - symbol: Qualified name of the symbol (primary input): a common-module method
            (`ОбщегоНазначения.ЗначениеРеквизитаОбъекта`), a metadata object (`Справочник.Товары`)
            or its attribute (`Справочник.Товары.Артикул`), an object/manager module method
            (`Документ.ЗаказКлиента.Провести`), or a platform member (`СтрНайти`, `Массив.Добавить`).
            Case-insensitive; the MdoType keyword accepts singular or plural, RU or EN.

        "###]].assert_eq(&rendered);
    }

    #[test]
    fn reference_tools_contract() {
        let rendered = render(&McpServer::reference_tool_router().list_all());
        expect![[r###"
            ## its_help
            Ask the ITS expert-help knowledge base a natural-language question about the 1C platform
            and development standards. Use for conceptual "how / why" questions. For one member's
            signature use `syntax_help`; for doc keyword search use `search`. Param: `question`
            (required).
              - question: Natural-language question for the ITS expert help.

            ## search
            Search the platform reference documentation index (not project code). Use to find
            platform API documentation by keyword or meaning. For project code search use the
            workspace profile's `search`; for one platform member's signature use `syntax_help`.
            Actions: `find_docs` / `search_docs` — doc search (`query` required; `limit` default 10,
            max 50); `status` — index readiness. While the index warms up it returns a retry
            envelope.
              - action: Workspace profile: `search_code` | `status`. Reference profile: `find_docs` |
            `search_docs` | `status`.
              - limit: Cap on returned hits (default 10, max 50).
              - max_output_tokens: Output budget in tokens (~4 chars each); over-budget results are truncated at a hit
            boundary with a note telling you to raise `limit` or narrow the query (default 6000).
              - query: Free-text query. Required for `search_code`/`find_docs`/`search_docs`.

            ## syntax_help
            Look up one platform member's reference card — signature, parameters, and description —
            from the built-in platform data. Use when you know the member name (e.g. `СтрНайти`) and
            want its exact signature. For free-text doc discovery use `search`; for broader
            conceptual guidance use `its_help`. Params: `name` (required), optional `type_name` when
            the member belongs to a specific platform type.
              - name: Platform member name to look up, e.g. `СтрНайти` or a type method.
              - type_name: Owning platform type when `name` is a member of a specific type (optional).

        "###]].assert_eq(&rendered);
    }
}

#[cfg(test)]
mod cancel_bridge {
    use super::join_unless_cancelled;
    use crate::diagnostics_state::SweepCancel;
    use std::sync::Arc;

    /// A token cancelled before the first poll deterministically wins over an
    /// already-completed join: the sweep registry is cancelled and no normal
    /// response can race out.
    #[tokio::test]
    async fn pre_cancelled_token_beats_a_completed_join() {
        let ct = tokio_util::sync::CancellationToken::new();
        ct.cancel();
        let cancel = Arc::new(SweepCancel::default());
        let join = tokio::task::spawn_blocking(|| 42);
        let _ = join.is_finished();

        let out = join_unless_cancelled(ct, Arc::clone(&cancel), join).await;
        assert!(out.is_none(), "a cancelled request must never produce a normal response");
        assert!(cancel.is_cancelled(), "the cancel must fan out to the sweep registry");
    }

    /// A cancel arriving while the blocking task is stuck (queued on the resident
    /// mutex in production) answers immediately instead of waiting the task out.
    #[tokio::test]
    async fn mid_flight_cancel_answers_without_waiting_for_the_join() {
        let ct = tokio_util::sync::CancellationToken::new();
        let cancel = Arc::new(SweepCancel::default());
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let join = tokio::task::spawn_blocking(move || rx.recv());

        let guard = join_unless_cancelled(ct.clone(), Arc::clone(&cancel), join);
        let canceller = async {
            tokio::task::yield_now().await;
            ct.cancel();
        };
        // The guard can only resolve through the cancel arm: the blocking task
        // stays parked on the channel until we release it below.
        let (out, ()) = tokio::join!(guard, canceller);
        assert!(out.is_none(), "cancellation must not wait for the blocked task");
        assert!(cancel.is_cancelled());

        tx.send(()).expect("the detached task is still alive and picks up the release");
    }

    /// A call that completes first returns the join result untouched, and a late
    /// cancel is a no-op for the (finished) sweep.
    #[tokio::test]
    async fn completed_join_is_returned_and_a_late_cancel_is_a_noop() {
        let ct = tokio_util::sync::CancellationToken::new();
        let cancel = Arc::new(SweepCancel::default());
        let join = tokio::task::spawn_blocking(|| 7);

        let out = join_unless_cancelled(ct.clone(), Arc::clone(&cancel), join).await;
        let value = out.expect("uncancelled call yields the join").expect("no panic");
        assert_eq!(value, 7);
        assert!(!cancel.is_cancelled(), "a completed call must not cancel anything");

        ct.cancel();
        tokio::task::yield_now().await;
        assert!(!cancel.is_cancelled(), "a cancel after completion has nothing to reach");
    }
}
