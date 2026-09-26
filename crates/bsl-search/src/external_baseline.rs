use crate::domain::{
    BaselineRef, ExternalBaselineBackend, ExternalBaselineConfig, IndexedDocument, LexicalHit,
    SemanticHit, Snapshot, SnapshotPublishMetadata, SnapshotPublishStats,
};
use crate::error::SearchError;
use crate::ports::{
    BaselineLexicalSearch, BaselineSemanticSearch, EmbeddingStore, SnapshotCatalog,
    SnapshotContentStore, SnapshotPublisher, WorkspaceBaselineManifest,
    WorkspaceBaselineManifestStore,
};
use std::collections::HashMap;
use std::time::Duration;

mod postgres;
pub use postgres::ensure_the_roots_mean_the_same_elsewhere;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineSnapshotRecord {
    pub snapshot_id: String,
    pub corpus: String,
    pub fingerprint: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub created_at: String,
    pub files: usize,
    pub documents: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineCollectionRecord {
    pub collection: String,
    pub files: usize,
    pub documents: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineSnapshotDetails {
    pub snapshot: BaselineSnapshotRecord,
    pub collections: Vec<BaselineCollectionRecord>,
    /// Publication identity read atomically with `snapshot`; absent when unqualified.
    pub semantic_publication: Option<BaselineSemanticPublication>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineSemanticPublication {
    pub model_id: String,
    pub dimension: usize,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineFileObjectRecord {
    pub file_object_id: String,
    pub collection: String,
    pub fingerprint: String,
    pub documents: usize,
    pub snapshots: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineFileObjectReference {
    pub snapshot_id: String,
    /// The source root the referencing file belongs to. Two roots may share one file object
    /// when their files have the same relative path and the same content — that is content
    /// sharing, not a duplicate — so without the root two references are indistinguishable
    /// and the question "where did this row come from" has no answer.
    pub root_id: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineFileObjectDetails {
    pub file_object: BaselineFileObjectRecord,
    pub references: Vec<BaselineFileObjectReference>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BaselineEmbeddingStats {
    pub stored: usize,
    pub reused: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineEmbeddingModelRecord {
    pub model_id: String,
    pub dimension: usize,
    pub embeddings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineEmbeddingCoverageRecord {
    pub model_id: String,
    pub dimension: usize,
    pub active_payloads: usize,
    pub covered_payloads: usize,
    pub embeddings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineGcReport {
    pub orphan_file_objects: usize,
    pub orphan_file_object_items: usize,
    pub orphan_semantic_embeddings: usize,
    pub deleted_file_objects: usize,
    pub deleted_file_object_items: usize,
    pub deleted_semantic_embeddings: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticPublishPhase {
    PrepareRows,
    CopyParentRows,
    WriteServingRows,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SemanticPublishProgress {
    Plan {
        strategy: String,
        changed_files: usize,
        deleted_paths: usize,
        parent_snapshot_id: Option<String>,
        phase_count: usize,
    },
    PhaseStarted {
        phase: SemanticPublishPhase,
        phase_index: usize,
        phase_count: usize,
        detail: String,
    },
    PhaseCompleted {
        phase: SemanticPublishPhase,
        phase_index: usize,
        phase_count: usize,
        elapsed: Duration,
        output_rows: usize,
    },
    Completed {
        total_rows: usize,
        copied_rows: usize,
        inserted_rows: usize,
        missing_embeddings: usize,
        total_elapsed: Duration,
    },
}

#[derive(Debug, Clone)]
pub enum ExternalBaselineAdapter {
    Postgres(postgres::PostgresBaselineAdapter),
}

impl ExternalBaselineAdapter {
    pub fn new(config: ExternalBaselineConfig) -> Result<Self, SearchError> {
        match config.backend {
            ExternalBaselineBackend::Postgres => {
                Ok(Self::Postgres(postgres::PostgresBaselineAdapter::new(config)?))
            }
        }
    }

    pub fn config(&self) -> &ExternalBaselineConfig {
        match self {
            Self::Postgres(adapter) => adapter.config(),
        }
    }

    pub fn list_snapshots(
        &self,
        corpus: Option<&str>,
        branch: Option<&str>,
        commit: Option<&str>,
        limit: usize,
    ) -> Result<Vec<BaselineSnapshotRecord>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.list_snapshots(corpus, branch, commit, limit),
        }
    }

    pub fn snapshot_details(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<BaselineSnapshotDetails>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.snapshot_details(snapshot_id),
        }
    }

    pub fn list_file_objects(
        &self,
        collection: Option<&str>,
        limit: usize,
    ) -> Result<Vec<BaselineFileObjectRecord>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.list_file_objects(collection, limit),
        }
    }

    pub fn file_object_details(
        &self,
        file_object_id: &str,
    ) -> Result<Option<BaselineFileObjectDetails>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.file_object_details(file_object_id),
        }
    }

    pub fn store_embeddings(
        &self,
        model_id: &str,
        dimension: usize,
        embeddings: &[(String, Vec<f32>)],
    ) -> Result<BaselineEmbeddingStats, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.store_embeddings(model_id, dimension, embeddings),
        }
    }

    pub fn load_embeddings(
        &self,
        embedding_keys: &[String],
        model_id: &str,
        dimension: usize,
    ) -> Result<HashMap<String, Vec<f32>>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.load_embeddings(embedding_keys, model_id, dimension),
        }
    }

    pub fn ensure_embedding_identity(
        &self,
        model_id: &str,
        dimension: usize,
    ) -> Result<(), SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.ensure_embedding_identity(model_id, dimension),
        }
    }

    pub fn read_embedding_identity(&self) -> Result<Option<(String, usize)>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.read_embedding_identity(),
        }
    }

    pub fn list_embedding_models(
        &self,
        model_id: Option<&str>,
        dimension: Option<usize>,
    ) -> Result<Vec<BaselineEmbeddingModelRecord>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.list_embedding_models(model_id, dimension),
        }
    }

    pub fn embedding_coverage(
        &self,
        model_id: Option<&str>,
        dimension: Option<usize>,
    ) -> Result<Vec<BaselineEmbeddingCoverageRecord>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.embedding_coverage(model_id, dimension),
        }
    }

    pub fn garbage_collect(&self, execute: bool) -> Result<BaselineGcReport, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.garbage_collect(execute),
        }
    }

    pub fn migrate_storage(&self) -> Result<(), SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.migrate_storage(),
        }
    }

    pub fn check_storage_readiness(&self) -> Result<(), SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.check_storage_readiness(),
        }
    }

    pub fn get_schema_version(&self) -> Result<Option<i32>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.get_schema_version(),
        }
    }

    pub fn lexical_search_baseline(
        &self,
        snapshot_id: &str,
        query: &str,
        collection: Option<&str>,
        limit: usize,
    ) -> Result<Vec<LexicalHit>, SearchError> {
        match self {
            Self::Postgres(adapter) => {
                adapter.lexical_search_baseline(snapshot_id, query, collection, limit)
            }
        }
    }

    pub fn populate_serving_semantic(
        &self,
        snapshot_id: &str,
        model_id: &str,
        dimension: usize,
    ) -> Result<usize, SearchError> {
        self.populate_serving_semantic_with_progress(snapshot_id, model_id, dimension, None)
    }

    pub fn populate_serving_semantic_with_progress(
        &self,
        snapshot_id: &str,
        model_id: &str,
        dimension: usize,
        progress: Option<&dyn Fn(SemanticPublishProgress)>,
    ) -> Result<usize, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.populate_serving_semantic_with_progress(
                snapshot_id,
                model_id,
                dimension,
                progress,
            ),
        }
    }

    pub fn semantic_search_baseline(
        &self,
        snapshot_id: &str,
        query_embedding: &[f32],
        model_id: &str,
        dimension: usize,
        collection: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SemanticHit>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.semantic_search_baseline(
                snapshot_id,
                query_embedding,
                model_id,
                dimension,
                collection,
                limit,
            ),
        }
    }

    pub fn load_baseline_manifest(
        &self,
        snapshot_id: &str,
    ) -> Result<WorkspaceBaselineManifest, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.load_baseline_manifest(snapshot_id),
        }
    }
}

impl EmbeddingStore for ExternalBaselineAdapter {
    fn load_embeddings(
        &self,
        embedding_keys: &[String],
        model_id: &str,
        dimension: usize,
    ) -> Result<HashMap<String, Vec<f32>>, SearchError> {
        ExternalBaselineAdapter::load_embeddings(self, embedding_keys, model_id, dimension)
    }

    fn store_embeddings(
        &self,
        model_id: &str,
        dimension: usize,
        embeddings: &[(String, Vec<f32>)],
    ) -> Result<BaselineEmbeddingStats, SearchError> {
        ExternalBaselineAdapter::store_embeddings(self, model_id, dimension, embeddings)
    }

    fn ensure_embedding_identity(
        &self,
        model_id: &str,
        dimension: usize,
    ) -> Result<(), SearchError> {
        ExternalBaselineAdapter::ensure_embedding_identity(self, model_id, dimension)
    }
}

impl SnapshotCatalog for ExternalBaselineAdapter {
    fn resolve_baseline(&self, baseline: &BaselineRef) -> Result<Option<Snapshot>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.resolve_baseline(baseline),
        }
    }
}

impl SnapshotContentStore for ExternalBaselineAdapter {
    fn load_snapshot_documents(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<IndexedDocument>, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.load_snapshot_documents(snapshot),
        }
    }
}

impl SnapshotPublisher for ExternalBaselineAdapter {
    fn publish_snapshot(
        &self,
        snapshot: &Snapshot,
        metadata: &SnapshotPublishMetadata,
        documents: &[IndexedDocument],
    ) -> Result<SnapshotPublishStats, SearchError> {
        match self {
            Self::Postgres(adapter) => adapter.publish_snapshot(snapshot, metadata, documents),
        }
    }
}

impl BaselineLexicalSearch for ExternalBaselineAdapter {
    fn lexical_search_baseline(
        &self,
        snapshot_id: &str,
        query: &str,
        collection: Option<&str>,
        limit: usize,
    ) -> Result<Vec<LexicalHit>, SearchError> {
        ExternalBaselineAdapter::lexical_search_baseline(
            self,
            snapshot_id,
            query,
            collection,
            limit,
        )
    }
}

impl BaselineSemanticSearch for ExternalBaselineAdapter {
    fn semantic_search_baseline(
        &self,
        snapshot_id: &str,
        query_embedding: &[f32],
        model_id: &str,
        dimension: usize,
        collection: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SemanticHit>, SearchError> {
        ExternalBaselineAdapter::semantic_search_baseline(
            self,
            snapshot_id,
            query_embedding,
            model_id,
            dimension,
            collection,
            limit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ExternalBaselineAdapter;
    use crate::domain::{CorpusId, ExternalBaselineConfig};

    #[test]
    fn external_adapter_constructs_postgres_backend() {
        let adapter = ExternalBaselineAdapter::new(
            ExternalBaselineConfig::postgres("postgres://example").with_schema("bsl_search"),
        )
        .unwrap();

        assert_eq!(adapter.config().schema.as_deref(), Some("bsl_search"));
        assert_eq!(adapter.config().connection, "postgres://example");
        assert!(matches!(adapter, ExternalBaselineAdapter::Postgres(_)));
    }

    #[test]
    fn invalid_schema_is_rejected_before_runtime_use() {
        let error = ExternalBaselineAdapter::new(
            ExternalBaselineConfig::postgres("postgres://example").with_schema("bad-schema"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("invalid postgres schema"));
        let _ = CorpusId::WorkspaceCode;
    }
}
