use bsl_search::{EmbeddingFailure, SearchEngine, SearchError};
use std::path::PathBuf;
use std::sync::Arc;

/// The search engine behind a mutex. It MUST stay a `Mutex` (not an `RwLock`): the engine
/// owns a `rusqlite::Connection`, which is `Send` but `!Sync` — its internal statement cache
/// mutates through a `RefCell` even on read-only SQL, so two threads may never hold `&engine`
/// at once. Searches therefore serialize here by necessity. The "overlay warming up" failure
/// under a concurrent batch is fixed in [`crate::tools::search`] by *blocking* on this lock
/// (queueing) rather than bailing out on brief contention, not by widening the lock.
pub(crate) type SharedSearchEngine = Arc<crate::tools::search::AdmittedEngine>;

/// A shared engine handle over `engine`, admitted in queue order (see
/// [`crate::tools::search::AdmittedEngine`]).
pub(crate) fn shared_engine(engine: Option<SearchEngine>) -> SharedSearchEngine {
    Arc::new(crate::tools::search::AdmittedEngine::new(engine))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceSearchMode {
    SqliteLocal,
    PostgresRemoteOverlay,
}

/// Outcome of the one-shot PostgresRemoteOverlay warmup that embeds local working-tree diffs
/// against the published baseline at startup. Tracked separately from [`SemanticRuntimeStatus`]
/// so `search status` can tell "no local diffs" (semantic is fully baseline-served, nothing to
/// build) apart from "warmup failed" (baseline still serves, but local edits are NOT in the
/// semantic index): a bare `Ready` + empty overlay is ambiguous between the two.
#[derive(Debug, Clone)]
pub(crate) enum OverlayWarmupState {
    /// Not started, in progress, or a non-overlay mode where no warmup runs.
    Pending,
    /// Warmup did not run: no embedder configured or no workspace root.
    Skipped(String),
    /// Completed; nothing in the working tree differed from the baseline.
    NoLocalDiffs,
    /// Completed; embedded `embedded` chunks across `overlay_files` locally-changed files.
    Synced { overlay_files: usize, embedded: usize },
    /// Completed, but the pass could not vouch for the whole tree: the scan left `unreadable`
    /// subtrees unread or walked `canonical_fallbacks` files without a physical spelling, or
    /// `read_failures` seen files could not be read. What WAS seen is published and serving;
    /// removals were withheld and stale entries may linger until a clean pass. Deliberately not
    /// `Failed`: the overlay is live, and `Failed`'s restart advice would be wrong here.
    Incomplete {
        unreadable: usize,
        canonical_fallbacks: usize,
        read_failures: usize,
        /// The publication landed in memory but the fingerprint-row persist failed: stale
        /// rows survive on disk, and the pass must be repeated once the store recovers.
        persist_failed: bool,
    },
    /// The plan was built against a state a wholesale invalidation replaced between its
    /// phases: nothing was published (only value-stable embeddings merged) and a fresh pass
    /// is owed. Not `Failed`: nothing is broken, the state simply moved on.
    Superseded,
    /// Prime or publish failed. The baseline semantic index still serves; local edits are not
    /// reflected semantically until the next MCP restart retries the warmup.
    Failed(String),
    /// An embedding failure belongs to this overlay attempt, independently of the main index.
    EmbeddingFailed(EmbeddingFailure),
}

impl OverlayWarmupState {
    pub(crate) fn from_search_error(error: &SearchError) -> Self {
        match error.embedding_failure() {
            Some(failure) => Self::EmbeddingFailed(failure),
            None => Self::Failed(error.to_string()),
        }
    }

    pub(crate) fn embedding_failure(&self) -> Option<EmbeddingFailure> {
        match self {
            Self::EmbeddingFailed(failure) => Some(*failure),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SemanticRuntimeStatus {
    Disabled,
    OverlaySyncing,
    /// The local SQLite semantic index is being built in the background after the
    /// engine was published early: lexical search and the call graph are already live,
    /// while the RAG vectors fill in over the longer embedding pass. Distinct from
    /// [`Self::OverlaySyncing`], which is the remote-baseline overlay warmup.
    Indexing,
    Ready,
    Failed(String),
    /// The background pass left because the daemon asked its owners to go. Terminal, and
    /// deliberately NOT [`Self::Indexing`]: that reads "come back in a moment", and after a
    /// shutdown nothing is coming. Deliberately not [`Self::Ready`] either — the index may
    /// have been left half filled, and a semantic search over it would answer a silent partial.
    ///
    /// Reported to callers exactly as [`Self::Failed`] is, so no reason code and no part of
    /// the MCP contract changes; what differs is that the log and the status line say shutdown
    /// rather than breakage.
    Stopped,
    EmbeddingFailed(EmbeddingFailure),
}

impl SemanticRuntimeStatus {
    pub(crate) fn from_search_error(error: &SearchError) -> Self {
        match error.embedding_failure() {
            Some(failure) => Self::EmbeddingFailed(failure),
            None => Self::Failed(error.to_string()),
        }
    }

    pub(crate) fn embedding_failure(&self) -> Option<EmbeddingFailure> {
        match self {
            Self::EmbeddingFailed(failure) => Some(*failure),
            _ => None,
        }
    }

    pub(crate) fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::EmbeddingFailed(_))
    }
}

/// How the boot must initialize the workspace overlay before the engine is published. The overlay
/// is inert until initialized — a point refresh captures nothing before that — so without
/// one of these the whole resident-fed incremental reindex (and overlay edit-freshness) is
/// unreachable in local SQLite mode.
pub(super) enum OverlayInit {
    /// The boot branch already re-ingested current disk into the store (fused parse ingest, or an
    /// `index_directory_deferred`/`index_directory_fts` walk+hash re-ingest), so the overlay
    /// baseline == working tree: mark it initialized with no entries. A prime here would scan the
    /// whole tree only to build zero diffs, so this is the zero-cost equivalent.
    Clean,
    /// The store was reused warm WITHOUT re-reconciling it against disk (FTS-only reuse skips
    /// re-indexing when chunks already exist), so a file changed while the daemon was down is not
    /// yet in the store and empty-init would be false-clean for it. A disk scan is required to build
    /// the overlay diff, so prime rather than empty-init.
    Prime,
    /// PostgresRemoteOverlay: the async remote warmup owns overlay initialization
    /// (`needs_overlay_warmup`), so the synchronous boot path does nothing here.
    RemoteWarmup,
}

pub(super) struct WorkspaceSearchInit {
    pub(super) engine: SearchEngine,
    pub(super) mode: WorkspaceSearchMode,
    /// Set by the fused cold-build path: the engine is published with FTS + graph
    /// context already written but embeddings still NULL, and this carries what the
    /// background pass needs to fill them on its own connection. `None` means the
    /// engine is fully ready (warm cache, FTS-only, or standalone reindex).
    pub(super) pending_embed: Option<PendingEmbed>,
    /// How to bring the workspace overlay online for this boot branch.
    pub(super) overlay_init: OverlayInit,
}

/// Inputs for the background embedding pass: its own database path and embedder config
/// so [`bsl_search::SearchEngine::embed_pending_chunks_standalone`] opens a separate WAL
/// connection and never holds the live engine's mutex during the long embed.
pub(super) struct PendingEmbed {
    pub(super) db_path: PathBuf,
    pub(super) config: bsl_search::SearchConfig,
}
