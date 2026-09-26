use crate::document::{semantic_key_for_indexed_document, semantic_text_for_indexed_document};
use crate::domain::{IndexedDocument, Snapshot, SnapshotPublishMetadata, SnapshotPublishStats};
use crate::error::SearchError;
use crate::ports::{EmbeddingGenerator, EmbeddingStore, SnapshotPublisher};
use crossbeam_channel::bounded;
use std::collections::BTreeMap;
use std::thread;
use tracing::info;

#[derive(Debug, Clone)]
pub enum EmbeddingProgress {
    Plan { total_unique: usize, cached: usize, to_compute: usize },
    Batch { processed: usize, total: usize, batches_done: usize, total_batches: usize },
}

const DEFAULT_BATCH_SIZE: usize = 32;
const DEFAULT_CONCURRENCY: usize = 10;
const DEFAULT_PROGRESS_INTERVAL: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingExecutionPolicy {
    pub batch_size: usize,
    pub concurrency: usize,
    pub progress_interval: usize,
}

impl EmbeddingExecutionPolicy {
    pub fn batch_size(&self) -> usize {
        self.batch_size.max(1)
    }

    pub fn concurrency(&self) -> usize {
        self.concurrency.max(1)
    }

    pub fn progress_interval(&self) -> usize {
        self.progress_interval.max(1)
    }
}

impl Default for EmbeddingExecutionPolicy {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            concurrency: DEFAULT_CONCURRENCY,
            progress_interval: DEFAULT_PROGRESS_INTERVAL,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedEmbeddingPublishStats {
    pub model_id: String,
    pub dimension: usize,
    pub reused: usize,
    pub stored: usize,
    pub total_unique: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselinePublishReport {
    pub snapshot: SnapshotPublishStats,
    pub embeddings: Option<SharedEmbeddingPublishStats>,
}

#[derive(Debug, Clone)]
pub struct SharedEmbeddingPublisher {
    policy: EmbeddingExecutionPolicy,
}

impl SharedEmbeddingPublisher {
    pub fn new(policy: EmbeddingExecutionPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> &EmbeddingExecutionPolicy {
        &self.policy
    }

    pub fn publish<S, E>(
        &self,
        store: &S,
        embedder: &E,
        documents: &[IndexedDocument],
        progress: Option<&dyn Fn(EmbeddingProgress)>,
    ) -> Result<SharedEmbeddingPublishStats, SearchError>
    where
        S: EmbeddingStore,
        E: EmbeddingGenerator + Clone + Send + 'static,
    {
        embedder.batch_ranges(&[], self.policy.batch_size())?;
        let dimension = embedder.dimension();
        let model_id = embedder.model_id().to_owned();
        store.ensure_embedding_identity(&model_id, dimension)?;
        if documents.is_empty() {
            if let Some(on_progress) = progress {
                on_progress(EmbeddingProgress::Plan { total_unique: 0, cached: 0, to_compute: 0 });
            }
            return Ok(SharedEmbeddingPublishStats {
                model_id,
                dimension,
                reused: 0,
                stored: 0,
                total_unique: 0,
            });
        }

        let unique_documents = documents
            .iter()
            .map(|document| {
                (
                    semantic_key_for_indexed_document(document),
                    semantic_text_for_indexed_document(document),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let total_unique = unique_documents.len();
        let embedding_keys = unique_documents.keys().cloned().collect::<Vec<_>>();
        let existing = store.load_embeddings(&embedding_keys, &model_id, dimension)?;
        let reused = existing.len();

        let missing = unique_documents
            .into_iter()
            .filter(|(key, _)| !existing.contains_key(key))
            .collect::<Vec<_>>();

        if let Some(on_progress) = progress {
            on_progress(EmbeddingProgress::Plan {
                total_unique,
                cached: reused,
                to_compute: missing.len(),
            });
        }

        if missing.is_empty() {
            return Ok(SharedEmbeddingPublishStats {
                model_id,
                dimension,
                reused,
                stored: 0,
                total_unique,
            });
        }

        let texts: Vec<&str> = missing.iter().map(|(_, text)| text.as_str()).collect();
        let ranges = embedder.batch_ranges(&texts, self.policy.batch_size())?;
        let progress_interval = self.policy.progress_interval();
        let total_missing = missing.len();
        let total_batches = ranges.len();
        let concurrency = self.policy.concurrency().min(total_batches.max(1));

        let (task_tx, task_rx) = bounded::<Vec<(String, String)>>(concurrency * 2);
        let (result_tx, result_rx) =
            bounded::<Result<Vec<(String, Vec<f32>)>, SearchError>>(concurrency * 2);

        let workers = (0..concurrency)
            .map(|_| {
                let rx = task_rx.clone();
                let tx = result_tx.clone();
                let emb = embedder.clone();
                thread::spawn(move || {
                    while let Ok(batch) = rx.recv() {
                        let texts = batch.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>();
                        let result = emb.embed_batch(&texts).and_then(|vectors| {
                            if vectors.len() != batch.len()
                                || vectors.iter().any(|vector| {
                                    vector.len() != emb.dimension()
                                        || vector.iter().any(|value| !value.is_finite())
                                })
                            {
                                return Err(crate::EmbeddingFailure::new(
                                    crate::EmbeddingFailureCode::EmbeddingInvalidResponse,
                                )
                                .into());
                            }
                            Ok(batch
                                .into_iter()
                                .zip(vectors)
                                .map(|((embedding_key, _), embedding)| (embedding_key, embedding))
                                .collect::<Vec<_>>())
                        });
                        if tx.send(result).is_err() {
                            break;
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        drop(task_rx);
        drop(result_tx);

        let producer = thread::spawn(move || {
            for range in ranges {
                if task_tx.send(missing[range].to_vec()).is_err() {
                    break;
                }
            }
        });

        let mut processed = 0usize;
        let mut stored_total = 0usize;
        let mut reused_total = reused;
        let mut batch_index = 0usize;
        let mut first_error = None;

        while batch_index < total_batches {
            let result = match result_rx.recv() {
                Ok(result) => result,
                Err(_) => {
                    first_error.get_or_insert_with(|| {
                        crate::EmbeddingFailure::new(crate::EmbeddingFailureCode::EmbeddingFailed)
                            .into()
                    });
                    break;
                }
            };
            batch_index += 1;

            match result {
                Ok(generated) if first_error.is_none() => {
                    let stats = match store.store_embeddings(&model_id, dimension, &generated) {
                        Ok(stats) => stats,
                        Err(error) => {
                            first_error.get_or_insert(error);
                            continue;
                        }
                    };
                    stored_total += stats.stored;
                    reused_total += stats.reused;
                    processed += generated.len();
                    if batch_index.is_multiple_of(progress_interval) || processed == total_missing {
                        info!(
                            model_id = %model_id,
                            processed,
                            total_missing,
                            batches_done = batch_index,
                            total_batches,
                            stored_total,
                            reused_total,
                            "shared embedding publish progress"
                        );
                        if let Some(on_progress) = progress {
                            on_progress(EmbeddingProgress::Batch {
                                processed,
                                total: total_missing,
                                batches_done: batch_index,
                                total_batches,
                            });
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }

        drop(result_rx);
        if producer.join().is_err() {
            first_error.get_or_insert_with(|| {
                crate::EmbeddingFailure::new(crate::EmbeddingFailureCode::EmbeddingFailed).into()
            });
        }
        for worker in workers {
            if worker.join().is_err() {
                first_error.get_or_insert_with(|| {
                    crate::EmbeddingFailure::new(crate::EmbeddingFailureCode::EmbeddingFailed)
                        .into()
                });
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }

        Ok(SharedEmbeddingPublishStats {
            model_id,
            dimension,
            reused: reused_total,
            stored: stored_total,
            total_unique,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BaselinePublisher {
    shared_embeddings: SharedEmbeddingPublisher,
}

impl BaselinePublisher {
    pub fn new(policy: EmbeddingExecutionPolicy) -> Self {
        Self { shared_embeddings: SharedEmbeddingPublisher::new(policy) }
    }

    pub fn shared_embeddings(&self) -> &SharedEmbeddingPublisher {
        &self.shared_embeddings
    }

    pub fn publish<S, E>(
        &self,
        store: &S,
        snapshot: &Snapshot,
        metadata: &SnapshotPublishMetadata,
        documents: &[IndexedDocument],
        embedder: Option<&E>,
        embedding_progress: Option<&dyn Fn(EmbeddingProgress)>,
    ) -> Result<BaselinePublishReport, SearchError>
    where
        S: SnapshotPublisher + EmbeddingStore,
        E: EmbeddingGenerator + Clone + Send + 'static,
    {
        let snapshot_stats = store.publish_snapshot(snapshot, metadata, documents)?;
        let embeddings = match embedder {
            Some(embedder) => Some(self.shared_embeddings.publish(
                store,
                embedder,
                documents,
                embedding_progress,
            )?),
            None => None,
        };
        Ok(BaselinePublishReport { snapshot: snapshot_stats, embeddings })
    }
}

#[cfg(test)]
mod tests {
    use super::{BaselinePublisher, EmbeddingExecutionPolicy, SharedEmbeddingPublisher};
    use crate::domain::{
        CorpusId, IndexedDocument, Snapshot, SnapshotPublishMetadata, SnapshotPublishStats,
    };
    use crate::error::SearchError;
    use crate::external_baseline::BaselineEmbeddingStats;
    use crate::ports::{EmbeddingGenerator, EmbeddingStore, SnapshotPublisher};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type SharedEmbeddingMap = Arc<Mutex<HashMap<(String, String, usize), Vec<f32>>>>;

    #[derive(Clone, Default)]
    struct FakeEmbeddingStore {
        embeddings: SharedEmbeddingMap,
        stored_batches: Arc<Mutex<Vec<usize>>>,
        publish_calls: Arc<Mutex<usize>>,
    }

    impl EmbeddingStore for FakeEmbeddingStore {
        fn load_embeddings(
            &self,
            embedding_keys: &[String],
            model_id: &str,
            dimension: usize,
        ) -> Result<HashMap<String, Vec<f32>>, SearchError> {
            let embeddings = self.embeddings.lock().unwrap();
            Ok(embedding_keys
                .iter()
                .filter_map(|key| {
                    embeddings
                        .get(&(key.clone(), model_id.to_owned(), dimension))
                        .cloned()
                        .map(|value| (key.clone(), value))
                })
                .collect())
        }

        fn store_embeddings(
            &self,
            model_id: &str,
            dimension: usize,
            embeddings: &[(String, Vec<f32>)],
        ) -> Result<BaselineEmbeddingStats, SearchError> {
            self.stored_batches.lock().unwrap().push(embeddings.len());
            let mut stored = 0usize;
            let mut reused = 0usize;
            let mut state = self.embeddings.lock().unwrap();
            for (key, value) in embeddings {
                let entry = (key.clone(), model_id.to_owned(), dimension);
                if state.insert(entry, value.clone()).is_some() {
                    reused += 1;
                } else {
                    stored += 1;
                }
            }
            Ok(BaselineEmbeddingStats { stored, reused })
        }
    }

    impl SnapshotPublisher for FakeEmbeddingStore {
        fn publish_snapshot(
            &self,
            _snapshot: &Snapshot,
            _metadata: &SnapshotPublishMetadata,
            _documents: &[IndexedDocument],
        ) -> Result<SnapshotPublishStats, SearchError> {
            *self.publish_calls.lock().unwrap() += 1;
            Ok(SnapshotPublishStats {
                reused_files: 3,
                written_files: 2,
                deleted_files: 1,
                reused_documents: 10,
                written_documents: 4,
            })
        }
    }

    #[derive(Clone, Default)]
    struct FakeEmbedder {
        calls: Arc<Mutex<Vec<usize>>>,
    }

    impl EmbeddingGenerator for FakeEmbedder {
        fn model_id(&self) -> &str {
            "fake-model"
        }

        fn dimension(&self) -> usize {
            3
        }

        fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SearchError> {
            self.calls.lock().unwrap().push(texts.len());
            Ok(texts.iter().map(|text| vec![text.len() as f32, 1.0, 0.0]).collect())
        }
    }

    #[test]
    fn shared_embedding_publisher_reuses_existing_and_batches_missing() {
        let store = FakeEmbeddingStore::default();
        let existing_doc = indexed_document("path/0.bsl", "existing");
        let existing_key = crate::semantic_key_for_indexed_document(&existing_doc);
        store
            .embeddings
            .lock()
            .unwrap()
            .insert((existing_key, "fake-model".to_owned(), 3), vec![1.0, 1.0, 1.0]);

        let documents = vec![
            existing_doc,
            indexed_document("path/1.bsl", "one"),
            indexed_document("path/2.bsl", "two"),
            indexed_document("path/3.bsl", "three"),
            indexed_document("path/4.bsl", "four"),
        ];

        let publisher = SharedEmbeddingPublisher::new(EmbeddingExecutionPolicy {
            batch_size: 2,
            concurrency: 3,
            progress_interval: 1,
        });
        let stats = publisher.publish(&store, &FakeEmbedder::default(), &documents, None).unwrap();

        assert_eq!(stats.model_id, "fake-model");
        assert_eq!(stats.dimension, 3);
        assert_eq!(stats.reused, 1);
        assert_eq!(stats.stored, 4);
        assert_eq!(stats.total_unique, 5);
        let mut batch_sizes = store.stored_batches.lock().unwrap().clone();
        batch_sizes.sort_unstable();
        assert_eq!(batch_sizes, vec![2, 2]);
    }

    #[derive(Clone, Default)]
    struct RejectingIdentityStore {
        inner: FakeEmbeddingStore,
    }

    impl EmbeddingStore for RejectingIdentityStore {
        fn load_embeddings(
            &self,
            embedding_keys: &[String],
            model_id: &str,
            dimension: usize,
        ) -> Result<HashMap<String, Vec<f32>>, SearchError> {
            self.inner.load_embeddings(embedding_keys, model_id, dimension)
        }

        fn store_embeddings(
            &self,
            model_id: &str,
            dimension: usize,
            embeddings: &[(String, Vec<f32>)],
        ) -> Result<BaselineEmbeddingStats, SearchError> {
            self.inner.store_embeddings(model_id, dimension, embeddings)
        }

        fn ensure_embedding_identity(
            &self,
            _model_id: &str,
            _dimension: usize,
        ) -> Result<(), SearchError> {
            Err(SearchError::ExternalBaseline("boom".into()))
        }
    }

    #[test]
    fn shared_embedding_publisher_propagates_identity_mismatch() {
        let store = RejectingIdentityStore::default();
        let documents = vec![indexed_document("path/1.bsl", "one")];
        let publisher = SharedEmbeddingPublisher::new(EmbeddingExecutionPolicy::default());
        let result = publisher.publish(&store, &FakeEmbedder::default(), &documents, None);
        assert!(matches!(result, Err(SearchError::ExternalBaseline(message)) if message == "boom"));
        assert!(store.inner.stored_batches.lock().unwrap().is_empty());
    }

    #[test]
    fn baseline_publisher_orchestrates_snapshot_and_embeddings() {
        let store = FakeEmbeddingStore::default();
        let documents = vec![indexed_document("path/1.bsl", "one")];
        let publisher = BaselinePublisher::new(EmbeddingExecutionPolicy::default());
        let report = publisher
            .publish(
                &store,
                &Snapshot::new("workspace-code:test@1", CorpusId::WorkspaceCode),
                &SnapshotPublishMetadata {
                    branch: Some("test".to_owned()),
                    commit: Some("1".to_owned()),
                },
                &documents,
                Some(&FakeEmbedder::default()),
                None,
            )
            .unwrap();

        assert_eq!(*store.publish_calls.lock().unwrap(), 1);
        assert_eq!(report.snapshot.written_files, 2);
        assert!(report.embeddings.is_some());
        assert_eq!(report.embeddings.unwrap().stored, 1);
    }

    fn indexed_document(path: &str, text: &str) -> IndexedDocument {
        IndexedDocument {
            collection: "code".to_owned(),
            root_id: crate::CONFIGURATION_ROOT_ID.to_owned(),
            path: path.to_owned(),
            symbol_name: path.to_owned(),
            kind: "procedure".to_owned(),
            line_start: 1,
            line_end: 2,
            text: text.to_owned(),
            content_hash: format!("hash:{path}:{text}"),
            graph_context: None,
        }
    }

    #[test]
    fn payload_configuration_publisher_rejects_zero_for_empty_or_cached_work() {
        use crate::embedder::payload_tests::{success, vector, PayloadServer};
        let server = PayloadServer::new(success);
        let store = FakeEmbeddingStore::default();
        let documents = [indexed_document("Cached.bsl", "cached input")];
        store.embeddings.lock().unwrap().insert(
            (crate::semantic_key_for_indexed_document(&documents[0]), "fixture".into(), 3),
            vector(&crate::semantic_text_for_indexed_document(&documents[0])),
        );
        let publisher = SharedEmbeddingPublisher::new(EmbeddingExecutionPolicy::default());
        for input in [&documents[..0], &documents[..]] {
            assert_eq!(
                publisher
                    .publish(&store, &crate::Embedder::new(server.config(0)), input, None)
                    .unwrap_err()
                    .to_string(),
                "embedding_invalid_config"
            );
            publisher
                .publish(&store, &crate::Embedder::new(server.config(4096)), input, None)
                .unwrap();
        }
        assert!(server.requests().is_empty());
        assert!(store.stored_batches.lock().unwrap().is_empty());
    }

    #[test]
    fn payload_shared_publish_trait_plan_and_key_mapping() {
        use crate::embedder::payload_tests::{success, vector, PayloadServer};
        let documents: Vec<_> = (0..6)
            .map(|i| indexed_document(&format!("Модуль{i}.bsl"), &format!("Текст {i}")))
            .collect();
        let limit = documents.iter().map(|doc| serde_json::json!({"model":"fixture","input":[crate::semantic_text_for_indexed_document(doc)],"dimensions":3}).to_string().len()).max().unwrap();
        let server = PayloadServer::new(success);
        let embedder = crate::Embedder::new(server.config(limit));
        let store = FakeEmbeddingStore::default();
        let key = crate::semantic_key_for_indexed_document(&documents[2]);
        store.embeddings.lock().unwrap().insert(
            (key, "fixture".into(), 3),
            vector(&crate::semantic_text_for_indexed_document(&documents[2])),
        );
        let progress = Mutex::new(Vec::new());
        let stats = SharedEmbeddingPublisher::new(EmbeddingExecutionPolicy {
            batch_size: 32,
            concurrency: 2,
            progress_interval: 1,
        })
        .publish(&store, &embedder, &documents, Some(&|event| progress.lock().unwrap().push(event)))
        .unwrap();
        assert_eq!((stats.stored, stats.reused), (5, 1));
        assert_eq!(server.requests().len(), 5);
        assert!(server.requests().iter().all(|body| body.len() <= limit));
        for doc in &documents {
            let key = (crate::semantic_key_for_indexed_document(doc), "fixture".into(), 3);
            assert_eq!(
                store.embeddings.lock().unwrap()[&key],
                vector(&crate::semantic_text_for_indexed_document(doc))
            );
        }
        assert!(matches!(
            progress.lock().unwrap().last(),
            Some(super::EmbeddingProgress::Batch { total_batches: 5, batches_done: 5, .. })
        ));
        let fake = FakeEmbedder::default();
        assert_eq!(fake.batch_ranges(&["a", "b", "c"], 2).unwrap(), vec![0..2, 2..3]);
        assert_eq!(fake.batch_ranges(&["a", "b"], 0).unwrap(), vec![0..1, 1..2]);
        SharedEmbeddingPublisher::new(EmbeddingExecutionPolicy {
            batch_size: 2,
            concurrency: 2,
            progress_interval: 1,
        })
        .publish(&FakeEmbeddingStore::default(), &fake, &documents[..5], None)
        .unwrap();
        let mut counts = fake.calls.lock().unwrap().clone();
        counts.sort_unstable();
        assert_eq!(counts, vec![1, 2, 2]);
    }

    #[test]
    fn payload_shared_publish_failure_retains_prior_store_and_returns_no_completion() {
        use crate::embedder::payload_tests::{success, PayloadServer};
        let server =
            PayloadServer::new(
                |i, body| if i == 1 { (413, "refused".into()) } else { success(i, body) },
            );
        let embedder = crate::Embedder::new(server.config(4096));
        let documents: Vec<_> =
            (0..3).map(|i| indexed_document(&format!("M{i}.bsl"), "input")).collect();
        let store = FakeEmbeddingStore::default();
        let result = BaselinePublisher::new(EmbeddingExecutionPolicy {
            batch_size: 1,
            concurrency: 1,
            progress_interval: 1,
        })
        .publish(
            &store,
            &Snapshot::new("payload", CorpusId::WorkspaceCode),
            &SnapshotPublishMetadata { branch: None, commit: None },
            &documents,
            Some(&embedder),
            None,
        );
        assert_eq!(result.unwrap_err().to_string(), "embedding_request_too_large");
        assert_eq!(store.embeddings.lock().unwrap().len(), 1);
        assert_eq!(*store.stored_batches.lock().unwrap(), vec![1]);
        // This is the existing snapshot counter. The CLI propagates our Err before its separate semantic completion write.
        assert_eq!(*store.publish_calls.lock().unwrap(), 1);
    }

    #[test]
    fn payload_shared_publish_preserves_worker_concurrency_bound() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        #[derive(Clone)]
        struct Gated {
            started: crossbeam_channel::Sender<()>,
            release: crossbeam_channel::Receiver<()>,
            active: Arc<AtomicUsize>,
            peak: Arc<AtomicUsize>,
        }
        impl EmbeddingGenerator for Gated {
            fn model_id(&self) -> &str {
                "gated"
            }
            fn dimension(&self) -> usize {
                3
            }
            fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SearchError> {
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(active, Ordering::SeqCst);
                self.started.send(()).unwrap();
                let released = self.release.recv_timeout(std::time::Duration::from_secs(2));
                self.active.fetch_sub(1, Ordering::SeqCst);
                released.map_err(|_| SearchError::Embedder("fixture watchdog".into()))?;
                Ok(texts.iter().map(|_| vec![1.0, 2.0, 3.0]).collect())
            }
        }
        let (started_tx, started_rx) = crossbeam_channel::unbounded();
        let (release_tx, release_rx) = crossbeam_channel::unbounded();
        let peak = Arc::new(AtomicUsize::new(0));
        let embedder = Gated {
            started: started_tx,
            release: release_rx,
            active: Arc::new(AtomicUsize::new(0)),
            peak: peak.clone(),
        };
        let worker = std::thread::spawn(move || {
            let documents: Vec<_> =
                (0..4).map(|i| indexed_document(&format!("M{i}"), "input")).collect();
            SharedEmbeddingPublisher::new(EmbeddingExecutionPolicy {
                batch_size: 1,
                concurrency: 2,
                progress_interval: 1,
            })
            .publish(&FakeEmbeddingStore::default(), &embedder, &documents, None)
        });
        for _ in 0..2 {
            for _ in 0..2 {
                started_rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
            }
            assert!(started_rx.try_recv().is_err());
            for _ in 0..2 {
                release_tx.send(()).unwrap();
            }
        }
        assert_eq!(worker.join().unwrap().unwrap().stored, 4);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }
}
