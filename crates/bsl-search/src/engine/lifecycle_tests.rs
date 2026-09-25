use super::*;
use serde_json::Value;
use std::io::{Read, Write};

fn capture(f: impl FnOnce()) -> Vec<Value> {
    capture_level(tracing::level_filters::LevelFilter::DEBUG, f)
}

fn capture_level(level: tracing::level_filters::LevelFilter, f: impl FnOnce()) -> Vec<Value> {
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
                        let record: Value = serde_json::from_str(text).unwrap();
                        if record["kind"] == "startup_snapshot" {
                            CONSTRUCTOR_APPLY_ACTIVE.with(|active| {
                                assert!(!active.get(), "snapshot ran inside constructor fence")
                            });
                        }
                        self.0.push(record);
                    }
                }
            }
            if event.metadata().target() == lifecycle::TARGET {
                event.record(&mut Visitor(&mut self.0.lock().unwrap()));
            }
        }
    }
    let records = Arc::new(Mutex::new(Vec::new()));
    crate::lifecycle::test_with_subscriber(
        tracing_subscriber::registry().with(Capture(records.clone()).with_filter(level)),
        f,
    );
    Arc::try_unwrap(records).unwrap().into_inner().unwrap()
}

/// Finite local fake: returns exactly the requests the test expects, with no model/provider.
fn server(requests: usize) -> (Embedder, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let mut submitted = Vec::new();
        for stream in listener.incoming().take(requests) {
            let mut stream = stream.unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 2048];
            let body = loop {
                let read = stream.read(&mut buffer).unwrap();
                assert_ne!(read, 0);
                bytes.extend_from_slice(&buffer[..read]);
                let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                let headers = String::from_utf8_lossy(&bytes[..split]).to_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                if bytes.len() >= split + 4 + length {
                    break serde_json::from_slice::<Value>(&bytes[split + 4..]).unwrap();
                }
            };
            let inputs = body["input"].as_array().unwrap();
            submitted.extend(inputs.iter().map(|text| text.as_str().unwrap().to_owned()));
            let data: Vec<_> = (0..inputs.len())
                .map(|index| serde_json::json!({"index":index,"embedding":[1.0,0.0,0.0]}))
                .collect();
            let body = serde_json::json!({"data":data}).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
        submitted
    });
    (
        Embedder::new(EmbedderConfig {
            base_url: format!("http://{address}"),
            model: "fixture".into(),
            dim: Some(3),
            api_key: None,
            provider: None,
        }),
        thread,
    )
}

fn seed(path: &Path, complete: usize, pending: usize) -> Store {
    let mut store = Store::open(path).unwrap();
    for i in 0..complete + pending {
        let name = format!("Method{i}");
        let chunk = crate::Chunk {
            kind: crate::ChunkKind::Procedure,
            name: name.clone(),
            is_export: false,
            annotations: Vec::new(),
            line_start: 1,
            line_end: 2,
            text: format!("Процедура {name}()\nКонецПроцедуры"),
        };
        let vectors = vec![vec![0.0, 1.0, 0.0]];
        store
            .reindex_file(
                "",
                &format!("M{i}.bsl"),
                b"hash",
                &[chunk],
                (i < complete).then_some(vectors.as_slice()),
            )
            .unwrap();
    }
    store
}

#[test]
fn vector_lifecycle_artifact_publish_refuses_changed_generation_without_installing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("search.db");
    let store = seed(&path, 1, 0);
    let (generation, data) = store.load_all_embeddings_with_generation(3).unwrap();
    let index = VectorIndex::build(3, &data).unwrap();
    let key = crate::vector_persist::PersistKey { db_path: &path, model_id: "fixture", dim: 3 };
    let mut prepared = crate::vector_persist::prepare(&index, &key, generation).unwrap();
    store.set_chunk_embeddings(&[(data[0].0, vec![1.0, 0.0, 0.0])]).unwrap();
    let records =
        capture(|| assert!(SearchEngine::install_prepared_built(&store, &mut prepared).is_err()));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["kind"], "artifact_publish");
    assert_eq!(records[0]["reason"], "artifact_stale");
    assert_eq!(records[0]["outcome"], "refused");
    assert_eq!(records[0]["counts"]["sqlite_vectors_removed"], 0);
    assert!(!dir.path().join("search.db.usearch").exists());
    assert_eq!(store.load_all_embeddings(3).unwrap(), vec![(data[0].0, vec![1.0, 0.0, 0.0])]);
    rusqlite::Connection::open(&path).unwrap().execute("DROP TABLE meta", []).unwrap();
    let records =
        capture(|| assert!(SearchEngine::install_prepared_built(&store, &mut prepared).is_err()));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["reason"], "read_error");
    assert_eq!(records[0]["outcome"], "failed");
    assert_eq!(records[0]["examples"][0], "generation_read");
    assert_eq!(records[0]["counts"]["sqlite_vectors_removed"], 0);
}

fn terminal(records: &[Value]) -> &Value {
    records
        .iter()
        .rev()
        .find(|r| {
            r["kind"] == "embedding_pass" && r["outcome"] != "started" && r["outcome"] != "progress"
        })
        .unwrap()
}

#[test]
fn vector_lifecycle_partial_resume_only_submits_pending_then_warm_skip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("search.db");
    let store = seed(&path, 1, 2);
    let preserved = store.load_all_embeddings(3).unwrap()[0].clone();
    drop(store);
    let (embedder, requests) = server(2);
    let records = capture(|| {
        let store = Store::open_existing(&path).unwrap();
        let (index, outcome) = SearchEngine::run_embedding_pass(
            &store,
            &embedder,
            3,
            1,
            1,
            None,
            None,
            &mut |op| FenceOutcome::Applied(op()),
            None,
        )
        .unwrap();
        assert!(matches!(outcome, FenceOutcome::Applied(())));
        assert_eq!(index.len(), 3);
        assert!(store.load_all_embeddings(3).unwrap().contains(&preserved));
    });
    let inputs = requests.join().unwrap();
    assert_eq!(inputs.len(), 2);
    assert!(inputs.iter().all(|text| !text.contains("Method0")));
    assert_eq!(terminal(&records)["outcome"], "completed");
    assert_eq!(terminal(&records)["committed_totals"]["embeddings_written"], 2);
    assert_eq!(terminal(&records)["pending"], 0);
    let generation = Store::open_existing(&path).unwrap().embedding_generation().unwrap();
    let records = capture(|| {
        let store = Store::open_existing(&path).unwrap();
        let (index, _) = SearchEngine::run_embedding_pass(
            &store,
            &embedder,
            3,
            1,
            1,
            None,
            None,
            &mut |op| FenceOutcome::Applied(op()),
            None,
        )
        .unwrap();
        assert_eq!(index.len(), 3);
        assert_eq!(store.embedding_generation().unwrap(), generation);
        assert!(store.load_all_embeddings(3).unwrap().contains(&preserved));
    });
    assert_eq!(terminal(&records)["outcome"], "skipped");
    assert_eq!(terminal(&records)["committed_totals"]["embeddings_written"], 0);
}

#[test]
fn vector_lifecycle_refusal_and_failure_retain_earlier_committed_batch() {
    for failed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = seed(&dir.path().join("search.db"), 0, 2);
        let (embedder, requests) = server(2);
        let mut commits = 0;
        let records = capture(|| {
            let result = SearchEngine::run_embedding_pass(
                &store,
                &embedder,
                3,
                1,
                1,
                None,
                None,
                &mut |op| {
                    commits += 1;
                    if commits == 2 {
                        if failed {
                            FenceOutcome::Applied(Err(SearchError::Index("fixture failure".into())))
                        } else {
                            FenceOutcome::TransientRefusal
                        }
                    } else {
                        FenceOutcome::Applied(op())
                    }
                },
                None,
            );
            if failed {
                assert!(result.is_err());
            } else {
                assert!(matches!(result.unwrap().1, FenceOutcome::TransientRefusal));
            }
        });
        assert_eq!(requests.join().unwrap().len(), 2);
        assert_eq!(store.load_all_embeddings(3).unwrap().len(), 1);
        assert_eq!(store.load_pending_embedding_documents("code").unwrap().len(), 1);
        let terminal = terminal(&records);
        assert_eq!(terminal["outcome"], if failed { "failed" } else { "refused" });
        assert_eq!(terminal["committed_totals"]["embeddings_written"], 1);
        assert_eq!(terminal["pending"], 1);
    }
}

#[test]
fn vector_lifecycle_startup_snapshot_precedes_fence_with_root_mode_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("search.db");
    drop(seed(&path, 1, 1));
    let records = capture(|| {
        lifecycle::with_startup_roots("local", vec![dir.path().to_owned()], || {
            let opened = SearchEngine::fts_only_fenced(&path, |op| {
                CONSTRUCTOR_APPLY_ACTIVE.with(|active| {
                    assert!(!active.replace(true));
                    let result = op(&mut || ControlFlow::Continue(())).continue_value().unwrap();
                    active.set(false);
                    FenceOutcome::Applied(result)
                })
            })
            .unwrap();
            assert!(matches!(opened, FenceOutcome::Applied(_)));
        })
    });
    let snapshot = records.iter().find(|r| r["kind"] == "startup_snapshot").unwrap();
    assert_eq!(snapshot["snapshot"]["vectors"], 1);
    assert_eq!(snapshot["snapshot"]["chunks"], 2);
    assert_eq!(snapshot["snapshot"]["mode"], "local");
    assert_eq!(snapshot["snapshot"]["roots_count"], 1);
    assert!(snapshot["snapshot"]["roots_digest"].as_str().is_some());
}

#[test]
fn vector_lifecycle_each_open_snapshots_once_before_schema_reset() {
    for fenced in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search.db");
        drop(seed(&path, 1, 1));
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("UPDATE meta SET value = '999' WHERE key = 'schema_version'", [])
            .unwrap();
        let records = capture(|| {
            if fenced {
                let opened = SearchEngine::fts_only_fenced(&path, |op| {
                    CONSTRUCTOR_APPLY_ACTIVE.with(|active| {
                        assert!(!active.replace(true));
                        let result =
                            op(&mut || ControlFlow::Continue(())).continue_value().unwrap();
                        active.set(false);
                        FenceOutcome::Applied(result)
                    })
                })
                .unwrap();
                assert!(matches!(opened, FenceOutcome::Applied(_)));
            } else {
                Store::open(&path).unwrap();
            }
        });
        let snapshots: Vec<_> =
            records.iter().filter(|r| r["kind"] == "startup_snapshot").collect();
        assert_eq!(snapshots.len(), 1);
        let snapshot = snapshots[0];
        assert_eq!(snapshot["snapshot"]["vectors"], 1);
        assert_eq!(snapshot["snapshot"]["chunks"], 2);
        assert_eq!(snapshot["snapshot"]["schema_version"], 999);
        let reset = records
            .iter()
            .find(|r| r["reason"] == "schema_reset" && r["outcome"] == "committed")
            .unwrap();
        assert!(snapshot["event_seq"].as_u64().unwrap() < reset["event_seq"].as_u64().unwrap());
        assert_eq!(reset["counts"]["sqlite_vectors_removed"], 1);
        assert!(Store::open_existing(&path).unwrap().load_all_embeddings(3).unwrap().is_empty());
    }
}

#[test]
fn vector_lifecycle_context_and_mode_decisions_preserve_exact_loss() {
    struct Context;
    impl crate::ports::GraphContextProvider for Context {
        fn graph_context(&self, _: &str, _: &str, _: &str) -> Option<String> {
            Some("fixture context".into())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("search.db");
    drop(seed(&path, 1, 1));
    let mut engine = SearchEngine::fts_only(&path).unwrap();
    for name in ["M0.bsl", "M1.bsl"] {
        engine.store().mark_context_dirty("code", "", name).unwrap();
    }
    let records = capture(|| {
        engine.refresh_dirty_contexts(&Context, i64::MAX).unwrap();
        engine.set_serves_external_baseline(true).unwrap();
        engine.set_serves_external_baseline(false).unwrap();
    });
    let context = records
        .iter()
        .find(|r| {
            r["kind"] == "mutation_summary"
                && r["reason"] == "context_changed"
                && r["outcome"] == "completed"
        })
        .unwrap();
    assert_eq!(context["committed_totals"]["sqlite_vectors_removed"], 1);
    assert_eq!(context["reasons"]["context_changed"], 2);
    assert_eq!(context["files"], 2);
    let modes: Vec<_> = records
        .iter()
        .filter(|r| r["kind"] == "mode_transition" && r["outcome"] == "completed")
        .collect();
    assert_eq!(modes.len(), 2);
    assert_eq!(modes[0]["snapshot"]["mode"], "external_baseline");
    assert_eq!(modes[1]["snapshot"]["mode"], "local");
    assert!(modes.iter().all(|r| r["counts"]["sqlite_vectors_removed"] == 0));
    assert_eq!(engine.store().load_pending_embedding_documents("code").unwrap().len(), 2);
}

#[test]
fn vector_lifecycle_bulk_removal_bounds_info_and_retains_commits_before_refusal() {
    for reconcile in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search.db");
        drop(seed(&path, 257, 0));
        let mut engine = SearchEngine::fts_only(&path).unwrap();
        engine.set_workspace_roots(crate::WorkspaceRoots::build(dir.path(), dir.path(), &[]).0);
        let records = capture_level(tracing::level_filters::LevelFilter::INFO, || {
            if reconcile {
                let mut admitted = 0;
                let result = engine
                    .reconcile_workspace_files_fenced(&HashSet::new(), |op| {
                        admitted += 1;
                        if admitted > 129 {
                            FenceOutcome::TransientRefusal
                        } else {
                            FenceOutcome::Applied(op())
                        }
                    })
                    .unwrap();
                assert!(matches!(result, FenceOutcome::TransientRefusal));
            } else {
                let keys = (0..257).map(|i| FileKey::configuration(format!("M{i}.bsl"))).collect();
                assert_eq!(engine.remove_workspace_keys(keys).unwrap(), 257);
            }
        });
        assert!(records.iter().all(|r| r["kind"] == "mutation_summary"));
        let elapsed_seconds = (records.last().unwrap()["timestamp_ms"].as_u64().unwrap()
            - records[0]["timestamp_ms"].as_u64().unwrap())
            / 1000;
        // Each flush also emits the next group's intent. Allow the one-second
        // flushes on a slow machine without turning this into a timing benchmark.
        assert!(records.len() as u64 <= 6 + 2 * elapsed_seconds);
        assert!(records.iter().all(|r| r["examples"].as_array().unwrap().len() <= 10));
        let terminal = records.last().unwrap();
        assert_eq!(terminal["outcome"], if reconcile { "refused" } else { "completed" });
        let removed = if reconcile { 129 } else { 257 };
        assert_eq!(terminal["committed_totals"]["sqlite_vectors_removed"], removed);
        assert_eq!(engine.store().load_all_embeddings(3).unwrap().len(), 257 - removed);
    }
}
