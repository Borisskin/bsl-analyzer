use bsl_search::{SearchError, SearchHit, Snapshot};
use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

pub(super) const DIRECT_SEARCH_INITIAL_WINDOW_MULTIPLIER: usize = 3;
pub(super) const DIRECT_SEARCH_MAX_WINDOW_MULTIPLIER: usize = 10;
pub(super) const DIRECT_SEARCH_MIN_MAX_WINDOW: usize = 100;
pub(super) const DIRECT_SEARCH_MAX_REFILL_ROUNDS: usize = 4;

/// How much wider than the caller's `limit` each modality is queried before fusion, so a hit
/// ranked just outside `limit` in one modality but boosted by the other can still surface.
pub(super) const HYBRID_FETCH_MULTIPLIER: usize = 2;

/// The version of the `search` structured hit contract: the fields of one hit object and the
/// envelope around the list. Bump it whenever that shape changes — a machine consumer pins
/// against this, whereas the text listing is a human mirror with no such promise.
///
/// `2` adds `root_id` to every code hit: with extensions in the index the same relative path
/// exists under several roots, so the owning root became part of a hit's identity.
///
/// `3` adds the location contract: a `location` (or a machine `location_unavailable` reason)
/// per code hit and, for `search_code`, a `freshness` envelope. The legacy 1-based
/// `line_start`/`line_end` are untouched.
///
/// `5` — `search_code` only — adds `freshness.drift_watch` and the completeness reasons the
/// overlay's own state gives. `6` adds optional semantic failure diagnostics; the independent
/// documentation schema advances from `4` to `5` for the same diagnostics.
pub(super) const SEARCH_CODE_SCHEMA_VERSION: &str = "6";
pub(super) const DOCS_SCHEMA_VERSION: &str = "5";
// Schema-only mirrors keep the MCP dependency out of the native search crate.
// The parity test below validates every native failure code against this schema.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, reason = "schema-only mirror of the native embedding failure")]
struct SemanticFailureSchema {
    code: SemanticFailureCodeSchema,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(default, with = "usize", extend("maximum" = u64::MAX))]
    request_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(default, with = "usize", extend("maximum" = u64::MAX))]
    max_request_bytes: Option<usize>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
#[allow(
    dead_code,
    clippy::enum_variant_names,
    reason = "schema-only mirror of the native embedding failure codes"
)]
enum SemanticFailureCodeSchema {
    EmbeddingInvalidConfig,
    EmbeddingInputTooLarge,
    EmbeddingRequestTooLarge,
    EmbeddingResponseTooLarge,
    EmbeddingTimeout,
    EmbeddingTransportError,
    EmbeddingProviderError,
    EmbeddingInvalidResponse,
    EmbeddingFailed,
}

/// The schema version an action's answer carries.
pub(super) fn search_schema_version(action: &str) -> &'static str {
    if action == "search_code" {
        SEARCH_CODE_SCHEMA_VERSION
    } else {
        DOCS_SCHEMA_VERSION
    }
}

// `C` is the version `search_code` answers carry; everything else is the same union in both
// profiles. Not a doc comment: it would land in the published schema as its description.
#[derive(JsonSchema, Serialize)]
#[serde(untagged)]
#[allow(dead_code, reason = "schema-only union published by tools/list")]
enum SearchOutput<C> {
    SearchCode(SearchHits<SearchCodeAction, C>),
    FindDocs(SearchHits<FindDocsAction, SearchSchemaVersion>),
    SearchDocs(SearchHits<SearchDocsAction, SearchSchemaVersion>),
    SearchCodeNotReady(SearchNotReady<SearchCodeAction, C>),
    FindDocsNotReady(SearchNotReady<FindDocsAction, SearchSchemaVersion>),
    SearchDocsNotReady(SearchNotReady<SearchDocsAction, SearchSchemaVersion>),
    ListPlatform {
        action: ListPlatformAction,
        schema_version: ListPlatformSchemaVersion,
        items: Vec<crate::tools::platform::PlatformReference>,
        shown: usize,
        total: usize,
        budget_exhausted: bool,
        budget_hint: Option<String>,
    },
    Status {
        action: StatusAction,
        schema_version: StatusSchemaVersion,
        profile: SearchProfile,
        state: SearchState,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[schemars(default, with = "SemanticFailureSchema")]
        semantic_failure: Option<bsl_search::EmbeddingFailure>,
    },
}

#[derive(JsonSchema, Serialize)]
struct SearchHits<A, V> {
    action: A,
    schema_version: V,
    hits: Vec<Value>,
    shown: usize,
    total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(default, with = "SemanticFailureSchema")]
    semantic_failure: Option<bsl_search::EmbeddingFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_exhausted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_hint: Option<String>,
}

#[derive(JsonSchema, Serialize)]
struct SearchNotReady<A, V> {
    action: A,
    schema_version: V,
    status: NotReadyStatus,
    retry_after_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(default, with = "SemanticFailureSchema")]
    semantic_failure: Option<bsl_search::EmbeddingFailure>,
}

macro_rules! const_enum {
    ($name:ident, $variant:ident, $wire:literal) => {
        #[derive(JsonSchema, Serialize)]
        #[allow(dead_code, reason = "schema-only const discriminator")]
        enum $name {
            #[serde(rename = $wire)]
            $variant,
        }
    };
}

const_enum!(SearchCodeAction, SearchCode, "search_code");
const_enum!(FindDocsAction, FindDocs, "find_docs");
const_enum!(SearchDocsAction, SearchDocs, "search_docs");
const_enum!(ListPlatformAction, ListPlatform, "list_platform");
const_enum!(StatusAction, Status, "status");
const_enum!(SearchCodeSchemaVersion, V6, "6");
const_enum!(SearchSchemaVersion, V5, "5");
const_enum!(ListPlatformSchemaVersion, V1, "1");
const_enum!(StatusSchemaVersion, V2, "2");
const_enum!(NotReadyStatus, NotReady, "not_ready");

#[derive(JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code, reason = "schema-only enum")]
enum SearchProfile {
    Workspace,
    Reference,
}

#[derive(JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code, reason = "schema-only enum")]
enum SearchState {
    Ready,
    Loading,
    Busy,
    Failed,
}

/// The `workspace` profile's `search` answers: `search_code` on its own version.
pub(crate) fn search_output_schema() -> Arc<serde_json::Map<String, Value>> {
    output_schema::<SearchOutput<SearchCodeSchemaVersion>>()
}

/// The `reference` profile's `search` answers. It serves no `search_code`, and its schema is
/// the one it published before `search_code` got a number of its own.
pub(crate) fn reference_search_output_schema() -> Arc<serde_json::Map<String, Value>> {
    output_schema::<SearchOutput<SearchSchemaVersion>>()
}

fn output_schema<T: JsonSchema + std::any::Any>() -> Arc<serde_json::Map<String, Value>> {
    let mut schema = (*rmcp::handler::server::tool::schema_for_type::<T>()).clone();
    crate::contract::ensure_object_root(&mut schema);
    Arc::new(schema)
}

/// The outcome of producing one modality's code hits, separated from presentation so the
/// hybrid path can fuse two modalities. Hard policy/terminal failures stay `Err(McpError)`;
/// these soft states let `hybrid_code` reproduce today's lexical messages and degrade
/// gracefully on a semantic shortfall.
pub(super) enum CodeHits {
    /// Hits (possibly empty) plus the root table the graph-id bridge anchors them with.
    /// The whole table, not one root: a hit's path is relative to the root that owns it, and
    /// which root that is differs per hit.
    Ready {
        hits: Vec<SearchHit>,
        roots: Option<bsl_search::WorkspaceRoots>,
        /// The workspace overlay's `(pending, unread)` counts, read under the same engine lock
        /// as the hits, when the answer came from the engine at all.
        overlay: Option<(usize, usize)>,
    },
    /// The index/overlay is still warming or building — no hits yet, emit `message`.
    Pending(String),
    /// The semantic modality cannot serve this request; `hybrid_code` degrades to lexical.
    Unavailable(SemanticUnavailable),
}

/// Why semantic search could not serve a request — carried so `hybrid_code` can name the
/// reason in its degradation note instead of hard-failing the whole search.
pub(super) enum SemanticUnavailable {
    NotConfigured,
    RuntimeFailed,
    BaselineNotReady,
    BaselineRequired,
    /// The reader's configured embedding model/dimension differs from what the shared baseline
    /// was indexed with, so its query vectors cannot be compared against the stored ones. The
    /// carried string names both identities and the env/config knobs to reconcile them.
    IdentityMismatch(String),
    /// A safe build/query failure; query diagnostics remain response-local.
    EmbeddingFailed(bsl_search::EmbeddingFailure),
}

impl SemanticUnavailable {
    pub(super) fn note(&self) -> String {
        match self {
            Self::NotConfigured => {
                "semantic skipped: not configured (set EMBEDDING_URL)".to_owned()
            }
            Self::RuntimeFailed => "semantic skipped: runtime initialization failed".to_owned(),
            Self::BaselineNotReady => {
                "semantic skipped: PostgreSQL baseline semantic not ready".to_owned()
            }
            Self::BaselineRequired => {
                "semantic skipped: requires PostgreSQL baseline serving".to_owned()
            }
            Self::IdentityMismatch(message) => message.clone(),
            Self::EmbeddingFailed(_) => "semantic skipped: embedding failed".to_owned(),
        }
    }

    pub(super) fn embedding_failure(&self) -> Option<bsl_search::EmbeddingFailure> {
        match self {
            Self::EmbeddingFailed(failure) => Some(*failure),
            _ => None,
        }
    }
}

/// How a search body ends short of an answer.
///
/// Cancellation is a VALUE that flows back through `Result`, never an unwind: the engine
/// guard is a `std::sync::Mutex` guard, and unwinding through it would poison the lock for
/// every later search. A cancelled body returns `Cancelled` from its next cooperative point
/// and releases whatever it held by ordinary return.
///
/// Kept apart from [`McpError`] on purpose — an error is rendered to the client, while a
/// cancelled call renders nothing (see `docs/mcp/LOCATION_CONTRACT.md`).
#[derive(Debug)]
pub(crate) enum SearchFailure {
    /// The client cancelled the request (or the transport went away).
    Cancelled,
    /// A real failure, rendered to the client as-is.
    Error(McpError),
}

impl From<McpError> for SearchFailure {
    fn from(error: McpError) -> Self {
        Self::Error(error)
    }
}

impl From<super::wait::Withdrawn> for SearchFailure {
    fn from(_: super::wait::Withdrawn) -> Self {
        Self::Cancelled
    }
}

impl SearchFailure {
    /// The error of a call that was not cancelled — for tests that drive a search with a
    /// token they never cancel and assert on the error it produced.
    #[cfg(test)]
    pub(crate) fn expect_error(self) -> McpError {
        match self {
            Self::Error(error) => error,
            Self::Cancelled => panic!("the call was cancelled, but the test never cancelled it"),
        }
    }
}

/// Why [`super::try_acquire_engine`] could not hand back the engine
/// guard. The cases need different caller responses, so they stay distinct rather than
/// collapsing into one `None`: a poisoned lock is a real failure (retrying is futile), a
/// timeout is a stall (retrying or degrading to the baseline is reasonable), and a cancelled
/// wait produces no answer at all.
pub(crate) enum AcquireFailure {
    /// A holder panicked while holding the lock; waiting cannot recover it.
    Poisoned,
    /// The lock stayed held past the safety cap — a genuine stall, not ordinary contention.
    TimedOut,
    /// The request was cancelled while waiting; nothing was acquired.
    Cancelled,
}

#[derive(Debug)]
pub(super) enum DirectResult {
    Found(Vec<SearchHit>),
    Unavailable,
    Terminal(SearchError),
    /// The request was cancelled while waiting on the baseline actor. Its own variant, not
    /// `Unavailable`: an unavailable baseline is answered with a fallback or a retry
    /// envelope, and a cancelled call must produce neither.
    Cancelled,
}

/// Outcome of the lock-free baseline readiness check that runs before the query embed.
pub(super) enum DirectResolve {
    /// The baseline is reachable and has a snapshot; carry the ids needed for the search.
    Ready { snapshot: Snapshot, model_id: String, dim: usize },
    /// The baseline is not ready or the engine has no embedding model/dim.
    Unavailable,
    /// A terminal error from the baseline actor (network/auth failure that retrying cannot fix).
    Terminal(SearchError),
    /// The request was cancelled while waiting on the baseline actor.
    Cancelled,
}

pub(super) fn direct_search_initial_window(limit: usize) -> usize {
    limit.max(1).saturating_mul(DIRECT_SEARCH_INITIAL_WINDOW_MULTIPLIER)
}

pub(super) fn direct_search_max_window(limit: usize) -> usize {
    direct_search_initial_window(limit).max(
        limit.saturating_mul(DIRECT_SEARCH_MAX_WINDOW_MULTIPLIER).max(DIRECT_SEARCH_MIN_MAX_WINDOW),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bsl_search::{EmbeddingFailure, EmbeddingFailureCode};
    use serde_json::json;

    #[test]
    fn payload_failure_schema_matches_native_serialization_and_rejects_unsafe_shapes() {
        let schema = serde_json::to_value(schemars::schema_for!(SemanticFailureSchema)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let codes = [
            EmbeddingFailureCode::EmbeddingInvalidConfig,
            EmbeddingFailureCode::EmbeddingInputTooLarge,
            EmbeddingFailureCode::EmbeddingRequestTooLarge,
            EmbeddingFailureCode::EmbeddingResponseTooLarge,
            EmbeddingFailureCode::EmbeddingTimeout,
            EmbeddingFailureCode::EmbeddingTransportError,
            EmbeddingFailureCode::EmbeddingProviderError,
            EmbeddingFailureCode::EmbeddingInvalidResponse,
            EmbeddingFailureCode::EmbeddingFailed,
        ];
        assert_eq!(
            schema["$defs"]["SemanticFailureCodeSchema"]["enum"].as_array().unwrap().len(),
            codes.len()
        );
        for code in codes {
            let value = serde_json::to_value(EmbeddingFailure::new(code)).unwrap();
            validator.validate(&value).unwrap_or_else(|error| panic!("{value}: {error}"));
            assert_eq!(value, json!({"code": code.as_str()}));
        }
        let failure = EmbeddingFailure {
            code: EmbeddingFailureCode::EmbeddingInputTooLarge,
            request_bytes: Some(4097),
            max_request_bytes: Some(4096),
        };
        let value = serde_json::to_value(failure).unwrap();
        assert!(validator.is_valid(&value));
        for invalid in [
            json!({"code": "unknown"}),
            json!({"code": "embedding_failed", "detail": "private"}),
            json!({"code": "embedding_failed", "url": "private"}),
            json!({"request_bytes": 4097}),
        ] {
            assert!(!validator.is_valid(&invalid), "accepted {invalid}");
        }
        for field in ["request_bytes", "max_request_bytes"] {
            for invalid in [json!(-1), json!(1.5), json!(1e30), json!("4096"), Value::Null] {
                let mut bad = value.clone();
                bad[field] = invalid;
                assert!(!validator.is_valid(&bad), "accepted {bad}");
            }
        }

        let output_schema = Value::Object((*search_output_schema()).clone());
        let output_validator = jsonschema::validator_for(&output_schema).unwrap();
        for action in ["search_code", "find_docs", "search_docs"] {
            let ready = super::super::render::no_hits_response(
                None,
                super::super::render::Envelope::No,
                action,
                None,
            )
            .structured_content
            .unwrap();
            let pending = super::super::status::docs_not_ready(action).structured_content.unwrap();
            for mut output in [ready, pending] {
                assert!(output.get("semantic_failure").is_none());
                assert!(output_validator.is_valid(&output), "{output}");
                output["semantic_failure"]["code"] = json!("unknown");
                assert!(!output_validator.is_valid(&output), "accepted {output}");
            }
            let ready = super::super::render::failure_hits_response(
                Vec::new(),
                "",
                None,
                super::super::render::Envelope::No,
                action,
                failure,
                6000,
                None,
            );
            let pending = super::super::render::not_ready_with_failure(
                super::super::status::docs_not_ready(action),
                Some(failure),
            );
            for result in [ready, pending] {
                let output = result.structured_content.unwrap();
                assert_eq!(output["semantic_failure"], value);
                assert!(output_validator.is_valid(&output), "{output}");
            }
        }
        use crate::state::{OverlayWarmupState, SemanticRuntimeStatus, WorkspaceSearchMode};
        for profile in [crate::McpProfile::Workspace, crate::McpProfile::Reference] {
            for known_failure in [None, Some(failure)] {
                let runtime = known_failure
                    .map(SemanticRuntimeStatus::EmbeddingFailed)
                    .unwrap_or(SemanticRuntimeStatus::Ready);
                let result = super::super::status::search_status_with_cap(
                    profile,
                    &crate::state::shared_engine(None),
                    &bsl_search::IndexProgress::new(),
                    &Arc::new(std::sync::Mutex::new(runtime)),
                    WorkspaceSearchMode::SqliteLocal,
                    OverlayWarmupState::Pending,
                    None,
                    None,
                    false,
                    std::time::Duration::ZERO,
                )
                .unwrap();
                let mut output = result.structured_content.unwrap();
                assert_eq!(output.get("semantic_failure"), known_failure.map(|_| &value));
                assert!(output_validator.is_valid(&output), "{output}");
                output["semantic_failure"]["extra"] = json!(true);
                assert!(!output_validator.is_valid(&output));
            }
        }
        let error = super::super::render::embedding_mcp_error(failure);
        assert_eq!(error.message, "Semantic embedding failed; use lexical search or retry.");
        let data = error.data.unwrap();
        assert_eq!(data, json!({"semantic_failure": value}));
        assert!(validator.is_valid(&data["semantic_failure"]));
        let reason = SemanticUnavailable::EmbeddingFailed(failure);
        assert_eq!(reason.embedding_failure(), Some(failure));
        assert_eq!(reason.note(), "semantic skipped: embedding failed");
    }

    #[test]
    fn identity_mismatch_note_surfaces_the_carried_actionable_message() {
        let message = "semantic skipped: this baseline was indexed with model 'a' (dim 768), \
                       but the reader is configured with model 'b' (dim 1024); set \
                       EMBEDDING_MODEL/EMBEDDING_DIM (or [search.baseline.embedding] in \
                       bsl-analyzer.toml) to match and restart";
        let reason = SemanticUnavailable::IdentityMismatch(message.to_owned());

        assert_eq!(reason.note(), message);
    }
}
