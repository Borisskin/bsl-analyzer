//! Persisting the in-memory usearch vector index next to its SQLite database.
//!
//! Rebuilding the HNSW from every embedding at each cold start is the dominant warmup cost
//! (measured ~392s for ~695k vectors; see `examples/bench_vector_index.rs`). Loading a
//! prebuilt index is ~10s. This module saves the index to `<db>.usearch` with a
//! `<db>.usearch.json` sidecar that lets a later start validate the file against the current
//! embeddings and fall back to a rebuild whenever anything is off.
//!
//! Validity is content-true, not a count/rowid proxy:
//! - scalar gates (schema, usearch version, build options, model, dim, embed-text version)
//!   fail fast on a configuration change;
//! - the `embedding_generation` counter (a DB-trigger-maintained monotonic version of the
//!   `(chunks.id, chunks.embedding)` set — see [`crate::store`]) catches a re-embed, an in-place
//!   vector update, or a crash between writing embeddings and re-saving the index, with a single
//!   one-row read instead of scanning every embedding BLOB;
//! - `index_sha` binds the sidecar to a specific index file, so a torn write from two backends
//!   (e.g. during a version rollout that shares the same database) or a truncated/corrupt file
//!   is rejected rather than loaded — no cross-process lock needed.
//!
//! Every step degrades to "rebuild": a missing/old/corrupt file is never served as if valid. A
//! destructive structural-schema wipe resets the generation counter, so the store deletes these
//! artifacts ([`remove_artifacts`]) in that path — a reset counter can never match a stale sidecar.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::SearchError;
use crate::index::VectorIndex;
use crate::lifecycle::{Outcome, Reason, Record};
use crate::store::{Store, EMBED_TEXT_VERSION};

const SIDECAR_SCHEMA: u32 = 2;

/// What the persisted index was built from. The loader rebuilds unless every field still
/// matches the current database and the on-disk index file.
#[derive(Serialize, Deserialize)]
struct Sidecar {
    schema: u32,
    usearch_version: String,
    options: String,
    model_id: String,
    dim: usize,
    embed_text_version: i64,
    count: usize,
    /// The `embedding_generation` the index was built at. The load-time content check is a single
    /// read of the current counter against this value — no BLOB scan.
    generation: i64,
    /// blake3 of the saved index file — binds this sidecar to that exact file.
    index_sha: String,
}

/// Inputs that identify a persisted index for a given engine.
pub struct PersistKey<'a> {
    pub db_path: &'a Path,
    pub model_id: &'a str,
    pub dim: usize,
}

/// Fully written, fsynced and hashed vector artifacts awaiting only their two atomic replaces.
pub struct PreparedPersist {
    db_path: PathBuf,
    index_tmp: Option<PathBuf>,
    index_path: PathBuf,
    sidecar_tmp: Option<PathBuf>,
    sidecar_path: PathBuf,
    generation: i64,
    installed: bool,
}

impl PreparedPersist {
    pub fn generation(&self) -> i64 {
        self.generation
    }

    /// Publish already prepared files. No work here scales with the workspace.
    pub fn install(&mut self) -> Result<(), SearchError> {
        for (temporary, destination, kind, error_label) in [
            (
                &mut self.index_tmp,
                &self.index_path,
                "artifact_install_index",
                "install vector index",
            ),
            (
                &mut self.sidecar_tmp,
                &self.sidecar_path,
                "artifact_install_sidecar",
                "install index sidecar",
            ),
        ] {
            let mut record = Record::new(&self.db_path, kind, Reason::ExplicitRebuild);
            record.emit(false);
            let result =
                fs::rename(temporary.as_ref().expect("prepared artifact temp"), destination);
            record.outcome = if result.is_ok() { Outcome::Committed } else { Outcome::Failed };
            record.emit(false);
            result.map_err(|e| SearchError::Index(format!("{error_label}: {e}")))?;
            *temporary = None;
        }
        self.installed = true;
        Ok(())
    }

    /// Crash-durability hardening is not part of the ownership-protected replace.
    pub fn finish(&self) {
        if self.installed {
            fsync_parent_dir(&self.index_path);
        }
    }
}

impl Drop for PreparedPersist {
    fn drop(&mut self) {
        if let Some(path) = self.index_tmp.take() {
            let _ = fs::remove_file(path);
        }
        if let Some(path) = self.sidecar_tmp.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn index_path(db_path: &Path) -> PathBuf {
    sibling(db_path, "usearch")
}

fn sidecar_path(db_path: &Path) -> PathBuf {
    sibling(db_path, "usearch.json")
}

/// Remove the persisted index + sidecar beside `db_path`. Called by the store when it wipes the
/// structural schema: that wipe resets `embedding_generation` to 0, so a surviving gen-0 sidecar +
/// matching index could false-accept over the emptied database. The sidecar is deleted FIRST and
/// its removal is fallible — `try_load` reads the sidecar before anything else, so its absence alone
/// prevents a stale load, and a failure to remove it must abort the wipe (the caller propagates the
/// error before committing) rather than leave an emptied DB paired with a loadable sidecar. A
/// already-absent sidecar is success. The index file is harmless without a sidecar, so its removal
/// stays best-effort.
pub(crate) fn remove_artifacts(db_path: &Path) -> Result<(), SearchError> {
    remove_file_if_exists(db_path, &sidecar_path(db_path), "artifact_remove_sidecar")?;
    let _ = remove_file_if_exists(db_path, &index_path(db_path), "artifact_remove_index");
    Ok(())
}

fn remove_file_if_exists(
    db_path: &Path,
    path: &Path,
    kind: &'static str,
) -> Result<(), SearchError> {
    let mut record = Record::new(db_path, kind, Reason::GenerationMissing);
    record.emit(false);
    let result = match fs::remove_file(path) {
        Ok(()) => {
            record.outcome = Outcome::Committed;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            record.outcome = Outcome::NoOp;
            Ok(())
        }
        Err(e) => {
            record.outcome = Outcome::Failed;
            Err(SearchError::Index(format!("remove stale vector sidecar {}: {e}", path.display())))
        }
    };
    record.emit(false);
    result
}

/// `<db_path>.<ext>` (kept beside the database so it shares the project's `.build` dir).
fn sibling(db_path: &Path, ext: &str) -> PathBuf {
    let mut s = db_path.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

fn file_blake3(path: &Path) -> Result<String, (SearchError, Reason)> {
    let mut hasher = blake3::Hasher::new();
    let mut file = fs::File::open(path)
        .map_err(|e| (SearchError::Index(format!("open index for hashing: {e}")), io_reason(&e)))?;
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| (SearchError::Index(format!("hash index file: {e}")), io_reason(&e)))?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Try to load a persisted index consistent with the current embeddings. `None` means the
/// caller must rebuild and prepare a new publication. Never returns a stale/wrong index.
pub fn try_load(store: &Store, key: &PersistKey) -> Option<VectorIndex> {
    let result = (|| {
        let sidecar =
            read_sidecar(&sidecar_path(key.db_path)).map_err(|reason| (reason, "sidecar"))?;

        // Existing scalar gates remain ahead of every database/hash read.
        if sidecar.schema != SIDECAR_SCHEMA
            || sidecar.usearch_version != usearch::version()
            || sidecar.options != VectorIndex::options_signature(key.dim)
            || sidecar.model_id != key.model_id
            || sidecar.dim != key.dim
            || sidecar.embed_text_version != EMBED_TEXT_VERSION
        {
            return Err((Reason::ArtifactStale, "compatibility_mismatch"));
        }
        // The generation is still the sole O(1) content gate; diagnostics adds no scan.
        let generation =
            store.embedding_generation().map_err(|_| (Reason::ReadError, "generation_read"))?;
        if generation != sidecar.generation {
            let reason =
                if generation == -1 { Reason::GenerationMissing } else { Reason::ArtifactStale };
            return Err((reason, "generation_mismatch"));
        }
        let idx_path = index_path(key.db_path);
        if file_blake3(&idx_path).map_err(|(_, reason)| (reason, "index_read"))?
            != sidecar.index_sha
        {
            return Err((Reason::ArtifactInvalid, "digest_mismatch"));
        }
        let index = VectorIndex::load(key.dim, &idx_path)
            .map_err(|_| (Reason::ArtifactInvalid, "index_decode"))?;
        if index.len() != sidecar.count {
            return Err((Reason::ArtifactInvalid, "count_mismatch"));
        }
        Ok(index)
    })();
    let (kind, reason, outcome) = match &result {
        Ok(_) => ("artifact_load", Reason::Unchanged, Outcome::Completed),
        Err((reason, _)) => ("artifact_reject", *reason, Outcome::Skipped),
    };
    let mut record = Record::new(key.db_path, kind, reason);
    record.outcome = outcome;
    if let Err((_, gate)) = &result {
        record.examples.push((*gate).to_owned());
    }
    record.emit(false);
    result.ok()
}

/// Perform every workspace-sized persistence step before the publication fence.
pub fn prepare(
    index: &VectorIndex,
    key: &PersistKey,
    generation: i64,
) -> Result<PreparedPersist, SearchError> {
    let idx_path = index_path(key.db_path);

    // Write the index to a unique temp (never a shared `.tmp`, which would itself race), fsync,
    // then atomically rename into place.
    let tmp = unique_temp(&idx_path);
    let index_sha = match (|| {
        index.save(&tmp)?;
        fsync_file(&tmp)?;
        file_blake3(&tmp).map_err(|(error, _)| error)
    })() {
        Ok(hash) => hash,
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
    };
    // Hash OUR temp's exact bytes BEFORE publishing. Hashing the shared `<db>.usearch` after the
    // rename would race a competing writer that overwrites it in between, pairing this sidecar's
    // digest with another writer's index. Hashing the temp binds the sidecar to the bytes this
    // writer published; if another writer's index wins the path, the load-time file hash mismatches
    // and rejects rather than loading a mixed snapshot.
    let sidecar = Sidecar {
        schema: SIDECAR_SCHEMA,
        usearch_version: usearch::version().to_owned(),
        options: VectorIndex::options_signature(key.dim),
        model_id: key.model_id.to_owned(),
        dim: key.dim,
        embed_text_version: EMBED_TEXT_VERSION,
        count: index.len(),
        generation,
        index_sha,
    };
    let sidecar_path = sidecar_path(key.db_path);
    let sidecar_tmp = match write_sidecar_temp(&sidecar_path, &sidecar) {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
    };
    Ok(PreparedPersist {
        db_path: key.db_path.to_path_buf(),
        index_tmp: Some(tmp),
        index_path: idx_path,
        sidecar_tmp: Some(sidecar_tmp),
        sidecar_path,
        generation,
        installed: false,
    })
}

#[cfg(test)]
pub fn persist(index: &VectorIndex, key: &PersistKey, generation: i64) -> Result<(), SearchError> {
    let mut prepared = prepare(index, key, generation)?;
    prepared.install()?;
    prepared.finish();
    Ok(())
}

fn io_reason(error: &std::io::Error) -> Reason {
    if error.kind() == std::io::ErrorKind::NotFound {
        Reason::ArtifactMissing
    } else {
        Reason::ReadError
    }
}

fn read_sidecar(path: &Path) -> Result<Sidecar, Reason> {
    let bytes = fs::read(path).map_err(|error| io_reason(&error))?;
    serde_json::from_slice(&bytes).map_err(|_| Reason::ArtifactInvalid)
}

fn write_sidecar_temp(path: &Path, sidecar: &Sidecar) -> Result<PathBuf, SearchError> {
    let json = serde_json::to_vec_pretty(sidecar)
        .map_err(|e| SearchError::Index(format!("serialize index sidecar: {e}")))?;
    let tmp = unique_temp(path);
    {
        let mut file = fs::File::create(&tmp)
            .map_err(|e| SearchError::Index(format!("create sidecar temp: {e}")))?;
        file.write_all(&json)
            .map_err(|e| SearchError::Index(format!("write sidecar temp: {e}")))?;
        file.sync_all().map_err(|e| SearchError::Index(format!("fsync sidecar temp: {e}")))?;
    }
    Ok(tmp)
}

fn fsync_file(path: &Path) -> Result<(), SearchError> {
    // Windows FlushFileBuffers requires a writable handle; keep the saved bytes intact.
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| SearchError::Index(format!("fsync index temp: {e}")))
}

/// Best-effort fsync of the directory so a rename survives power loss; never fatal (the rename
/// itself is already atomic for in-process consistency, this only hardens crash durability).
fn fsync_parent_dir(path: &Path) {
    if let Some(dir) = path.parent() {
        if let Ok(handle) = fs::File::open(dir) {
            let _ = handle.sync_all();
        }
    }
}

/// A unique sibling temp path. Uniqueness (pid + the target's own bytes) keeps two concurrent
/// writers from clobbering each other's in-progress file before the atomic rename.
fn unique_temp(target: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    sibling(target, &format!("tmp-{}-{}", std::process::id(), stamp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_roots::CONFIGURATION_ROOT_ID;
    use code_chunk::{Chunk, ChunkKind};

    const DIM: usize = 8;

    fn chunk(name: &str) -> Chunk {
        Chunk {
            kind: ChunkKind::Procedure,
            name: name.to_owned(),
            is_export: true,
            annotations: vec![],
            line_start: 0,
            line_end: 1,
            text: format!("Процедура {name}() КонецПроцедуры"),
        }
    }

    fn emb(seed: f32) -> Vec<f32> {
        (0..DIM).map(|i| seed + i as f32 * 0.01).collect()
    }

    /// A file-backed store seeded with `n` embedded chunks (in-memory stores can't persist).
    fn seeded_store(dir: &Path, n: usize) -> Store {
        let mut store = Store::open(&dir.join("search.db")).unwrap();
        let chunks: Vec<Chunk> = (0..n).map(|i| chunk(&format!("P{i}"))).collect();
        let embs: Vec<Vec<f32>> = (0..n).map(|i| emb(i as f32)).collect();
        store.reindex_file(CONFIGURATION_ROOT_ID, "f.bsl", b"h0", &chunks, Some(&embs)).unwrap();
        store
    }

    fn key(store: &Store) -> PersistKey<'_> {
        PersistKey { db_path: store.db_path(), model_id: "test-model", dim: DIM }
    }

    /// Build the index from the current embeddings and persist it, stamping the snapshot's
    /// generation (as the engine does via `load_all_embeddings_with_generation`).
    fn build_and_persist(store: &Store) {
        let (generation, data) = store.load_all_embeddings_with_generation(DIM).unwrap();
        let index = VectorIndex::build(DIM, &data).unwrap();
        let mut prepared = prepare(&index, &key(store), generation).unwrap();
        prepared.install().unwrap();
        prepared.finish();
    }

    #[test]
    fn prepared_bundle_waits_for_install_and_drop_cleans_temps() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 2);
        let (generation, data) = store.load_all_embeddings_with_generation(DIM).unwrap();
        let index = VectorIndex::build(DIM, &data).unwrap();
        let prepared = prepare(&index, &key(&store), generation).unwrap();
        let index_tmp = prepared.index_tmp.clone().unwrap();
        let sidecar_tmp = prepared.sidecar_tmp.clone().unwrap();

        assert!(index_tmp.exists());
        assert!(sidecar_tmp.exists());
        assert!(!index_path(store.db_path()).exists());
        assert!(!sidecar_path(store.db_path()).exists());

        drop(prepared);
        assert!(!index_tmp.exists());
        assert!(!sidecar_tmp.exists());
    }

    #[test]
    fn persist_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 5);
        build_and_persist(&store);

        let loaded = try_load(&store, &key(&store)).expect("a valid sidecar loads");
        assert_eq!(loaded.len(), 5);
        // The loaded index answers queries.
        assert!(!loaded.search(&emb(0.0), 3).unwrap().is_empty());
    }

    #[test]
    fn missing_sidecar_means_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 3);
        assert!(try_load(&store, &key(&store)).is_none());
    }

    #[test]
    fn model_mismatch_means_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 3);
        build_and_persist(&store);

        let other = PersistKey { db_path: store.db_path(), model_id: "other-model", dim: DIM };
        assert!(try_load(&store, &other).is_none());
    }

    #[test]
    fn changed_embedding_means_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 3);
        build_and_persist(&store);
        assert!(try_load(&store, &key(&store)).is_some());

        // Replace one embedding in place (same row id, same count) — the generation counter must
        // advance and force a rebuild, where a count/rowid proxy would wrongly accept it.
        let id = store.load_all_embeddings(DIM).unwrap()[0].0;
        store.set_chunk_embedding(id, &emb(99.0)).unwrap();
        assert!(try_load(&store, &key(&store)).is_none());
    }

    #[test]
    fn inserted_chunk_means_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = seeded_store(dir.path(), 3);
        build_and_persist(&store);
        assert!(try_load(&store, &key(&store)).is_some());

        // A new embedded chunk in another file advances the generation even though the existing
        // rows are untouched, so the persisted index (missing the new vector) is rebuilt.
        store
            .reindex_file(CONFIGURATION_ROOT_ID, "g.bsl", b"h1", &[chunk("New")], Some(&[emb(7.0)]))
            .unwrap();
        assert!(try_load(&store, &key(&store)).is_none());
    }

    #[test]
    fn removed_file_means_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 3);
        build_and_persist(&store);
        assert!(try_load(&store, &key(&store)).is_some());

        // Deleting the file cascades to its chunks; `files_gen_del` advances the generation so the
        // index built over the now-deleted vectors is rejected.
        store.remove_file(CONFIGURATION_ROOT_ID, "f.bsl", "code").unwrap();
        assert!(try_load(&store, &key(&store)).is_none());
    }

    #[test]
    fn corrupt_index_file_means_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 3);
        build_and_persist(&store);

        // Truncate/garble the index file: its bytes no longer match `index_sha`.
        std::fs::write(index_path(store.db_path()), b"not a usearch index").unwrap();
        assert!(try_load(&store, &key(&store)).is_none());
    }
    fn capture(f: impl FnOnce()) -> Vec<serde_json::Value> {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::prelude::*;
        struct Capture(Arc<Mutex<Vec<serde_json::Value>>>);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                struct Visitor<'a>(&'a mut Vec<serde_json::Value>);
                impl tracing::field::Visit for Visitor<'_> {
                    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {
                    }
                    fn record_str(&mut self, field: &tracing::field::Field, text: &str) {
                        if field.name() == "record" {
                            self.0.push(serde_json::from_str(text).unwrap());
                        }
                    }
                }
                if event.metadata().target() == crate::lifecycle::TARGET {
                    event.record(&mut Visitor(&mut self.0.lock().unwrap()));
                }
            }
        }
        let records = Arc::new(Mutex::new(Vec::new()));
        crate::lifecycle::test_with_subscriber(
            tracing_subscriber::registry().with(Capture(records.clone())),
            f,
        );
        Arc::try_unwrap(records).unwrap().into_inner().unwrap()
    }

    #[test]
    fn lifecycle_artifact_rejection_reasons_and_warm_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 2);
        let records = capture(|| {
            assert!(try_load(&store, &key(&store)).is_none());
        });
        assert_eq!(records[0]["reason"], "artifact_missing");
        fs::write(sidecar_path(store.db_path()), b"canary-invalid-json").unwrap();
        let records = capture(|| {
            assert!(try_load(&store, &key(&store)).is_none());
        });
        assert_eq!(records[0]["reason"], "artifact_invalid");
        assert!(!serde_json::to_string(&records).unwrap().contains("canary-invalid-json"));
        build_and_persist(&store);
        let records = capture(|| {
            assert!(try_load(&store, &key(&store)).is_some());
        });
        assert_eq!(records[0]["kind"], "artifact_load");
        let other = PersistKey { model_id: "other", ..key(&store) };
        let records = capture(|| {
            assert!(try_load(&store, &other).is_none());
        });
        assert_eq!(records[0]["reason"], "artifact_stale");
        assert_eq!(records[0]["examples"][0], "compatibility_mismatch");
        let id = store.load_all_embeddings(DIM).unwrap()[0].0;
        store.set_chunk_embedding(id, &emb(9.0)).unwrap();
        let records = capture(|| {
            assert!(try_load(&store, &key(&store)).is_none());
        });
        assert_eq!(records[0]["examples"][0], "generation_mismatch");
        build_and_persist(&store);
        fs::write(index_path(store.db_path()), b"invalid index").unwrap();
        let records = capture(|| {
            assert!(try_load(&store, &key(&store)).is_none());
        });
        assert_eq!(records[0]["reason"], "artifact_invalid");
        assert_eq!(records[0]["examples"][0], "digest_mismatch");
        fs::remove_file(index_path(store.db_path())).unwrap();
        let records = capture(|| {
            assert!(try_load(&store, &key(&store)).is_none());
        });
        assert_eq!(records[0]["reason"], "artifact_missing");
        fs::remove_file(sidecar_path(store.db_path())).unwrap();
        fs::create_dir(sidecar_path(store.db_path())).unwrap();
        let records = capture(|| {
            assert!(try_load(&store, &key(&store)).is_none());
        });
        assert_eq!(records[0]["reason"], "read_error");
    }

    #[test]
    fn lifecycle_artifact_removal_reports_partial_results_without_sql_loss() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 2);
        build_and_persist(&store);
        fs::remove_file(index_path(store.db_path())).unwrap();
        fs::create_dir(index_path(store.db_path())).unwrap();
        let records = capture(|| remove_artifacts(store.db_path()).unwrap());
        assert_eq!(
            records.iter().map(|r| r["outcome"].as_str().unwrap()).collect::<Vec<_>>(),
            ["started", "committed", "started", "failed"]
        );
        for record in &records {
            assert_eq!(record["counts"]["sqlite_vectors_removed"], 0);
        }
        assert_eq!(store.load_all_embeddings(DIM).unwrap().len(), 2);
        fs::create_dir(sidecar_path(store.db_path())).unwrap();
        let records = capture(|| assert!(remove_artifacts(store.db_path()).is_err()));
        assert_eq!(records.len(), 2);
        assert_eq!(records[1]["kind"], "artifact_remove_sidecar");
        assert_eq!(records[1]["outcome"], "failed");
    }

    #[test]
    fn lifecycle_artifact_install_preserves_partial_rename_outcomes() {
        for fail_index in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = seeded_store(dir.path(), 2);
            let (generation, data) = store.load_all_embeddings_with_generation(DIM).unwrap();
            let index = VectorIndex::build(DIM, &data).unwrap();
            let mut prepared = prepare(&index, &key(&store), generation).unwrap();
            let blocked = if fail_index {
                index_path(store.db_path())
            } else {
                sidecar_path(store.db_path())
            };
            fs::create_dir(&blocked).unwrap();
            let records = capture(|| assert!(prepared.install().is_err()));
            let outcomes: Vec<_> = records.iter().map(|r| r["outcome"].as_str().unwrap()).collect();
            if fail_index {
                assert_eq!(outcomes, ["started", "failed"]);
                assert_eq!(records[1]["kind"], "artifact_install_index");
                assert!(prepared.index_tmp.is_some());
            } else {
                assert_eq!(outcomes, ["started", "committed", "started", "failed"]);
                assert_eq!(records[1]["kind"], "artifact_install_index");
                assert_eq!(records[3]["kind"], "artifact_install_sidecar");
                assert!(prepared.index_tmp.is_none());
                assert!(index_path(store.db_path()).is_file());
            }
            assert!(!prepared.installed);
            assert!(prepared.sidecar_tmp.is_some());
            assert!(records.iter().all(|r| r["counts"]["sqlite_vectors_removed"] == 0));
            assert_eq!(store.embedding_generation().unwrap(), generation);
            assert_eq!(store.load_all_embeddings(DIM).unwrap(), data);
            fs::remove_dir(blocked).unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 2);
        let records = capture(|| build_and_persist(&store));
        assert_eq!(records.iter().filter(|r| r["outcome"] == "committed").count(), 2);
        assert!(try_load(&store, &key(&store)).is_some());
    }

    #[test]
    fn lifecycle_artifact_removal_survives_sql_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let store = seeded_store(dir.path(), 2);
        build_and_persist(&store);
        let mut connection = rusqlite::Connection::open(store.db_path()).unwrap();
        let tx = connection.transaction().unwrap();
        tx.execute("DELETE FROM chunks", []).unwrap();
        let records = capture(|| remove_artifacts(store.db_path()).unwrap());
        tx.rollback().unwrap();
        assert_eq!(store.load_all_embeddings(DIM).unwrap().len(), 2);
        assert!(!sidecar_path(store.db_path()).exists());
        assert!(!index_path(store.db_path()).exists());
        assert_eq!(records.iter().filter(|r| r["outcome"] == "committed").count(), 2);
        assert!(records.iter().all(|r| r["counts"]["sqlite_vectors_removed"] == 0));
    }
}
