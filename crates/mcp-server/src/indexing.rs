//! One bounded projection of native indexing state for every MCP response surface.
use bsl_search::{IndexPassState, IndexPhase, IndexProgressSnapshot};
use rmcp::model::{CallToolResult, ContentBlock};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Graph,
    Lexical,
    Semantic,
    Reference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Waiting,
    Running,
    Ready,
    Disabled,
    Failed,
    Cancelled,
    Superseded,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Initializing,
    Parsing,
    LexicalIndexing,
    Embedding,
    Persisting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    Initializing,
    PendingWork,
    SemanticDisabled,
    CoverageUnverified,
    IdentityUnverified,
    BaselineUnavailable,
    OverlayPending,
    StaleGeneration,
    SnapshotUnavailable,
    NativeFailure,
    Cancelled,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code, reason = "closed wire vocabulary includes native file and batch producers")]
pub(crate) enum Unit {
    Files,
    Chunks,
    Batches,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = require_all_fields)]
pub(crate) struct Progress {
    completed: usize,
    total: Option<usize>,
    unit: Unit,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = require_all_fields)]
pub(crate) struct Target {
    pub(crate) kind: Kind,
    pub(crate) state: State,
    phase: Option<Phase>,
    progress: Option<Progress>,
    #[schemars(length(min = 34, max = 53), regex(pattern = "^[0-9a-f]{32}:[1-9][0-9]{0,19}$"))]
    pass_id: Option<String>,
    pub(crate) reason_code: Option<Reason>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Indexing {
    schema_version: Version,
    #[schemars(length(min = 1, max = 4))]
    targets: Vec<Target>,
    #[serde(skip)]
    #[schemars(skip)]
    legacy: Option<Value>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
enum Version {
    #[serde(rename = "1")]
    V1,
}

impl Target {
    pub(crate) fn new(kind: Kind, state: State, reason_code: Option<Reason>) -> Self {
        Self { kind, state, phase: None, progress: None, pass_id: None, reason_code }
    }

    pub(crate) fn unknown(kind: Kind) -> Self {
        Self::new(kind, State::Unknown, Some(Reason::SnapshotUnavailable))
    }

    pub(crate) fn with_attempt(mut self, sample: Option<&IndexProgressSnapshot>) -> Self {
        self.pass_id = sample.and_then(|sample| sample.pass_id.clone());
        self
    }

    pub(crate) fn native(kind: Kind, sample: &IndexProgressSnapshot) -> Self {
        let state = match sample.state {
            IndexPassState::Waiting => State::Waiting,
            IndexPassState::Running if !sample.active => State::Waiting,
            IndexPassState::Running => State::Running,
            IndexPassState::Ready => State::Ready,
            IndexPassState::Disabled => State::Disabled,
            IndexPassState::Failed => State::Failed,
            IndexPassState::Cancelled => State::Cancelled,
            IndexPassState::Superseded => State::Superseded,
            IndexPassState::Unknown => State::Unknown,
        };
        let reason = match state {
            State::Waiting => Some(Reason::PendingWork),
            State::Disabled => Some(Reason::SemanticDisabled),
            State::Failed => Some(Reason::NativeFailure),
            State::Cancelled => Some(Reason::Cancelled),
            State::Superseded => Some(Reason::Superseded),
            State::Unknown => Some(Reason::SnapshotUnavailable),
            State::Ready | State::Running => None,
        };
        let mut target = Self::new(kind, state, reason);
        target.pass_id = sample.pass_id.clone();
        if state == State::Running {
            target.phase = sample.phase.map(|phase| match phase {
                IndexPhase::Initializing => Phase::Initializing,
                IndexPhase::Parsing => Phase::Parsing,
                IndexPhase::LexicalIndexing => Phase::LexicalIndexing,
                IndexPhase::Embedding => Phase::Embedding,
                IndexPhase::Persisting => Phase::Persisting,
            });
            if let Some(counters) = &sample.counters {
                if sample.active
                    && counters.total_chunks.is_none_or(|total| counters.done_chunks <= total)
                    && counters.total_batches.is_none_or(|total| counters.done_batches <= total)
                {
                    target.progress = Some(
                        if counters.total_chunks.is_none() && counters.total_batches.is_some() {
                            Progress {
                                completed: counters.done_batches,
                                total: counters.total_batches,
                                unit: Unit::Batches,
                            }
                        } else {
                            Progress {
                                completed: counters.done_chunks,
                                total: counters.total_chunks,
                                unit: Unit::Chunks,
                            }
                        },
                    );
                }
            }
        }
        target
    }
}

impl Indexing {
    pub(crate) fn single(target: Target) -> Self {
        Self { schema_version: Version::V1, targets: vec![target], legacy: None }
    }

    pub(crate) fn workspace(
        lexical: Target,
        semantic: Target,
        sample: Option<&IndexProgressSnapshot>,
    ) -> Self {
        debug_assert_eq!(lexical.kind, Kind::Lexical);
        debug_assert_eq!(semantic.kind, Kind::Semantic);
        Self {
            schema_version: Version::V1,
            targets: vec![lexical, semantic],
            legacy: Some(legacy_progress(sample)),
        }
    }

    pub(crate) fn attach(&self, response: &mut CallToolResult) {
        let Some(body) = response.structured_content.as_mut() else { return };
        let old_mirror = serde_json::to_string(body).expect("JSON serializes");
        body["indexing"] = serde_json::to_value(self).expect("indexing serializes");
        if body.get("progress").is_some() {
            if let Some(legacy) = &self.legacy {
                body["progress"] = legacy.clone();
            }
        }
        let mirror = serde_json::to_string(body).expect("JSON serializes");
        let mut mirrored = false;
        for content in &mut response.content {
            if let ContentBlock::Text(text) = content {
                if text.text == old_mirror {
                    text.text.clone_from(&mirror);
                    mirrored = true;
                }
            }
        }
        if !mirrored {
            response.content.push(ContentBlock::text(self.text()));
        }
    }

    fn text(&self) -> String {
        self.targets
            .iter()
            .map(|target| {
                let kind = serde_json::to_value(target.kind).expect("enum serializes");
                let state = serde_json::to_value(target.state).expect("enum serializes");
                let mut line =
                    format!("Indexing {}: {}", kind.as_str().unwrap(), state.as_str().unwrap());
                if let Some(phase) = target.phase {
                    let phase = serde_json::to_value(phase).expect("enum serializes");
                    line.push_str(&format!(" ({})", phase.as_str().unwrap()));
                }
                if let Some(progress) = &target.progress {
                    let unit = serde_json::to_value(progress.unit).expect("enum serializes");
                    let total =
                        progress.total.map_or_else(|| "unknown".to_owned(), |n| n.to_string());
                    line.push_str(&format!(
                        ": {}/{total} {}",
                        progress.completed,
                        unit.as_str().unwrap()
                    ));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub(crate) fn legacy_progress(sample: Option<&IndexProgressSnapshot>) -> Value {
    let mut value = json!({"active": sample.is_some_and(|sample| sample.active)});
    if let Some(sample) =
        sample.filter(|sample| sample.active && sample.state == IndexPassState::Running)
    {
        if let Some(counters) = &sample.counters {
            if counters.total_chunks.is_none_or(|total| counters.done_chunks <= total)
                && counters.total_batches.is_none_or(|total| counters.done_batches <= total)
            {
                value["chunks"] =
                    json!({"done": counters.done_chunks, "total": counters.total_chunks});
                value["batches"] =
                    json!({"done": counters.done_batches, "total": counters.total_batches});
                if let Some(total) = counters.total_chunks.filter(|total| *total > 0) {
                    value["pct"] =
                        json!((counters.done_chunks as u128 * 100 / total as u128) as usize);
                }
            }
        }
    }
    value
}

// `required` on an Option field removes null from its schema; require the key instead.
fn require_all_fields(schema: &mut schemars::Schema) {
    let keys: Vec<_> = schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|properties| properties.keys().cloned())
        .collect();
    schema.insert("required".to_owned(), json!(keys));
}

#[cfg(test)]
mod indexing_wire_projection {
    use super::*;
    use bsl_search::{IndexCounters, IndexProgress};

    #[test]
    fn counters_nulls_and_schema_share_one_snapshot() {
        let progress = IndexProgress::new();
        let mut pass = progress.begin_pass();
        pass.token().set_totals(1, 8, 2);
        pass.token().advance(4, 1);
        let sample = progress.snapshot().unwrap();
        let indexing = Indexing::workspace(
            Target::new(Kind::Lexical, State::Ready, None),
            Target::native(Kind::Semantic, &sample),
            Some(&sample),
        );
        pass.token().advance(4, 1);
        let mut response =
            crate::tools::response::structured(json!({"progress": {"active": false}}));
        indexing.attach(&mut response);
        let body = response.structured_content.unwrap();
        assert_eq!(body["indexing"]["targets"][1]["progress"]["completed"], 4);
        assert_eq!(body["progress"]["chunks"]["done"], 4);
        assert_eq!(body["progress"]["pct"], 50);
        assert_eq!(
            serde_json::from_str::<Value>(&response.content[0].as_text().unwrap().text).unwrap(),
            body
        );
        let schema = serde_json::to_value(schemars::schema_for!(Indexing)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(
            validator.is_valid(&body["indexing"]),
            "{}",
            validator
                .iter_errors(&body["indexing"])
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        );
        let mut missing = body["indexing"].clone();
        missing["targets"][0].as_object_mut().unwrap().remove("phase");
        assert!(!validator.is_valid(&missing));
        pass.finish(IndexPassState::Failed);
        let terminal = progress.snapshot().unwrap();
        assert_eq!(Target::native(Kind::Semantic, &terminal).state, State::Failed);
        assert!(Target::native(Kind::Semantic, &terminal).progress.is_none());
        assert!(legacy_progress(Some(&terminal)).get("chunks").is_none());
    }

    #[test]
    fn unknown_zero_and_inconsistent_totals_never_fabricate_percent() {
        for total in [None, Some(0), Some(2)] {
            let sample = IndexProgressSnapshot {
                active: true,
                state: IndexPassState::Running,
                phase: Some(IndexPhase::Embedding),
                pass_id: None,
                counters: Some(IndexCounters {
                    total_files: 0,
                    total_chunks: total,
                    total_batches: None,
                    done_chunks: if total == Some(2) { 3 } else { 0 },
                    done_batches: 0,
                }),
            };
            assert!(legacy_progress(Some(&sample)).get("pct").is_none());
            let target = Target::native(Kind::Semantic, &sample);
            if total == Some(2) {
                assert!(target.progress.is_none());
            } else {
                assert_eq!(target.progress.unwrap().total, total);
            }
        }
        let mut batches = IndexProgressSnapshot {
            active: true,
            state: IndexPassState::Running,
            phase: Some(IndexPhase::Embedding),
            pass_id: None,
            counters: Some(IndexCounters {
                total_files: 0,
                total_chunks: None,
                total_batches: Some(3),
                done_chunks: 0,
                done_batches: 2,
            }),
        };
        assert_eq!(Target::native(Kind::Semantic, &batches).progress.unwrap().unit, Unit::Batches);
        batches.counters.as_mut().unwrap().done_batches = 4;
        assert!(Target::native(Kind::Semantic, &batches).progress.is_none());
        assert!(legacy_progress(Some(&batches)).get("batches").is_none());
        batches.active = false;
        batches.counters.as_mut().unwrap().done_batches = 2;
        assert!(Target::native(Kind::Semantic, &batches).progress.is_none());
        let unknown = Indexing::single(Target::unknown(Kind::Graph));
        let encoded = serde_json::to_value(&unknown).unwrap();
        assert_eq!(encoded["targets"][0].as_object().unwrap().len(), 6);
        assert!(encoded["targets"][0]["pass_id"].is_null());
        assert_eq!(encoded["targets"][0]["reason_code"], "snapshot_unavailable");
        assert!(serde_json::to_vec(&unknown).unwrap().len() < 512);
    }
}
