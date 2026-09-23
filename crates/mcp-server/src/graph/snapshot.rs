use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Instant, UNIX_EPOCH};

use crate::change_hub::{ChangeEntry, ChangeKind};
#[cfg(windows)]
use crate::graph_query::detached_snapshot;
use crate::graph_query::GraphDb;

#[cfg(test)]
use super::state::ReloadState;
use super::state::{lock_recover, GraphState, Published};
#[cfg(test)]
use super::types::Freshness;
use super::types::GraphStatus;

/// How often the query-path freshness fold must come from a real walk instead of the
/// event-maintained map. Bounds how long a change the hub cannot observe can keep
/// freshness wrong.
pub(super) const WALK_VERIFY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// `canonical path → (mtime nanos, len)` state maintained from hub deliveries and
/// periodically re-anchored by a complete walk. `topology` carries the topology
/// hash observed at the last walk: hub deliveries patch only file stats, and a
/// config-file delivery (which may change the topology) drops the whole map so
/// the next check walks — and re-derives the project — instead of folding a
/// stale topology under fresh file stats.
#[derive(Default)]
pub(super) struct FpMapState {
    pub(super) map: Option<std::collections::BTreeMap<String, (u128, u64)>>,
    pub(super) walked_at: Option<Instant>,
    pub(super) topology: u64,
    /// Verdict of the walk that anchored `map`: hub deliveries patch stats but
    /// cannot re-judge completeness, so the last walk's verdict rides along.
    pub(super) clean: bool,
}

/// Throttled cache of the last on-disk fingerprint scan. Guarded by its own mutex
/// held across the walk, so concurrent callers serialize onto one scan per window.
pub(super) struct ScanCache {
    pub(super) at: Instant,
    /// The hub position this scan can vouch for: read BEFORE the walk, so every fact at or
    /// below it reached the hub before the disk was looked at. A comparison that answers by a
    /// number read later answers facts its own walk never saw — which is the same defect as a
    /// build publishing a proof wider than its admission.
    pub(super) observed_through: u64,
    pub(super) disk_fp: crate::graph_db::GraphFp,
    /// Whether the scan behind `disk_fp` covered the whole tree — the reload
    /// decision needs it to retire a `force_stale` build once the tree heals.
    pub(super) clean: bool,
}

/// Every publication prepares this many independent read handles before becoming ready.
pub(crate) const SNAPSHOT_POOL_CAP: usize = 4;

#[derive(Debug)]
pub(crate) enum BackgroundSnapshotError {
    Changed,
    Operation(anyhow::Error),
}

#[cfg(test)]
type SnapshotOpenHook = Box<dyn FnOnce()>;
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum BackgroundSnapshotFailure {
    Changed,
    Open,
    PrepareSecondChanged,
}
#[cfg(test)]
thread_local! {
    static SNAPSHOT_OPEN_HOOK: std::cell::RefCell<Option<SnapshotOpenHook>> =
        const { std::cell::RefCell::new(None) };
    static SNAPSHOT_CHECKOUT_HOOK: std::cell::RefCell<Option<SnapshotOpenHook>> =
        const { std::cell::RefCell::new(None) };
    static REFUSE_SNAPSHOT_INSTALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(super) fn set_snapshot_install_hook(hook: SnapshotOpenHook) {
    SNAPSHOT_OPEN_HOOK.with(|slot| slot.replace(Some(hook)));
}

#[cfg(test)]
pub(super) fn refuse_snapshot_install_for_test() {
    REFUSE_SNAPSHOT_INSTALL.with(|refuse| refuse.set(true));
}

#[cfg(test)]
fn set_snapshot_checkout_hook(hook: SnapshotOpenHook) {
    SNAPSHOT_CHECKOUT_HOOK.with(|slot| slot.replace(Some(hook)));
}

/// A pooled idle read handle plus the freshness token it was opened under.
pub(super) struct PooledSnapshotEntry {
    pub(super) generation: u64,
    pub(super) fingerprint: crate::graph_db::GraphFp,
    pub(super) force_stale: bool,
    db: GraphDb,
    // SQLite closes before the last owner removes the detached Windows file.
    #[cfg(windows)]
    _backing: Arc<tempfile::TempPath>,
}

#[derive(Default)]
pub(super) struct SnapshotPool {
    generation: u64,
    entries: Vec<PooledSnapshotEntry>,
}

impl std::ops::Deref for SnapshotPool {
    type Target = Vec<PooledSnapshotEntry>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl std::ops::DerefMut for SnapshotPool {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct WindowsFileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct WindowsFileInformation {
    attributes: u32,
    creation_time: WindowsFileTime,
    last_access_time: WindowsFileTime,
    last_write_time: WindowsFileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetFileInformationByHandle"]
    fn get_file_information_by_handle(
        file: std::os::windows::io::RawHandle,
        information: *mut WindowsFileInformation,
    ) -> i32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GraphPathIdentity {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
}

impl GraphPathIdentity {
    fn read(path: &Path) -> std::io::Result<Self> {
        #[cfg(windows)]
        let (metadata, volume_serial_number, file_index) = {
            use std::os::windows::io::AsRawHandle;

            let file = std::fs::File::open(path)?;
            let metadata = file.metadata()?;
            let mut info = WindowsFileInformation::default();
            // SAFETY: `file` owns a live handle and `info` is valid writable storage.
            if unsafe { get_file_information_by_handle(file.as_raw_handle(), &mut info) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            (
                metadata,
                info.volume_serial_number,
                (u64::from(info.file_index_high) << 32) | u64::from(info.file_index_low),
            )
        };
        #[cfg(not(windows))]
        let metadata = std::fs::metadata(path)?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            dev: metadata.dev(),
            #[cfg(unix)]
            ino: metadata.ino(),
            #[cfg(windows)]
            volume_serial_number,
            #[cfg(windows)]
            file_index,
        })
    }
}

pub(super) struct PreparedSnapshotPool {
    // The final fence must still see canonical in-place changes, even with restored mtime.
    #[cfg(windows)]
    validation: GraphDb,
    entries: Vec<PooledSnapshotEntry>,
    /// The artefact's own unread set, read STRICTLY and bound to the generation validated
    /// above. `None` when the metadata would not read: that is a failure to look, and it says
    /// nothing at all about what this publication owes.
    declared_unread: Option<Vec<String>>,
    path_identity: GraphPathIdentity,
    expected_generation: u64,
    expected_fingerprint: crate::graph_db::GraphFp,
    expected_force_stale: bool,
}

impl PreparedSnapshotPool {
    /// What the artefact this pool holds declares unread, read strictly at prepare time.
    pub(super) fn declared_unread(&self) -> Option<&[String]> {
        self.declared_unread.as_deref()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SnapshotInstallError {
    Changed,
    Operation(String),
}

#[derive(Debug)]
pub(super) enum SnapshotPrepareError {
    Changed,
    Open(anyhow::Error),
}

fn prepare_path_identity(path: &Path) -> Result<GraphPathIdentity, SnapshotPrepareError> {
    GraphPathIdentity::read(path).map_err(classify_prepare_identity_error)
}

fn classify_prepare_identity_error(error: std::io::Error) -> SnapshotPrepareError {
    if error.kind() == std::io::ErrorKind::NotFound {
        SnapshotPrepareError::Changed
    } else {
        SnapshotPrepareError::Open(error.into())
    }
}

/// A served graph handle plus the freshness token it was built at. Capturing the
/// generation/fingerprint at snapshot time (not at response time) keeps the
/// envelope's `revision`/`stale` consistent with the data actually returned, even
/// if a reload publishes a newer generation while the query runs. The handle is an
/// own read-only connection opened against the on-disk SQLite graph.
pub(crate) struct GraphSnapshot {
    pub graph: PooledGraphDb,
    pub(super) generation: u64,
    pub(super) fingerprint: crate::graph_db::GraphFp,
    pub(super) force_stale: bool,
    /// Modules this artefact was built without being able to read. Makes `stale`
    /// true — the graph is missing their nodes and edges — WITHOUT making
    /// `wants_reload` true, since rebuilding cannot read them either.
    unread_files: usize,
    /// The root table this workspace publishes, for turning a node's stored file path into
    /// a `(root_id, path)` pair. `None` on a boot that published a cached graph before the
    /// project was loaded — a real serving state, not a test-only one.
    workspace_roots: Option<bsl_search::WorkspaceRoots>,
}

impl GraphSnapshot {
    /// Modules this artefact could not read when it was built or last patched.
    pub(crate) fn unread_files(&self) -> usize {
        self.unread_files
    }

    /// The root table, when this snapshot has one.
    pub(crate) fn workspace_roots(&self) -> Option<&bsl_search::WorkspaceRoots> {
        self.workspace_roots.as_ref()
    }
}

/// A read handle checked out of (and returned to) [`GraphState::snapshot_pool`].
/// Dereferences to the underlying [`GraphDb`]; on drop the handle goes back to the
/// pool (up to [`SNAPSHOT_POOL_CAP`]) so the next query skips the multi-GB open.
pub(crate) struct PooledGraphDb {
    entry: Option<PooledSnapshotEntry>,
    pool: Arc<Mutex<SnapshotPool>>,
}

impl std::ops::Deref for PooledGraphDb {
    type Target = GraphDb;

    fn deref(&self) -> &GraphDb {
        &self.entry.as_ref().expect("pooled handle is present until drop").db
    }
}

impl Drop for PooledGraphDb {
    fn drop(&mut self) {
        if let Some(entry) = self.entry.take() {
            let mut pool = self.pool.lock().unwrap_or_else(|e| e.into_inner());
            if pool.generation == entry.generation && pool.len() < SNAPSHOT_POOL_CAP {
                pool.push(entry);
            }
        }
    }
}

/// What a publication actually did, for the obligations that were outstanding when it was
/// prepared. Not a verdict — the raw facts the three positive proofs are read off.
pub(super) enum RecoveryCoverage<'a> {
    /// A build that enumerated the scope itself: every required address is either in its
    /// universe or absent from it, and the walk says whether it may speak for the whole tree.
    Walked {
        scope: super::debt::RecoveryScope,
        /// Every address the walk listed, borrowed from the walk — the universe is not cloned
        /// to say what a handful of obligations were covered by.
        enumerated: &'a std::collections::HashSet<&'a str>,
        complete: bool,
        /// Whether the tree moved while this build ran. Such a build enumerated a world that
        /// no longer stands, so what it did not list is not thereby gone.
        straddled: bool,
    },
    /// A patch that re-projected exactly these addresses. It proves nothing about absence:
    /// a point rewrite never looked at what it did not touch.
    Patched { rewritten: &'a std::collections::HashSet<&'a str> },
    /// No fresh coverage authority at all — a cache served as it stands, or a test adapter.
    None,
}

/// The scope descriptor of one actually loaded project, carried with whatever that project
/// produced. `scan_roots`, never `search_roots`: what a walk can reach is what a walk was told
/// to walk.
pub(super) fn recovery_scope_of(
    project: &super::input::ProjectSnapshot,
) -> super::debt::RecoveryScope {
    super::debt::RecoveryScope::of(&project.scan_roots, &project.excluded, project.validated)
}

/// What a recovery probe learned.
///
/// Three outcomes, not two: a probe that could not take a snapshot at all learned NOTHING, and
/// reading that as "nothing has healed" pushes the next probe twice as far out for an
/// observation that never happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProbeOutcome {
    /// The probe looked, and these are the levels it measured. Whether any of them is NEWS is
    /// the schedule's question, not the prober's: only the schedule knows what was measured
    /// last time, and a capability that was already positive is a repeat, not a healing.
    Looked {
        levels: Vec<(super::debt::Capability, super::debt::Level)>,
        /// The scope the walk actually covered, when one was owed — from the same traversal
        /// that reached the verdict beside it.
        scope: Option<super::debt::RecoveryScope>,
    },
    /// The probe could not look: the lease was held, the handle would not open, the
    /// generation moved. The pause stays where it is and the probe is owed again.
    CouldNotLook,
}

impl GraphState {
    #[cfg(test)]
    pub(crate) fn set_background_snapshot_failure_for_test(
        &self,
        failure: Option<BackgroundSnapshotFailure>,
    ) {
        self.background_snapshot_failure.store(
            match failure {
                None => 0,
                Some(BackgroundSnapshotFailure::Changed) => 1,
                Some(BackgroundSnapshotFailure::Open) => 2,
                Some(BackgroundSnapshotFailure::PrepareSecondChanged) => 3,
            },
            std::sync::atomic::Ordering::SeqCst,
        );
    }

    /// Open and validate a complete request pool without holding the lease fence.
    pub(super) fn prepare_snapshot_pool(
        &self,
        expected_generation: u64,
        expected_fingerprint: crate::graph_db::GraphFp,
        expected_force_stale: bool,
    ) -> Result<PreparedSnapshotPool, SnapshotPrepareError> {
        let path = self
            .graph_db_path()
            .ok_or_else(|| SnapshotPrepareError::Open(anyhow::anyhow!("graph path unavailable")))?;
        let before = prepare_path_identity(&path)?;
        #[cfg(windows)]
        let validation = GraphDb::open(&path).map_err(SnapshotPrepareError::Open)?;
        #[cfg(windows)]
        let backing = detached_snapshot(&path).map_err(SnapshotPrepareError::Open)?;
        #[cfg(windows)]
        let read_path: &Path = backing.as_ref();
        #[cfg(not(windows))]
        let read_path: &Path = &path;
        let mut entries = Vec::with_capacity(SNAPSHOT_POOL_CAP);
        for _index in 0..SNAPSHOT_POOL_CAP {
            #[cfg(test)]
            if _index == 1
                && self.background_snapshot_failure.load(std::sync::atomic::Ordering::SeqCst) == 3
            {
                return Err(SnapshotPrepareError::Changed);
            }
            let db = GraphDb::open(read_path).map_err(SnapshotPrepareError::Open)?;
            let (generation, fingerprint, force_stale) =
                db.freshness_token().map_err(SnapshotPrepareError::Open)?;
            if generation != expected_generation
                || fingerprint != expected_fingerprint
                || force_stale != expected_force_stale
            {
                return Err(SnapshotPrepareError::Changed);
            }
            entries.push(PooledSnapshotEntry {
                generation,
                fingerprint,
                force_stale,
                db,
                #[cfg(windows)]
                _backing: Arc::clone(&backing),
            });
        }
        let after = prepare_path_identity(&path)?;
        if before != after {
            return Err(SnapshotPrepareError::Changed);
        }
        // Read here, where the generation has just been checked against the expectation and
        // the path identity brackets the read: a list taken later could belong to another
        // artefact at the same name.
        let declared_unread = entries.first().and_then(|entry| entry.db.unread_paths_strict().ok());
        Ok(PreparedSnapshotPool {
            #[cfg(windows)]
            validation,
            entries,
            declared_unread,
            path_identity: after,
            expected_generation,
            expected_fingerprint,
            expected_force_stale,
        })
    }

    /// Revalidate the prepared path under a short ownership fence, then install the
    /// descriptors and readiness metadata while request snapshots are excluded.
    ///
    /// `forced_through` is the fact the ticket carried — the demand this publication is
    /// entitled to discharge — and `None` when it ran as an ordinary catch-up. It comes from
    /// the ticket fixed at the admission, never from re-reading the debts here: a builder that
    /// asks what is owed NOW answers for demands it was never admitted for.
    ///
    /// Two different numbers travel with a publication and they are not interchangeable. The
    /// SPONSOR CUTOFF (`forced_through`) says which demand paid for this build; the
    /// OBSERVATION (`Published::observed_through`) says how far its scan can vouch, and marks
    /// are consumed against that. A build admitted for one fact may observe no further, and it
    /// must not claim to have answered anything above either line.
    ///
    /// The debts are discharged inside the same critical section that installs the snapshot,
    /// so an observer holding `inner` — such as [`GraphState::try_claim_reload`] — sees the
    /// new publication and the discharged demand as ONE state. Discharging after the section
    /// leaves a window in which the graph reads "reloaded, and still owing a reload", and a
    /// claim landing there starts a second full rebuild of what was just published.
    ///
    /// Lock order, which this call sits inside and never inverts:
    /// publication gate → lease → `inner` → debt.
    ///
    /// Only a successful install discharges: a refused lease, a `Changed` revalidation
    /// and a failed build all leave the obligation outstanding, so the forced reload is
    /// retried rather than silently dropped.
    pub(super) fn install_prepared_snapshot(
        &self,
        mut prepared: PreparedSnapshotPool,
        published: Published,
        status: GraphStatus,
        forced_through: Option<u64>,
        recovery_through: Option<u64>,
        recovery: super::debt::RecoveryPublicationProof,
    ) -> crate::workspace_lease::LeaseOperationOutcome<(), SnapshotInstallError> {
        #[cfg(test)]
        SNAPSHOT_OPEN_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        // The install is the other half of the critical section `consume_observed_marks` and
        // `notify_published_pass` hold: they charge marks against whatever is published when
        // their hook runs, so a publication swapped in midway would take marks that were
        // cleared against another — and an unsound one would take marks no publication may
        // consume at all. Taken BEFORE the fence, never inside it: a hook running under the
        // gate publishes through the lease, so gate → lease is the only order both sides can
        // agree on.
        let _gate = crate::graph::state::lock_recover(&self.publication_gate);
        // What the ledger hands back to be thrown away. Filled inside the section and dropped
        // below it: freeing the answered obligations and the consumed proof is O(P) of
        // allocator work, and the gate, the lease and `inner` are all held in there.
        let retired = std::cell::RefCell::new(super::debt::RetiredPayload::default());
        let discard = &retired;
        let outcome = self.lease.publish_short(&mut prepared, move |prepared| {
            #[cfg(test)]
            if REFUSE_SNAPSHOT_INSTALL.with(|refuse| refuse.replace(false))
                || self
                    .refused_installs
                    .fetch_update(
                        std::sync::atomic::Ordering::SeqCst,
                        std::sync::atomic::Ordering::SeqCst,
                        |left| left.checked_sub(1),
                    )
                    .is_ok()
            {
                return Err(SnapshotInstallError::Changed);
            }
            let expected = (
                prepared.expected_generation,
                prepared.expected_fingerprint,
                prepared.expected_force_stale,
            );
            let path = self.graph_db_path().ok_or_else(|| {
                SnapshotInstallError::Operation("graph path unavailable".to_owned())
            })?;
            match GraphPathIdentity::read(&path) {
                Ok(identity) if identity == prepared.path_identity => {}
                Ok(_) => return Err(SnapshotInstallError::Changed),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(SnapshotInstallError::Changed)
                }
                Err(error) => return Err(SnapshotInstallError::Operation(error.to_string())),
            }
            #[cfg(windows)]
            let validation = &prepared.validation;
            #[cfg(not(windows))]
            let validation = &prepared
                .entries
                .first()
                .ok_or_else(|| SnapshotInstallError::Operation("empty snapshot pool".to_owned()))?
                .db;
            let actual = validation
                .freshness_token()
                .map_err(|error| SnapshotInstallError::Operation(error.to_string()))?;
            if actual != expected
                || (published.generation, published.fingerprint, published.force_stale) != expected
            {
                return Err(SnapshotInstallError::Changed);
            }
            let observed_through = published.observed_through;
            // What only a probe can heal: a scan that could not vouch for itself, and files
            // whose bytes could not be read. Nothing on the fact stream announces either.
            //
            // Staleness by itself is neither. A knowingly stale publication is a cache served
            // on purpose while the build that replaces it is already claimed, and its
            // fingerprint differs from disk — the ordinary comparison owes that rebuild and
            // will make it, so being behind disk asks for no probe of its own.
            //
            // What such a publication DECLARES UNREAD is a different thing and keeps every
            // obligation it always had: those are the paths its own metadata names, opened
            // here as for any other artefact, and a probe answers them one path at a time.
            // Not a walk of the whole tree — an episode over strict unread carries no scan
            // obligation, and reading it as one would refuse recovery to exactly the
            // publication that needs it.
            // Whether this artefact can vouch for itself travels WITH the proof, prepared
            // outside every lock from the strict reader. What was here instead — a fresh
            // lenient count of unread rows, read under the publication gate — answered
            // "nothing left unread" for a database that would not answer at all.
            let mut recovery = recovery;
            recovery.straddled |= published.force_stale;
            let mut inner = lock_recover(&self.inner);
            let mut pool = lock_recover(&self.snapshot_pool);
            pool.generation = published.generation;
            pool.entries = std::mem::take(&mut prepared.entries);
            inner.published = Some(published);
            inner.status = status;
            // Inside the publishing critical section, under `inner`: a reader holding it never
            // sees a publication whose debts still name what it has just discharged.
            *discard.borrow_mut() = lock_recover(&self.debt).record_publication(
                std::time::Instant::now(),
                observed_through,
                forced_through.is_some(),
                recovery_through,
                recovery,
            );
            #[cfg(test)]
            if let Some(hook) = &self.install_section_hook {
                // Still inside: the gate, the lease and `inner` are all held here.
                hook("under-locks");
            }
            Ok(())
        });
        drop(_gate);
        #[cfg(test)]
        if let Some(hook) = &self.install_section_hook {
            // Outside every lock, and before the payload goes: what the next line frees is
            // what the section would otherwise have freed with the gate held.
            hook("gate-released");
        }
        drop(retired);
        outcome
    }

    /// Snapshot the graph for a blocking query, if built. The returned
    /// [`GraphSnapshot`] owns a read-only SQLite handle and its freshness token,
    /// and can be moved onto a blocking task without holding the lock during the
    /// query.
    pub(crate) fn snapshot(&self) -> Option<GraphSnapshot> {
        let inner = lock_recover(&self.inner);
        let published = inner.published.as_ref()?;
        // Held through pool checkout in the same inner → pool order as publication. A reader
        // tagged with the old generation can therefore never drain a newly installed pool.
        let published_generation = published.generation;
        let workspace_roots = published.search_roots.clone();
        #[cfg(test)]
        SNAPSHOT_CHECKOUT_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        let mut pool = lock_recover(&self.snapshot_pool);
        while let Some(entry) = pool.pop() {
            if entry.generation == published_generation {
                let (generation, fingerprint, force_stale) =
                    (entry.generation, entry.fingerprint, entry.force_stale);
                let unread_files = entry.db.unread_files();
                return Some(GraphSnapshot {
                    graph: PooledGraphDb {
                        entry: Some(entry),
                        pool: Arc::clone(&self.snapshot_pool),
                    },
                    generation,
                    fingerprint,
                    force_stale,
                    unread_files,
                    workspace_roots,
                });
            }
        }
        None
    }

    /// Acquire a snapshot for the one background consumer that may wait for lease I/O.
    /// Requests use [`Self::snapshot`] and never reach this fallback.
    pub(crate) fn snapshot_blocking(
        &self,
    ) -> crate::workspace_lease::LeaseOperationOutcome<Option<GraphSnapshot>, BackgroundSnapshotError>
    {
        use crate::workspace_lease::{LeaseOperationError, LeaseOperationOutcome};

        if let Some(snapshot) = self.snapshot() {
            return LeaseOperationOutcome::Applied(Some(snapshot));
        }
        let (generation, fingerprint, force_stale, workspace_roots) = {
            let inner = lock_recover(&self.inner);
            let Some(published) = inner.published.as_ref() else {
                return LeaseOperationOutcome::Applied(None);
            };
            (
                published.generation,
                published.fingerprint,
                published.force_stale,
                published.search_roots.clone(),
            )
        };
        // Asked BEFORE the open, not after it. Opening the graph database is seconds of I/O on
        // a large configuration, and a generation that has lost the workspace has no business
        // paying for it: the fence below would refuse the result anyway, but only once the
        // cost was already sunk.
        // Each outcome keeps its own name: a released lease is not a superseded one, and the
        // caller's handling differs.
        if self.lease.is_superseded() {
            return LeaseOperationOutcome::Superseded;
        }
        if self.lease.is_released() {
            return LeaseOperationOutcome::Released;
        }
        let opened = (|| -> anyhow::Result<(PooledSnapshotEntry, GraphPathIdentity)> {
            #[cfg(test)]
            if self.background_snapshot_failure.load(std::sync::atomic::Ordering::SeqCst) == 2 {
                anyhow::bail!("forced background snapshot open failure");
            }
            let path =
                self.graph_db_path().ok_or_else(|| anyhow::anyhow!("graph path unavailable"))?;
            let before = GraphPathIdentity::read(&path)?;
            #[cfg(windows)]
            let backing = detached_snapshot(&path)?;
            #[cfg(windows)]
            let read_path: &Path = backing.as_ref();
            #[cfg(not(windows))]
            let read_path: &Path = &path;
            let db = GraphDb::open(read_path)?;
            if db.freshness_token()? != (generation, fingerprint, force_stale) {
                anyhow::bail!("graph changed while opening a background snapshot");
            }
            let after = GraphPathIdentity::read(&path)?;
            if before != after {
                anyhow::bail!("graph path changed while opening a background snapshot");
            }
            Ok((
                PooledSnapshotEntry {
                    generation,
                    fingerprint,
                    force_stale,
                    db,
                    #[cfg(windows)]
                    _backing: backing,
                },
                after,
            ))
        })();
        let (entry, identity) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                return LeaseOperationOutcome::OperationError(LeaseOperationError::Operation(
                    BackgroundSnapshotError::Operation(error),
                ))
            }
        };

        #[cfg(test)]
        SNAPSHOT_OPEN_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        let mut prepared = Some((entry, identity));
        match self.lease.publish_short(&mut prepared, |prepared| {
            let (_, identity) = prepared
                .as_ref()
                .expect("background snapshot publication keeps its prepared value until commit");
            #[cfg(test)]
            if self.background_snapshot_failure.load(std::sync::atomic::Ordering::SeqCst) == 1 {
                return Err(SnapshotInstallError::Changed);
            }
            let path = self.graph_db_path().ok_or_else(|| {
                SnapshotInstallError::Operation("graph path unavailable".to_owned())
            })?;
            match GraphPathIdentity::read(&path) {
                Ok(current) if current == *identity => {}
                Ok(_) => return Err(SnapshotInstallError::Changed),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(SnapshotInstallError::Changed)
                }
                Err(error) => return Err(SnapshotInstallError::Operation(error.to_string())),
            }
            let inner = lock_recover(&self.inner);
            let published = inner.published.as_ref().ok_or(SnapshotInstallError::Changed)?;
            if (published.generation, published.fingerprint, published.force_stale)
                != (generation, fingerprint, force_stale)
            {
                return Err(SnapshotInstallError::Changed);
            }
            Ok(())
        }) {
            LeaseOperationOutcome::Applied(()) => {
                let (entry, _) = prepared.take().expect("successful publication retains entry");
                let unread_files = entry.db.unread_files();
                LeaseOperationOutcome::Applied(Some(GraphSnapshot {
                    graph: PooledGraphDb {
                        entry: Some(entry),
                        pool: Arc::clone(&self.snapshot_pool),
                    },
                    generation,
                    fingerprint,
                    force_stale,
                    unread_files,
                    workspace_roots,
                }))
            }
            LeaseOperationOutcome::OperationError(LeaseOperationError::Operation(
                SnapshotInstallError::Changed,
            )) => LeaseOperationOutcome::OperationError(LeaseOperationError::Operation(
                BackgroundSnapshotError::Changed,
            )),
            LeaseOperationOutcome::OperationError(LeaseOperationError::Operation(
                SnapshotInstallError::Operation(message),
            )) => LeaseOperationOutcome::OperationError(LeaseOperationError::Operation(
                BackgroundSnapshotError::Operation(anyhow::anyhow!(message)),
            )),
            LeaseOperationOutcome::OperationError(LeaseOperationError::Lease(error)) => {
                LeaseOperationOutcome::OperationError(LeaseOperationError::Lease(error))
            }
            LeaseOperationOutcome::TransientRefusal => LeaseOperationOutcome::TransientRefusal,
            LeaseOperationOutcome::Superseded => LeaseOperationOutcome::Superseded,
            LeaseOperationOutcome::Released => LeaseOperationOutcome::Released,
        }
    }

    /// Test-only legacy freshness path. Production request handlers use
    /// `cached_freshness` and never walk disk or start a reload.
    #[cfg(test)]
    pub(crate) fn freshness(&self, snapshot: &GraphSnapshot) -> Freshness {
        let disk = self.current_disk_fp();
        let stale = snapshot.force_stale
            || snapshot.unread_files > 0
            || disk.map(|(fp, _)| fp != snapshot.fingerprint).unwrap_or(false);
        let may_build = self.may_build();

        let mut inner = lock_recover(&self.inner);
        let Some(published) = inner.published.as_mut() else {
            return Freshness {
                revision: snapshot.generation,
                stale,
                reload: "none",
                topology: snapshot.fingerprint.topology,
                drift_watch: self.drift_watch(),
            };
        };
        let mut reload = published.reload.label();
        let claim_reload =
            published.wants_reload(disk) && published.reload != ReloadState::Running && may_build;
        if claim_reload {
            published.reload = ReloadState::Running;
            reload = "running";
        }
        drop(inner);

        if claim_reload {
            let state = self.clone();
            let spawned = std::thread::Builder::new()
                .name("bsl-graph-reload".to_owned())
                .spawn(move || state.run_load(true));
            if let Err(e) = spawned {
                let mut inner = lock_recover(&self.inner);
                if let Some(p) = inner.published.as_mut() {
                    p.reload = ReloadState::Failed(format!("could not spawn reload: {e}"));
                }
                reload = "failed";
            }
        }

        Freshness {
            revision: snapshot.generation,
            stale,
            reload,
            topology: snapshot.fingerprint.topology,
            drift_watch: self.drift_watch(),
        }
    }

    /// Look at what the published build could not read: is the tree whole again, and can the
    /// modules it recorded as unread be opened now?
    ///
    /// The only owner a publication that cannot vouch for itself can have. A restored
    /// permission is not a file change, so nothing on the fact stream will ever announce it;
    /// without this the graph stays `stale` for the life of the daemon. Runs on the watcher's
    /// thread, never on a request — a request may only ask for it sooner.
    pub(super) fn recovery_probe(&self, plan: &super::debt::ProbePlan) -> ProbeOutcome {
        // Ask what we can SEE before paying for the walk. The walk drops the cached
        // fingerprint and the event-maintained map and then stats the whole tree; doing it
        // first meant a probe that could not take a snapshot at all — a held lease, a database
        // that will not open — paid for the walk every time before discovering it had nothing
        // to compare against, and threw the caches away for nothing besides.
        use crate::workspace_lease::LeaseOperationOutcome;
        match self.snapshot_blocking() {
            // And it must be a snapshot of the publication these obligations are the gaps OF.
            // Acquired without asking, a walk could measure the tree against one artefact
            // while reporting about another's gaps: the receipt is refused at completion for
            // the same reason, and paying for the whole pass first is the part that is
            // avoidable.
            LeaseOperationOutcome::Applied(Some(snapshot))
                if snapshot.generation != plan.basis.generation() =>
            {
                return ProbeOutcome::CouldNotLook
            }
            LeaseOperationOutcome::Applied(_) => {}
            // A failure to LOOK, not a look that found nothing.
            _ => return ProbeOutcome::CouldNotLook,
        }

        // The BACKGROUND acquire, not the request pool's — and P, not the newest unread list.
        // An obligation the latest enumeration did not mention is still owed an observation:
        // a short scan that stopped naming it answered nothing about it, and dropping it from
        // the walk would leave it with memory and no observer. One level per path, BY path:
        // an aggregate is true for the rest of the episode as soon as any one path opens, and
        // the next path to heal is then invisible. A path that is GONE is told apart from one
        // that will not open — a removal is a measured change of composition, not an open to
        // retry for ever.
        let mut levels: Vec<(super::debt::Capability, super::debt::Level)> = plan
            .open
            .iter()
            .map(|path| {
                let level = match std::fs::File::open(path) {
                    Ok(_) => super::debt::Level::Granted,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        super::debt::Level::Absent
                    }
                    Err(_) => super::debt::Level::Denied,
                };
                (super::debt::Capability::Open(path.clone()), level)
            })
            .collect();

        // The walk is a capability only while one is owed. Scope and verdict come back from
        // the SAME traversal: read as three separate questions of three mutable caches, a
        // verdict could be paired with the roots of a walk that never produced it.
        let walked = plan.scan.as_ref().and_then(|_| {
            *lock_recover(&self.scan) = None;
            {
                let mut fp_state = lock_recover(&self.fp_map);
                fp_state.map = None;
                fp_state.walked_at = None;
            }
            self.walk_scan_receipt()
        });
        if let Some((scope, complete)) = &walked {
            levels.push((
                super::debt::Capability::ScanRoots,
                if *complete { super::debt::Level::Granted } else { super::debt::Level::Denied },
            ));
            let _ = scope;
        }
        ProbeOutcome::Looked { levels, scope: walked.map(|(scope, _)| scope) }
    }

    /// Pair what this publication did with what is outstanding. Called BEFORE the publication
    /// gate and outside every lock: reading what is outstanding and matching it against what
    /// this build covered is O(P + U) of comparisons, and the critical section is for the
    /// swap, not for that.
    ///
    /// Only three things retire an obligation, and each is something the installed result
    /// actually shows: it read the address, a complete identity-exact walk of the scope that
    /// required it did not find it, or a validated declaration no longer asks for it. A
    /// shorter unread list is none of those — an enumeration that came up short has answered
    /// nothing.
    pub(super) fn recovery_proof(
        &self,
        generation: u64,
        declared_unread: Option<&[String]>,
        coverage: RecoveryCoverage<'_>,
    ) -> super::debt::RecoveryPublicationProof {
        let outstanding = lock_recover(&self.debt).outstanding_recovery();
        let mut proof = super::debt::RecoveryPublicationProof {
            generation,
            captured_seq: outstanding.captured_seq,
            declared_unread: declared_unread.map(<[String]>::to_vec),
            ..Default::default()
        };
        // Without a strict unread list this publication cannot say what it managed to read:
        // the lenient reader turns a broken database into "everything was read", and that
        // would retire every outstanding address on the strength of an error.
        let still_unread: Option<std::collections::HashSet<&str>> =
            declared_unread.map(|unread| unread.iter().map(String::as_str).collect());
        match coverage {
            RecoveryCoverage::Walked { scope, enumerated, complete, straddled } => {
                for (key, occurrence) in outstanding.keys {
                    // Whatever else is true of this address, an artefact that names it unread
                    // requires it. Both halves come from the same installed database, and the
                    // unread list is the half with authority.
                    let Some(unread) = still_unread.as_ref() else { continue };
                    if unread.contains(key.as_str()) {
                        continue;
                    }
                    let listed = enumerated.contains(key.as_str());
                    if listed {
                        proof.read_covered.push((key, occurrence));
                    } else if scope.speaks_for_removal() && !scope.requires(&key) {
                        // A validated declaration whose roots all resolved no longer asks for
                        // this address. Not a choice to forget it — a measured change of what
                        // is required. A walk that LISTED the address says the opposite,
                        // whatever spelling it arrives under, so it is answered above.
                        proof.out_of_scope_covered.push((key, occurrence));
                    } else if complete
                        && !straddled
                        && scope.speaks_for_removal()
                        && scope.requires(&key)
                    {
                        // The walk could speak for the whole of the scope that required it,
                        // and did not list it: it is gone, not merely unlisted. A restricted
                        // fallback may not say that — it is what the loader does when it
                        // cannot read the project, and a path it never looked for is not a
                        // path that went away.
                        proof.absent_covered.push((key, occurrence));
                    }
                }
                proof.scan_complete = Some(complete);
                proof.straddled = straddled;
                proof.scope = Some(scope);
            }
            RecoveryCoverage::Patched { rewritten } => {
                for (key, occurrence) in outstanding.keys {
                    let read = rewritten.contains(key.as_str())
                        && still_unread
                            .as_ref()
                            .is_some_and(|unread| !unread.contains(key.as_str()));
                    if read {
                        proof.read_covered.push((key, occurrence));
                    }
                }
            }
            RecoveryCoverage::None => {}
        }
        proof
    }

    /// One traversal, one receipt: the scope actually covered and whether the walk behind it
    /// may speak for the whole tree.
    ///
    /// The two belong together. Asked separately — the verdict from one cache, the roots from
    /// another — a verdict can be paired with a composition that never produced it, and the
    /// pairing is exactly what says an obligation has been answered.
    pub(super) fn walk_scan_receipt(&self) -> Option<(super::debt::RecoveryScope, bool)> {
        let root = self.workspace_root.as_deref()?;
        // Counted where the tree is actually walked. A walk is the expensive half of a
        // recovery pass, and a cost nothing counts is a cost nobody can measure.
        self.scan_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let project = super::input::ProjectSnapshot::load_excluding(root, &self.cache_exclusions());
        let universe = super::universe::ScannedUniverse::scan_excluding(
            &project.scan_roots,
            &project.excluded,
        );
        #[cfg(test)]
        if let Some(hook) = self.scan_receipt_hook.clone() {
            // AFTER the walk and before the receipt is made: the window another owner's
            // replacement lands in, and the one a receipt assembled from two reads would
            // straddle without saying so.
            hook(self);
        }
        Some((recovery_scope_of(&project), universe.clean()))
    }

    /// Give back the cursor the comparison keeps for its own scan cache.
    ///
    /// It is subscribed lazily, by the first comparison, and it is the graph's — not the
    /// watcher's. When the observation ends nothing compares again for this generation, and a
    /// cursor nobody drains makes the hub hold entries for a consumer that has left.
    pub(super) fn release_scan_cursor(&self) {
        let Some(hub) = &self.change_hub else { return };
        let cursor = lock_recover(&self.hub_cursor).close();
        if let Some(cursor) = cursor {
            hub.unsubscribe(cursor);
        }
    }

    pub(super) fn current_disk_fp(&self) -> Option<(crate::graph_db::GraphFp, bool)> {
        let root = self.workspace_root.as_deref()?;
        self.invalidate_scan_on_hub_drift();
        let mut cache = lock_recover(&self.scan);
        if let Some(c) = cache.as_ref() {
            if c.at.elapsed() < self.drift_interval {
                return Some((c.disk_fp, c.clean));
            }
        }
        // Asked about OUR cursor, not about the hub at large: `invalidate_scan_on_hub_drift`
        // above has just drained it, so any debt left here is the hub's own incompleteness
        // — while a shared verdict would also carry the debt of a consumer that simply
        // stopped draining, and put this one on a full walk for as long as that lasted.
        let hub_healthy = matches!(
            &self.change_hub,
            Some(hub) if matches!(
                hub.health_for(lock_recover(&self.hub_cursor).peek()),
                crate::change_hub::Health::Healthy
            )
        );
        // Before any look at disk, cached or fresh.
        let observed_through = self.observation();
        if !hub_healthy {
            let mut fp_state = lock_recover(&self.fp_map);
            fp_state.map = None;
            fp_state.walked_at = None;
        }
        if hub_healthy {
            let fp_state = lock_recover(&self.fp_map);
            if let (Some(map), Some(walked_at)) = (fp_state.map.as_ref(), fp_state.walked_at) {
                if walked_at.elapsed() < WALK_VERIFY_INTERVAL {
                    let entries: Vec<(String, u128, u64)> =
                        map.iter().map(|(p, (m, l))| (p.clone(), *m, *l)).collect();
                    let fp = crate::graph_db::GraphFp {
                        files: fold_fingerprint_entries(&entries),
                        topology: fp_state.topology,
                    };
                    let clean = fp_state.clean;
                    *cache = Some(ScanCache {
                        at: Instant::now(),
                        disk_fp: fp,
                        clean,
                        observed_through,
                    });
                    return Some((fp, clean));
                }
            }
        }
        self.scan_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // ONE project load serves both components: the roots the stats walk and
        // the topology hash come from the same snapshot, so the fold can never
        // pair one project state's files with another's topology.
        let project = super::input::ProjectSnapshot::load_excluding(root, &self.cache_exclusions());
        let universe = super::universe::ScannedUniverse::scan_excluding(
            &project.scan_roots,
            &project.excluded,
        );
        let clean = universe.clean();
        #[cfg(test)]
        if let Some(hook) = self.scan_window_hook.clone() {
            // The tree has been enumerated and the position this look may vouch for was read
            // before it: a change delivered here belongs to neither.
            hook(self);
        }
        let mut entries: Vec<(String, u128, u64)> =
            universe.stats.into_iter().map(|s| (s.path, s.mtime, s.len)).collect();
        entries.sort();
        let topology = super::scan::topology_u64(&project.configs);
        let fp = crate::graph_db::GraphFp { files: fold_fingerprint_entries(&entries), topology };
        {
            let mut fp_state = lock_recover(&self.fp_map);
            fp_state.map = Some(entries.into_iter().map(|(p, m, l)| (p, (m, l))).collect());
            fp_state.walked_at = Some(Instant::now());
            fp_state.topology = topology;
            fp_state.clean = clean;
        }
        *cache = Some(ScanCache { at: Instant::now(), disk_fp: fp, clean, observed_through });
        Some((fp, clean))
    }

    /// Forget every cached look at disk, so the next one walks the tree again.
    ///
    /// A statement about what is ON disk cannot be read through the throttle: the cached
    /// answer is kept for pacing, and whether it still describes the tree depends on a
    /// watcher having delivered the edit — a race, not an oracle.
    #[cfg(test)]
    pub(super) fn forget_the_disk_look(&self) {
        *lock_recover(&self.scan) = None;
        let mut fp_state = lock_recover(&self.fp_map);
        fp_state.map = None;
        fp_state.walked_at = None;
    }

    /// The hub position the comparison's current fingerprint can vouch for.
    ///
    /// A comparison answers what its own walk saw and no more. Answering by a number read at
    /// the moment of the verdict retires facts delivered after the walk — including one the
    /// walk could not possibly have covered, whose only record is that debt.
    pub(super) fn scan_watermark(&self) -> u64 {
        lock_recover(&self.scan)
            .as_ref()
            .map_or_else(|| self.observation(), |cache| cache.observed_through)
    }

    fn invalidate_scan_on_hub_drift(&self) {
        let Some(hub) = &self.change_hub else {
            return;
        };
        // Taken with the epoch that names this slot's current life. The drain below runs with
        // the lock released; the epoch is what tells a write-back that the cursor it is
        // holding is still the slot's, and not one a release has already ended.
        let Some((cursor, epoch)) = lock_recover(&self.hub_cursor).open(hub) else {
            // Closed: the observation is over and nothing subscribes again.
            return;
        };
        let batch = hub.drain(cursor);
        if !lock_recover(&self.hub_cursor).advance(epoch, batch.cursor) {
            // The slot moved on or closed while this drain ran. Whoever ends a cursor's life
            // unsubscribes it, and that is this drain: storing it would resurrect an id the
            // hub has already forgotten.
            hub.unsubscribe(batch.cursor);
            return;
        }
        if batch.rescan_required {
            *lock_recover(&self.scan) = None;
            let mut fp_state = lock_recover(&self.fp_map);
            fp_state.map = None;
            fp_state.walked_at = None;
            return;
        }
        let relevant: Vec<&ChangeEntry> =
            batch.entries.iter().filter(|e| entry_touches_scan_universe(e)).collect();
        if relevant.is_empty() {
            return;
        }
        *lock_recover(&self.scan) = None;
        let mut fp_state = lock_recover(&self.fp_map);
        // A subtree removal invalidates paths the entry list cannot enumerate; a
        // config-file change may alter the topology AND the scan-root universe.
        // Either way the patched map would lie — drop it so the next check walks
        // (and re-derives the project).
        if relevant.iter().any(|e| e.kind == ChangeKind::SubtreeRemoved || entry_is_config_file(e))
        {
            fp_state.map = None;
            fp_state.walked_at = None;
            return;
        }
        let Some(map) = fp_state.map.as_mut() else {
            return;
        };
        for entry in relevant {
            let key = entry.canonical.to_string_lossy().into_owned();
            match stat_pair(&entry.canonical) {
                Some(pair) => {
                    map.insert(key, pair);
                }
                None => {
                    map.remove(&key);
                }
            }
        }
    }
}

pub(super) fn entry_touches_scan_universe(entry: &ChangeEntry) -> bool {
    if entry.kind == ChangeKind::SubtreeRemoved {
        return true;
    }
    let is_scan_ext = |path: &Path| {
        bsl_conventions::has_extension(path, bsl_conventions::BSL_EXTENSION)
            || bsl_conventions::has_extension(path, bsl_conventions::XML_EXTENSION)
    };
    is_scan_ext(&entry.canonical) || is_scan_ext(&entry.raw) || entry_is_config_file(entry)
}

/// Whether a delivered change is one of the analyzer config files — an edit there
/// can change the extension topology (and with it the scan-root universe) without
/// touching a single `.bsl`/`.xml`.
pub(super) fn entry_is_config_file(entry: &ChangeEntry) -> bool {
    let is_config = |path: &Path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(project_model::is_project_input_file_name)
    };
    is_config(&entry.canonical) || is_config(&entry.raw)
}

pub(super) fn fold_fingerprint_entries(entries: &[(String, u128, u64)]) -> u64 {
    let mut hasher = DefaultHasher::new();
    entries.hash(&mut hasher);
    hasher.finish()
}

fn stat_pair(path: &Path) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Some((mtime, meta.len()))
}

#[cfg(test)]
mod tests {

    /// The three interleavings, run concurrently with real barriers rather than in sequence.
    ///
    /// The drain is what makes them races: it runs with the slot lock RELEASED, so a release
    /// can land inside it. A — a copy written back after that release. B — a comparison
    /// starting after it and subscribing a cursor nobody would unsubscribe. C — two drains
    /// overlapping the release.
    #[test]
    fn a_closing_cursor_races_its_own_drain_and_a_second_comparison() {
        use crate::graph::state::lock_recover;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Barrier};

        for round in 0..8 {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            crate::graph::test_support::sample_workspace(root);
            let hub = crate::change_hub::WorkspaceChangeHub::start(vec![root.to_path_buf()]);
            assert!(hub.wait_until_watching(std::time::Duration::from_secs(5)));
            let before = hub.active_cursor_count();
            let graph = Arc::new(
                crate::graph::state::GraphState::for_workspace(root.to_path_buf())
                    .with_change_hub(hub.clone()),
            );
            graph.invalidate_scan_on_hub_drift();

            // A: a drain already holding a copy when the release lands.
            let opened = lock_recover(&graph.hub_cursor).open(&hub).expect("an open slot");
            let gate = Arc::new(Barrier::new(3));
            let refused = Arc::new(AtomicUsize::new(0));

            let drainer = {
                let (graph, hub, gate, refused) =
                    (Arc::clone(&graph), hub.clone(), Arc::clone(&gate), Arc::clone(&refused));
                std::thread::spawn(move || {
                    let (cursor, epoch) = opened;
                    gate.wait();
                    let batch = hub.drain(cursor);
                    if !lock_recover(&graph.hub_cursor).advance(epoch, batch.cursor) {
                        // Whoever ends a cursor's life unsubscribes it, and that is this drain.
                        hub.unsubscribe(batch.cursor);
                        refused.fetch_add(1, Ordering::SeqCst);
                    }
                })
            };
            // B and C: a release, and a second comparison starting beside it.
            let releaser = {
                let (graph, gate) = (Arc::clone(&graph), Arc::clone(&gate));
                std::thread::spawn(move || {
                    gate.wait();
                    graph.release_scan_cursor();
                })
            };
            let second = {
                let (graph, gate) = (Arc::clone(&graph), Arc::clone(&gate));
                std::thread::spawn(move || {
                    gate.wait();
                    graph.invalidate_scan_on_hub_drift();
                })
            };
            drainer.join().unwrap();
            releaser.join().unwrap();
            second.join().unwrap();

            let slot = lock_recover(&graph.hub_cursor);
            assert!(slot.is_closed(), "round {round}: close is final");
            assert!(
                slot.holds().is_none(),
                "round {round}: a closed slot holds an id the hub has already forgotten",
            );
            drop(slot);
            assert_eq!(
                hub.active_cursor_count(),
                before,
                "round {round}: a cursor outlived the observation — {} write-backs refused",
                refused.load(Ordering::SeqCst),
            );
            hub.shutdown();
        }
    }

    /// A cursor's life ends where the observation ends, and the three ways that end could be
    /// raced all had the same shape: the drain runs with the slot lock released.
    ///
    /// A — a copied cursor written back after a release resurrected an id the hub had already
    /// forgotten. B — a release that emptied the slot let the next comparison subscribe one
    /// nobody would ever unsubscribe. C — two comparisons overlapping a release left the slot
    /// holding whichever finished last.
    #[test]
    fn closing_a_scan_cursor_prevents_resubscription_and_stale_writeback() {
        use crate::graph::state::lock_recover;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        crate::graph::test_support::sample_workspace(root);
        let hub = crate::change_hub::WorkspaceChangeHub::start(vec![root.to_path_buf()]);
        assert!(hub.wait_until_watching(std::time::Duration::from_secs(5)));
        let before = hub.active_cursor_count();

        let graph = crate::graph::state::GraphState::for_workspace(root.to_path_buf())
            .with_change_hub(hub.clone());

        // The comparison subscribes on its own.
        graph.invalidate_scan_on_hub_drift();
        assert_eq!(hub.active_cursor_count(), before + 1, "the comparison subscribed one cursor");

        // A: the copy a drain is holding, taken before the release.
        let (held, epoch) = lock_recover(&graph.hub_cursor).open(&hub).expect("an open slot");
        graph.release_scan_cursor();
        assert_eq!(hub.active_cursor_count(), before, "the release took the cursor with it");
        assert!(
            !lock_recover(&graph.hub_cursor).advance(epoch, held),
            "a closed slot accepted a write-back and resurrected a dead cursor",
        );

        // B and C: nothing subscribes again, however many comparisons come through.
        for _ in 0..3 {
            graph.invalidate_scan_on_hub_drift();
        }
        assert!(lock_recover(&graph.hub_cursor).is_closed(), "the slot stays closed");
        assert_eq!(
            hub.active_cursor_count(),
            before,
            "a comparison after the release subscribed a cursor nobody will unsubscribe",
        );
        hub.shutdown();
    }
    use super::super::scan::workspace_fingerprint;
    use super::super::state::{lock_recover, GraphState};
    use super::super::test_support::{
        sample_workspace, seed_cache, wait_ready, wait_until, wait_until_within, write,
    };
    use super::*;
    use crate::change_hub::WorkspaceChangeHub;
    use std::time::Duration;

    #[test]
    fn prepare_identity_errors_preserve_operation_provenance() {
        assert!(matches!(
            classify_prepare_identity_error(std::io::Error::from(std::io::ErrorKind::NotFound)),
            SnapshotPrepareError::Changed
        ));
        assert!(matches!(
            classify_prepare_identity_error(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied
            )),
            SnapshotPrepareError::Open(_)
        ));
    }

    #[test]
    fn path_identity_detects_equal_size_and_time_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("graph.db");
        let replacement = dir.path().join("replacement.db");
        let old = dir.path().join("old.db");
        std::fs::write(&current, b"old").unwrap();
        let modified = std::fs::metadata(&current).unwrap().modified().unwrap();
        let before = GraphPathIdentity::read(&current).unwrap();

        std::fs::write(&replacement, b"new").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        std::fs::rename(&current, old).unwrap();
        std::fs::rename(replacement, &current).unwrap();
        let after = GraphPathIdentity::read(&current).unwrap();

        assert_eq!(before.len, after.len);
        assert_eq!(before.modified, after.modified);
        assert_ne!(before, after, "stable file identity detects the replacement");
    }

    #[test]
    fn a_case_variant_module_still_touches_the_scan_universe() {
        let path = std::path::PathBuf::from("/w/CommonModules/X/Ext/Module.BSL");
        let entry = ChangeEntry {
            canonical: path.clone(),
            raw: path,
            kind: ChangeKind::MaybeChanged,
            seq: 1,
        };
        assert!(
            entry_touches_scan_universe(&entry),
            "Module.BSL входит во вселенную скана — хаб обязан сбросить кэш отпечатка"
        );
    }

    /// Every file the project is derived from shapes the extension topology, so a
    /// change to any of them must touch the scan universe. Enumerated from the
    /// shared list rather than spelled out here: a point that stops recognising one
    /// of them reddens this test instead of going unnoticed.
    #[test]
    fn every_project_input_touches_the_scan_universe() {
        for name in project_model::PROJECT_INPUT_FILE_NAMES {
            let path = std::path::PathBuf::from("/w").join(name);
            let entry = ChangeEntry {
                canonical: path.clone(),
                raw: path,
                kind: ChangeKind::MaybeChanged,
                seq: 1,
            };
            assert!(
                entry_touches_scan_universe(&entry),
                "a change to {name} must touch the scan universe"
            );
        }
    }

    /// A request miss never reopens a replaced shared file, even when its token looks compatible.
    #[test]
    fn a_replaced_graph_file_is_not_opened_on_request_miss() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);
        seed_cache(root, workspace_fingerprint(root));

        let graph = GraphState::for_workspace(root.to_path_buf());
        graph.ensure_loading();
        wait_ready(&graph);

        lock_recover(&graph.snapshot_pool).clear();
        seed_cache(root, workspace_fingerprint(root));
        assert!(
            graph.snapshot().is_none(),
            "request miss is pool-only and never opens the replacement",
        );
    }

    #[test]
    fn final_install_rechecks_token_through_the_preopened_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);
        seed_cache(root, workspace_fingerprint(root));
        let graph = GraphState::for_workspace(root.to_path_buf());
        graph.ensure_loading();
        wait_ready(&graph);
        let (generation, fingerprint, force_stale) = {
            let inner = lock_recover(&graph.inner);
            let published = inner.published.as_ref().unwrap();
            (published.generation, published.fingerprint, published.force_stale)
        };
        let prepared = graph.prepare_snapshot_pool(generation, fingerprint, force_stale).unwrap();
        let path = graph.graph_db_path().unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        set_snapshot_install_hook(Box::new(move || {
            rusqlite::Connection::open(&path)
                .unwrap()
                .execute(
                    "UPDATE meta SET value = CAST(value AS INTEGER) + 1 WHERE key = 'revision'",
                    [],
                )
                .unwrap();
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
        }));
        let outcome = graph.install_prepared_snapshot(
            prepared,
            Published {
                generation,
                fingerprint,
                stale: false,
                reload: ReloadState::Idle,
                force_stale,
                search_roots: None,
                observed_through: Some(0),
            },
            GraphStatus::Ready { files: 0 },
            None,
            None,
            super::super::debt::RecoveryPublicationProof::without_coverage(generation),
        );
        assert!(matches!(
            outcome,
            crate::workspace_lease::LeaseOperationOutcome::OperationError(
                crate::workspace_lease::LeaseOperationError::Operation(
                    SnapshotInstallError::Changed
                )
            )
        ));
    }

    #[test]
    fn superseded_graph_serves_only_preopened_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);
        let cache = crate::cache::WorkspaceCacheLayout::for_workspace(root);
        cache.ensure().unwrap();
        let old = crate::workspace_lease::WorkspaceLease::claim_cache(&cache);
        let graph = GraphState::for_workspace_with_cache(root.to_path_buf(), cache.clone())
            .with_lease(old.clone());
        graph.ensure_loading();
        wait_ready(&graph);

        let held: Vec<_> = (0..SNAPSHOT_POOL_CAP)
            .map(|_| graph.snapshot().expect("the prepared descriptor is available"))
            .collect();
        let newer = crate::workspace_lease::WorkspaceLease::claim_cache(&cache);
        assert!(graph.snapshot().is_none(), "an empty pool never consults the foreign owner");
        newer.release();
        assert!(graph.snapshot().is_none(), "owner release cannot refill the empty pool");

        drop(held);
        assert!(graph.snapshot().is_some(), "a returned preopened descriptor remains readable");
        lock_recover(&graph.snapshot_pool).clear();
        assert!(
            graph.snapshot().is_none(),
            "a cleared pool is never refilled after terminal supersession"
        );

        let transient_dir = tempfile::tempdir().unwrap();
        let transient_root = transient_dir.path();
        sample_workspace(transient_root);
        let transient_cache = crate::cache::WorkspaceCacheLayout::for_workspace(transient_root);
        transient_cache.ensure().unwrap();
        let transient_lease = crate::workspace_lease::WorkspaceLease::claim_cache(&transient_cache);
        let transient = GraphState::for_workspace_with_cache(
            transient_root.to_path_buf(),
            transient_cache.clone(),
        )
        .with_lease(transient_lease.clone());
        transient.ensure_loading();
        wait_ready(&transient);
        let occupied: Vec<_> = (0..SNAPSHOT_POOL_CAP)
            .map(|_| transient.snapshot().expect("prepared descriptor"))
            .collect();
        let held_lock = transient_lease.hold_file_lock_for_test();
        let started = Instant::now();
        assert!(transient.snapshot().is_none(), "the fifth request gets an immediate miss");
        assert!(started.elapsed() < Duration::from_millis(100));
        drop(held_lock);
        drop(occupied);
    }

    #[test]
    fn snapshot_pool_reuses_and_discards_superseded_handles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);
        seed_cache(root, workspace_fingerprint(root));

        let graph = GraphState::for_workspace(root.to_path_buf());
        graph.ensure_loading();
        wait_ready(&graph);

        let pool_len = || lock_recover(&graph.snapshot_pool).len();
        assert_eq!(pool_len(), SNAPSHOT_POOL_CAP, "publication prepares the full pool");
        let s1 = graph.snapshot().expect("snapshots");
        assert_eq!(pool_len(), SNAPSHOT_POOL_CAP - 1);
        drop(s1);
        assert_eq!(pool_len(), SNAPSHOT_POOL_CAP, "the dropped handle returns to the pool");
        let s2 = graph.snapshot().expect("snapshots");
        assert_eq!(pool_len(), SNAPSHOT_POOL_CAP - 1);
        drop(s2);
        assert_eq!(pool_len(), SNAPSHOT_POOL_CAP);

        {
            let mut pool = lock_recover(&graph.snapshot_pool);
            let entry = pool.pop().expect("one parked entry");
            pool.push(PooledSnapshotEntry { generation: entry.generation + 100, ..entry });
        }
        let s3 = graph.snapshot().expect("snapshots");
        assert_eq!(s3.generation, 7, "a superseded handle never serves a new request");
        assert_eq!(pool_len(), SNAPSHOT_POOL_CAP - 2, "the stale entry was discarded");
        drop(s3);

        let old = graph.snapshot().expect("old generation checkout");
        {
            let mut pool = lock_recover(&graph.snapshot_pool);
            pool.clear();
            pool.generation += 1;
        }
        drop(old);
        assert_eq!(pool_len(), 0, "a returned old-generation handle cannot poison a new pool");
    }

    #[test]
    fn checkout_cannot_discard_a_concurrently_published_pool() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);
        seed_cache(root, workspace_fingerprint(root));

        let graph = GraphState::for_workspace(root.to_path_buf());
        graph.ensure_loading();
        wait_ready(&graph);
        let (generation, fingerprint, force_stale) = {
            let inner = lock_recover(&graph.inner);
            let published = inner.published.as_ref().unwrap();
            (published.generation, published.fingerprint, published.force_stale)
        };
        let mut prepared =
            graph.prepare_snapshot_pool(generation, fingerprint, force_stale).unwrap();
        for entry in &mut prepared.entries {
            entry.generation = generation + 1;
        }

        let pending = Arc::new(Mutex::new(Some(prepared)));
        let publisher = Arc::new(Mutex::new(None));
        let publishing_graph = graph.clone();
        let pending_for_hook = Arc::clone(&pending);
        let publisher_for_hook = Arc::clone(&publisher);
        set_snapshot_checkout_hook(Box::new(move || {
            let graph = publishing_graph.clone();
            let publish = move || {
                let prepared = lock_recover(&pending_for_hook).take().unwrap();
                let mut inner = lock_recover(&graph.inner);
                let mut pool = lock_recover(&graph.snapshot_pool);
                pool.generation = generation + 1;
                pool.entries = prepared.entries;
                inner.published.as_mut().unwrap().generation = generation + 1;
            };
            match publishing_graph.inner.try_lock() {
                Ok(guard) => {
                    drop(guard);
                    publish();
                }
                Err(_) => {
                    *lock_recover(&publisher_for_hook) = Some(std::thread::spawn(publish));
                }
            }
        }));

        let old = graph.snapshot().expect("checkout wins before the queued publication");
        lock_recover(&publisher).take().unwrap().join().unwrap();
        drop(old);

        let pool = lock_recover(&graph.snapshot_pool);
        assert_eq!(pool.generation, generation + 1);
        assert_eq!(pool.len(), SNAPSHOT_POOL_CAP, "the complete new pool survives old checkout");
    }

    #[test]
    fn background_snapshot_preserves_all_typed_outcomes() {
        use crate::workspace_lease::{LeaseOperationError, LeaseOperationOutcome};

        let ready_graph = || {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            sample_workspace(root);
            let cache = crate::cache::WorkspaceCacheLayout::for_workspace(root);
            cache.ensure().unwrap();
            let lease = crate::workspace_lease::WorkspaceLease::claim_cache(&cache);
            let graph = GraphState::for_workspace_with_cache(root.to_path_buf(), cache)
                .with_lease(lease.clone());
            graph.ensure_loading();
            wait_ready(&graph);
            (dir, graph, lease)
        };
        let occupy = |graph: &GraphState| -> Vec<_> {
            (0..SNAPSHOT_POOL_CAP)
                .map(|_| graph.snapshot().expect("published descriptor"))
                .collect()
        };

        let (_dir, graph, lease) = ready_graph();
        let held_lock = lease.hold_file_lock_for_test();
        let started = Instant::now();
        assert!(matches!(graph.snapshot_blocking(), LeaseOperationOutcome::Applied(Some(_))));
        assert!(started.elapsed() < Duration::from_millis(100), "pool checkout skips preflight");
        drop(held_lock);

        let _occupied = occupy(&graph);
        let held_lock = lease.hold_file_lock_for_test();
        assert!(matches!(graph.snapshot_blocking(), LeaseOperationOutcome::TransientRefusal));
        drop(held_lock);

        let (_dir, graph, _lease) = ready_graph();
        let _occupied = occupy(&graph);
        let changed = graph.clone();
        set_snapshot_install_hook(Box::new(move || {
            lock_recover(&changed.inner).published.as_mut().unwrap().generation += 1;
        }));
        assert!(matches!(
            graph.snapshot_blocking(),
            LeaseOperationOutcome::OperationError(LeaseOperationError::Operation(_))
        ));

        let (_dir, graph, _lease) = ready_graph();
        let _occupied = occupy(&graph);
        graph.set_background_snapshot_failure_for_test(Some(BackgroundSnapshotFailure::Open));
        let result = graph.snapshot_blocking();
        graph.set_background_snapshot_failure_for_test(None);
        assert!(matches!(
            result,
            LeaseOperationOutcome::OperationError(LeaseOperationError::Operation(_))
        ));

        let missing = GraphState::for_workspace(tempfile::tempdir().unwrap().path().to_path_buf());
        assert!(matches!(missing.snapshot_blocking(), LeaseOperationOutcome::Applied(None)));

        let (_dir, graph, lease) = ready_graph();
        let _occupied = occupy(&graph);
        lease.release();
        assert!(matches!(graph.snapshot_blocking(), LeaseOperationOutcome::Released));

        let (_dir, graph, old) = ready_graph();
        let _occupied = occupy(&graph);
        let cache = graph.cache().unwrap().clone();
        let newer = crate::workspace_lease::WorkspaceLease::claim_cache(&cache);
        assert!(matches!(graph.snapshot_blocking(), LeaseOperationOutcome::Superseded));
        old.release();
        newer.release();
    }

    #[test]
    fn publication_without_a_complete_descriptor_pool_cannot_be_ready() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);
        let graph = GraphState::for_workspace(root.to_path_buf());
        graph.set_background_snapshot_failure_for_test(Some(
            BackgroundSnapshotFailure::PrepareSecondChanged,
        ));
        graph.ensure_loading();
        wait_until_within(&graph, Duration::from_secs(5), "the publication to fail", || {
            matches!(graph.status(), GraphStatus::Failed { .. })
        });
        assert!(graph.snapshot().is_none());
        graph.set_background_snapshot_failure_for_test(None);
        graph.ensure_loading();
        wait_ready(&graph);
        assert!(graph.snapshot().is_some(), "the failed publication remains retryable");
    }

    #[test]
    fn drift_marks_stale_and_async_reload_bumps_generation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);

        let mut graph = GraphState::for_workspace(root.to_path_buf());
        graph.drift_interval = Duration::ZERO;
        graph.ensure_loading();
        wait_ready(&graph);

        let snap1 = graph.snapshot().expect("ready graph snapshots");
        let old_token = snap1.graph.freshness_token().unwrap();
        #[cfg(windows)]
        let old_copy = {
            let entry = snap1.graph.entry.as_ref().unwrap();
            let pool = lock_recover(&graph.snapshot_pool);
            assert!(pool.iter().all(|other| Arc::ptr_eq(&entry._backing, &other._backing)));
            entry._backing.to_path_buf()
        };
        let fresh = graph.freshness(&snap1);
        assert_eq!(fresh.revision, 1);
        assert!(!fresh.stale);
        assert_eq!(fresh.reload, "none");

        write(
            root,
            "CommonModules/Сервер/Ext/Module.bsl",
            "&НаСервере\nФункция Считать() Экспорт Возврат 42; КонецФункции",
        );
        let drifted = graph.freshness(&snap1);
        assert!(drifted.stale, "an on-disk edit must read as stale");
        assert_eq!(drifted.revision, 1, "the stale response still serves the old generation");
        assert!(matches!(drifted.reload, "running" | "failed"));

        wait_until_within(
            &graph,
            Duration::from_secs(2),
            "the reload to publish generation 2",
            || graph.snapshot().is_some_and(|snap| snap.generation == 2),
        );
        let settled = graph.freshness(&graph.snapshot().expect("the reload published"));
        assert!(!settled.stale);
        assert_eq!(settled.revision, 2);
        assert_eq!(settled.reload, "none");
        assert_eq!(snap1.graph.freshness_token().unwrap(), old_token);
        #[cfg(windows)]
        assert!(old_copy.exists(), "the checked-out old generation keeps its copy alive");
        drop(snap1);
        #[cfg(windows)]
        assert!(!old_copy.exists(), "the last old reader removes its detached copy");
    }

    #[test]
    fn graph_freshness_invalidates_on_hub_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Configuration.xml"), "<Configuration/>").unwrap();
        sample_workspace(root);

        let hub = WorkspaceChangeHub::start(vec![root.to_path_buf()]);
        assert!(hub.wait_until_watching(Duration::from_secs(5)));
        let mut graph = GraphState::for_workspace(root.to_path_buf()).with_change_hub(hub.clone());
        graph.drift_interval = Duration::from_secs(120);
        graph.ensure_loading();
        wait_ready(&graph);

        let snap = graph.snapshot().expect("ready");
        assert!(!graph.freshness(&snap).stale, "a freshly built graph is not stale");

        let mut observer = hub.subscribe();
        std::thread::sleep(Duration::from_millis(10));
        write(
            root,
            "CommonModules/Сервер/Ext/Module.bsl",
            "&НаСервере\nФункция Считать() Экспорт Возврат 1; КонецФункции",
        );
        // Waits on the hub's delivery queue, not on graph state: a graph-state summary
        // would say nothing about whether inotify delivered.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut delivered = false;
        while Instant::now() < deadline {
            let batch = hub.drain(observer);
            observer = batch.cursor;
            if batch.entries.iter().any(|e| e.raw.to_string_lossy().contains("Module.bsl")) {
                delivered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(delivered, "the hub delivered the edit");
        assert!(
            graph.freshness(&snap).stale,
            "a hub-delivered edit is seen without waiting out the drift throttle",
        );
    }

    /// The full live-daemon chain for a topology-only change: a served graph must
    /// read stale after a `dependsOn`-only config edit (no `.bsl`/`.xml` touched),
    /// and the kicked reload must publish a fresh generation that reads clean.
    #[test]
    fn a_depends_on_only_edit_marks_a_served_graph_stale_and_reloads() {
        use super::super::test_support::{write_extension_config, write_extension_workspace};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_extension_workspace(root, false);

        let mut graph = GraphState::for_workspace(root.to_path_buf());
        graph.drift_interval = Duration::ZERO;
        graph.ensure_loading();
        wait_ready(&graph);

        let snap = graph.snapshot().expect("ready graph snapshots");
        assert!(!graph.freshness(&snap).stale, "a freshly built graph is not stale");

        write_extension_config(root, true);
        let drifted = graph.freshness(&snap);
        assert!(drifted.stale, "a dependsOn-only edit must read as stale");
        assert!(matches!(drifted.reload, "running" | "failed"));

        wait_until_within(
            &graph,
            Duration::from_secs(5),
            "the topology-triggered reload to publish generation 2",
            || graph.snapshot().is_some_and(|snap| snap.generation == 2),
        );
        let settled = graph.freshness(&graph.snapshot().expect("the reload published"));
        assert!(!settled.stale, "the reloaded graph reflects the new topology");
    }

    /// End-to-end root re-arm: an extension root added by a topology reload lies
    /// OUTSIDE the hub's original coverage, and after the reload publishes, events
    /// under that root must be hub-delivered — proof the rebuild re-pointed the
    /// live watcher instead of leaving the new subtree to the reconciler.
    #[test]
    fn a_topology_reload_rearms_the_hub_onto_the_new_extension_root() {
        use super::super::test_support::write;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ext_dir = tempfile::tempdir().unwrap();
        let ext = ext_dir.path();
        super::super::test_support::sample_workspace(root);
        write(root, "Configuration.xml", "<Configuration/>");
        write(ext, "Configuration.xml", "<Configuration/>");

        let hub = WorkspaceChangeHub::start(vec![root.to_path_buf()]);
        assert!(hub.wait_until_watching(Duration::from_secs(5)));
        let mut graph = GraphState::for_workspace(root.to_path_buf()).with_change_hub(hub.clone());
        graph.drift_interval = Duration::ZERO;
        graph.ensure_loading();
        super::super::test_support::wait_ready(&graph);
        let snap = graph.snapshot().expect("ready");
        assert!(!graph.freshness(&snap).stale);

        // Declare the out-of-tree extension: a topology-only reload trigger.
        std::fs::write(
            root.join("bsl-analyzer.toml"),
            format!(
                "[source]\nroot = \".\"\nextensions = [{{ name = \"a\", path = {:?} }}]\n",
                ext.to_string_lossy()
            ),
        )
        .unwrap();
        // Staleness lands once the hub delivers the config event (the throttled
        // fast path deliberately serves the cached topology until then).
        wait_until_within(
            &graph,
            Duration::from_secs(5),
            "the new extension root to read as drift",
            || graph.freshness(&snap).stale,
        );
        wait_until(&graph, "the drift reload to publish generation 2", || {
            graph.snapshot().map(|s| s.generation) == Some(2)
        });

        // The re-armed hub must deliver events under the NEW root. The write is
        // repeated per poll so a delivery is observed even if the ack landed a
        // moment after the generation became visible.
        let cursor = hub.subscribe();
        let file = ext.join("Новый.bsl");
        // The watcher reports the spelling the platform hands it, which need not be the
        // one written through (macOS resolves the temp dir's link before delivering). The
        // delivery is therefore recognised by `canonical`, the key the hub guarantees
        // against the scan universe.
        let delivered = ext.canonicalize().expect("the extension root exists").join("Новый.bsl");
        // Waits on the hub's delivery queue, not on graph state: a graph-state summary
        // would say nothing about whether inotify delivered.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut cursor = cursor;
        let mut seen = false;
        while Instant::now() < deadline {
            std::fs::write(&file, "Процедура П()\nКонецПроцедуры").unwrap();
            std::thread::sleep(Duration::from_millis(50));
            let batch = hub.drain(cursor);
            cursor = batch.cursor;
            if batch.entries.iter().any(|e| e.canonical == delivered) {
                seen = true;
                break;
            }
        }
        assert!(seen, "the hub must deliver events under the newly-added extension root");
    }

    /// Another consumer that stopped draining owes its own reconcile. Answering that debt
    /// here used to drop the graph's fingerprint map and buy a full tree walk on every
    /// freshness check — for as long as the other consumer stayed silent, which is
    /// forever if its thread is gone.
    #[test]
    fn a_foreign_cursors_debt_does_not_cost_the_graph_a_walk() {
        use crate::change_hub::WorkspaceChangeHub;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);

        let hub = WorkspaceChangeHub::start(vec![root.to_path_buf()]);
        assert!(hub.wait_until_watching(Duration::from_secs(5)));
        let mut graph = GraphState::for_workspace(root.to_path_buf()).with_change_hub(hub.clone());
        // Without this the throttled scan cache answers before the health question is ever
        // asked, and this test would pass no matter what the answer would have been.
        graph.drift_interval = Duration::ZERO;
        graph.ensure_loading();
        wait_ready(&graph);
        // The graph's own cursor exists and is clean from here on: `current_disk_fp`
        // drains it before it asks anything. Two calls settle the map and its own debt.
        let _ = graph.current_disk_fp();
        let _ = graph.current_disk_fp();

        // A stranger subscribes and never drains; then everyone is asked to reconcile.
        // The graph answers for ITSELF with one walk — that debt is genuinely its own —
        // and the stranger's stays outstanding for ever after.
        let _stranger = hub.subscribe();
        hub.degrade_external();
        let _ = graph.current_disk_fp();

        let walks = graph.scan_count.load(std::sync::atomic::Ordering::SeqCst);
        let _ = graph.current_disk_fp();
        assert_eq!(
            graph.scan_count.load(std::sync::atomic::Ordering::SeqCst),
            walks,
            "somebody else's outstanding reconcile is not the graph's to pay for"
        );
    }

    /// The other half, and the one that keeps the first honest: when the HUB cannot
    /// deliver, the graph must keep walking however clean its own cursor is. Without this
    /// leg, replacing the health question with an unconditional fast path passes every
    /// other gate here while going quietly blind.
    ///
    /// The carrier is a hub whose thread never started, not a blind root: the graph
    /// re-declares the hub's targets as it builds, which would take an unwatched root out
    /// of the declaration and leave the hub honestly healthy — a stand that proves nothing.
    #[test]
    fn a_hub_that_cannot_deliver_still_sends_the_graph_back_to_a_walk() {
        use crate::change_hub::{WatchTarget, WorkspaceChangeHub};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        sample_workspace(root);
        let hub = WorkspaceChangeHub::start_with_unstartable_thread(vec![WatchTarget::recursive(
            root.to_path_buf(),
        )]);

        let mut graph = GraphState::for_workspace(root.to_path_buf()).with_change_hub(hub);
        graph.drift_interval = Duration::ZERO;
        graph.ensure_loading();
        wait_ready(&graph);
        let _ = graph.current_disk_fp();
        let walks = graph.scan_count.load(std::sync::atomic::Ordering::SeqCst);

        let _ = graph.current_disk_fp();
        assert!(
            graph.scan_count.load(std::sync::atomic::Ordering::SeqCst) > walks,
            "a hub that will never deliver leaves the graph nothing to trust"
        );
    }

    /// A config-file change delivered by the hub must invalidate the throttled
    /// fingerprint cache AND the event-maintained stat map immediately — the map
    /// can only patch file stats, not the topology, so serving its fold after a
    /// config edit would keep a stale topology fresh for up to the walk interval.
    #[test]
    fn graph_freshness_sees_a_config_edit_through_the_hub() {
        use super::super::test_support::{write_extension_config, write_extension_workspace};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_extension_workspace(root, false);

        let hub = WorkspaceChangeHub::start(vec![root.to_path_buf()]);
        assert!(hub.wait_until_watching(Duration::from_secs(5)));
        let mut graph = GraphState::for_workspace(root.to_path_buf()).with_change_hub(hub.clone());
        graph.drift_interval = Duration::from_secs(120);
        graph.ensure_loading();
        wait_ready(&graph);

        let snap = graph.snapshot().expect("ready");
        assert!(!graph.freshness(&snap).stale, "a freshly built graph is not stale");

        let mut observer = hub.subscribe();
        std::thread::sleep(Duration::from_millis(10));
        // Re-written per poll: under a fully parallel test run the inotify queue
        // can lag well past a single write's event window.
        // Waits on the hub's delivery queue, not on graph state: a graph-state summary
        // would say nothing about whether inotify delivered.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut delivered = false;
        while Instant::now() < deadline {
            write_extension_config(root, true);
            std::thread::sleep(Duration::from_millis(20));
            let batch = hub.drain(observer);
            observer = batch.cursor;
            if batch.entries.iter().any(|e| e.raw.to_string_lossy().contains("bsl-analyzer.toml")) {
                delivered = true;
                break;
            }
        }
        assert!(
            delivered,
            "the hub delivered the config edit (events_seen={}, health={:?})",
            hub.events_seen(),
            hub.health(),
        );
        assert!(
            graph.freshness(&snap).stale,
            "a hub-delivered config edit is seen without waiting out the drift throttle",
        );
    }

    #[test]
    fn graph_freshness_ignores_non_scan_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Configuration.xml"), "<Configuration/>").unwrap();
        sample_workspace(root);

        let hub = WorkspaceChangeHub::start(vec![root.to_path_buf()]);
        assert!(hub.wait_until_watching(Duration::from_secs(5)));
        let mut graph = GraphState::for_workspace(root.to_path_buf()).with_change_hub(hub.clone());
        graph.drift_interval = Duration::from_secs(120);
        graph.ensure_loading();
        wait_ready(&graph);

        let snap = graph.snapshot().expect("ready");
        assert!(!graph.freshness(&snap).stale, "a freshly built graph is not stale");
        let scans_after_prime = graph.scan_count();

        let mut observer = hub.subscribe();
        std::thread::sleep(Duration::from_millis(10));
        write(root, "CommonModules/Сервер/Ext/Module.bsl.tmp", "editor swap file");
        // Waits on the hub's delivery queue, not on graph state: a graph-state summary
        // would say nothing about whether inotify delivered.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut delivered = false;
        while Instant::now() < deadline {
            let batch = hub.drain(observer);
            observer = batch.cursor;
            if batch.entries.iter().any(|e| e.raw.to_string_lossy().contains(".tmp")) {
                delivered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(delivered, "the hub delivered the .tmp file");
        assert!(!graph.freshness(&snap).stale, "a temp file does not make the graph stale");
        assert_eq!(
            graph.scan_count(),
            scans_after_prime,
            "an irrelevant temp file must not invalidate the cache and re-trigger a scan",
        );
    }

    /// The three positive proofs, and everything that is NOT one of them.
    ///
    /// This is the producer, where the policy lives: what the debt applies is whatever arrives
    /// in these lists, so the lists are where "a complete walk of a validated scope" has to be
    /// told apart from a short one, a restricted fallback, and a patch that never looked.
    #[test]
    fn only_three_positives_retire_an_obligation() {
        let dir = tempfile::tempdir().unwrap();
        let graph = GraphState::for_workspace(dir.path().to_path_buf());
        // Real places on disk, because a descriptor whose roots do not resolve may not say
        // that an address has gone away — and a test that asserted removal against a spelling
        // nothing answers to would prove the opposite of what it claims.
        let root = dir.path().join("ws");
        let excluded = root.join("Исключено");
        let other_root = dir.path().join("другое");
        std::fs::create_dir_all(&excluded).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();
        let validated = super::super::debt::RecoveryScope::of(
            std::slice::from_ref(&root),
            std::slice::from_ref(&excluded),
            true,
        );
        let fallback =
            super::super::debt::RecoveryScope::of(std::slice::from_ref(&root), &[], false);
        let module = root.join("Модуль.bsl");
        let another = root.join("Другой.bsl");
        std::fs::write(&module, "").unwrap();
        std::fs::write(&another, "").unwrap();
        let module = module.canonicalize().unwrap().to_string_lossy().into_owned();
        let another = another.canonicalize().unwrap().to_string_lossy().into_owned();
        let required: &str = &module;
        let other: &str = &another;

        // Declare the obligation the way an unsound publication does.
        lock_recover(&graph.debt).record_publication(
            std::time::Instant::now(),
            Some(1),
            false,
            None,
            super::super::debt::RecoveryPublicationProof {
                generation: 1,
                declared_unread: Some(vec![required.to_owned()]),
                ..Default::default()
            },
        );

        let enumerated_with: std::collections::HashSet<&str> =
            [required, other].into_iter().collect();
        let enumerated_without: std::collections::HashSet<&str> = [other].into_iter().collect();
        let unread_none: Vec<String> = Vec::new();
        let unread_it = vec![required.to_owned()];

        let read = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Walked {
                scope: validated.clone(),
                enumerated: &enumerated_with,
                complete: true,
                straddled: false,
            },
        );
        assert_eq!(read.read_covered.len(), 1, "a build that enumerated and read it proves so");
        assert!(read.absent_covered.is_empty() && read.out_of_scope_covered.is_empty());

        let still_unread = graph.recovery_proof(
            2,
            Some(&unread_it),
            RecoveryCoverage::Walked {
                scope: validated.clone(),
                enumerated: &enumerated_with,
                complete: true,
                straddled: false,
            },
        );
        assert!(
            still_unread.read_covered.is_empty(),
            "a build that listed it and could not read it proved it read it",
        );

        let absent = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Walked {
                scope: validated.clone(),
                enumerated: &enumerated_without,
                complete: true,
                straddled: false,
            },
        );
        assert_eq!(absent.absent_covered.len(), 1, "a complete walk that did not list it");

        let short = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Walked {
                scope: validated.clone(),
                enumerated: &enumerated_without,
                complete: false,
                straddled: false,
            },
        );
        assert!(short.absent_covered.is_empty(), "a SHORT walk proved an absence");

        let restricted = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Walked {
                scope: fallback,
                enumerated: &enumerated_without,
                complete: true,
                straddled: false,
            },
        );
        assert!(
            restricted.absent_covered.is_empty() && restricted.out_of_scope_covered.is_empty(),
            "a restricted fallback declared a path gone",
        );

        // A narrower validated declaration no longer asks for it — that IS a proof.
        let narrowed =
            super::super::debt::RecoveryScope::of(std::slice::from_ref(&other_root), &[], true);
        let out_of_scope = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Walked {
                scope: narrowed,
                enumerated: &enumerated_without,
                complete: true,
                straddled: false,
            },
        );
        assert_eq!(out_of_scope.out_of_scope_covered.len(), 1, "the roots no longer cover it");

        // A point patch proves only what it rewrote, and never an absence.
        let rewritten: std::collections::HashSet<&str> = [required].into_iter().collect();
        let patched = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Patched { rewritten: &rewritten },
        );
        assert_eq!(patched.read_covered.len(), 1, "the patch re-projected it");
        assert!(patched.absent_covered.is_empty(), "a patch proved an absence");

        let untouched: std::collections::HashSet<&str> = [other].into_iter().collect();
        let elsewhere = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Patched { rewritten: &untouched },
        );
        assert!(elsewhere.read_covered.is_empty(), "a patch that never touched it covered it");
        assert!(
            elsewhere.absent_covered.is_empty() && elsewhere.out_of_scope_covered.is_empty(),
            "a patch proved something about an address it never looked at",
        );

        // A walk that straddled a write enumerated a world that has already been replaced.
        let straddled = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Walked {
                scope: validated.clone(),
                enumerated: &enumerated_without,
                complete: true,
                straddled: true,
            },
        );
        assert!(
            straddled.absent_covered.is_empty(),
            "a build whose tree moved under it proved an absence",
        );
        assert_eq!(straddled.scan_complete, Some(true), "the walk itself was still whole");
        assert!(straddled.straddled, "and the publication still cannot vouch for it");

        // A root that could not be resolved keeps its declared spelling, under which the
        // canonical addresses beneath it match nothing. That is not a removal.
        let gone = dir.path().join("ушёл");
        let unresolved =
            super::super::debt::RecoveryScope::of(std::slice::from_ref(&gone), &[], true);
        let denied_root = graph.recovery_proof(
            2,
            Some(&unread_none),
            RecoveryCoverage::Walked {
                scope: unresolved,
                enumerated: &enumerated_without,
                complete: true,
                straddled: false,
            },
        );
        assert!(
            denied_root.out_of_scope_covered.is_empty() && denied_root.absent_covered.is_empty(),
            "a root that would not resolve declared its subtree gone",
        );

        // An address the walk LISTED is required, whatever spelling the roots canonicalise
        // to: a descendant reached through a symlink is enumerated under the walked name and
        // canonicalises elsewhere.
        let alias: std::collections::HashSet<&str> = [required, other].into_iter().collect();
        let aliased = graph.recovery_proof(
            2,
            Some(&unread_it),
            RecoveryCoverage::Walked {
                scope: super::super::debt::RecoveryScope::of(
                    std::slice::from_ref(&other_root),
                    &[],
                    true,
                ),
                enumerated: &alias,
                complete: true,
                straddled: false,
            },
        );
        assert!(
            aliased.out_of_scope_covered.is_empty(),
            "an address the walk listed and the artefact could not read was retired anyway",
        );

        // No strict unread list at all: the database would not say what it managed to read,
        // so nothing it did proves anything about an obligation.
        let blind_metadata = graph.recovery_proof(
            2,
            None,
            RecoveryCoverage::Walked {
                scope: validated.clone(),
                enumerated: &enumerated_with,
                complete: true,
                straddled: false,
            },
        );
        assert!(
            blind_metadata.read_covered.is_empty()
                && blind_metadata.absent_covered.is_empty()
                && blind_metadata.out_of_scope_covered.is_empty(),
            "a publication that could not read its own metadata retired an obligation",
        );

        // And a cache, which did neither.
        let cached = graph.recovery_proof(2, Some(&unread_none), RecoveryCoverage::None);
        assert!(
            cached.read_covered.is_empty()
                && cached.absent_covered.is_empty()
                && cached.out_of_scope_covered.is_empty(),
            "a cache served as it stands proved something",
        );
    }

    /// Unreadable metadata is a failure to LOOK, and it answers nothing.
    ///
    /// `read_unread_paths` returns an empty list for a database that will not read — the
    /// lenient form the serving API has always used. Taken as authority it says "this build
    /// read everything", which would retire every outstanding obligation on the strength of an
    /// error.
    #[test]
    fn strict_unread_errors_never_become_an_empty_answer() {
        let dir = tempfile::tempdir().unwrap();
        let graph = GraphState::for_workspace(dir.path().to_path_buf());
        lock_recover(&graph.debt).record_publication(
            std::time::Instant::now(),
            Some(1),
            false,
            None,
            super::super::debt::RecoveryPublicationProof {
                generation: 1,
                declared_unread: Some(vec!["/ws/Модуль.bsl".to_owned()]),
                ..Default::default()
            },
        );
        let enumerated: std::collections::HashSet<&str> = ["/ws/Модуль.bsl"].into_iter().collect();
        let blind = graph.recovery_proof(
            2,
            None,
            RecoveryCoverage::Walked {
                scope: super::super::debt::RecoveryScope::of(
                    &[std::path::PathBuf::from("/ws")],
                    &[],
                    true,
                ),
                enumerated: &enumerated,
                complete: true,
                straddled: false,
            },
        );
        assert!(
            blind.read_covered.is_empty(),
            "a publication whose metadata would not read claimed it had read the file",
        );
        assert!(
            blind.declared_unread.is_none(),
            "and it claimed authority over what is still unread",
        );

        // The strict reader itself: an absent key is an empty set, a broken payload is an
        // error.
        let path = dir.path().join("meta.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)", []).unwrap();
        assert_eq!(
            crate::graph_db::read_unread_paths_strict(&conn).unwrap(),
            Vec::<String>::new(),
            "an artefact that recorded nothing unread has nothing unread",
        );
        conn.execute("INSERT INTO meta (key, value) VALUES ('unread_paths', 'not json')", [])
            .unwrap();
        assert!(
            crate::graph_db::read_unread_paths_strict(&conn).is_err(),
            "a payload that will not decode was read as an empty answer",
        );
        assert!(
            crate::graph_db::read_unread_paths(&conn).is_empty(),
            "control: the lenient reader still answers empty, which is why it is not authority",
        );
    }
}
