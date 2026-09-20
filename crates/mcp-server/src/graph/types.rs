#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GraphStatus {
    /// Not a workspace profile — the graph is unavailable.
    Disabled,
    /// A workspace is configured but the graph has not been loaded yet; the
    /// first `graph` tool call triggers the load.
    Idle,
    /// Background load in progress.
    Loading,
    /// Ready to serve, with the indexed `.bsl` file count.
    Ready { files: usize },
    /// Load failed.
    Failed(String),
}

pub(crate) const SUPERSEDED_GRAPH_ERROR: &str =
    "another daemon generation superseded this graph; reconnect to use the current cache";

/// What the graph hands its publish hook after a build publishes.
#[derive(Clone, Debug)]
pub(crate) struct GraphPublishSignal {
    /// A fresher reload is already catching up: a fast-path hint the consumer may use to
    /// skip this round and let that reload's publish do the re-render. Not correctness-bearing.
    pub(crate) drift_pending: bool,
    /// The highest context-dirty mark this publication may consume: marks placed for facts
    /// the publication observed (see [`crate::graph::GraphState::marks_placed`]), never one
    /// whose fact reached the hub after its build scanned disk. `0` consumes nothing. This
    /// bound is what makes the consumption correct.
    pub(crate) mark_bound: i64,
    /// The published build's extension topology differs from the previously published
    /// one (or nothing was published before, so persisted search contexts cannot be
    /// trusted). The consumer must conservatively re-render EVERY document's graph
    /// context — a topology change re-shapes visibility with no per-object mark to go by.
    pub(crate) topology_changed: bool,
    /// The extension topology of the snapshot this publish made current. A consumer that
    /// re-opens the graph database by path must check it against the file it actually got:
    /// a daemon of another generation may have renamed ITS build into the same path. The
    /// topology alone is insufficient: two generations can share it while carrying different
    /// files and roots, so consumers compare the complete token below.
    pub(crate) topology: u64,
    /// The revision recorded by the published graph database. Together with `fingerprint`,
    /// this identifies the exact generation whose graph rows and roots the hook may use.
    pub(crate) revision: u64,
    /// The complete graph fingerprint recorded by the published graph database. It is paired
    /// with `revision` to prevent a rename race from mixing generations that share a topology.
    pub(crate) fingerprint: crate::graph_db::GraphFp,
    /// Whether the consumer must compare/install the search root table carried by this
    /// publication. False for a context-only retry, so a root transition failure never
    /// inflates into an unrelated whole-collection context refresh.
    pub(crate) roots_refresh_requested: bool,
    /// Search roots paired with this publication. A fresh build carries the exact
    /// [`crate::graph::ProjectSnapshot`] it built from; cached adoption carries the current
    /// validated project snapshot after proving its graph fingerprint matches the artifact.
    /// `None` means project loading failed; consumers keep their last-known-good table and
    /// report the root request unhandled.
    pub(crate) workspace_roots: Option<bsl_search::WorkspaceRoots>,
}

/// Independent results of one graph publish hook invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GraphPublishOutcome {
    pub(crate) topology_handled: bool,
    pub(crate) roots_handled: bool,
}

impl GraphPublishOutcome {
    pub(crate) const HANDLED: Self = Self { topology_handled: true, roots_handled: true };
}

/// Freshness verdict for one `graph` response.
pub(crate) struct Freshness {
    /// The generation of the snapshot that served this response.
    pub revision: u64,
    /// The workspace drifted on disk since this snapshot was built.
    pub stale: bool,
    /// Reload state: `"none"`, `"running"`, or `"failed"`.
    pub reload: &'static str,
    /// The extension topology the serving snapshot was built for, published so an answer
    /// names the root set it describes.
    pub topology: u64,
    /// Who watches this graph's drift, and how: what `stale` rests on.
    pub drift_watch: crate::tools::location::DriftWatch,
}

/// The graph's lifecycle snapshot for the `status` action — the parallel of the
/// `diagnostics` status, so an agent can start the lazy build and poll its progress
/// instead of polling a data action and reading a flat `loading` envelope.
pub(crate) struct GraphStatusReport {
    /// `disabled` | `loading` | `ready` | `failed`.
    pub state: &'static str,
    /// Indexed `.bsl` file count (when `ready`).
    pub files: Option<usize>,
    /// Modules the build could not read (when `ready`). They contributed no nodes and
    /// no edges, so the graph is incomplete in a way no fingerprint reveals.
    pub unread_files: Option<usize>,
    /// Served snapshot generation (when `ready`).
    pub revision: Option<u64>,
    /// Whether the workspace drifted on disk since the build (when `ready`).
    pub stale: Option<bool>,
    /// Background reload state `none`/`running`/`failed` (when `ready`).
    pub reload: Option<&'static str>,
    /// Failure message (when `failed`).
    pub error: Option<String>,
    /// `Some(true)` when a newer daemon generation owns this workspace's derived caches (see
    /// [`crate::workspace_lease`]): this graph still serves, but it does not rebuild, so a
    /// `stale` snapshot stays stale here. Emitted only when true — it explains a drift that
    /// never gets picked up, which would otherwise look like a reload that never runs.
    pub superseded: Option<bool>,
    /// Who watches this graph's drift (when the graph is a workspace's).
    pub drift_watch: Option<&'static str>,
    /// While polling: how long an edit that keeps its size and mtime can go unnoticed.
    pub poll_cycle_secs: Option<u64>,
}

/// Whether the SqliteLocal startup graph decision already populated the search index.
pub(crate) enum FusedStartup {
    /// Fused cold-build ran: graph + search chunks were written from one parse pass;
    /// the caller must fill embeddings via [`bsl_search::SearchEngine::embed_pending_chunks_standalone`].
    Fused,
    /// Graph served from cache or built the normal lazy way; the caller indexes the
    /// search engine via the standalone path.
    Standalone,
}
