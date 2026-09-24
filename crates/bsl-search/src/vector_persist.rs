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
        fs::rename(self.index_tmp.as_ref().expect("prepared index temp"), &self.index_path)
            .map_err(|e| SearchError::Index(format!("install vector index: {e}")))?;
        self.index_tmp = None;
        fs::rename(self.sidecar_tmp.as_ref().expect("prepared sidecar temp"), &self.sidecar_path)
            .map_err(|e| SearchError::Index(format!("install index sidecar: {e}")))?;
        self.sidecar_tmp = None;
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
    remove_file_if_exists(&sidecar_path(db_path))?;
    let _ = fs::remove_file(index_path(db_path));
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<(), SearchError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            Err(SearchError::Index(format!("remove stale vector sidecar {}: {e}", path.display())))
        }
    }
}

/// `<db_path>.<ext>` (kept beside the database so it shares the project's `.build` dir).
fn sibling(db_path: &Path, ext: &str) -> PathBuf {
    let mut s = db_path.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

fn file_blake3(path: &Path) -> Result<String, SearchError> {
    let mut hasher = blake3::Hasher::new();
    let mut file = fs::File::open(path)
        .map_err(|e| SearchError::Index(format!("open index for hashing: {e}")))?;
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| SearchError::Index(format!("hash index file: {e}")))?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Try to load a persisted index consistent with the current embeddings. `None` means the
/// caller must rebuild and prepare a new publication. Never returns a stale/wrong index.
pub fn try_load(store: &Store, key: &PersistKey) -> Option<VectorIndex> {
    let sidecar = read_sidecar(&sidecar_path(key.db_path))?;

    // Cheap scalar gates first — fail fast when configuration plainly changed.
    if sidecar.schema != SIDECAR_SCHEMA
        || sidecar.usearch_version != usearch::version()
        || sidecar.options != VectorIndex::options_signature(key.dim)
        || sidecar.model_id != key.model_id
        || sidecar.dim != key.dim
        || sidecar.embed_text_version != EMBED_TEXT_VERSION
    {
        return None;
    }

    // Content gate (O(1)): the `embedding_generation` counter advances on every write that can
    // change the indexed `(chunks.id, chunks.embedding)` set, so an unchanged counter means the
    // current embeddings are exactly what this index was built from — no BLOB scan needed. Any
    // re-embed, in-place vector update, insert, or delete (including a structural wipe, after which
    // the artifacts are gone) moves it and forces a rebuild.
    if store.embedding_generation().ok()? != sidecar.generation {
        return None;
    }

    // File gate: reject a torn/corrupt index whose bytes do not match this sidecar.
    let idx_path = index_path(key.db_path);
    if file_blake3(&idx_path).ok()? != sidecar.index_sha {
        return None;
    }

    let index = VectorIndex::load(key.dim, &idx_path).ok()?;
    if index.len() != sidecar.count {
        return None;
    }
    Some(index)
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
        file_blake3(&tmp)
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

fn read_sidecar(path: &Path) -> Option<Sidecar> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
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
}
