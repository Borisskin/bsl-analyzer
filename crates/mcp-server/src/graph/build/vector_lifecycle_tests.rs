use super::FusedChunkWriter;
use bsl_search::SearchEngine;
use ide::FusedChunkSink;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn capture(f: impl FnOnce()) -> Vec<Value> {
    use tracing_subscriber::prelude::*;
    struct Capture(Arc<Mutex<Vec<Value>>>);
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Visitor<'a>(&'a mut Vec<Value>);
            impl tracing::field::Visit for Visitor<'_> {
                fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
                fn record_str(&mut self, field: &tracing::field::Field, text: &str) {
                    if field.name() == "record" {
                        self.0.push(serde_json::from_str(text).unwrap());
                    }
                }
            }
            if event.metadata().target() == bsl_search::lifecycle::TARGET {
                event.record(&mut Visitor(&mut self.0.lock().unwrap()));
            }
        }
    }
    let records = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::with_default(
        tracing_subscriber::registry().with(Capture(records.clone())),
        f,
    );
    Arc::try_unwrap(records).unwrap().into_inner().unwrap()
}

fn emit(engine: &mut SearchEngine, source: &Path, rows: &[ide::ChunkRow]) {
    let mut writer = FusedChunkWriter::new(
        engine,
        source.to_owned(),
        crate::workspace_lease::WorkspaceLease::unmanaged(),
    );
    writer.emit_chunks(rows).unwrap();
    writer.finish(bsl_search::lifecycle::Outcome::Completed);
}

fn seed(source: &Path) -> (SearchEngine, Vec<ide::ChunkRow>) {
    let file = source.join("Module.bsl");
    std::fs::write(&file, "Процедура Делать()\nКонецПроцедуры").unwrap();
    let rows = ["A", "B"]
        .into_iter()
        .map(|name| ide::ChunkRow {
            path: file.canonicalize().unwrap().to_string_lossy().replace('\\', "/"),
            symbol: name.into(),
            kind: bsl_search::ChunkKind::Procedure,
            is_export: false,
            annotations: Vec::new(),
            line_start: 1,
            line_end: 2,
            text: "canary source text must not enter lifecycle records".into(),
            graph_context: None,
        })
        .collect::<Vec<_>>();
    let mut engine = SearchEngine::fts_only(&source.join("search.db")).unwrap();
    emit(&mut engine, source, &rows);
    let pending = engine.store().load_pending_embedding_documents("code").unwrap();
    assert_eq!(pending.len(), 2);
    engine.store().set_chunk_embedding(pending[0].0, &[0.1, 0.2]).unwrap();
    (engine, rows)
}

fn assert_replacement(records: &[Value], reason: &str) {
    let decision = records.iter().find(|r| r["kind"] == "decision").unwrap();
    assert_eq!(decision["reason"], reason);
    let mutations = records
        .iter()
        .filter(|r| r["kind"] == "mutation" && r["outcome"] == "committed")
        .collect::<Vec<_>>();
    assert_eq!(mutations.len(), 1);
    let mutation = mutations[0];
    assert_eq!(mutation["reason"], reason);
    assert_eq!(mutation["counts"]["sqlite_vectors_removed"], 1);
    assert_eq!(mutation["count_quality"], "exact");
    assert_eq!(mutation["process_id"], decision["process_id"]);
    assert_eq!(mutation["parent_operation_id"], decision["parent_operation_id"]);
    assert!(mutation["parent_operation_id"].as_u64().is_some());
    let terminal = records
        .iter()
        .find(|r| r["kind"] == "mutation_summary" && r["outcome"] == "completed")
        .unwrap();
    assert_eq!(terminal["committed_totals"]["sqlite_vectors_removed"], 1);
    assert!(!serde_json::to_string(records).unwrap().contains("canary source"));
}

#[test]
fn vector_lifecycle_unchanged_warm_reopen_preserves_mixed_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path();
    let (engine, rows) = seed(source);
    let before = engine.store().load_all_embeddings(2).unwrap();
    let generation = engine.store().embedding_generation().unwrap();
    drop(engine);
    let records = capture(|| {
        let mut engine = SearchEngine::fts_only(&source.join("search.db")).unwrap();
        emit(&mut engine, source, &rows);
        assert_eq!(engine.store().load_all_embeddings(2).unwrap(), before);
        assert_eq!(engine.store().embedding_generation().unwrap(), generation);
        assert_eq!(engine.store().load_pending_embedding_documents("code").unwrap().len(), 1);
    });
    let snapshot = records.iter().find(|r| r["kind"] == "startup_snapshot").unwrap();
    assert_eq!(snapshot["snapshot"]["state"], "observed");
    assert_eq!(snapshot["snapshot"]["files"], 1);
    assert_eq!(snapshot["snapshot"]["chunks"], 2);
    assert_eq!(snapshot["snapshot"]["vectors"], 1);
    let decision = records.iter().find(|r| r["kind"] == "decision").unwrap();
    assert_eq!(decision["reason"], "unchanged");
    assert!(records.iter().all(|r| r["counts"]["sqlite_vectors_removed"] == 0));
}

#[test]
fn vector_lifecycle_changed_file_attributes_actual_non_null_loss() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, rows) = seed(dir.path());
    std::fs::write(dir.path().join("Module.bsl"), "Процедура Другая()\nКонецПроцедуры").unwrap();
    let records = capture(|| emit(&mut engine, dir.path(), &rows));
    assert_replacement(&records, "hash_changed");
    assert_eq!(engine.store().load_pending_embedding_documents("code").unwrap().len(), 2);
}

#[test]
fn vector_lifecycle_hash_lookup_error_reindexes_without_claiming_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, rows) = seed(dir.path());
    let connection = rusqlite::Connection::open(engine.store().db_path()).unwrap();
    connection.execute("UPDATE files SET hash = 17 WHERE path = 'Module.bsl'", []).unwrap();
    assert!(engine.store().file_hash("", "Module.bsl").is_err());
    let records = capture(|| emit(&mut engine, dir.path(), &rows));
    assert_replacement(&records, "hash_lookup_error");
    assert_eq!(engine.store().load_pending_embedding_documents("code").unwrap().len(), 2);
    assert!(engine.store().file_hash("", "Module.bsl").unwrap().is_some());
}

#[test]
fn vector_lifecycle_cleared_hash_and_missing_record_keep_distinct_causes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, rows) = seed(dir.path());
    engine.store().clear_file_hashes("code").unwrap();
    let records = capture(|| emit(&mut engine, dir.path(), &rows));
    assert_replacement(&records, "hash_cleared");
    // No historical initiating actor is inferred from an empty stored hash.
    assert!(!records.iter().any(|r| r["reason"] == "hash_changed"));

    engine.store().remove_file("", "Module.bsl", "code").unwrap();
    let records = capture(|| emit(&mut engine, dir.path(), &rows));
    let decision = records.iter().find(|r| r["kind"] == "decision").unwrap();
    assert_eq!(decision["reason"], "missing_record");
    let mutation =
        records.iter().find(|r| r["kind"] == "mutation" && r["outcome"] == "committed").unwrap();
    assert_eq!(mutation["reason"], "missing_record");
    assert_eq!(mutation["counts"]["sqlite_vectors_removed"], 0);
    assert_eq!(decision["parent_operation_id"], mutation["parent_operation_id"]);
    assert_eq!(engine.store().load_pending_embedding_documents("code").unwrap().len(), 2);
}

#[test]
fn vector_lifecycle_read_error_preserves_rows_and_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let (mut engine, rows) = seed(dir.path());
    let before = engine.store().load_all_embeddings(2).unwrap();
    let generation = engine.store().embedding_generation().unwrap();
    std::fs::remove_file(dir.path().join("Module.bsl")).unwrap();
    let records = capture(|| emit(&mut engine, dir.path(), &rows));
    let decision = records.iter().find(|r| r["kind"] == "decision").unwrap();
    assert_eq!(decision["reason"], "read_error");
    assert!(!records.iter().any(|r| r["kind"] == "mutation"));
    assert_eq!(engine.store().load_all_embeddings(2).unwrap(), before);
    assert_eq!(engine.store().embedding_generation().unwrap(), generation);
    assert_eq!(engine.store().chunk_count().unwrap(), 2);
}

#[test]
fn vector_lifecycle_fused_writer_retains_totals_across_emit_batches_and_takeover() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path();
    let (mut engine, rows) = seed(source);
    let cache = crate::cache::WorkspaceCacheLayout::for_workspace(source);
    cache.ensure().unwrap();
    let lease = crate::workspace_lease::WorkspaceLease::claim_cache(&cache);
    assert!(lease.owns_caches_now());
    let newer = Arc::new(Mutex::new(None));
    let newer_hook = newer.clone();
    super::FUSED_FILE_COMMITTED_HOOK.with(|hook| {
        hook.replace(Some(Box::new(move || {
            let claim = crate::workspace_lease::WorkspaceLease::claim_cache(&cache);
            assert!(claim.owns_caches_now());
            *newer_hook.lock().unwrap() = Some(claim);
        })))
    });
    let records = capture(|| {
        let mut writer = FusedChunkWriter::new(&mut engine, source.to_owned(), lease.clone());
        std::fs::write(source.join("Module.bsl"), "first change").unwrap();
        writer.emit_chunks(&rows).unwrap();
        std::fs::write(source.join("Module.bsl"), "second change").unwrap();
        assert!(writer.emit_chunks(&rows).is_err());
        let outcome = writer.failure.as_ref().unwrap().lifecycle_outcome();
        writer.finish(outcome);
    });
    assert!(lease.is_superseded());
    newer.lock().unwrap().take().unwrap().release();
    let summaries: Vec<_> = records.iter().filter(|r| r["kind"] == "mutation_summary").collect();
    assert_eq!(summaries.iter().filter(|r| r["outcome"] == "started").count(), 1);
    let terminal = summaries.last().unwrap();
    assert_eq!(terminal["outcome"], "interrupted");
    assert_eq!(terminal["committed_totals"]["sqlite_vectors_removed"], 1);
    let decisions: Vec<_> = records.iter().filter(|r| r["kind"] == "decision").collect();
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0]["parent_operation_id"], decisions[1]["parent_operation_id"]);
    assert_eq!(engine.store().chunk_count().unwrap(), 2);
}
