//! Process-wide memory of graph file content hashes, keyed by the file's stat identity.
//!
//! The graph's identity is the content of its files, so a moved or copied tree is recognised
//! by its bytes. Re-reading every byte on every walk to establish that is what made a walk six
//! times dearer than a `stat`; a file whose stat identity has not moved since its bytes were
//! hashed is not re-read. The identity includes the change time, inode and device where the
//! platform exposes them, so a write that restores the size and modification time is still
//! seen; on Windows it is size and modification time only.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A hash is not trusted for a file modified this close to the moment the hash was taken: a
/// second write inside the timestamp resolution leaves every stat field unchanged. Two seconds
/// covers the coarsest filesystem timestamps in use.
pub(crate) const RACY_WINDOW: Duration = Duration::from_secs(2);

/// What a `stat` says about a file's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatIdentity {
    pub(crate) len: u64,
    pub(crate) mtime_ns: u128,
    /// `None` where the platform does not expose it (Windows).
    pub(crate) change: Option<ChangeIdentity>,
}

/// The part of a stat identity a write cannot restore: every write moves the change time, and
/// a replacement file has another inode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChangeIdentity {
    pub(crate) ctime_ns: i128,
    pub(crate) ino: u64,
    pub(crate) dev: u64,
}

impl StatIdentity {
    pub(crate) fn of(metadata: &std::fs::Metadata) -> StatIdentity {
        StatIdentity {
            len: metadata.len(),
            mtime_ns: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0),
            change: change_identity(metadata),
        }
    }
}

#[cfg(unix)]
fn change_identity(metadata: &std::fs::Metadata) -> Option<ChangeIdentity> {
    use std::os::unix::fs::MetadataExt;
    Some(ChangeIdentity {
        ctime_ns: i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec()),
        ino: metadata.ino(),
        dev: metadata.dev(),
    })
}

#[cfg(not(unix))]
fn change_identity(_: &std::fs::Metadata) -> Option<ChangeIdentity> {
    None
}

/// A content hash together with the stat identity it was taken under and when it was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) stat: StatIdentity,
    pub(crate) hash: [u8; 32],
    pub(crate) observed_at_ns: u128,
}

impl Observation {
    /// Whether this hash still describes a file whose current stat identity is `stat`.
    pub(crate) fn trusted_for(&self, stat: &StatIdentity) -> bool {
        self.stat == *stat
            && self.stat.mtime_ns.saturating_add(RACY_WINDOW.as_nanos()) < self.observed_at_ns
    }
}

/// The content of one file as a walk observed it.
pub(crate) struct ContentRead {
    /// `None` when the bytes could not be read.
    pub(crate) hash: Option<[u8; 32]>,
    /// When the bytes behind `hash` were read; `None` for an unreadable file.
    pub(crate) observed_at_ns: Option<u128>,
    /// Whether this call read the file rather than reusing a remembered hash.
    pub(crate) read: bool,
}

fn memory() -> &'static Mutex<HashMap<PathBuf, Observation>> {
    static MEMORY: OnceLock<Mutex<HashMap<PathBuf, Observation>>> = OnceLock::new();
    MEMORY.get_or_init(Default::default)
}

fn now_ns() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| duration.as_nanos()).unwrap_or(0)
}

/// The content hash of `canonical`, whose current stat identity is `stat`: remembered when the
/// identity has not moved since the hash was taken, read otherwise.
pub(crate) fn content_of(canonical: &Path, stat: StatIdentity) -> ContentRead {
    let remembered = super::state::lock_recover(memory()).get(canonical).copied();
    if let Some(observation) = remembered.filter(|observation| observation.trusted_for(&stat)) {
        return ContentRead {
            hash: Some(observation.hash),
            observed_at_ns: Some(observation.observed_at_ns),
            read: false,
        };
    }
    // Taken before the read: a write landing during the read then falls inside the racy window
    // of this observation instead of after it.
    let observed_at_ns = now_ns();
    match std::fs::read(canonical) {
        Ok(bytes) => {
            let hash = *blake3::hash(&bytes).as_bytes();
            super::state::lock_recover(memory())
                .insert(canonical.to_path_buf(), Observation { stat, hash, observed_at_ns });
            ContentRead { hash: Some(hash), observed_at_ns: Some(observed_at_ns), read: true }
        }
        Err(_) => {
            super::state::lock_recover(memory()).remove(canonical);
            ContentRead { hash: None, observed_at_ns: None, read: true }
        }
    }
}

/// Forget remembered files under `roots` that a complete walk of those roots did not list.
pub(crate) fn retain_listed(roots: &[PathBuf], listed: &HashSet<&Path>) {
    let roots: Vec<PathBuf> = roots
        .iter()
        .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect();
    super::state::lock_recover(memory()).retain(|path, _| {
        listed.contains(path.as_path()) || !roots.iter().any(|root| path.starts_with(root))
    });
}

/// Remember observations persisted by an earlier process. An observation already held is
/// kept: it was taken by this process and is at least as recent.
pub(crate) fn seed(observations: impl IntoIterator<Item = (PathBuf, Observation)>) {
    let mut memory = super::state::lock_recover(memory());
    for (path, observation) in observations {
        memory.entry(path).or_insert(observation);
    }
}

/// Whether a hash is remembered for `canonical`.
#[cfg(test)]
pub(crate) fn remembers(canonical: &Path) -> bool {
    super::state::lock_recover(memory()).contains_key(canonical)
}

/// Forget everything remembered under `root`, as a restarted process would.
#[cfg(test)]
pub(crate) fn forget_under(root: &Path) {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    super::state::lock_recover(memory()).retain(|path, _| !path.starts_with(&root));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(len: u64, mtime_ns: u128) -> StatIdentity {
        StatIdentity { len, mtime_ns, change: None }
    }

    #[test]
    fn a_hash_taken_inside_the_racy_window_is_not_trusted() {
        let window = RACY_WINDOW.as_nanos();
        let mtime = 1_000 * window;
        let observed =
            |observed_at_ns| Observation { stat: stat(7, mtime), hash: [1; 32], observed_at_ns };
        assert!(!observed(mtime).trusted_for(&stat(7, mtime)), "taken in the same tick");
        assert!(!observed(mtime + window).trusted_for(&stat(7, mtime)), "on the window edge");
        assert!(observed(mtime + window + 1).trusted_for(&stat(7, mtime)), "past the window");
        assert!(
            !observed(mtime + window + 1).trusted_for(&stat(8, mtime)),
            "another size is another file",
        );
    }
}
