mod bootstrap;
mod embed;
mod sync;
#[cfg(test)]
mod test_support;
mod types;

use crate::baseline::DeferredBaselineRuntime;
use crate::change_hub::WorkspaceChangeHub;
use crate::diagnostics_state::DiagnosticsState;
use crate::graph::GraphState;
use bsl_search::IndexProgress;
use onec_client::Client as OnecClient;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) use types::{
    OverlayWarmupState, SemanticRuntimeStatus, SharedSearchEngine, WorkspaceSearchMode,
};

/// Per-query cap on how many dirty overlay paths [`SharedState::prefetch_resident_overlay`]
/// resolves from the shared resident parse. A branch switch can dirty thousands of paths;
/// prefetching them all on the query thread would be unbounded work. Paths beyond the cap stay
/// dirty and are served by the query's own lazy disk refresh and by subsequent queries' prefetch
/// passes, so nothing is lost — the cap is purely a per-query budget. 64 keeps the pre-pass cheap
/// while covering the common "edit a handful of files, then search" case in one shot.
const MAX_RESIDENT_PREFETCH_PATHS_PER_QUERY: usize = 64;

#[derive(Clone)]
pub struct SharedState {
    workspace_root: Option<PathBuf>,
    /// The configuration root (the `Configuration.xml`-bearing directory, e.g. `src/cf`),
    /// which may be nested under `workspace_root`. File-tree lookups such as
    /// `metadata(form)` resolve object directories relative to THIS root, not the repo root.
    source_root: Option<PathBuf>,
    onec_client: Option<OnecClient>,
    onec_connections: BTreeMap<String, OnecConnection>,
    debug_session: Arc<Mutex<Option<bsl_debug::session::DebugSession>>>,
    search_engine: SharedSearchEngine,
    index_progress: Arc<IndexProgress>,
    semantic_runtime: Arc<Mutex<SemanticRuntimeStatus>>,
    /// Outcome of the startup overlay warmup, so `search status` can distinguish "no local
    /// diffs" from "warmup failed" instead of leaving a bare `Ready` ambiguous.
    overlay_warmup: Arc<Mutex<OverlayWarmupState>>,
    workspace_search_mode: WorkspaceSearchMode,
    /// Baseline runtime behind its connect lifecycle: the PG source is built on a
    /// background thread, so construction (and thus the MCP `initialize` handshake)
    /// never waits on the network. Readers see an explicit pending state meanwhile.
    baseline: DeferredBaselineRuntime,
    graph: GraphState,
    diagnostics: DiagnosticsState,
    /// Daemon-owned filesystem change hub. Created before any consumer subscribes
    /// so its lifecycle is independent of the search engine's (which starts later,
    /// in a background init thread). `None` for the reference/shared profiles,
    /// which have no workspace tree to watch. Held so additional sinks (diagnostics
    /// drain-on-read, graph invalidation) can subscribe once they land; the search
    /// sink already runs off a clone taken at construction.
    #[allow(dead_code)]
    change_hub: Option<WorkspaceChangeHub>,
    /// Number of background index/embedding tasks currently in flight. The broker
    /// backend keeps itself alive while this is non-zero so it never idle-exits (and
    /// kills) a long embedding pass. Incremented at the start of each such task and
    /// decremented by [`BackgroundWorkGuard`] on every exit path — including early `?`
    /// returns and panics — so it can never get stuck above zero.
    background_indexers: Arc<AtomicUsize>,
    /// The ONE embed single-flight shared by the boot pass and the post-refresh re-embed kick,
    /// held here so `init_search` reuses the same flight the publish hook does — otherwise the
    /// two could race an index swap and last-writer-wins would install a stale index.
    embed_flight: Arc<embed::EmbedFlight>,
}

#[derive(Clone)]
pub struct OnecConnection {
    client: OnecClient,
    allow_execute: bool,
}

impl OnecConnection {
    pub fn new(client: OnecClient, allow_execute: bool) -> Self {
        Self { client, allow_execute }
    }

    pub fn client(&self) -> &OnecClient {
        &self.client
    }

    pub fn allow_execute(&self) -> bool {
        self.allow_execute
    }
}

impl SharedState {
    pub(crate) fn graph(&self) -> &GraphState {
        &self.graph
    }

    /// Start building the diagnostics resident now instead of on the first tool call.
    ///
    /// A serve path calls this right after construction so the resident (seconds of
    /// enumerate + metadata substrate on a large configuration) is ready before the
    /// agent's first `diagnostics` request rather than billed to it. Deliberately not
    /// part of [`Self::workspace`]: state is also constructed by tests and short-lived
    /// commands that never serve diagnostics, and those must not pay for (or race) a
    /// background resident build. No-op without a workspace root.
    pub fn warm_start(&self) {
        self.diagnostics.ensure_loading();
    }

    pub(crate) fn diagnostics(&self) -> &DiagnosticsState {
        &self.diagnostics
    }

    // Consumed by the diagnostics/graph sinks once they subscribe; exposed now so
    // the hub the daemon owns is reachable from the tool layer.
    #[allow(dead_code)]
    pub(crate) fn change_hub(&self) -> Option<&WorkspaceChangeHub> {
        self.change_hub.as_ref()
    }

    pub fn set_onec_client(&mut self, client: OnecClient) {
        self.onec_client = Some(client);
    }

    pub fn onec_client(&self) -> Option<&OnecClient> {
        self.onec_client.as_ref()
    }

    pub fn add_onec_connection(&mut self, name: String, connection: OnecConnection) {
        self.onec_connections.insert(name, connection);
    }

    pub fn onec_connection(&self, name: Option<&str>) -> Result<OnecConnection, String> {
        if let Some(name) = name {
            return self.onec_connections.get(name).cloned().ok_or_else(|| {
                if self.onec_connections.is_empty() {
                    format!(
                        "Unknown 1C connection '{name}'. No named connections are configured; \
                         omit `connection` to use the --onec-url client."
                    )
                } else {
                    let available =
                        self.onec_connections.keys().cloned().collect::<Vec<_>>().join(", ");
                    format!("Unknown 1C connection '{name}'. Available: {available}")
                }
            });
        }
        if let Some(client) = &self.onec_client {
            // The legacy `--onec-url` client predates per-connection gating; keep run/eval
            // enabled for it — execution is still guarded by the 1C-side role split.
            return Ok(OnecConnection::new(client.clone(), true));
        }
        if self.onec_connections.len() == 1 {
            return Ok(self.onec_connections.values().next().expect("one connection").clone());
        }
        if self.onec_connections.is_empty() {
            return Err(
                "1C HTTP клиент не настроен. Укажите --onec-url или BSL_ONEC_CONNECTIONS_FILE."
                    .to_string(),
            );
        }
        let available = self.onec_connections.keys().cloned().collect::<Vec<_>>().join(", ");
        Err(format!("1C connection is required. Available: {available}"))
    }

    pub fn set_workspace_root(&mut self, root: PathBuf) {
        self.workspace_root = Some(root);
    }

    pub fn workspace_root(&self) -> Option<&PathBuf> {
        self.workspace_root.as_ref()
    }

    /// The configuration root (`Configuration.xml`-bearing directory, e.g. `src/cf`), under
    /// which metadata object directories live. Falls back to `workspace_root` when no nested
    /// configuration root was discovered (a flat layout where the two coincide).
    pub fn source_root(&self) -> Option<&PathBuf> {
        self.source_root.as_ref().or(self.workspace_root.as_ref())
    }

    pub fn debug_session(&self) -> &Arc<Mutex<Option<bsl_debug::session::DebugSession>>> {
        &self.debug_session
    }

    pub fn search_engine(&self) -> &SharedSearchEngine {
        &self.search_engine
    }

    pub fn index_progress(&self) -> &Arc<IndexProgress> {
        &self.index_progress
    }

    /// Whether a long-running background index/embedding task is in flight. The broker
    /// backend uses this so it does not idle-exit (and kill the task) just because no
    /// client is currently connected — the expensive embedding run, which can take far
    /// longer than the idle window, must be allowed to finish so its already-spent work
    /// is not wasted on the next cold start. Backed by a guarded counter that is released
    /// on every task exit path (including panics), so the signal cannot get stuck.
    pub fn background_indexing_active(&self) -> bool {
        self.background_indexers.load(Ordering::SeqCst) > 0
    }

    pub(crate) fn semantic_runtime(&self) -> Arc<Mutex<SemanticRuntimeStatus>> {
        Arc::clone(&self.semantic_runtime)
    }

    pub(crate) fn overlay_warmup(&self) -> Arc<Mutex<OverlayWarmupState>> {
        Arc::clone(&self.overlay_warmup)
    }

    pub(crate) fn workspace_search_mode(&self) -> WorkspaceSearchMode {
        self.workspace_search_mode.clone()
    }

    /// A single-lock snapshot of the baseline lifecycle — the only read surface for
    /// tool handlers. While the deferred connect is `pending`, gates answer "warming —
    /// retry shortly" instead of a config error; one snapshot per request keeps the
    /// pending flag and the runtime pieces describing the same instant.
    pub(crate) fn baseline_view(&self) -> crate::baseline::BaselineView {
        self.baseline.view()
    }

    pub fn shutdown(&self) {
        self.baseline.shutdown();
        self.diagnostics.shutdown();
    }

    /// Prefetch resident snapshots for the overlay's dirty paths and feed them into the
    /// incremental reindex, so a following query serves chunks cut from the SHARED resident
    /// parse instead of a second disk read+parse. Called at the top of a code-search request,
    /// before the query acquires the engine lock.
    ///
    /// Bounded to [`MAX_RESIDENT_PREFETCH_PATHS_PER_QUERY`] paths per call.
    ///
    /// Lock discipline: the resident read must never overlap the engine lock. So this
    /// reads the dirty-path list and the source handle under a brief engine lock, RELEASES it,
    /// fetches the snapshots with NO lock held, then applies them under a second brief engine
    /// lock that only touches the overlay cache (never the resident). A resident that is
    /// absent/loading, or a path it cannot serve, is simply missing from the map and the
    /// reindex disk-reads it — so search never regresses when the resident is unavailable.
    pub(crate) fn prefetch_resident_overlay(engine: &SharedSearchEngine) {
        sync::prefetch_resident_overlay(engine);
    }
}

#[cfg(test)]
mod onec_connection_tests {
    use super::*;

    #[test]
    fn named_connection_is_selected_and_carries_execute_policy() {
        let mut state = SharedState::shared();
        state.add_onec_connection(
            "test".into(),
            OnecConnection::new(OnecClient::new("http://localhost/test", "", ""), true),
        );
        assert!(state.onec_connection(Some("test")).unwrap().allow_execute());
        let error = match state.onec_connection(Some("missing")) {
            Ok(_) => panic!("missing connection must fail"),
            Err(error) => error,
        };
        assert!(error.contains("test"));
    }

    #[test]
    fn legacy_client_keeps_execute_enabled() {
        let mut state = SharedState::shared();
        state.set_onec_client(OnecClient::new("http://localhost/legacy", "", ""));
        assert!(state.onec_connection(None).unwrap().allow_execute());
    }

    #[test]
    fn sole_named_connection_is_default() {
        let mut state = SharedState::shared();
        state.add_onec_connection(
            "only".into(),
            OnecConnection::new(OnecClient::new("http://localhost/only", "", ""), false),
        );
        assert!(!state.onec_connection(None).unwrap().allow_execute());
    }
}
