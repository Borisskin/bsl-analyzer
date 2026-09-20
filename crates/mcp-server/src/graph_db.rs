//! On-disk SQLite store for the workspace call graph.
//!
//! The whole-config in-memory graph does not fit in RAM on large configurations
//! (a 25k-file ERP needs tens of GB). The graph is therefore built in bounded
//! batches and streamed into a SQLite file, which is then queried to serve `graph`
//! tool calls without ever materialising the full node/edge set in memory.
//!
//! The database is a **derived cache**: every row is reconstructable from the
//! sources, so the writer favours bulk-insert throughput (in-memory journal, no
//! per-row fsync) over crash durability. A truncated or corrupt file is detected
//! on open and rebuilt rather than trusted.
//!
//! Durable node ids ([`NodeRow::id`]) are produced by the build-time encoder in
//! [`hir::graph_index`], byte-identical to the ids the in-memory serving path
//! emits, so ids an agent holds survive the in-memory → SQLite switch.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use ide::graph_index::{EdgeRow, NodeRow};
use ide::{GraphBuildSummary, GraphBuildTicker, MethodCallDigest, ModuleId, RootDatabaseImpl};
use rusqlite::{params, Connection, OptionalExtension};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use vfs::FileId;

#[cfg(test)]
use crate::graph::input::enumerate_bsl_files;
use crate::graph::input::{build_source_root, db_for_files};

/// Bumped whenever the table layout OR the persisted edge/node content changes so a
/// stale on-disk cache from an older binary is rejected (via the `meta` row) and
/// rebuilt. Version 5 adds the `notify_ref`/`idle_handler` callback edges; version 6
/// adds the `event_subscription` handler edges; version 7 changes persisted edge
/// content again — literal manager dispatch now stores `resolved` provenance, a
/// `Новый ОписаниеОповещения` error handler becomes a second `notify_ref` edge, and
/// `Движения.<Регистр>.<метод>()` movements become `register_movement` edges. Version 8
/// resolves idle handlers to a unique global common module (new cross-module edges).
/// Version 9 adds `subsystem_membership` edges (subsystem → member object / child subsystem).
/// Version 10 adds `role_reference` edges (role → object it grants rights on, plus RLS
/// condition objects). Version 11 adds `register_records` edges (document → register it
/// declares it posts, from the document's `RegisterRecords` metadata). Version 12 adds
/// `register_record_set` edges (code → register reached through a literal record-set creator
/// `РегистрыНакопления.<X>.СоздатьНаборЗаписей()`) and resolves locally-literal dynamic
/// `Движения[…]` indices to `register_movement` edges. Version 13 persists resolved
/// constant-manager method calls as method-to-method `call` edges. Version 14 builds
/// edges under dependency-aware extension visibility (`dependsOn`), so graphs built by
/// a pre-dependency binary must be rejected and rebuilt. Version 15 records the
/// extension-topology fingerprint (`topology_fp`) in the freshness meta, so a cached
/// graph without it can never be mistaken for topology-fresh.
// 16: `meta` gained `unread_paths` — the modules whose bytes could not be read when
// the artefact was built. An older artefact has no such key, and reading its absence
// as "nothing was unread" would certify a graph built partly blind, so the bump
// routes it to a rebuild through the existing mismatch path.
// 17: an unread body now bars callers from resolving into any body behind it, so a
// version-16 artefact holds edges — and `unresolved_calls` rows — this binary would
// never produce. Nothing about the workspace changed, so no fingerprint moves and
// nothing else would ever force the rebuild; only the version says the projection
// itself is of another generation.
// 18: `mdo` rows carry the file the object is defined by. A version-17 artefact
// holds them with a null file, and nothing about the workspace changed, so no
// fingerprint moves: a stale artefact would keep answering about metadata objects
// with a durable id and no place, which is the split this version closes.
// 19: `edges` rows carry the call site — the byte range of the call in the `from` node's
// file, or the code saying why there is none. A version-18 artefact has neither, and its
// edges are otherwise indistinguishable from this binary's, so serving call sites from it
// would answer "no place" for every edge that has one.
// 20: the row encoder now strips its rel against the RESOLVED workspace root, so a workspace
// declared through a link mints `method/file/<rel>::<name>` where a version-19 build minted
// `method/file/<basename>::<name>` and called it unaddressable. Same tree, same bytes: no
// fingerprint moves and no patch would ever rewrite a module nobody edited, so a reused
// artefact would answer `not_found` for the ids this binary now hands out — and an
// incremental patch would leave one database holding both spellings for nodes of one kind.
// 21: graph file addresses are persisted as root_id/path pairs and the files
// table carries full BLAKE3 content hashes, so absolute paths and stat-only
// identity cannot survive as the current format.
pub(crate) const SCHEMA_VERSION: u32 = 21;

/// One file's persisted identity in the `files` table: its complete content hash,
/// a compact projection retained for the existing drift API, and (for `.bsl`) its
/// resolution-signature hash. Persisting these per key lets a reload classify drift
/// granularly instead of only knowing the whole-workspace fingerprint moved.
pub(crate) struct FileFingerprint {
    /// Root-relative durable identity. The pair is never flattened into a
    /// single string in the SQLite key space.
    pub root_id: String,
    pub path: String,
    /// Full BLAKE3 digest of the file bytes.
    pub content_hash: [u8; 32],
    /// Compact projection retained for the existing drift API.
    pub fingerprint: u64,
    /// Resolution-signature hash, `None` for `.xml` (filled in by the body-only fast
    /// path; currently always `None`).
    pub sig_hash: Option<u64>,
}

/// The workspace identity a graph build reflects, as two independent components.
/// `files` folds every graph-relevant file's `(root_id, path, content_hash)`; `topology`
/// identifies the extension dependency graph (declared roots + `dependsOn`
/// closures). Kept structured — not XOR-folded into one word — so a change in one
/// component can never algebraically cancel a change in the other, and so a
/// consumer can tell a topology-triggered rebuild from a plain file edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GraphFp {
    /// Stable BLAKE3 fold of the durable file keys and complete content hashes.
    pub files: u64,
    /// Stable hash of the extension-topology fingerprint.
    pub topology: u64,
}

/// Build-level metadata recorded in the `meta` table, used on reopen to decide
/// whether a cached database still matches the current sources and binary. Node
/// and edge counts are derived from the bulk data at finalize time, not supplied.
pub struct GraphMeta {
    /// The [`GraphState`](crate::graph) generation this build reflects.
    pub revision: u64,
    /// Workspace identity (portable file contents + extension topology) at build time.
    pub fingerprint: GraphFp,
    /// Number of `.bsl` files indexed.
    pub files: usize,
    /// RFC 3339 build timestamp.
    pub built_at: String,
}

/// Read the canonical method-call digest from an existing bounded-build SQLite graph.
///
/// `SetAction` registrations are intentionally excluded: the call hierarchy only
/// represents direct, notification, and idle-handler method calls.
pub fn read_sqlite_method_call_digest(path: &Path) -> anyhow::Result<MethodCallDigest> {
    let conn = Connection::open(path)
        .with_context(|| format!("opening graph database at {}", path.display()))?;
    let mut statement = conn.prepare(
        "SELECT edge.to_id, edge.from_id \
         FROM edges AS edge \
         JOIN nodes AS target ON target.id = edge.to_id \
         JOIN nodes AS caller ON caller.id = edge.from_id \
         WHERE target.kind = 'method' \
           AND caller.kind = 'method' \
           AND edge.kind IN ('call', 'notify_ref', 'idle_handler') \
         ORDER BY edge.to_id, edge.from_id",
    )?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<(String, String)>>>()
        .context("reading method-to-method call edges from graph database")?;
    Ok(MethodCallDigest::from_rows(rows))
}

/// Read the method-call digest whose caller and target both belong to one source root.
///
/// Callers obtain `source_root_files` by resolving the anchor file's `SourceRootId` and
/// enumerating that root's file set. Requiring both endpoints matches the compact index,
/// which retains only modules from that same source root.
pub fn read_source_root_scoped_sqlite_method_call_digest<I>(
    path: &Path,
    source_root_files: I,
) -> anyhow::Result<MethodCallDigest>
where
    I: IntoIterator<Item = bsl_search::FileKey>,
{
    let mut conn = Connection::open(path)
        .with_context(|| format!("opening graph database at {}", path.display()))?;
    let tx = conn.transaction().context("starting source-root scope transaction")?;
    tx.execute_batch(
        "CREATE TEMP TABLE source_root_files (
             root_id TEXT NOT NULL,
             path TEXT NOT NULL,
             PRIMARY KEY (root_id, path)
         ) WITHOUT ROWID;",
    )
    .context("creating source-root file scope")?;
    {
        let mut insert = tx
            .prepare("INSERT OR IGNORE INTO source_root_files (root_id, path) VALUES (?1, ?2)")
            .context("preparing source-root file scope insert")?;
        for file in source_root_files {
            insert
                .execute(params![file.root_id, file.path])
                .context("adding file to source-root scope")?;
        }
    }

    let mut statement = tx.prepare(
        "SELECT edge.to_id, edge.from_id \
         FROM edges AS edge \
         JOIN nodes AS target ON target.id = edge.to_id \
         JOIN nodes AS caller ON caller.id = edge.from_id \
         JOIN source_root_files AS target_file ON target_file.root_id = target.file_root_id \
             AND target_file.path = target.file_path \
         JOIN source_root_files AS caller_file ON caller_file.root_id = caller.file_root_id \
             AND caller_file.path = caller.file_path \
         WHERE target.kind = 'method' \
           AND caller.kind = 'method' \
           AND edge.kind IN ('call', 'notify_ref', 'idle_handler') \
         ORDER BY edge.to_id, edge.from_id",
    )?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<(String, String)>>>()
        .context("reading source-root-scoped method-to-method call edges from graph database")?;
    Ok(MethodCallDigest::from_rows(rows))
}

/// Streams graph rows into a fresh SQLite file. Created once per build; nodes and
/// edges are appended in batches, then [`finalize`](Self::finalize) builds the
/// secondary indexes and the in-degree table in one pass over the bulk data.
pub(crate) struct GraphDbWriter {
    conn: Connection,
    roots: Option<bsl_search::WorkspaceRoots>,
    /// Canonical and walked spellings from the one scan that produced this graph.
    /// A row may carry either spelling; both must resolve to the same durable key
    /// when a symlink target lies outside the canonical root table.
    file_key_aliases: FxHashMap<String, bsl_search::FileKey>,
}

impl GraphDbWriter {
    /// Open `path` as a fresh database, discarding any prior file at that path so
    /// a stale schema can never leak into the new build. Sets bulk-load pragmas.
    pub(crate) fn create(path: &Path) -> anyhow::Result<Self> {
        for suffix in ["", "-wal", "-shm"] {
            let sibling = path.with_file_name(format!(
                "{}{suffix}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("bsl-graph.db")
            ));
            let _ = std::fs::remove_file(&sibling);
        }

        let conn = Connection::open(path)
            .with_context(|| format!("opening graph database at {}", path.display()))?;
        // A rebuildable cache: trade durability for bulk-insert throughput.
        conn.execute_batch(
            "
            PRAGMA journal_mode = MEMORY;
            PRAGMA synchronous = OFF;
            PRAGMA temp_store = MEMORY;
            PRAGMA cache_size = -65536;

            CREATE TABLE nodes (
                id          TEXT PRIMARY KEY,
                kind        TEXT NOT NULL,
                name        TEXT NOT NULL,
                qualified   TEXT NOT NULL,
                module      TEXT,
                file_root_id TEXT,
                file_path   TEXT,
                name_offset INTEGER,
                sig_end     INTEGER,
                src_start   INTEGER,
                src_end     INTEGER,
                dispatch    TEXT,
                is_export   INTEGER,
                addressable INTEGER NOT NULL
            ) WITHOUT ROWID;

            CREATE TABLE edges (
                from_id    TEXT NOT NULL,
                to_id      TEXT NOT NULL,
                kind       TEXT NOT NULL,
                provenance TEXT NOT NULL,
                call_start INTEGER,
                call_end   INTEGER,
                call_absent TEXT,
                crosses    INTEGER NOT NULL
            );

            CREATE TABLE in_degree (
                id     TEXT PRIMARY KEY,
                degree INTEGER NOT NULL
            ) WITHOUT ROWID;

            CREATE TABLE meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE files (
                root_id      TEXT NOT NULL,
                path         TEXT NOT NULL,
                content_hash BLOB NOT NULL,
                fingerprint  INTEGER NOT NULL,
                sig_hash     INTEGER,
                PRIMARY KEY (root_id, path)
            ) WITHOUT ROWID;

            CREATE TABLE unresolved_calls (
                target_scope TEXT NOT NULL,
                method_lower TEXT NOT NULL,
                caller_root_id TEXT NOT NULL,
                caller_path    TEXT NOT NULL,
                PRIMARY KEY (target_scope, method_lower, caller_root_id, caller_path)
            ) WITHOUT ROWID;
            ",
        )
        .context("initialising graph schema")?;

        Ok(Self { conn, roots: None, file_key_aliases: FxHashMap::default() })
    }

    pub(crate) fn set_workspace_roots(&mut self, roots: Option<&bsl_search::WorkspaceRoots>) {
        self.roots = roots.cloned();
    }

    pub(crate) fn set_file_key_aliases(
        &mut self,
        aliases: &FxHashMap<String, bsl_search::FileKey>,
    ) {
        self.file_key_aliases = aliases.clone();
    }

    /// Append a batch of nodes. A node id may be projected more than once across
    /// batches (the same MDO/method reached from several callers); the first
    /// spelling wins and later duplicates are ignored, matching the in-memory
    /// graph's first-seen node identity.
    pub(crate) fn write_nodes(&mut self, rows: &[NodeRow]) -> anyhow::Result<()> {
        let roots = self.roots.clone();
        let aliases = &self.file_key_aliases;
        let tx = self.conn.transaction().context("begin node batch")?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO nodes \
                 (id, kind, name, qualified, module, file_root_id, file_path, name_offset, sig_end, src_start, \
                  src_end, dispatch, is_export, addressable) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?;
            for row in rows {
                let dispatch =
                    if row.dispatch.is_empty() { None } else { Some(row.dispatch.join(",")) };
                let (file_root_id, file_path) =
                    durable_file_key_with_aliases(roots.as_ref(), row.file.as_deref(), aliases);
                if row.file.is_some()
                    && roots.is_some()
                    && (file_root_id.is_none() || file_path.is_none())
                {
                    anyhow::bail!(
                        "graph node source is outside registered workspace roots: {}",
                        row.file.as_deref().unwrap_or_default()
                    );
                }
                stmt.execute(params![
                    row.id,
                    row.kind,
                    row.name,
                    row.qualified,
                    row.module,
                    file_root_id,
                    file_path,
                    row.name_offset,
                    row.sig_end,
                    row.src_start,
                    row.src_end,
                    dispatch,
                    row.is_export.map(|b| b as i64),
                    row.addressable as i64,
                ])?;
            }
        }
        tx.commit().context("commit node batch")?;
        Ok(())
    }

    /// Append a batch of edges verbatim. Edge multiplicity is preserved as given;
    /// de-duplication, if any, is the build orchestrator's policy.
    pub(crate) fn write_edges(&mut self, rows: &[EdgeRow]) -> anyhow::Result<()> {
        let tx = self.conn.transaction().context("begin edge batch")?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO edges \
                 (from_id, to_id, kind, provenance, call_start, call_end, call_absent, crosses) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for row in rows {
                stmt.execute(params![
                    row.from_id,
                    row.to_id,
                    row.kind,
                    row.provenance,
                    row.call_start,
                    row.call_end,
                    row.call_site_absent,
                    row.crosses as i64,
                ])?;
            }
        }
        tx.commit().context("commit edge batch")?;
        Ok(())
    }

    /// Persist the per-file fingerprints into the `files` table. Used on reload to
    /// classify which files drifted instead of only knowing the workspace-wide
    /// fingerprint moved. `INSERT OR REPLACE` so a re-run at the same path is
    /// idempotent.
    pub(crate) fn write_files(&mut self, rows: &[FileFingerprint]) -> anyhow::Result<()> {
        let tx = self.conn.transaction().context("begin files batch")?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO files (root_id, path, content_hash, fingerprint, sig_hash) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for row in rows {
                stmt.execute(params![
                    row.root_id,
                    row.path,
                    row.content_hash.as_slice(),
                    row.fingerprint as i64,
                    row.sig_hash.map(|h| h as i64),
                ])?;
            }
        }
        tx.commit().context("commit files batch")?;
        Ok(())
    }

    /// Persist the set of inconsistently-cased objects into a single `meta` row
    /// (`casing_variants`, newline-joined). The incremental fast path reads it to
    /// refuse a body-only update that touches such an object. Empty for the common,
    /// consistently-cased configuration.
    pub(crate) fn write_casing_variants(&mut self, keys: &[String]) -> anyhow::Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('casing_variants', ?1)",
                params![keys.join("\n")],
            )
            .context("writing casing variants")?;
        Ok(())
    }

    /// Persist the module-located-but-unresolved qualified/manager call sites into the
    /// `unresolved_calls` reverse index. The PK dedups repeated call sites, so the
    /// content is order-independent. Used by the incremental fast path to find callers
    /// that would newly resolve when a target module gains/exports a method.
    pub(crate) fn write_unresolved_calls(
        &mut self,
        rows: &[(String, String, String)],
    ) -> anyhow::Result<()> {
        let roots = self.roots.clone();
        let aliases = &self.file_key_aliases;
        let tx = self.conn.transaction().context("begin unresolved_calls batch")?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO unresolved_calls \
                 (target_scope, method_lower, caller_root_id, caller_path) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (target_scope, method_lower, caller_file) in rows {
                let (root_id, path) =
                    durable_file_key_with_aliases(roots.as_ref(), Some(caller_file), aliases);
                let (Some(root_id), Some(path)) = (root_id, path) else {
                    anyhow::bail!(
                        "unresolved call caller is outside registered workspace roots: {caller_file}"
                    );
                };
                stmt.execute(params![target_scope, method_lower, root_id, path])?;
            }
        }
        tx.commit().context("commit unresolved_calls batch")?;
        Ok(())
    }

    /// Build secondary indexes, materialise the in-degree table from the bulk
    /// edges, and record build metadata (including derived node/edge counts).
    /// Consumes the writer — no further rows may be appended.
    pub(crate) fn finalize(mut self, meta: &GraphMeta) -> anyhow::Result<()> {
        self.conn
            .execute_batch(
                "
                CREATE INDEX edges_from ON edges(from_id);
                CREATE INDEX edges_to ON edges(to_id);
                CREATE INDEX nodes_kind ON nodes(kind);

                INSERT INTO in_degree (id, degree)
                    SELECT to_id, COUNT(*) FROM edges GROUP BY to_id;
                ",
            )
            .context("finalising graph indexes")?;

        let nodes: i64 = self.conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        let edges: i64 = self.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;

        let rows: [(&str, String); 8] = [
            ("schema_version", SCHEMA_VERSION.to_string()),
            ("revision", meta.revision.to_string()),
            ("fingerprint", meta.fingerprint.files.to_string()),
            ("topology_fp", meta.fingerprint.topology.to_string()),
            ("files", meta.files.to_string()),
            ("built_at", meta.built_at.clone()),
            ("nodes", nodes.to_string()),
            ("edges", edges.to_string()),
        ];
        let tx = self.conn.transaction().context("begin meta write")?;
        {
            let mut stmt = tx.prepare_cached("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
            for (key, value) in &rows {
                stmt.execute(params![key, value])?;
            }
        }
        tx.commit().context("commit meta write")?;
        self.conn.execute_batch("ANALYZE;").context("analyse graph database")?;
        Ok(())
    }
}

fn durable_file_key(
    roots: Option<&bsl_search::WorkspaceRoots>,
    file: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(file) = file else { return (None, None) };
    if let Some(roots) = roots {
        if let Some(key) = roots.key_of_path(Path::new(file)) {
            return (Some(key.root_id), Some(key.path));
        }
        // A source row outside the registered roots is deliberately not made
        // readable through a guessed absolute fallback.
        return (None, None);
    }
    // Unit fixtures that construct a writer without a project root use
    // already-relative paths. Keep the legacy column populated only for those
    // fixtures; production writers always install WorkspaceRoots.
    (Some(String::new()), Some(file.replace('\\', "/")))
}

fn durable_file_key_with_aliases(
    roots: Option<&bsl_search::WorkspaceRoots>,
    file: Option<&str>,
    aliases: &FxHashMap<String, bsl_search::FileKey>,
) -> (Option<String>, Option<String>) {
    if let Some(file) = file {
        if let Some(key) = aliases.get(file).or_else(|| aliases.get(&file.replace('\\', "/"))) {
            return (Some(key.root_id.clone()), Some(key.path.clone()));
        }
    }
    durable_file_key(roots, file)
}

fn install_changed_file_keys(
    conn: &Connection,
    changed_files: &[String],
    roots: Option<&bsl_search::WorkspaceRoots>,
) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TEMP TABLE changed_file_keys (
             root_id TEXT NOT NULL,
             path TEXT NOT NULL,
             PRIMARY KEY (root_id, path)
         ) WITHOUT ROWID;",
    )?;
    let mut insert = conn.prepare_cached(
        "INSERT OR IGNORE INTO changed_file_keys (root_id, path) VALUES (?1, ?2)",
    )?;
    for file in changed_files {
        let (Some(root_id), Some(path)) = durable_file_key(roots, Some(file)) else {
            anyhow::bail!("incremental update: changed file is outside registered roots: {file}");
        };
        insert.execute(params![root_id, path])?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredFileKey {
    root_id: String,
    path: String,
}

fn write_unread_keys(
    conn: &rusqlite::Connection,
    unread: &std::collections::BTreeSet<bsl_search::FileKey>,
) -> anyhow::Result<()> {
    let list: Vec<StoredFileKey> = unread
        .iter()
        .map(|key| StoredFileKey { root_id: key.root_id.clone(), path: key.path.clone() })
        .collect();
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('unread_paths', ?1)",
        rusqlite::params![serde_json::to_string(&list)?],
    )?;
    Ok(())
}

fn unread_keys_from_paths(
    unread: &BTreeSet<PathBuf>,
    roots: Option<&bsl_search::WorkspaceRoots>,
    universe: Option<&crate::graph::universe::ScannedUniverse>,
) -> anyhow::Result<BTreeSet<bsl_search::FileKey>> {
    unread
        .iter()
        .map(|path| {
            if let Some(roots) = roots {
                universe
                    .and_then(|universe| universe.key_for_path(roots, path))
                    .or_else(|| roots.key_of_path(path))
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "unread file is outside registered roots: {}",
                            path.display()
                        )
                    })
            } else {
                Ok(bsl_search::FileKey::configuration(path.to_string_lossy().into_owned()))
            }
        })
        .collect()
}

/// The same modules, read STRICTLY: an absent key means "nothing was unread" — the schema
/// version gates out artefacts that predate the key — but a query that fails or a payload that
/// will not decode is a failure to LOOK, not an empty answer. The structured key is retained
/// all the way to recovery; it must never be flattened with a separator that can collide with a
/// root or path.
pub(crate) fn read_unread_keys_strict(
    conn: &rusqlite::Connection,
) -> anyhow::Result<Vec<bsl_search::FileKey>> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'unread_paths'", [], |r| r.get(0))
        .optional()
        .context("reading unread_paths")?;
    let Some(raw) = raw else {
        anyhow::bail!("missing unread_paths metadata");
    };
    let keys = serde_json::from_str::<Vec<StoredFileKey>>(&raw)
        .context("decoding structured unread_paths")?;
    Ok(keys.into_iter().map(|key| bsl_search::FileKey::new(key.root_id, key.path)).collect())
}

/// The modules an artefact recorded as unreadable when it was built or last patched. This
/// lenient view is used for counts and diagnostics; it keeps the structured key even when a
/// caller elects to ignore a metadata read error, and is deliberately not an authority for
/// retiring recovery obligations.
pub(crate) fn read_unread_paths(conn: &rusqlite::Connection) -> Vec<bsl_search::FileKey> {
    read_unread_keys_strict(conn).unwrap_or_default()
}

/// Build the whole-workspace call graph straight into a fresh SQLite file at
/// `out_path`, in RAM-bounded batches. The in-memory graph does not fit on large
/// configurations (a 25k-module ERP blows past 8 GB in a single database), so this
/// is the path that makes a whole-config graph available at all.
///
/// The file universe arrives ALREADY SCANNED (`universe`): the id↔path map, the
/// persisted `files` rows and the caller's fingerprint bracket all project one walk,
/// so no pass of the operation can see a tree another pass did not. Each batch's
/// texts are loaded into a throwaway database (dropped before the next), with
/// cross-batch call targets resolved through the resident compact method index —
/// never another batch's database. Peak memory is therefore bounded by the batch
/// size plus that index, not by the whole config.
///
/// Returns the build tally; node/edge counts in the database are recorded in its
/// `meta` table by [`GraphDbWriter::finalize`], and the paths whose bytes could not be
/// read go beside them under `unread_paths` — the artefact carries its own gaps, so no
/// caller has to thread them through.
pub(crate) fn build_graph_database(
    project: &crate::graph::ProjectSnapshot,
    universe: &crate::graph::universe::ScannedUniverse,
    out_path: &Path,
    batch_size: usize,
    meta: &GraphMeta,
) -> anyhow::Result<GraphBuildSummary> {
    build_graph_database_inner(project, universe, out_path, batch_size, meta, None)
}

/// As [`build_graph_database`], but also streams the search index's code chunks (with
/// graph context) from the same parse pass into `chunk_sink` — the compute half of the
/// graph/search fusion. The graph rows written are byte-identical to the plain build.
pub(crate) fn build_graph_database_fused(
    project: &crate::graph::ProjectSnapshot,
    universe: &crate::graph::universe::ScannedUniverse,
    out_path: &Path,
    batch_size: usize,
    meta: &GraphMeta,
    chunk_sink: &mut dyn ide::FusedChunkSink,
) -> anyhow::Result<GraphBuildSummary> {
    build_graph_database_inner(project, universe, out_path, batch_size, meta, Some(chunk_sink))
}

/// Default seconds without build progress before the watchdog reports a stall.
const GRAPH_STALL_REPORT_SECS: u64 = 600;

/// Monitor thread for a running graph build. A deadlock in the build's parallel
/// region freezes the process silently — alive, zero CPU, zero disk growth — so
/// the watchdog turns that into an actionable `error!` record: how long the build
/// has been stuck, at which phase/batch, and every thread's kernel state. It
/// re-reports once per stall interval while the stall lasts.
///
/// `BSL_GRAPH_STALL_SECS` overrides the reporting threshold;
/// `BSL_GRAPH_STALL_ABORT=1` additionally aborts the process on the first report
/// (for supervised deployments where a restart beats a wedged daemon). The
/// watchdog only observes — by default a stalled build is left running so it can
/// still be inspected with a debugger.
struct BuildWatchdog {
    stop: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for BuildWatchdog {
    fn drop(&mut self) {
        let (lock, signal) = &*self.stop;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        signal.notify_all();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Append one stall episode to the report file next to the graph database. The
/// daemon's file logging is opt-in, so this one-shot artifact is what survives a
/// wedged build in a default deployment: nothing is written in healthy runs,
/// and each episode appends a timestamped position + thread-state block.
fn write_stall_report(dir: &Path, stalled_secs: u64, position: &str, threads: &str) {
    use std::io::Write;
    let path = dir.join(crate::cache::STALL_REPORT_FILE);
    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = format!(
        "[epoch {epoch_secs}] graph build stalled for {stalled_secs}s\n\
         position: {position}\nthreads: {threads}\n\n"
    );
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(entry.as_bytes()));
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), "could not write stall report: {e}");
    }
}

fn spawn_build_watchdog(
    ticker: Arc<GraphBuildTicker>,
    report_dir: Option<PathBuf>,
) -> BuildWatchdog {
    let stop = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let stop_pair = Arc::clone(&stop);
    // Misconfiguration must be loud: an operator reproducing a wedged build relies
    // on these knobs actually being in effect, and a silent fallback costs them a
    // multi-hour cold-build cycle.
    let threshold_secs = match std::env::var("BSL_GRAPH_STALL_SECS") {
        Ok(value) => match value.parse::<u64>() {
            Ok(secs) if secs > 0 => secs,
            _ => {
                tracing::warn!(
                    value = %value,
                    default_secs = GRAPH_STALL_REPORT_SECS,
                    "invalid BSL_GRAPH_STALL_SECS (want a positive integer); using the default"
                );
                GRAPH_STALL_REPORT_SECS
            }
        },
        Err(_) => GRAPH_STALL_REPORT_SECS,
    };
    let abort_on_stall = match std::env::var("BSL_GRAPH_STALL_ABORT") {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "" | "0" | "false" | "no" | "off" => false,
            _ => {
                tracing::warn!(
                    value = %value,
                    "unrecognized BSL_GRAPH_STALL_ABORT (want 1/true/yes/on); abort disabled"
                );
                false
            }
        },
        Err(_) => false,
    };
    tracing::info!(
        target: "bsl_graph",
        threshold_secs,
        abort_on_stall,
        "graph build watchdog armed"
    );
    let spawned =
        std::thread::Builder::new().name("bsl-graph-watchdog".to_owned()).spawn(move || {
            let (lock, signal) = &*stop_pair;
            let mut reported_episodes = 0;
            let mut stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
            while !*stopped {
                // Condvar wait instead of a plain sleep so dropping the watchdog
                // (every build teardown, including tests) returns immediately
                // rather than after the current poll tick.
                let (guard, _) = signal
                    .wait_timeout(stopped, Duration::from_secs(1))
                    .unwrap_or_else(|e| e.into_inner());
                stopped = guard;
                if *stopped {
                    break;
                }
                let stalled_ms = ticker.ms_since_progress();
                let episodes = stalled_ms / (threshold_secs * 1000);
                if episodes == 0 {
                    reported_episodes = 0;
                } else if episodes > reported_episodes {
                    reported_episodes = episodes;
                    let position = ticker.position();
                    let threads = thread_state_summary();
                    tracing::error!(
                        stalled_secs = stalled_ms / 1000,
                        position = %position,
                        threads = %threads,
                        "graph build has made no progress; its parallel region may be deadlocked"
                    );
                    if let Some(dir) = &report_dir {
                        write_stall_report(dir, stalled_ms / 1000, &position, &threads);
                    }
                    if abort_on_stall {
                        tracing::error!("BSL_GRAPH_STALL_ABORT=1: aborting the stalled process");
                        std::process::abort();
                    }
                }
            }
        });
    let handle = match spawned {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!("could not spawn graph build watchdog: {e}");
            None
        }
    };
    BuildWatchdog { stop, handle }
}

/// One compact `name:state:wchan` entry per OS thread of this process — the same
/// facts a by-hand `/proc` inspection collects, captured at the moment of a stall.
#[cfg(target_os = "linux")]
fn thread_state_summary() -> String {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return "unavailable".to_owned();
    };
    let mut entries: Vec<String> = Vec::new();
    for task in tasks.flatten() {
        let read = |name: &str| {
            std::fs::read_to_string(task.path().join(name)).unwrap_or_default().trim().to_owned()
        };
        let comm = read("comm");
        let wchan = read("wchan");
        // The state field follows the parenthesised comm, which may itself
        // contain spaces — split after the closing paren, not on raw whitespace.
        let stat = read("stat");
        let state = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .unwrap_or("?")
            .to_owned();
        entries.push(format!("{comm}:{state}:{wchan}"));
    }
    entries.join(" ")
}

#[cfg(not(target_os = "linux"))]
fn thread_state_summary() -> String {
    "unavailable on this platform".to_owned()
}

fn build_graph_database_inner(
    project: &crate::graph::ProjectSnapshot,
    universe: &crate::graph::universe::ScannedUniverse,
    out_path: &Path,
    batch_size: usize,
    meta: &GraphMeta,
    chunk_sink: Option<&mut dyn ide::FusedChunkSink>,
) -> anyhow::Result<GraphBuildSummary> {
    if !project.validated || project.search_roots.is_none() {
        anyhow::bail!("cannot persist a portable graph without validated workspace roots");
    }
    let files = &universe.files;
    let modules: Vec<ModuleId> = files.iter().map(|(f, _)| ModuleId::new(*f)).collect();
    let paths: FxHashMap<FileId, String> = files
        .iter()
        .map(|(f, p)| {
            let walked = universe.walked_path_for(p).unwrap_or(p);
            (*f, walked.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    let file_paths: FxHashMap<FileId, PathBuf> =
        files.iter().map(|(f, p)| (*f, p.clone())).collect();

    // Where each metadata object is defined, read off the universe this build
    // already scanned rather than a fresh walk of the disk.
    let mdo_files = crate::graph::mdo_files::mdo_files(
        &project.configs,
        &bsl_conventions::PathSetTree::from_files(
            universe.stats.iter().map(|stat| PathBuf::from(&stat.path)),
        ),
    );

    // The whole-workspace source root, built once and shared (cheap `Arc` clone)
    // into every per-batch database, so the 25k-path file set is assembled a single
    // time for the build rather than re-cloned per batch.
    let source_root = build_source_root(files);

    let mut writer = GraphDbWriter::create(out_path)?;
    writer.set_workspace_roots(project.search_roots.as_ref());
    let roots = project.search_roots.as_ref().expect("validated roots");
    let aliases = universe
        .file_key_aliases(roots)
        .ok_or_else(|| anyhow::anyhow!("graph scan file is outside registered workspace roots"))?;
    writer.set_file_key_aliases(&aliases);

    // One configuration cache shared across every batch database (and their per-job
    // clones), so the whole-config metadata load runs once for this build instead of
    // once per fresh batch database. A fresh cache per build keeps it a content
    // snapshot — see `ide_db`'s `GraphConfigCache`.
    let config_cache = std::sync::Arc::new(ide::GraphConfigCache::default());

    // Heartbeat + stall watchdog for the whole build (index, edge passes, fused
    // chunking): kept alive until after `finalize`, so a wedge anywhere in the
    // pipeline gets reported rather than freezing silently.
    let ticker = Arc::new(GraphBuildTicker::default());
    let _watchdog =
        spawn_build_watchdog(Arc::clone(&ticker), out_path.parent().map(Path::to_path_buf));

    // A SET, not a counter: `open_batch` is called once per batch per pass, and the
    // index pass alone re-opens every module, so a file that cannot be read would be
    // counted many times over.
    let mut unread: BTreeSet<PathBuf> = BTreeSet::new();

    // Scope the closures so their borrows end before `finalize`. `open_batch`
    // loads only the batch's texts (sharing the resident source root + config);
    // `sink` persists the freshly-encoded rows (the sole `&mut writer` borrow).
    let summary = {
        let mut open_batch = |batch: &[ModuleId]| -> RootDatabaseImpl {
            let batch_files: Vec<(FileId, PathBuf)> =
                batch.iter().map(|m| (m.file_id, file_paths[&m.file_id].clone())).collect();
            let loaded =
                db_for_files(&source_root, &batch_files, &project.configs, Some(&config_cache));
            unread.extend(loaded.unread);
            loaded.db
        };
        let mut sink = |nodes: &[NodeRow],
                        edges: &[EdgeRow]|
         -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            writer.write_nodes(nodes)?;
            writer.write_edges(edges)?;
            Ok(())
        };
        ide::build_workspace_graph_rows(
            &modules,
            &paths,
            Some(&project.workspace_root),
            &mdo_files,
            batch_size,
            &mut open_batch,
            &mut sink,
            chunk_sink,
            Some(&ticker),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?
    };

    // Persist a per-file fingerprint for every graph-relevant file (`.bsl` + `.xml`),
    // from the SAME scanned universe the build lowered — not a fresh walk, which
    // could see a tree the built modules do not. For `.bsl` modules also persist the
    // body-free signature hash from the build, so a body-only edit (sig unchanged)
    // is distinguishable from a resolution-affecting one. `.xml` rows keep NULL sig.
    //
    // `file_paths` holds each module's canonical path verbatim; the stats projection
    // stringifies the same canonical path, so keying by that string lines the two up.
    let sig_by_path: FxHashMap<String, u64> = summary
        .module_sig_hashes
        .iter()
        .filter_map(|(m, &h)| {
            file_paths.get(&m.file_id).map(|p| (p.to_string_lossy().into_owned(), h))
        })
        .collect();
    let file_rows: anyhow::Result<Vec<FileFingerprint>> = universe
        .stats
        .iter()
        .map(|s| {
            let Some(key) = s.key(roots) else {
                return Err(anyhow::anyhow!(
                    "graph scan file is outside registered workspace roots: {}",
                    s.path
                ));
            };
            let content_hash = s.persisted_content_hash();
            Ok(FileFingerprint {
                root_id: key.root_id,
                path: key.path,
                content_hash,
                fingerprint: s.fingerprint(),
                sig_hash: sig_by_path.get(&s.path).copied(),
            })
        })
        .collect();
    let file_rows = file_rows?;
    writer.write_files(&file_rows)?;
    writer.write_casing_variants(&summary.casing_variant_objects)?;
    writer.write_unresolved_calls(&summary.unresolved_calls)?;

    writer.finalize(meta)?;
    // Written by the BUILDER, not by whoever calls it. `finalize` stamps
    // `schema_version`, and the contract that an absent key means "nothing was
    // unread" rests on that version gating out older artefacts — so any caller who
    // forgot to add the key would produce a current-version artefact certifying a
    // graph it built blind. The set is born in this function; it is recorded here.
    {
        let conn = rusqlite::Connection::open(out_path)
            .with_context(|| format!("reopening {} to record unread paths", out_path.display()))?;
        let unread_keys =
            unread_keys_from_paths(&unread, project.search_roots.as_ref(), Some(universe))?;
        write_unread_keys(&conn, &unread_keys)?;
    }
    Ok(summary)
}

/// Canonicalise a freshly-projected aux (`mdo`/`attribute`) node/edge id against the
/// object spellings already in the store. The durable id embeds the source-written
/// object casing, but a full rebuild fixes it to the global first-seen owner; an
/// incremental reprojection of a subset only knows the subset's casing, so for an
/// object an unchanged module already owns we must reuse the stored spelling.
///
/// `existing_mdo` maps each stored `mdo/<Type>/<obj>` id, lowercased (Unicode-aware,
/// since SQLite's `lower()` folds ASCII only and object names are Cyrillic), to its
/// actual spelling. A `method`/`module` id, or an object the store does not yet know
/// (genuinely new — owned by the changed set in both paths), is returned unchanged.
fn canonicalize_aux_id(
    existing_mdo: &std::collections::HashMap<String, String>,
    id: &str,
) -> String {
    if id.starts_with("mdo/") {
        return existing_mdo.get(&id.to_lowercase()).cloned().unwrap_or_else(|| id.to_string());
    }
    if let Some(rest) = id.strip_prefix("attribute/") {
        // rest = <Type>/<object>/<attr>; only the object segment needs canonicalising
        // (Type is the stable english name, attr is the metadata-stable field name).
        let mut seg = rest.splitn(3, '/');
        if let (Some(etype), Some(_obj), Some(attr)) = (seg.next(), seg.next(), seg.next()) {
            let mdo_key = format!("mdo/{etype}/{_obj}").to_lowercase();
            if let Some(canon_mdo) = existing_mdo.get(&mdo_key) {
                if let Some(canon_obj) =
                    canon_mdo.strip_prefix("mdo/").and_then(|r| r.split_once('/')).map(|(_, o)| o)
                {
                    return format!("attribute/{etype}/{canon_obj}/{attr}");
                }
            }
        }
    }
    id.to_string()
}

/// Split an aux durable id into its `(EnglishType, object)` segments. `None` for a
/// `method`/`module` id. Object/type segments never contain `/` (BSL identifiers and
/// english type names exclude it), so the split is unambiguous.
fn aux_object(id: &str) -> Option<(&str, &str)> {
    if let Some(rest) = id.strip_prefix("mdo/") {
        return rest.split_once('/');
    }
    if let Some(rest) = id.strip_prefix("attribute/") {
        let mut seg = rest.splitn(3, '/');
        if let (Some(etype), Some(obj), Some(_attr)) = (seg.next(), seg.next(), seg.next()) {
            return Some((etype, obj));
        }
    }
    None
}

/// Refuse the body-only fast path for the two aux-spelling cases its DB-pinned
/// canonicalisation cannot reproduce byte-identically — both require cross-module
/// casing inconsistency, so a normal (consistent-casing) edit is unaffected:
///
/// - **(A) casing change of a referenced object** — a changed module references an
///   existing object with a different exact spelling. If that module is the object's
///   first-seen owner, a full rebuild would adopt the new spelling, but the fast path
///   pins to the stored one.
/// - **(B) ownership shift on drop** — a changed module drops its last reference to an
///   object that survives via another module; a full rebuild would re-derive the
///   canonical spelling from the surviving (possibly different-cased) owner.
///
/// In both cases we fall back to a full rebuild, which is always correct.
fn incremental_safety_check(
    conn: &Connection,
    changed_files: &[String],
    rows: &ide::ReprojectedRows,
    roots: Option<&bsl_search::WorkspaceRoots>,
) -> anyhow::Result<()> {
    use std::collections::{HashMap, HashSet};

    install_changed_file_keys(conn, changed_files, roots)?;

    // (C) Objects the full build saw with inconsistent casing across modules. Their
    // cross-module first-seen ordering is not reconstructable from the canonicalised
    // store, so the fast path must not touch them. Recorded as lowercased
    // `englishtype/object` keys in the `casing_variants` meta row.
    let variant_keys: HashSet<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'casing_variants'", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .into_iter()
        .flat_map(|v| v.lines().map(str::to_string).collect::<Vec<_>>())
        .filter(|s| !s.is_empty())
        .collect();
    let touches_variant = |id: &str| -> bool {
        aux_object(id).is_some_and(|(etype, obj)| {
            variant_keys.contains(&format!("{}/{}", etype.to_lowercase(), obj.to_lowercase()))
        })
    };

    // (A) Stored object spelling per (type, object), case-folded.
    let mut stored_obj: HashMap<(String, String), String> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT id FROM nodes WHERE kind IN ('mdo', 'attribute')")?;
        let ids = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for id in ids.flatten() {
            if let Some((etype, obj)) = aux_object(&id) {
                stored_obj
                    .entry((etype.to_lowercase(), obj.to_lowercase()))
                    .or_insert_with(|| obj.to_string());
            }
        }
    }
    let reprojected_aux = rows
        .nodes
        .iter()
        .filter(|n| n.kind == "mdo" || n.kind == "attribute")
        .map(|n| n.id.as_str())
        .chain(rows.edges.iter().map(|e| e.to_id.as_str()));
    for id in reprojected_aux {
        if touches_variant(id) {
            anyhow::bail!("incremental update: touches casing-variant object {id}; full rebuild");
        }
        if let Some((etype, obj)) = aux_object(id) {
            if let Some(stored) = stored_obj.get(&(etype.to_lowercase(), obj.to_lowercase())) {
                if stored != obj {
                    anyhow::bail!(
                        "incremental update: aux object casing drift ({obj} vs stored {stored}); full rebuild"
                    );
                }
            }
        }
    }

    // (B) Aux objects the changed modules referenced before the edit.
    let old_sql = "SELECT DISTINCT e.to_id FROM edges e JOIN nodes n ON e.from_id = n.id \
         JOIN changed_file_keys changed ON changed.root_id = n.file_root_id \
             AND changed.path = n.file_path \
         WHERE e.to_id LIKE 'mdo/%' OR e.to_id LIKE 'attribute/%'";
    let old_aux: HashSet<String> = {
        let mut stmt = conn.prepare(old_sql)?;
        let it = stmt.query_map([], |r| r.get::<_, String>(0))?;
        it.filter_map(|r| r.ok()).collect()
    };
    for id in &old_aux {
        if touches_variant(id) {
            anyhow::bail!(
                "incremental update: drops/keeps a casing-variant object {id}; full rebuild"
            );
        }
    }
    let new_aux: HashSet<&str> = rows
        .edges
        .iter()
        .map(|e| e.to_id.as_str())
        .filter(|t| t.starts_with("mdo/") || t.starts_with("attribute/"))
        .collect();
    // A surviving reference is one from BSL source outside the changed set. The
    // kind says so; `file` alone no longer does, now that an object carries one.
    let survivors_sql = "SELECT COUNT(*) FROM edges e JOIN nodes n ON e.from_id = n.id \
         WHERE e.to_id = ?1 AND n.kind IN ('method','module') \
           AND NOT EXISTS (SELECT 1 FROM changed_file_keys changed \
                           WHERE changed.root_id = n.file_root_id \
                             AND changed.path = n.file_path)";
    for dropped in old_aux.iter().filter(|x| !new_aux.contains(x.as_str())) {
        let survivors: i64 = conn.query_row(survivors_sql, params![dropped], |r| r.get(0))?;
        if survivors > 0 {
            anyhow::bail!(
                "incremental update: dropped aux ref {dropped} still referenced by an unchanged module; full rebuild"
            );
        }
    }
    Ok(())
}

/// Insert one node row, overriding only its `id` (for aux-id canonicalisation).
/// `INSERT OR IGNORE` keeps the first-seen spelling, exactly like the bulk writer.
fn insert_node_row(
    tx: &rusqlite::Transaction<'_>,
    row: &NodeRow,
    id: &str,
    roots: Option<&bsl_search::WorkspaceRoots>,
) -> anyhow::Result<()> {
    let dispatch = if row.dispatch.is_empty() { None } else { Some(row.dispatch.join(",")) };
    let (file_root_id, file_path) = durable_file_key(roots, row.file.as_deref());
    if row.file.is_some() && roots.is_some() && (file_root_id.is_none() || file_path.is_none()) {
        anyhow::bail!(
            "incremental graph node source is outside registered workspace roots: {}",
            row.file.as_deref().unwrap_or_default()
        );
    }
    tx.prepare_cached(
        "INSERT OR IGNORE INTO nodes \
         (id, kind, name, qualified, module, file_root_id, file_path, name_offset, sig_end, src_start, \
          src_end, dispatch, is_export, addressable) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?
    .execute(params![
        id,
        row.kind,
        row.name,
        row.qualified,
        row.module,
        file_root_id,
        file_path,
        row.name_offset,
        row.sig_end,
        row.src_start,
        row.src_end,
        dispatch,
        row.is_export.map(|b| b as i64),
        row.addressable as i64,
    ])?;
    Ok(())
}

/// Apply an incremental update: reproject ONLY the modules at `changed_paths` and
/// patch a COPY of `src_path` written to `out_path`, leaving every unchanged module's
/// rows in place. `changed_paths` is the FULL reprojection set the caller proved
/// sufficient — either the edited body-only modules (signature unchanged), or, for a
/// signature change, the changed modules PLUS their resolved callers (the
/// caller-delta set). The result is byte-identical to a full rebuild of the edited
/// tree. This function does not re-validate eligibility — the caller
/// (`try_incremental_reload`) owns the sig/caller-delta-safety gates.
///
/// Concurrency: the patch lands on a copy and the caller atomically renames it into
/// place — the same model the full build uses, so a live reader keeps its open
/// snapshot until it reopens and no in-place mutation races a query.
pub(crate) fn update_graph_database_bodies(
    project: &crate::graph::ProjectSnapshot,
    universe: &crate::graph::universe::ScannedUniverse,
    src_path: &Path,
    out_path: &Path,
    changed_paths: &[PathBuf],
    batch_size: usize,
    meta: &GraphMeta,
) -> anyhow::Result<GraphBuildSummary> {
    if !project.validated || project.search_roots.is_none() {
        anyhow::bail!("cannot patch a portable graph without validated workspace roots");
    }
    let files = &universe.files;
    let all_modules: Vec<ModuleId> = files.iter().map(|(f, _)| ModuleId::new(*f)).collect();
    let paths: FxHashMap<FileId, String> =
        files.iter().map(|(f, p)| (*f, p.to_string_lossy().replace('\\', "/"))).collect();
    let file_paths: FxHashMap<FileId, PathBuf> =
        files.iter().map(|(f, p)| (*f, p.clone())).collect();

    // Where each metadata object is defined, read off the universe this build
    // already scanned rather than a fresh walk of the disk.
    let mdo_files = crate::graph::mdo_files::mdo_files(
        &project.configs,
        &bsl_conventions::PathSetTree::from_files(
            universe.stats.iter().map(|stat| PathBuf::from(&stat.path)),
        ),
    );

    // Map changed canonical paths → ModuleIds, preserving file-id order so a new aux
    // object's first-seen spelling matches a full build's.
    let changed_set: std::collections::HashSet<&Path> =
        changed_paths.iter().map(|p| p.as_path()).collect();
    let changed_modules: Vec<ModuleId> = files
        .iter()
        .filter(|(_, p)| changed_set.contains(p.as_path()))
        .map(|(f, _)| ModuleId::new(*f))
        .collect();
    if changed_modules.len() != changed_paths.len() {
        anyhow::bail!(
            "incremental update: {} changed paths, {} matched modules (a path is not an indexed .bsl module)",
            changed_paths.len(),
            changed_modules.len()
        );
    }

    let source_root = build_source_root(files);
    let config_cache = std::sync::Arc::new(ide::GraphConfigCache::default());
    // The patch's own report covers the WHOLE universe, not just `changed`: the
    // index pass opens `all_modules` through this same closure.
    let mut unread: BTreeSet<PathBuf> = BTreeSet::new();
    let mut open_batch = |batch: &[ModuleId]| -> RootDatabaseImpl {
        let batch_files: Vec<(FileId, PathBuf)> =
            batch.iter().map(|m| (m.file_id, file_paths[&m.file_id].clone())).collect();
        let loaded =
            db_for_files(&source_root, &batch_files, &project.configs, Some(&config_cache));
        unread.extend(loaded.unread);
        loaded.db
    };

    // The reprojection's index pass runs the same guarded batch runners as a full
    // build, so it gets the same heartbeat + stall watchdog.
    let ticker = Arc::new(GraphBuildTicker::default());
    let _watchdog =
        spawn_build_watchdog(Arc::clone(&ticker), out_path.parent().map(Path::to_path_buf));

    let rows = ide::reproject_changed_modules(
        &all_modules,
        &changed_modules,
        &paths,
        Some(&project.workspace_root),
        &mdo_files,
        batch_size,
        &mut open_batch,
        Some(&ticker),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Normalised `(file_root_id, file_path)` keys for the changed modules — used both to gate the
    // fast path and to scope the per-module deletes below.
    let changed_files: Vec<String> =
        changed_modules.iter().map(|m| paths[&m.file_id].clone()).collect();

    // Bail to a full rebuild for the aux-casing cases the DB-pinned canonicalisation
    // cannot reproduce (a no-op for normal, consistent-casing edits).
    {
        let src = Connection::open_with_flags(src_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("opening graph db {} read-only", src_path.display()))?;
        incremental_safety_check(&src, &changed_files, &rows, project.search_roots.as_ref())?;
    }

    // Patch a copy, never the published file (a reader keeps its snapshot until the
    // caller renames `out_path` into place).
    std::fs::copy(src_path, out_path).with_context(|| {
        format!("copying graph db {} → {}", src_path.display(), out_path.display())
    })?;

    let stat_by_path: FxHashMap<String, &crate::graph::scan::FileStat> =
        universe.stats.iter().map(|s| (s.path.clone(), s)).collect();

    let mut conn = Connection::open(out_path)?;
    let changed_path_strings: Vec<String> =
        changed_paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    install_changed_file_keys(&conn, &changed_path_strings, project.search_roots.as_ref())?;
    {
        let tx = conn.transaction().context("begin incremental patch")?;

        // The first-seen object spellings the store already owns (Unicode-lowercased
        // key → actual id), loaded before inserting so new objects keep their casing.
        let existing_mdo: std::collections::HashMap<String, String> = {
            let mut stmt = tx.prepare("SELECT id FROM nodes WHERE kind = 'mdo'")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.filter_map(|r| r.ok()).map(|id| (id.to_lowercase(), id)).collect()
        };

        // Drop each changed module's outgoing edges, method nodes, AND module-code
        // node. The module-code node is re-emitted by the reprojection only if the
        // module still has a module-level edge — matching a full rebuild, which emits
        // it solely as an edge endpoint. Deleting it (rather than INSERT OR IGNORE)
        // is what lets a module that lost its last module-level edge shed the node.
        // Only the module's body-derived outgoing edges (from its method/module-code
        // nodes) are reprojected, so only those are deleted. `contains` edges have
        // `mdo`/`form` from-endpoints — never method/module — and the form pass is
        // full-build-only, so the kind filter keeps reprojection from dropping form
        // structure it cannot re-emit.
        tx.execute(
            "DELETE FROM edges WHERE from_id IN \
             (SELECT n.id FROM nodes n JOIN changed_file_keys changed \
              ON changed.root_id = n.file_root_id AND changed.path = n.file_path \
              WHERE n.kind IN ('method', 'module'))",
            [],
        )?;
        tx.execute(
            "DELETE FROM nodes WHERE id IN \
             (SELECT n.id FROM nodes n JOIN changed_file_keys changed \
              ON changed.root_id = n.file_root_id AND changed.path = n.file_path \
              WHERE n.kind IN ('method', 'module'))",
            [],
        )?;

        // Re-insert the reprojected nodes, canonicalising aux ids against the store.
        for row in &rows.nodes {
            match row.kind {
                "mdo" | "attribute" => {
                    let id = canonicalize_aux_id(&existing_mdo, &row.id);
                    insert_node_row(&tx, row, &id, project.search_roots.as_ref())?;
                }
                _ => insert_node_row(&tx, row, &row.id, project.search_roots.as_ref())?,
            }
        }
        // Re-insert the edges, canonicalising aux `to_id`s the same way.
        for edge in &rows.edges {
            let to_id = canonicalize_aux_id(&existing_mdo, &edge.to_id);
            tx.prepare_cached(
                "INSERT INTO edges \
                 (from_id, to_id, kind, provenance, call_start, call_end, call_absent, crosses) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![
                edge.from_id,
                to_id,
                edge.kind,
                edge.provenance,
                edge.call_start,
                edge.call_end,
                edge.call_site_absent,
                edge.crosses as i64
            ])?;
        }

        // GC aux nodes that lost their last reference. Restricted to pure-sink kinds:
        // module-code nodes are `from_id`-only sources, so a `to_id`-absence sweep
        // would wrongly delete a live caller. An `mdo` may also be a `from_id` — the
        // parent of a `contains` edge to a form — so it must survive on either role:
        // a full rebuild keeps such an object (materialised as the contains-from
        // endpoint) even with no inbound call/query edge. (The `from_id` clause is a
        // no-op when no form nodes exist: `mdo`/`attribute` are never edge sources then.)
        tx.execute(
            "DELETE FROM nodes WHERE kind IN ('mdo', 'attribute') \
             AND id NOT IN (SELECT to_id FROM edges) \
             AND id NOT IN (SELECT from_id FROM edges)",
            [],
        )?;

        // Recompute the whole in-degree table — a delta that forgot the deleted edges'
        // old targets would leave stale degrees.
        tx.execute("DELETE FROM in_degree", [])?;
        tx.execute(
            "INSERT INTO in_degree (id, degree) SELECT to_id, COUNT(*) FROM edges GROUP BY to_id",
            [],
        )?;

        // Merge any casing variants the reprojection observed AMONG the changed
        // modules into the persisted set, so a future reload still refuses the fast
        // path for a newly-inconsistent object a multi-file edit introduced.
        if !rows.casing_variant_objects.is_empty() {
            let existing: String = tx
                .query_row("SELECT value FROM meta WHERE key = 'casing_variants'", [], |r| r.get(0))
                .optional()?
                .unwrap_or_default();
            let mut set: std::collections::BTreeSet<String> =
                existing.lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
            set.extend(rows.casing_variant_objects.iter().cloned());
            tx.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('casing_variants', ?1)",
                params![set.into_iter().collect::<Vec<_>>().join("\n")],
            )?;
        }

        // Refresh the changed modules' persisted fingerprint + signature hash.
        for module in &changed_modules {
            let canonical = file_paths[&module.file_id].to_string_lossy().into_owned();
            let Some(stat) = stat_by_path.get(&canonical).copied() else {
                anyhow::bail!(
                    "incremental update: changed file disappeared from scan: {canonical}"
                );
            };
            let (Some(root_id), Some(path)) =
                durable_file_key(project.search_roots.as_ref(), Some(&canonical))
            else {
                anyhow::bail!(
                    "incremental update: changed file is outside registered roots: {canonical}"
                );
            };
            let content_hash = stat.persisted_content_hash();
            let sig = rows.sig_hashes.get(module).copied();
            tx.execute(
                "INSERT OR REPLACE INTO files (root_id, path, content_hash, fingerprint, sig_hash) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![root_id, path, content_hash.as_slice(), stat.fingerprint() as i64, sig.map(|h| h as i64)],
            )?;
        }

        // The artefact's hole set changes ONLY for the modules this patch rewrote —
        // the filter is needed on both sides, and for the same reason. A module's rows
        // are restored, or dropped, solely by the pass that lowered it: an inherited
        // hole that merely became readable still has no rows here, and a module that
        // went dark while nobody edited it still has the rows the build left. Recording
        // the latter would claim its rows are absent when they are only stale, and
        // nothing could ever take it back out — the subtraction releases a path only
        // when a later patch rewrites it, and a module nobody edits is never rewritten.
        // (That such a module is even a candidate is not an accident of `changed`: the
        // reprojection's index pass opens the WHOLE universe through the same loader,
        // so `unread` reports far more than this patch touched.)
        //
        // Keyed by the canonical spelling `changed_paths` carries, NOT by the
        // '/'-normalised `(file_root_id, file_path)` spelling: the unread paths are raw `PathBuf`s,
        // and on Windows the two differ.
        {
            let rewritten: std::collections::BTreeSet<bsl_search::FileKey> = changed_paths
                .iter()
                .filter_map(|path| {
                    let path_text = path.to_string_lossy();
                    let (root_id, key_path) =
                        durable_file_key(project.search_roots.as_ref(), Some(path_text.as_ref()));
                    root_id.zip(key_path).map(|(root, key)| bsl_search::FileKey::new(root, key))
                })
                .collect();
            let mut carried: BTreeSet<bsl_search::FileKey> = read_unread_keys_strict(&tx)?
                .into_iter()
                .filter(|key| !rewritten.contains(key))
                .collect();
            let unread_keys =
                unread_keys_from_paths(&unread, project.search_roots.as_ref(), Some(universe))?;
            carried.extend(unread_keys.into_iter().filter(|key| rewritten.contains(key)));
            write_unread_keys(&tx, &carried)?;
        }

        // Refresh the reverse index of unresolved calls for the reprojected modules:
        // drop their old rows, insert their fresh ones. Unchanged modules' rows stay.
        tx.execute(
            "DELETE FROM unresolved_calls WHERE EXISTS (
                 SELECT 1 FROM changed_file_keys changed
                 WHERE changed.root_id = unresolved_calls.caller_root_id
                   AND changed.path = unresolved_calls.caller_path
             )",
            [],
        )?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO unresolved_calls \
                 (target_scope, method_lower, caller_root_id, caller_path) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (target_scope, method_lower, caller_file) in &rows.unresolved_calls {
                let (Some(root_id), Some(path)) =
                    durable_file_key(project.search_roots.as_ref(), Some(caller_file))
                else {
                    anyhow::bail!(
                        "incremental unresolved call caller is outside registered workspace roots: {caller_file}"
                    );
                };
                stmt.execute(params![target_scope, method_lower, root_id, path])?;
            }
        }

        // Refresh build metadata + derived counts; a clean incremental snapshot is
        // never force-stale.
        let node_count: i64 = tx.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        let edge_count: i64 = tx.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
        let meta_rows: [(&str, String); 8] = [
            ("revision", meta.revision.to_string()),
            ("fingerprint", meta.fingerprint.files.to_string()),
            ("topology_fp", meta.fingerprint.topology.to_string()),
            ("files", all_modules.len().to_string()),
            ("built_at", meta.built_at.clone()),
            ("nodes", node_count.to_string()),
            ("edges", edge_count.to_string()),
            ("force_stale", "0".to_string()),
        ];
        for (key, value) in &meta_rows {
            tx.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
        }

        tx.commit().context("commit incremental patch")?;
    }

    let node_rows = rows.nodes.len();
    let edges: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
    Ok(GraphBuildSummary {
        modules: all_modules.len(),
        node_rows,
        edges: edges as usize,
        module_sig_hashes: rows.sig_hashes,
        // Variants observed among the changed modules (merged into the persisted set
        // above); pre-existing variants for untouched objects remain in the copied db.
        casing_variant_objects: rows.casing_variant_objects,
        // The reprojected modules' unresolved refs were refreshed in the patch above.
        unresolved_calls: rows.unresolved_calls,
    })
}

/// A changed module's recomputed body-free profile: its signature hash plus the
/// resolvable-name surface a caller-delta eligibility check needs — the lowercased
/// names of its exported methods, and whether any two methods fold to the same name
/// (a collision makes "exported name" ≠ "resolvable name", since resolution is
/// first-wins).
pub struct ModuleProfile {
    pub sig_hash: u64,
    pub exported_lower: std::collections::BTreeSet<String>,
    pub has_collision: bool,
    /// The module's bytes could not be read. Not a property of its declarations —
    /// which is exactly why the caller-delta cannot work with it, see
    /// [`caller_delta_plan`].
    pub unread: bool,
}

/// Recompute each module at `changed_paths`'s profile, for the incremental
/// eligibility checks (sig drift, and the caller-delta resolvable-name surface).
/// Builds a tiny resident index over only those modules — these reads are a module's
/// own item-tree + dispatch, no cross-module data — so it stays cheap. Keyed by
/// canonical path.
///
/// `files` is the operation's ALREADY-SCANNED enumeration: profiling must judge the
/// same universe the eligibility diff saw, not a fresh walk that may already differ.
pub fn recompute_module_profiles(
    project: &crate::graph::ProjectSnapshot,
    files: &[(FileId, PathBuf)],
    changed_paths: &[PathBuf],
) -> anyhow::Result<FxHashMap<String, ModuleProfile>> {
    use ide::graph_index::GraphIndex;

    let source_root = crate::graph::build_source_root(files);

    let changed_set: std::collections::HashSet<&Path> =
        changed_paths.iter().map(|p| p.as_path()).collect();
    let changed: Vec<(ModuleId, PathBuf)> = files
        .iter()
        .filter(|(_, p)| changed_set.contains(p.as_path()))
        .map(|(f, p)| (ModuleId::new(*f), p.clone()))
        .collect();

    let batch_files: Vec<(FileId, PathBuf)> =
        changed.iter().map(|(m, p)| (m.file_id, p.clone())).collect();
    // Profiling only compares signature hashes; the unread report belongs to the
    // passes that publish rows, not here.
    let db = db_for_files(&source_root, &batch_files, &project.configs, None).db;
    let modules: Vec<ModuleId> = changed.iter().map(|(m, _)| *m).collect();
    let index = GraphIndex::build(&db, &modules);

    let mut out = FxHashMap::default();
    for (module, path) in &changed {
        let Some(sig_hash) = index.module_sig_hash(*module) else {
            continue;
        };
        let methods = index.module_methods(*module).unwrap_or_default();
        let lowers: Vec<String> = methods.iter().map(|(n, _)| n.to_lowercase()).collect();
        let has_collision =
            lowers.iter().collect::<std::collections::HashSet<_>>().len() != lowers.len();
        let exported_lower: std::collections::BTreeSet<String> =
            methods.iter().filter(|(_, exp)| *exp).map(|(n, _)| n.to_lowercase()).collect();
        out.insert(
            path.to_string_lossy().into_owned(),
            ModuleProfile {
                sig_hash,
                exported_lower,
                has_collision,
                unread: index.is_unread(*module).unwrap_or(false),
            },
        );
    }
    Ok(out)
}

/// Plan the caller-delta for a set of signature-changed modules (the body-only fast
/// path is not eligible because their signature moved). Returns:
/// - `Ok(Some(caller_files))` — reprojecting the changed modules PLUS the returned
///   callers reproduces a full rebuild. Callers are the union of: modules with a
///   stored edge INTO a changed module (covers removal/unexport/dispatch/case-rename),
///   and modules whose previously-unresolved `B.<name>()` would newly resolve when B
///   gains a resolvable `name` (looked up in the `unresolved_calls` reverse index).
///   Excludes the changed modules themselves.
/// - `Ok(None)` — not eligible: a first-wins name collision (invalid-BSL shadowing,
///   old or new), or an added resolvable name on a module whose scope is not
///   name-keyed (so its callers cannot be found). The caller must do a full rebuild.
///
/// `sig_changed` pairs each changed module's normalised `(file_root_id, file_path)` key with its
/// freshly-recomputed [`ModuleProfile`].
pub fn caller_delta_plan(
    db_path: &Path,
    sig_changed: &[(&str, &ModuleProfile)],
    roots: Option<&bsl_search::WorkspaceRoots>,
) -> anyhow::Result<Option<Vec<PathBuf>>> {
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let changed_file_names: Vec<String> =
        sig_changed.iter().map(|(file, _)| (*file).to_owned()).collect();
    install_changed_file_keys(&conn, &changed_file_names, roots)?;

    // What the stored artefact recorded as unreadable. Compared verbatim: both this and
    // the keys of `sig_changed` are the raw canonical spelling of the same scanned
    // path — `unread_paths` from the batch loader's `PathBuf`s, the keys from
    // `files.path`. Normalising either side would break the match on the one platform
    // where the two spellings could differ at all.
    let was_unread: std::collections::HashSet<bsl_search::FileKey> =
        read_unread_keys_strict(&conn)?.into_iter().collect();

    let mut index_callers: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (file, profile) in sig_changed {
        let (root_id, path) = durable_file_key(roots, Some(file));
        let Some(file_key) =
            root_id.zip(path).map(|(root_id, path)| bsl_search::FileKey::new(root_id, path))
        else {
            return Ok(None);
        };
        if profile.has_collision {
            return Ok(None); // first-wins shadowing — exported set ≠ resolvable set
        }
        // A body that has just become unreadable bars callers from resolving into any
        // body BEHIND it — a sibling body of the same common module, in another file
        // entirely. Those callers hold a RESOLVED edge into the sibling's node and
        // nothing at all pointing here, so neither fan-out below can reach them and the
        // only correct answer is a full rebuild.
        if profile.unread {
            return Ok(None);
        }
        // The mirror image: this body has just become readable, and the calls its
        // barrier used to bar now resolve — into that same sibling body. What they were
        // barred from is this module's SCOPE, not any name this file declares, so the
        // by-name lookup below cannot find them (the disputed method may well be
        // declared only next door). Take every caller the artefact recorded as blocked
        // on this scope, whatever the name.
        if was_unread.contains(&file_key) {
            let Some(scope) = ide::scope_for_path(file) else {
                return Ok(None); // not name-keyed → its callers aren't indexable
            };
            let mut stmt = conn.prepare(
                "SELECT caller_root_id, caller_path FROM unresolved_calls WHERE target_scope = ?1",
            )?;
            let rows = stmt.query_map(params![scope], |r| {
                Ok(bsl_search::FileKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let key = row?;
                let Some(path) = roots.and_then(|roots| roots.resolve_walked(&key)) else {
                    return Ok(None);
                };
                index_callers.insert(path.to_string_lossy().into_owned());
            }
        }
        // OLD resolvable surface from the stored method nodes.
        let mut stmt = conn.prepare(
            "SELECT name, is_export FROM nodes \
             WHERE file_root_id = ?1 AND file_path = ?2 AND kind = 'method'",
        )?;
        let rows = stmt.query_map(params![file_key.root_id, file_key.path], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0) != 0))
        })?;
        let mut old_lowers: Vec<String> = Vec::new();
        let mut old_exported: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for row in rows {
            let (name, exported) = row?;
            let lower = name.to_lowercase();
            if exported {
                old_exported.insert(lower.clone());
            }
            old_lowers.push(lower);
        }
        let old_collision =
            old_lowers.iter().collect::<std::collections::HashSet<_>>().len() != old_lowers.len();
        if old_collision {
            return Ok(None);
        }
        // Newly-resolvable names: callers that called them were previously unresolved
        // (dropped from `edges`), so find them through the reverse index by scope+name.
        let added: Vec<&String> =
            profile.exported_lower.iter().filter(|n| !old_exported.contains(*n)).collect();
        if !added.is_empty() {
            let Some(scope) = ide::scope_for_path(file) else {
                return Ok(None); // not name-keyed → its callers aren't indexable
            };
            let mut stmt = conn.prepare(
                "SELECT caller_root_id, caller_path FROM unresolved_calls \
                 WHERE target_scope = ?1 AND method_lower = ?2",
            )?;
            for name in added {
                let rows = stmt.query_map(params![scope, name], |r| {
                    Ok(bsl_search::FileKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let key = row?;
                    let Some(path) = roots.and_then(|roots| roots.resolve_walked(&key)) else {
                        return Ok(None);
                    };
                    index_callers.insert(path.to_string_lossy().into_owned());
                }
            }
        }
    }

    // Resolved callers: modules with a stored edge into a changed module's method node.
    let changed_files: std::collections::BTreeSet<&str> =
        sig_changed.iter().map(|(f, _)| *f).collect();

    // A signature change in an event-subscription handler module can invalidate its
    // config-level `mdo -> method` subscription edge — but that edge's source is an
    // `mdo` node, which the resolved-caller fan-out below never selects (it asks for
    // BSL source by kind), and the body-only reproject never re-derives Phase F.
    // Bail to a full rebuild so a removed/unexported/renamed handler cannot leave a
    // dangling subscription edge.
    {
        let mut stmt = conn.prepare(
            "SELECT 1 FROM edges e JOIN nodes n1 ON e.to_id = n1.id \
             JOIN changed_file_keys changed ON changed.root_id = n1.file_root_id \
                 AND changed.path = n1.file_path \
             WHERE n1.kind = 'method' AND e.kind = 'event_subscription' LIMIT 1",
        )?;
        if stmt.exists([])? {
            return Ok(None);
        }
    }

    let mut stmt = conn.prepare(
        "SELECT DISTINCT n2.file_root_id, n2.file_path FROM edges e \
         JOIN nodes n1 ON e.to_id = n1.id \
         JOIN nodes n2 ON e.from_id = n2.id \
         JOIN changed_file_keys changed ON changed.root_id = n1.file_root_id \
             AND changed.path = n1.file_path \
         WHERE n1.kind = 'method' \
           AND n2.kind IN ('method','module') AND n2.file_path IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(bsl_search::FileKey::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut callers: std::collections::BTreeSet<String> = index_callers;
    for row in rows {
        let key = row?;
        let Some(path) = roots.and_then(|roots| roots.resolve_walked(&key)) else {
            return Ok(None);
        };
        callers.insert(path.to_string_lossy().into_owned());
    }
    Ok(Some(
        callers
            .into_iter()
            .filter(|f| !changed_files.contains(f.as_str()))
            .map(PathBuf::from)
            .collect(),
    ))
}

#[cfg(test)]
mod stall_report_tests {
    use super::write_stall_report;

    #[test]
    fn episodes_append_to_one_report_file() {
        let dir = tempfile::tempdir().unwrap();
        write_stall_report(dir.path(), 600, "call_edges batch 52/59", "t1:S:futex");
        write_stall_report(dir.path(), 1200, "call_edges batch 52/59", "t1:S:futex");
        let report =
            std::fs::read_to_string(dir.path().join("bsl-graph-stall-report.txt")).unwrap();
        assert!(report.contains("stalled for 600s"));
        assert!(report.contains("stalled for 1200s"));
    }

    #[test]
    fn missing_directory_is_a_warning_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        write_stall_report(&dir.path().join("gone"), 600, "index batch 1/2", "t1:S:futex");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method_node(id: &str, name: &str) -> NodeRow {
        NodeRow {
            id: id.to_string(),
            kind: "method",
            name: name.to_string(),
            qualified: format!("ОбщийМодуль.X.{name}"),
            module: Some("ОбщийМодуль.X".to_string()),
            file: Some("CommonModules/X/Ext/Module.bsl".to_string()),
            name_offset: Some(10),
            sig_end: Some(20),
            src_start: Some(0),
            src_end: Some(40),
            dispatch: vec!["server"],
            is_export: Some(true),
            addressable: true,
        }
    }

    fn edge(from: &str, to: &str) -> EdgeRow {
        EdgeRow {
            from_id: from.to_string(),
            to_id: to.to_string(),
            kind: "call",
            provenance: "resolved",
            call_start: None,
            call_end: None,
            call_site_absent: Some(ide::NO_CALL_SITE),
            crosses: false,
        }
    }

    fn open(path: &Path) -> Connection {
        Connection::open(path).unwrap()
    }

    #[test]
    fn writes_and_reads_back_nodes_and_edges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsl-graph.db");

        let mut w = GraphDbWriter::create(&path).unwrap();
        w.write_nodes(&[
            method_node("method/common/X/A", "A"),
            method_node("method/common/X/B", "B"),
        ])
        .unwrap();
        w.write_edges(&[edge("method/common/X/A", "method/common/X/B")]).unwrap();
        w.finalize(&GraphMeta {
            revision: 1,
            fingerprint: GraphFp { files: 42, topology: 7 },
            files: 1,
            built_at: "2026-06-01T00:00:00Z".to_string(),
        })
        .unwrap();

        let conn = open(&path);
        let nodes: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap();
        let edges: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
        assert_eq!((nodes, edges), (2, 1));

        // finalize derives the node/edge counts and records them in `meta`.
        let meta_nodes: String =
            conn.query_row("SELECT value FROM meta WHERE key = 'nodes'", [], |r| r.get(0)).unwrap();
        let meta_edges: String =
            conn.query_row("SELECT value FROM meta WHERE key = 'edges'", [], |r| r.get(0)).unwrap();
        assert_eq!((meta_nodes.as_str(), meta_edges.as_str()), ("2", "1"));

        let (name, dispatch, is_export, addressable): (String, String, i64, i64) = conn
            .query_row(
                "SELECT name, dispatch, is_export, addressable FROM nodes WHERE id = ?1",
                params!["method/common/X/A"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), dispatch.as_str(), is_export, addressable),
            ("A", "server", 1, 1)
        );

        let schema: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(schema, SCHEMA_VERSION.to_string());
    }

    #[test]
    fn first_node_spelling_wins_on_duplicate_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsl-graph.db");

        let mut first = method_node("method/common/X/A", "A");
        first.qualified = "first".to_string();
        let mut second = method_node("method/common/X/A", "A");
        second.qualified = "second".to_string();

        let mut w = GraphDbWriter::create(&path).unwrap();
        w.write_nodes(&[first]).unwrap();
        w.write_nodes(&[second]).unwrap();
        w.finalize(&GraphMeta {
            revision: 1,
            fingerprint: GraphFp::default(),
            files: 0,
            built_at: "t".to_string(),
        })
        .unwrap();

        let conn = open(&path);
        let qualified: String = conn
            .query_row(
                "SELECT qualified FROM nodes WHERE id = ?1",
                params!["method/common/X/A"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(qualified, "first", "INSERT OR IGNORE keeps the first-seen spelling");
    }

    #[test]
    fn write_files_round_trips_fingerprints_and_null_sig_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsl-graph.db");

        let mut w = GraphDbWriter::create(&path).unwrap();
        w.write_files(&[
            FileFingerprint {
                root_id: "".to_string(),
                path: "cfg/A.bsl".to_string(),
                content_hash: [1; 32],
                fingerprint: 111,
                sig_hash: None,
            },
            FileFingerprint {
                root_id: "".to_string(),
                path: "cfg/A.xml".to_string(),
                content_hash: [2; 32],
                fingerprint: 222,
                sig_hash: None,
            },
        ])
        .unwrap();
        w.finalize(&GraphMeta {
            revision: 1,
            fingerprint: GraphFp::default(),
            files: 0,
            built_at: "t".to_string(),
        })
        .unwrap();

        let conn = open(&path);
        let (fp, sig): (i64, Option<i64>) = conn
            .query_row(
                "SELECT fingerprint, sig_hash FROM files WHERE root_id = '' AND path = 'cfg/A.bsl'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(fp as u64, 111);
        assert_eq!(sig, None, "sig_hash is NULL until the body-only fast path fills it");

        let xml_fp: i64 = conn
            .query_row(
                "SELECT fingerprint FROM files WHERE root_id = '' AND path = 'cfg/A.xml'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(xml_fp as u64, 222);
    }

    #[test]
    fn in_degree_counts_incoming_edges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsl-graph.db");

        let mut w = GraphDbWriter::create(&path).unwrap();
        w.write_nodes(&[method_node("a", "A"), method_node("b", "B"), method_node("hub", "Hub")])
            .unwrap();
        w.write_edges(&[edge("a", "hub"), edge("b", "hub"), edge("a", "b")]).unwrap();
        w.finalize(&GraphMeta {
            revision: 1,
            fingerprint: GraphFp::default(),
            files: 0,
            built_at: "t".to_string(),
        })
        .unwrap();

        let conn = open(&path);
        let hub: i64 = conn
            .query_row("SELECT degree FROM in_degree WHERE id = 'hub'", [], |r| r.get(0))
            .unwrap();
        let b: i64 = conn
            .query_row("SELECT degree FROM in_degree WHERE id = 'b'", [], |r| r.get(0))
            .unwrap();
        assert_eq!((hub, b), (2, 1));
        // A source-only node has no in_degree row.
        let a_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM in_degree WHERE id = 'a'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(a_rows, 0);
    }

    #[test]
    fn create_truncates_a_prior_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsl-graph.db");

        let mut w = GraphDbWriter::create(&path).unwrap();
        w.write_nodes(&[method_node("stale", "Stale")]).unwrap();
        w.finalize(&GraphMeta {
            revision: 1,
            fingerprint: GraphFp::default(),
            files: 0,
            built_at: "t".to_string(),
        })
        .unwrap();

        // A second build at the same path must not see the prior row.
        let w2 = GraphDbWriter::create(&path).unwrap();
        w2.finalize(&GraphMeta {
            revision: 2,
            fingerprint: GraphFp::default(),
            files: 0,
            built_at: "t".to_string(),
        })
        .unwrap();

        let conn = open(&path);
        let nodes: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap();
        assert_eq!(nodes, 0, "create() discards the prior file");
    }

    #[test]
    fn caller_delta_bails_to_full_rebuild_for_subscription_handler() {
        // A signature change in a module that handles an event subscription must NOT take
        // the body-only caller-delta path: the subscription's `mdo -> method` edge has a
        // fileless source the delta never revisits, so it would go stale. The planner must
        // return None (force a full rebuild).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsl-graph.db");

        let handler = method_node("method/common/X/Обработчик", "Обработчик");
        let mut subscription = method_node("mdo/EventSubscription/ПриЗаписи", "ПриЗаписи");
        subscription.kind = "mdo";
        subscription.module = None;
        subscription.file = None; // config-level node: no owning file
        subscription.is_export = None;

        let mut w = GraphDbWriter::create(&path).unwrap();
        w.write_nodes(&[handler, subscription]).unwrap();
        let mut sub_edge = edge("mdo/EventSubscription/ПриЗаписи", "method/common/X/Обработчик");
        sub_edge.kind = "event_subscription";
        sub_edge.provenance = "string_resolved";
        w.write_edges(&[sub_edge]).unwrap();
        w.finalize(&GraphMeta {
            revision: 1,
            fingerprint: GraphFp { files: 1, topology: 0 },
            files: 1,
            built_at: "t".to_string(),
        })
        .unwrap();
        open(&path)
            .execute("INSERT INTO meta (key, value) VALUES ('unread_paths', '[]')", [])
            .unwrap();

        let profile = ModuleProfile {
            sig_hash: 999,
            exported_lower: std::collections::BTreeSet::new(),
            has_collision: false,
            unread: false,
        };
        let plan = caller_delta_plan(&path, &[("CommonModules/X/Ext/Module.bsl", &profile)], None)
            .unwrap();
        assert!(
            plan.is_none(),
            "a signature change to a subscription handler module must force a full rebuild"
        );
    }

    /// Everything a build needs to see one catalog and one module.
    fn stage_catalog_workspace(root: &Path, module_body: &str) {
        std::fs::create_dir_all(root.join("CommonModules/Модуль/Ext")).unwrap();
        std::fs::create_dir_all(root.join("Catalogs")).unwrap();
        std::fs::write(root.join("Configuration.xml"), "<Configuration/>").unwrap();
        std::fs::write(
            root.join("CommonModules/Модуль.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
    <CommonModule uuid="00000000-0000-0000-0000-000000000001">
        <Properties><Name>Модуль</Name><Server>true</Server></Properties>
    </CommonModule>
</MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(root.join("CommonModules/Модуль/Ext/Module.bsl"), module_body).unwrap();
    }

    fn write_catalog(root: &Path) {
        std::fs::write(
            root.join("Catalogs/Товары.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
    <Catalog uuid="00000000-0000-0000-0000-000000000002">
        <Properties><Name>Товары</Name></Properties>
    </Catalog>
</MetaDataObject>"#,
        )
        .unwrap();
    }

    fn build_meta() -> GraphMeta {
        GraphMeta {
            revision: 1,
            fingerprint: GraphFp::default(),
            files: 0,
            built_at: "t".to_string(),
        }
    }

    fn scanned_project(
        root: &Path,
    ) -> (crate::graph::ProjectSnapshot, crate::graph::universe::ScannedUniverse) {
        let project = crate::graph::ProjectSnapshot::load(root);
        let universe = crate::graph::universe::ScannedUniverse::scan(&project.scan_roots);
        (project, universe)
    }

    /// `None` covers both "no such row" and "a row with no file": neither is a
    /// placed object, and the tests below care only about that. A placed object is
    /// asserted by its durable `(root_id, path)` pair, never by a physical address.
    fn stored_file(db: &Path, id: &str) -> Option<(String, String)> {
        use rusqlite::OptionalExtension;
        Connection::open(db)
            .unwrap()
            .query_row("SELECT file_root_id, file_path FROM nodes WHERE id = ?1", [id], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .optional()
            .unwrap()
            .and_then(|(root_id, path)| root_id.zip(path))
    }

    /// The path the build is expected to store, taken from the tree rather than
    /// spelled out: a hand-built string would agree with a row written in any
    /// shape, and the shape is the whole point — the dictionary resolves this
    /// against the scanned universe.
    fn expected_file(_root: &Path, relative: &str) -> (String, String) {
        (String::new(), relative.replace('\\', "/"))
    }

    /// The full build is the path that places an object: the catalog, role and
    /// subsystem passes run there and nowhere else, and the incremental pass
    /// never overwrites a row it finds.
    #[test]
    fn a_full_build_stores_the_file_an_object_is_defined_by() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        stage_catalog_workspace(
            root,
            "&НаСервере\nПроцедура Выполнить() Экспорт\nСправочники.Товары.СоздатьЭлемент();\nКонецПроцедуры",
        );
        write_catalog(root);

        let db = root.join(".build/graph.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let (project, universe) = scanned_project(root);
        build_graph_database(&project, &universe, &db, 1, &build_meta()).unwrap();

        assert_eq!(
            stored_file(&db, "mdo/Catalog/Товары"),
            Some(expected_file(root, "Catalogs/Товары.xml")),
        );
    }

    /// The incremental path emits objects as the endpoints of the edges it
    /// reprojects. It cannot overwrite a row already stored, so the only case in
    /// which its map is observable is an object that did not exist at the last
    /// full build — which is exactly the case a relaxed eligibility gate would
    /// start delivering here.
    #[test]
    fn the_incremental_path_places_an_object_that_appeared_after_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        stage_catalog_workspace(root, "&НаСервере\nПроцедура Выполнить() Экспорт\nКонецПроцедуры");

        let db_pre = root.join(".build/pre.db");
        std::fs::create_dir_all(db_pre.parent().unwrap()).unwrap();
        let (project, universe) = scanned_project(root);
        build_graph_database(&project, &universe, &db_pre, 1, &build_meta()).unwrap();
        assert_eq!(
            stored_file(&db_pre, "mdo/Catalog/Товары"),
            None,
            "the object must be unplaced before the edit, or the incremental write is invisible",
        );

        write_catalog(root);
        let module = root.join("CommonModules/Модуль/Ext/Module.bsl");
        std::fs::write(
            &module,
            "&НаСервере\nПроцедура Выполнить() Экспорт\nСправочники.Товары.СоздатьЭлемент();\nКонецПроцедуры",
        )
        .unwrap();

        let (edited_project, edited_universe) = scanned_project(root);
        let db_incremental = root.join(".build/incremental.db");
        update_graph_database_bodies(
            &edited_project,
            &edited_universe,
            &db_pre,
            &db_incremental,
            &[module.canonicalize().unwrap()],
            1,
            &build_meta(),
        )
        .unwrap();

        assert_eq!(
            stored_file(&db_incremental, "mdo/Catalog/Товары"),
            Some(expected_file(root, "Catalogs/Товары.xml")),
        );
    }

    #[test]
    fn variable_insertion_preserves_durable_body_only_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let module_path = root.join("CommonModules/Модуль/Ext/Module.bsl");
        std::fs::create_dir_all(module_path.parent().expect("module path has a parent")).unwrap();
        std::fs::write(root.join("Configuration.xml"), "<Configuration/>").unwrap();
        std::fs::write(
            root.join("CommonModules/Модуль.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core">
    <CommonModule uuid="00000000-0000-0000-0000-000000000001">
        <Properties>
            <Name>Модуль</Name>
            <Global>false</Global>
            <ClientManagedApplication>false</ClientManagedApplication>
            <Server>true</Server>
            <ExternalConnection>false</ExternalConnection>
            <ClientOrdinaryApplication>false</ClientOrdinaryApplication>
            <ServerCall>false</ServerCall>
            <Privileged>false</Privileged>
            <ReturnValuesReuse>DontUse</ReturnValuesReuse>
        </Properties>
    </CommonModule>
</MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&module_path, "&НаСервере\nПроцедура Выполнить() Экспорт\nКонецПроцедуры")
            .unwrap();

        let meta = || GraphMeta {
            revision: 1,
            fingerprint: GraphFp::default(),
            files: 0,
            built_at: "t".to_string(),
        };
        let scanned = |root: &Path| {
            let project = crate::graph::ProjectSnapshot::load(root);
            let universe = crate::graph::universe::ScannedUniverse::scan(&project.scan_roots);
            (project, universe)
        };
        let db_pre = root.join(".build/pre.db");
        std::fs::create_dir_all(db_pre.parent().expect("database path has a parent")).unwrap();
        let (project, universe) = scanned(root);
        build_graph_database(&project, &universe, &db_pre, 1, &meta())
            .expect("initial build succeeds");

        let changed = vec![module_path.canonicalize().expect("module file exists")];
        let path_key = changed[0].to_string_lossy().into_owned();
        let stored_sig: i64 = Connection::open(&db_pre)
            .unwrap()
            .query_row("SELECT sig_hash FROM files WHERE path LIKE '%Module.bsl'", [], |row| {
                row.get(0)
            })
            .unwrap();

        // When: a top-level variable shifts method local ids but preserves its signature.
        std::fs::write(
            &module_path,
            "Перем Состояние;\n&НаСервере\nПроцедура Выполнить() Экспорт\nКонецПроцедуры",
        )
        .unwrap();
        let (edited_project, edited_universe) = scanned(root);
        let profiles = recompute_module_profiles(&edited_project, &edited_universe.files, &changed)
            .expect("profile recomputation succeeds");
        let profile = profiles.get(&path_key).expect("changed module has a profile");
        assert_eq!(
            profile.sig_hash,
            u64::from_ne_bytes(stored_sig.to_ne_bytes()),
            "the durable signature gate must retain the body-only path"
        );

        let db_incremental = root.join(".build/incremental.db");
        update_graph_database_bodies(
            &edited_project,
            &edited_universe,
            &db_pre,
            &db_incremental,
            &changed,
            1,
            &meta(),
        )
        .expect("body-only incremental update succeeds");
        let db_full = root.join(".build/full.db");
        let (project, universe) = scanned(root);
        build_graph_database(&project, &universe, &db_full, 1, &meta())
            .expect("full rebuild succeeds");

        let dump = |path: &Path| {
            let conn = Connection::open(path).unwrap();
            let mut output = Vec::new();
            for (label, query, columns) in [
                (
                    "nodes",
                    "SELECT id, kind, name, qualified, module, file_root_id, file_path, name_offset, \
                     sig_end, src_start, src_end, dispatch, is_export, addressable FROM nodes ORDER BY id",
                    14,
                ),
                (
                    "edges",
                    "SELECT from_id, to_id, kind, provenance, crosses FROM edges \
                     ORDER BY from_id, to_id, kind, provenance, crosses",
                    5,
                ),
                ("in_degree", "SELECT id, degree FROM in_degree ORDER BY id", 2),
                (
                    "unresolved_calls",
                    "SELECT target_scope, method_lower, caller_root_id, caller_path \
                     FROM unresolved_calls ORDER BY target_scope, method_lower, caller_root_id, caller_path",
                    4,
                ),
                (
                    "files",
                    "SELECT root_id, path, content_hash, fingerprint, sig_hash \
                     FROM files ORDER BY root_id, path",
                    5,
                ),
            ] {
                let mut statement = conn.prepare(query).unwrap();
                let rows = statement
                    .query_map([], |row| {
                        let mut values = Vec::with_capacity(columns);
                        for column in 0..columns {
                            values.push(
                                row.get::<_, rusqlite::types::Value>(column)
                                    .map(|value| format!("{value:?}"))?,
                            );
                        }
                        Ok(values.join("|"))
                    })
                    .unwrap();
                output.extend(rows.map(|row| format!("{label}:{}", row.unwrap())));
            }
            output
        };

        // Then: the durable body-only update exactly matches a full rebuild.
        assert_eq!(dump(&db_incremental), dump(&db_full));
    }

    #[test]
    fn constant_manager_call_persists_method_to_method_call_edge() {
        // Given: a constant manager exporting `Цель` and a common-module caller.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let manager_module = root.join("Constants/Тест/Ext/ManagerModule.bsl");
        let caller_module = root.join("CommonModules/Тест/Ext/Module.bsl");
        std::fs::create_dir_all(manager_module.parent().expect("manager module has a parent"))
            .unwrap();
        std::fs::create_dir_all(caller_module.parent().expect("caller module has a parent"))
            .unwrap();
        std::fs::write(root.join("Configuration.xml"), "<Configuration/>").unwrap();
        std::fs::write(
            root.join("Constants/Тест.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core">
    <Constant uuid="00000000-0000-0000-0000-000000000001">
        <Properties><Name>Тест</Name></Properties>
    </Constant>
</MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            root.join("CommonModules/Тест.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core">
    <CommonModule uuid="00000000-0000-0000-0000-000000000002">
        <Properties>
            <Name>Тест</Name>
            <Global>false</Global>
            <ClientManagedApplication>false</ClientManagedApplication>
            <Server>true</Server>
            <ExternalConnection>false</ExternalConnection>
            <ClientOrdinaryApplication>false</ClientOrdinaryApplication>
            <ServerCall>false</ServerCall>
            <Privileged>false</Privileged>
            <ReturnValuesReuse>DontUse</ReturnValuesReuse>
        </Properties>
    </CommonModule>
</MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&manager_module, "Процедура Цель() Экспорт\nКонецПроцедуры").unwrap();
        std::fs::write(
            &caller_module,
            "Процедура Источник() Экспорт\nКонстанты.Тест.Цель();\nКонецПроцедуры",
        )
        .unwrap();
        let path = root.join(".build/bsl-graph.db");
        std::fs::create_dir_all(path.parent().expect("graph database has a parent")).unwrap();

        // When: the workspace graph is persisted through the production builder.
        let project = crate::graph::ProjectSnapshot::load(root);
        let universe = crate::graph::universe::ScannedUniverse::scan(&project.scan_roots);
        build_graph_database(
            &project,
            &universe,
            &path,
            1,
            &GraphMeta {
                revision: 1,
                fingerprint: GraphFp::default(),
                files: 0,
                built_at: "t".to_string(),
            },
        )
        .unwrap();

        // Then: the resolved manager target is a durable method-to-method call, not only MDO access.
        let conn = open(&path);
        let call_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE from_id = ?1 AND to_id = ?2 AND kind = 'call'",
                params!["method/common/Тест/Источник", "method/manager/Constant/Тест/Цель",],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(call_count, 1, "constant-manager method call must persist as a call edge");
    }
    #[test]
    fn call_hierarchy_sqlite_method_digest() {
        // Given: persisted method calls alongside metadata and SetAction rows.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsl-graph.db");
        let mut subscription = method_node("mdo/EventSubscription/ПриЗаписи", "ПриЗаписи");
        subscription.kind = "mdo";
        subscription.module = None;
        subscription.file = None;
        subscription.is_export = None;

        let mut writer = GraphDbWriter::create(&path).unwrap();
        writer
            .write_nodes(&[
                method_node("method/common/Caller/Прямой", "Прямой"),
                method_node("method/common/Caller/Оповещение", "Оповещение"),
                method_node("method/common/Caller/Ожидание", "Ожидание"),
                method_node("method/common/Target/Цель", "Цель"),
                subscription,
            ])
            .unwrap();
        writer
            .write_edges(&[
                edge("method/common/Caller/Прямой", "method/common/Target/Цель"),
                EdgeRow {
                    from_id: "method/common/Caller/Оповещение".to_string(),
                    to_id: "method/common/Target/Цель".to_string(),
                    kind: "notify_ref",
                    provenance: "string_resolved",
                    call_start: None,
                    call_end: None,
                    call_site_absent: Some(ide::NO_CALL_SITE),
                    crosses: false,
                },
                EdgeRow {
                    from_id: "method/common/Caller/Ожидание".to_string(),
                    to_id: "method/common/Target/Цель".to_string(),
                    kind: "idle_handler",
                    provenance: "string_resolved",
                    call_start: None,
                    call_end: None,
                    call_site_absent: Some(ide::NO_CALL_SITE),
                    crosses: false,
                },
                EdgeRow {
                    from_id: "mdo/EventSubscription/ПриЗаписи".to_string(),
                    to_id: "method/common/Target/Цель".to_string(),
                    kind: "event_subscription",
                    provenance: "string_resolved",
                    call_start: None,
                    call_end: None,
                    call_site_absent: Some(ide::NO_CALL_SITE),
                    crosses: false,
                },
                EdgeRow {
                    from_id: "method/common/Caller/Прямой".to_string(),
                    to_id: "method/common/Target/Цель".to_string(),
                    kind: "set_action",
                    provenance: "string_resolved",
                    call_start: None,
                    call_end: None,
                    call_site_absent: Some(ide::NO_CALL_SITE),
                    crosses: false,
                },
            ])
            .unwrap();
        writer
            .finalize(&GraphMeta {
                revision: 1,
                fingerprint: GraphFp::default(),
                files: 0,
                built_at: "t".to_string(),
            })
            .unwrap();

        // When: the read-only SQLite oracle projects method-to-method calls.
        let digest = read_sqlite_method_call_digest(&path).unwrap();

        // Then: direct, notify, and idle handlers remain; metadata and SetAction do not.
        assert_eq!(
            digest.rows(),
            &[
                (
                    "method/common/Target/Цель".to_string(),
                    "method/common/Caller/Ожидание".to_string(),
                ),
                (
                    "method/common/Target/Цель".to_string(),
                    "method/common/Caller/Оповещение".to_string(),
                ),
                (
                    "method/common/Target/Цель".to_string(),
                    "method/common/Caller/Прямой".to_string(),
                ),
            ]
        );
        assert_eq!(digest.len(), 3);
    }

    #[test]
    fn source_root_scoped_method_digest_keeps_only_internal_pairs() {
        // Given: direct method calls within two distinct source roots and across their boundary.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bsl-graph.db");
        let root_a = dir.path().join("root-a");
        let root_b = dir.path().join("root-b");
        let path_a = root_a.join("Caller.bsl");
        let path_b = root_b.join("Caller.bsl");
        let mut a_caller = method_node("method/a/Caller", "Caller");
        a_caller.file = Some(path_a.to_string_lossy().into_owned());
        let mut a_target = method_node("method/a/Target", "Target");
        a_target.file = Some(root_a.join("Target.bsl").to_string_lossy().into_owned());
        let mut b_caller = method_node("method/b/Caller", "Caller");
        b_caller.file = Some(path_b.to_string_lossy().into_owned());
        let mut b_target = method_node("method/b/Target", "Target");
        b_target.file = Some(root_b.join("Target.bsl").to_string_lossy().into_owned());

        let mut writer = GraphDbWriter::create(&path).unwrap();
        writer.write_nodes(&[a_caller, a_target, b_caller, b_target]).unwrap();
        writer
            .write_edges(&[
                edge("method/a/Caller", "method/a/Target"),
                edge("method/a/Caller", "method/b/Target"),
                edge("method/b/Caller", "method/a/Target"),
                edge("method/b/Caller", "method/b/Target"),
            ])
            .unwrap();
        writer
            .finalize(&GraphMeta {
                revision: 1,
                fingerprint: GraphFp::default(),
                files: 0,
                built_at: "t".to_string(),
            })
            .unwrap();

        // When: source-root membership is derived from the anchor root's two module files.
        let digest = read_source_root_scoped_sqlite_method_call_digest(
            &path,
            [
                bsl_search::FileKey::new("", path_a.to_string_lossy().into_owned()),
                bsl_search::FileKey::new(
                    "",
                    root_a.join("Target.bsl").to_string_lossy().into_owned(),
                ),
                bsl_search::FileKey::new(
                    "",
                    root_a.join("Target.bsl").to_string_lossy().into_owned(),
                ),
            ],
        )
        .unwrap();

        // Then: only the pair with both method endpoints in the anchor root remains.
        assert_eq!(
            digest.rows(),
            &[("method/a/Target".to_string(), "method/a/Caller".to_string())]
        );
        assert_eq!(digest.len(), 1);
    }

    #[test]
    #[ignore = "requires BSL_GRAPH_DB and BSL_SOURCE_ROOT"]
    fn source_root_scoped_method_digest_from_environment() {
        let graph_db = std::env::var_os("BSL_GRAPH_DB").expect("BSL_GRAPH_DB is required");
        let source_root = std::env::var_os("BSL_SOURCE_ROOT").expect("BSL_SOURCE_ROOT is required");
        let graph_db = PathBuf::from(graph_db);
        let source_root = PathBuf::from(source_root);
        let project = crate::graph::ProjectSnapshot::load(&source_root);
        let files = enumerate_bsl_files(&project);

        // Given: the persisted graph and the exact BSL files in the anchor's source root.
        let digest = read_source_root_scoped_sqlite_method_call_digest(
            &graph_db,
            files.iter().filter_map(|(_, path)| {
                project.search_roots.as_ref().and_then(|roots| roots.key_of_path(path))
            }),
        )
        .unwrap();

        // When: durable target/caller rows are hashed using the parity-oracle byte contract.
        let mut hasher = blake3::Hasher::new();
        for (index, (target, caller)) in digest.rows().iter().enumerate() {
            if index > 0 {
                hasher.update(b"\n");
            }
            hasher.update(target.as_bytes());
            hasher.update(b"\t");
            hasher.update(caller.as_bytes());
        }

        // Then: the report is machine-readable and can be captured with --nocapture.
        println!(
            "{}",
            serde_json::json!({
                "source_root": source_root,
                "source_root_bsl_files": files.len(),
                "row_count": digest.len(),
                "digest": hasher.finalize().to_hex().to_string(),
            })
        );
    }
}
