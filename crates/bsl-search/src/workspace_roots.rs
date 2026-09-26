//! Which source root a workspace file belongs to, and under which key it is
//! stored.
//!
//! A `cfe` extension repeats the configuration's directory layout, so the path
//! relative to a root is not unique across roots: `CommonModules/M/Ext/Module.bsl`
//! exists under the configuration and under every extension at once. Store rows
//! are therefore keyed by the pair `(root_id, path)`, and this table is the only
//! seam that turns an absolute path into that pair and back.
//!
//! The table knows nothing about configurations or extensions beyond which root
//! is *the* configuration one; the caller builds it from the project model.

use std::path::{Path, PathBuf};

/// The configuration root's identity. Reserved: an extension never takes it, so
/// rows written before roots existed — all of them the configuration's — keep
/// their meaning under the composite key without being rewritten.
pub const CONFIGURATION_ROOT_ID: &str = "";

/// A declared root that did not become a registered one. Reported rather than
/// swallowed: the caller has to be able to say which root it dropped and why,
/// because in both cases some files end up labelled differently than declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedRoot {
    pub path: PathBuf,
    pub reason: Rejection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// Canonically inside the configuration root. Its files stay searchable as
    /// the configuration's, but their root label is the configuration's too.
    InsideConfiguration { root: PathBuf },
    /// Its identifier is already taken by another root, so registering it would
    /// give two different directories one key space and let the second overwrite
    /// the first. Reachable because the identifier is a lossy string: two paths
    /// that differ only in bytes no `str` can hold render identically.
    IdentifierTaken { id: String },
}

/// The identity of an indexed file: the root it belongs to and its path relative
/// to that root.
///
/// A pair rather than one string because the root cannot be folded into the path
/// without a separator, and every separator collides with a legal configuration
/// path. It is also what the store rows are keyed by, so a key that travels as
/// one value cannot lose half of itself on the way to a lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileKey {
    pub root_id: String,
    pub path: String,
}

impl FileKey {
    pub fn new(root_id: impl Into<String>, path: impl Into<String>) -> Self {
        Self { root_id: root_id.into(), path: path.into() }
    }

    /// A file of the configuration root.
    ///
    /// A constructor for the common case, not a statement about what a corpus contains: the
    /// publisher walks every registered root, and the PostgreSQL side keys a file by
    /// `(collection, root_id, path)` throughout — lexical and semantic serving alike — storing
    /// those identities as they are.
    pub fn configuration(path: impl Into<String>) -> Self {
        Self::new(CONFIGURATION_ROOT_ID, path)
    }

    pub fn is_configuration(&self) -> bool {
        self.root_id == CONFIGURATION_ROOT_ID
    }

    /// Whether this key names a file strictly inside the directory `dir` names.
    ///
    /// Compared by whole components, not by text: a textual prefix would let `Dir`
    /// swallow `Dir2`, and the two are unrelated directories. Roots must match — the
    /// same relative path under two roots is two different files.
    pub fn is_under(&self, dir: &FileKey) -> bool {
        self.root_id == dir.root_id && starts_at(Path::new(&self.path), Path::new(&dir.path))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Root {
    id: String,
    /// The spelling the project declared, which is what exists on disk for a
    /// caller that wants to read the file back.
    declared: PathBuf,
    /// The spelling every topology decision is made on.
    canonical: PathBuf,
}

/// The registered source roots of one workspace.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceRoots {
    /// The workspace itself. Not a source root — root ids are relative to it,
    /// and callers that speak of "the project directory" (the graph, the
    /// resident host) mean this and not the configuration root, which may sit in
    /// a subdirectory.
    workspace: PathBuf,
    /// The workspace's own spelling with every link resolved, read off the disk ONCE
    /// when this table was built.
    ///
    /// It is what a durable path-keyed id is stripped against: the file universe of the
    /// generation this table describes is enumerated canonically, so only the canonical
    /// workspace is a prefix of those paths. Kept here rather than re-read per use
    /// because it is a fact about THIS generation — a root reached through a link that
    /// is later retargeted names a different directory, and a base read at answer time
    /// would then decode ids the generation never minted.
    workspace_canonical: PathBuf,
    roots: Vec<Root>,
    /// Subtrees inside the roots that a scan must not descend into.
    ///
    /// Carried here because the roots are what every scan of this workspace is driven
    /// from, and a scan that walked a hole the watcher does not follow would index
    /// files whose later edits never arrive — present in the corpus, frozen at the
    /// content of the last full walk. The value belongs to whoever owns the subtree
    /// (the server's derived cache); this type only carries it to the walk.
    ///
    /// Not part of root identity: the transition machinery compares roots, and a cache
    /// moved without the roots moving is not a topology change.
    excluded: Vec<PathBuf>,
}

/// Equality is the ROOTS, and the doc on `excluded` is what the machinery around this
/// type acts on: whole values are compared in both directions — as "the roots did not
/// move, take the fast path" and as "these are not the roots the plan was made for,
/// drop it". A derived `PartialEq` puts the cache path on both of those, so the same
/// unrelated difference would force a full root transition in one place and discard a
/// valid transition in another.
///
/// `workspace_canonical` is compared, and it cannot be derived from what already is: a root
/// is allowed to lie OUTSIDE the workspace and be identified by its absolute spelling
/// ([`root_id_for`]), so a workspace whose link is retargeted while every registered root
/// stays put moves no other field here. Two tables that strip their ids against different
/// directories are not one registration, whatever their roots are spelled like — leaving it
/// out would let such a pair pass as "the roots did not move" and keep serving a base the
/// next build would not mint against.
impl PartialEq for WorkspaceRoots {
    fn eq(&self, other: &Self) -> bool {
        self.workspace == other.workspace
            && self.workspace_canonical == other.workspace_canonical
            && self.roots == other.roots
    }
}

impl Eq for WorkspaceRoots {}

impl WorkspaceRoots {
    /// Build the table from the configuration root and the declared extension
    /// roots, both spelled as the project declares them.
    ///
    /// `workspace_root` is what root ids are relative to. Extension roots that
    /// canonically lie inside the configuration root are left out and returned
    /// separately: those files are already reachable — and already published in
    /// a baseline — as the configuration's, and giving them a second identity
    /// would mean two rows for one file and a split from the published snapshot.
    pub fn build(
        workspace_root: &Path,
        configuration: &Path,
        extensions: &[PathBuf],
    ) -> (Self, Vec<RejectedRoot>) {
        Self::build_optional(workspace_root, Some(configuration), extensions)
    }

    /// Build a root table for a project whose base configuration is optional.
    /// Extension-only projects register only extension roots; no synthetic
    /// configuration root is introduced.
    pub fn build_optional(
        workspace_root: &Path,
        configuration: Option<&Path>,
        extensions: &[PathBuf],
    ) -> (Self, Vec<RejectedRoot>) {
        let workspace_canonical = canonicalize(workspace_root);
        let configuration_canonical = configuration.map(canonicalize);
        let mut roots = Vec::new();
        if let (Some(configuration), Some(canonical)) =
            (configuration, configuration_canonical.as_ref())
        {
            roots.push(Root {
                id: CONFIGURATION_ROOT_ID.to_owned(),
                declared: configuration.to_path_buf(),
                canonical: canonical.clone(),
            });
        }
        let mut rejected = Vec::new();

        for extension in extensions {
            let canonical = canonicalize(extension);
            if let (Some(configuration), Some(configuration_canonical)) =
                (configuration, configuration_canonical.as_ref())
            {
                if canonical == *configuration_canonical
                    || starts_at(&canonical, configuration_canonical)
                {
                    rejected.push(RejectedRoot {
                        path: extension.clone(),
                        reason: Rejection::InsideConfiguration {
                            root: configuration.to_path_buf(),
                        },
                    });
                    continue;
                }
            }
            if roots.iter().any(|root| root.canonical == canonical) {
                continue;
            }
            let id = root_id_for(&workspace_canonical, &canonical, extension);
            if roots.iter().any(|root| root.id == id) {
                rejected.push(RejectedRoot {
                    path: extension.clone(),
                    reason: Rejection::IdentifierTaken { id },
                });
                continue;
            }
            roots.push(Root { id, declared: extension.clone(), canonical });
        }

        (
            Self {
                workspace: workspace_root.to_path_buf(),
                workspace_canonical,
                roots,
                excluded: Vec::new(),
            },
            rejected,
        )
    }

    /// The root a file belongs to and the key it is stored under, or `None` when
    /// the file lies outside every root.
    ///
    /// Two spellings go in because the enumerator has two: `canonical` is what
    /// the semantics rank roots by and what makes attribution independent of
    /// which alias the walk happened to arrive through, while `walked` is the
    /// only handle on a file the walk reached through a symlink that leaves
    /// every root. Such a file is not dropped — the graph has always seen it —
    /// it is keyed by the walking root instead.
    ///
    /// Both spellings are matched against the roots BYTE for byte, which decides what
    /// happens to a path that reached the caller already rendered — the graph keeps its
    /// file paths as strings, so bytes no `str` can carry come back replaced:
    ///
    /// - rendered BELOW the deepest root that still matches, the answer is the key the row
    ///   lives under: the key of the relative part is a rendering too ([`key_of`]), so both
    ///   spellings arrive at one key. This is the ordinary case and nothing is guessed;
    /// - rendered inside a root's own name, that root stops matching. An ANCESTOR root may
    ///   still match — nested extension roots are legal — and then the answer is the
    ///   ancestor's key, while the file's own row is keyed by the inner root. Usually no row
    ///   carries that key and a mark written under it clears itself unused; where the key
    ///   space's own collision puts a real file there, that file is re-rendered needlessly —
    ///   its own context, so nothing wrong is stored — and the real one stays stale;
    /// - with no root matching at all, the answer is `None` and the caller skips its mark.
    ///
    /// What is deliberately NOT done is ranking such a path in the rendered alphabet. That
    /// was tried and taken out: the rendering is not reversible, so it fits several roots at
    /// once (two roots differing only in those bytes; a root and a neighbour holding a
    /// genuine `U+FFFD`; a nesting that exists only after rendering), and a key guessed from
    /// it does not merely mark the wrong file — this is the seam a REMOVAL resolves its keys
    /// through. What remains is the key space's own long-standing property: two names that
    /// differ only in unrepresentable bytes share a key, whichever spelling reaches it.
    pub fn root_of(&self, walked: &Path, canonical: &Path) -> Option<FileKey> {
        self.longest_match(canonical, |root| &root.canonical)
            .or_else(|| self.longest_match(walked, |root| &root.declared))
    }

    /// The owning root ranked by the DECLARED spellings alone. For a file whose canonical
    /// target must not take part in attribution (a non-source target), plain [`Self::root_of`]
    /// would still rank the walked path against the CANONICAL root spellings first — and under
    /// a root declared through a link, the walked path also lies under the enclosing root's
    /// canonical spelling, handing the key to the wrong root.
    pub(crate) fn root_of_declared(&self, walked: &Path) -> Option<FileKey> {
        self.longest_match(walked, |root| &root.declared)
    }

    /// The two spellings attribution ranks a path by.
    ///
    /// A relative path is read against the CONFIGURATION root: that is how every
    /// stored path with the reserved id is spelled, and it is the prefix callers
    /// strip before handing one over. The table's workspace exists to make root
    /// identifiers relative and is a directory higher whenever the configuration
    /// sits in a subdirectory.
    pub(crate) fn spellings_of(&self, path: &Path) -> (PathBuf, PathBuf) {
        let walked = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.configuration().unwrap_or_else(|| self.workspace()).join(path)
        };
        let canonical = canonical_spelling(&walked);
        (walked, canonical)
    }

    /// The key a workspace path carries whatever its role — the question "which
    /// root does this path belong to, and where in it" asked of a path that is
    /// not a stored document: a metadata descriptor, a directory.
    ///
    /// The role branch [`crate::SearchEngine`] applies to a `.bsl` has no meaning
    /// here: it exists to keep a key REMOVABLE under the spelling its file was
    /// indexed with, and nothing is indexed under this one. Where the path
    /// physically lies is the whole answer, so the canonical spelling ranks
    /// first, as it does for a live file.
    pub fn key_of_path(&self, path: &Path) -> Option<FileKey> {
        let (walked, canonical) = self.spellings_of(path);
        self.root_of(&walked, &canonical)
    }

    /// The file a stored key points at, spelled as the project declared it.
    pub fn resolve(&self, key: &FileKey) -> Option<PathBuf> {
        let root = self.roots.iter().find(|root| root.id == key.root_id)?;
        validate_relative_key_path(&key.path)?;
        Some(root.declared.join(&key.path))
    }

    /// The file a stored key points at, spelled as the WALK reached it.
    ///
    /// [`Self::resolve`] is for opening the file, and the declared spelling is what a reader
    /// opens. This one is for naming it: a durable path-keyed id is the walk's spelling minus
    /// the workspace, and the walk descended from the canonical roots this table fixed when it
    /// was built. Asking the disk for the same answer at use time would resolve the DECLARED
    /// spelling through whatever the links point at now, which is a different file whenever one
    /// of them has been retargeted — and then the id names a node this generation never held.
    pub fn resolve_walked(&self, key: &FileKey) -> Option<PathBuf> {
        let root = self.roots.iter().find(|root| root.id == key.root_id)?;
        validate_relative_key_path(&key.path)?;
        Some(root.canonical.join(&key.path))
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// The workspace spelling a durable path-keyed id is stripped against — see the field.
    pub fn workspace_canonical(&self) -> &Path {
        &self.workspace_canonical
    }

    /// The declared spelling of the configuration root.
    pub fn configuration(&self) -> Option<&Path> {
        self.roots
            .iter()
            .find(|root| root.id == CONFIGURATION_ROOT_ID)
            .map(|root| root.declared.as_path())
    }

    /// Whether a root is registered under this id.
    pub fn contains_id(&self, root_id: &str) -> bool {
        self.roots.iter().any(|root| root.id == root_id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.roots.iter().map(|root| root.id.as_str())
    }

    /// Root ids present in both tables whose directory binding changed.
    ///
    /// The id alone survives a move: the configuration keeps the empty id wherever its directory
    /// is, and an extension alias can be retargeted. A rebound id keeps its key space, so matching
    /// bytes may keep their rows — but nothing learned from the old directory about a key whose
    /// new bytes were not compared may be kept.
    pub(crate) fn rebound_root_ids(&self, next: &Self) -> std::collections::HashSet<String> {
        self.roots
            .iter()
            .filter(|root| {
                next.roots
                    .iter()
                    .find(|candidate| candidate.id == root.id)
                    .is_some_and(|candidate| candidate != *root)
            })
            .map(|root| root.id.clone())
            .collect()
    }

    /// Registered roots as `(id, declared path)`, in registration order.
    /// Take the subtrees a scan of these roots must not descend into.
    ///
    /// A builder step rather than a `build` parameter: every one of the hundred-odd
    /// call sites that has no cache to speak of keeps working unchanged, and the few
    /// that own one say so explicitly.
    #[must_use]
    pub fn with_excluded(mut self, excluded: Vec<PathBuf>) -> Self {
        self.excluded = excluded;
        self
    }

    /// See [`Self::with_excluded`].
    pub fn excluded(&self) -> &[PathBuf] {
        &self.excluded
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.roots.iter().map(|root| (root.id.as_str(), root.declared.as_path()))
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// The root with the longest matching prefix, under the given spelling.
    ///
    /// Longest rather than first: roots may nest, and the innermost one is what
    /// the semantics call the file's owner. The configuration competes like any
    /// other root, so an extension declared *around* it keeps only the files
    /// outside it.
    fn longest_match(&self, path: &Path, spelling: impl Fn(&Root) -> &PathBuf) -> Option<FileKey> {
        self.roots
            .iter()
            .filter_map(|root| {
                let rel = path.strip_prefix(spelling(root)).ok()?;
                // `strip_prefix` compares whole components, so `cf` never
                // swallows `cf_ext`. An empty remainder means the path *is* the
                // root, which is not a file in it.
                (rel.components().next().is_some()).then(|| {
                    (spelling(root).components().count(), FileKey::new(&root.id, key_of(rel)))
                })
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, key)| key)
    }
}

/// A persisted file key is data, not a path supplied by the current process.
/// Keep the check in this module so every graph/search reader gets the same
/// rejection rules before joining it to a root.  In particular, `Path::join`
/// would otherwise accept an absolute right hand side and escape the declared
/// root, while a parent component could reach an unrelated directory.
fn validate_relative_key_path(path: &str) -> Option<()> {
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    // Durable keys are emitted with the current platform's separator, but a key
    // may have been persisted on Windows and inspected on another platform.  Do
    // the security check for both separator conventions before asking `Path` to
    // validate the native spelling; otherwise Linux treats `..\\outside` as one
    // ordinary filename and Windows can later escape the declared root with it.
    if path.starts_with('/')
        || path.starts_with('\\')
        || path.as_bytes().get(1) == Some(&b':')
        || path.contains(':')
    {
        return None;
    }
    if path
        .split(['/', '\\'])
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return None;
    }
    if Path::new(path)
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(())
}

/// The store key of a path relative to its root.
fn key_of(rel: &Path) -> String {
    rel.to_string_lossy().into_owned()
}

fn canonicalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `path` lies strictly inside `prefix`.
fn starts_at(path: &Path, prefix: &Path) -> bool {
    path.strip_prefix(prefix).is_ok_and(|rest| rest.components().next().is_some())
}

/// A root's identity: its path relative to the workspace, or absolute when it
/// lies outside.
///
/// Not the extension's name — several entries are allowed to share one — and
/// not an ordinal, which would not survive a restart.
fn root_id_for(workspace_canonical: &Path, canonical: &Path, declared: &Path) -> String {
    let relative = canonical.strip_prefix(workspace_canonical).ok().map(key_of);
    match relative {
        // A root spelled `.` relative to the workspace would take the
        // configuration's reserved empty id and overwrite its rows.
        Some(relative) if relative.is_empty() => ".".to_owned(),
        Some(relative) => relative,
        None if canonical.is_absolute() => key_of(canonical),
        // Neither under the workspace nor absolute: keep the declared spelling
        // rather than invent one, and make sure it cannot be read as empty.
        None => {
            let declared = key_of(declared);
            if declared.is_empty() || declared == "." {
                ".".to_owned()
            } else {
                declared
            }
        }
    }
}

/// The canonical spelling of a path, falling back to the canonical spelling of
/// its directory when the file itself is gone.
///
/// Deletion is the case this exists for: a removed file cannot be canonicalized,
/// and dropping all the way to the walked spelling would leave attribution
/// ranking roots by their declared paths alone. A file that lived under a root
/// reached through an alias would then be removed under a DIFFERENT root's key —
/// tombstone and all — while its real row stayed behind serving a dead hit.
pub(crate) fn canonical_spelling(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent)
            .map(|parent| parent.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roots must exist on disk for the canonical spelling to mean anything, so
    /// every fixture builds real directories.
    fn dirs(root: &Path, rels: &[&str]) -> Vec<PathBuf> {
        rels.iter()
            .map(|rel| {
                let path = root.join(rel);
                std::fs::create_dir_all(&path).unwrap();
                path
            })
            .collect()
    }

    /// The fixture root, spelled canonically.
    ///
    /// A temporary directory may itself be reached through a link (`/var` is one to
    /// `/private/var` on macOS). A fixture that mixes directories that exist with names
    /// no filesystem can hold would then carry two spellings of one directory: `build`
    /// canonicalizes what it can reach and keeps the declared spelling of the rest.
    #[cfg(unix)]
    fn canonical_root(dir: &tempfile::TempDir) -> PathBuf {
        std::fs::canonicalize(dir.path()).unwrap()
    }

    /// Two tables that strip their ids against different directories are not one
    /// registration, even when every root path is spelled identically in both.
    ///
    /// A root may lie outside the workspace and be identified by its absolute spelling, and
    /// then a workspace link retargeted onto another directory moves nothing else here. What
    /// it does move is the base every path-keyed id is stripped against, so a comparison that
    /// missed it would report "the roots did not move" and keep serving a base the next build
    /// would not mint against.
    #[cfg(unix)]
    #[test]
    fn a_workspace_that_resolves_elsewhere_is_another_registration() {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical_root(&dir);
        let configuration = dirs(&root, &["a/sub/cf"]).remove(0);
        let workspace = root.join("workspace");
        std::os::unix::fs::symlink(root.join("a"), &workspace).unwrap();
        let (before, _) = WorkspaceRoots::build(&workspace, &configuration, &[]);

        std::fs::remove_file(&workspace).unwrap();
        std::os::unix::fs::symlink(root.join("a/sub"), &workspace).unwrap();
        let (after, _) = WorkspaceRoots::build(&workspace, &configuration, &[]);

        assert_eq!(
            before.workspace(),
            after.workspace(),
            "control: the workspace is declared by the same path in both",
        );
        assert_ne!(
            before.workspace_canonical(),
            after.workspace_canonical(),
            "control: and the link makes that one path name two directories",
        );
        assert_ne!(before, after, "so the two tables are not the same registration");
    }

    /// A directory named by bytes no `str` can hold, created where the filesystem takes
    /// such a name.
    ///
    /// APFS keeps names in valid UTF-8 and refuses anything else with `EILSEQ`, so on
    /// macOS this directory cannot exist and the path is all there is of it. Attribution
    /// answers the same either way: `build` falls back to the declared spelling of a path
    /// it cannot canonicalize, and under a canonical root that spelling IS the canonical
    /// one.
    #[cfg(unix)]
    fn unrepresentable_dir(path: PathBuf) -> PathBuf {
        assert!(path.to_str().is_none(), "the fixture is about bytes no `str` can carry");
        let _ = std::fs::create_dir_all(&path);
        path
    }

    /// The attribution of a file that exists only as a path: both spellings are
    /// the same when no alias is involved.
    fn owner(roots: &WorkspaceRoots, file: &Path) -> Option<FileKey> {
        roots.root_of(file, file)
    }

    const MODULE: &str = "CommonModules/М/Ext/Module.bsl";

    /// Where the cache lives is not a topology fact, and equality is read in both
    /// directions: as "the roots did not move, take the fast path" and as "these are
    /// not the roots this plan was made for, drop it". Counting the cache in would
    /// force a full root transition on one path and discard a valid transition on the
    /// other — from the same unrelated difference.
    #[test]
    fn where_the_cache_lives_is_not_part_of_root_identity() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["cf", "cfe/one"]);
        let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone()]);

        let with_cache = roots.clone().with_excluded(vec![dir.path().join(".build")]);
        let with_another = roots.clone().with_excluded(vec![dir.path().join("elsewhere")]);
        assert_eq!(with_cache, with_another, "the cache path entered root identity");
        assert_eq!(with_cache, roots, "declaring a cache at all entered root identity");

        // Positive control: the roots themselves still decide, or the assertions above
        // would hold on a build where every table equals every other.
        let (moved, _) = WorkspaceRoots::build(dir.path(), &made[1], &[]);
        assert_ne!(with_cache, moved, "two different root sets compared equal");
    }

    #[test]
    fn optional_roots_do_not_invent_a_configuration_for_extension_only_projects() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["Расширения/FeatureA", "Расширения/FeatureB"]);
        let (roots, rejected) =
            WorkspaceRoots::build_optional(dir.path(), None, &[made[0].clone(), made[1].clone()]);

        assert!(rejected.is_empty());
        assert!(roots.configuration().is_none(), "workspace root is not a synthetic configuration");
        let entries: Vec<_> =
            roots.entries().map(|(id, path)| (id.to_owned(), path.to_path_buf())).collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            owner(&roots, &made[0].join(MODULE)),
            Some(FileKey::new("Расширения/FeatureA", MODULE))
        );
        assert_eq!(
            owner(&roots, &made[1].join(MODULE)),
            Some(FileKey::new("Расширения/FeatureB", MODULE))
        );
    }

    #[test]
    fn each_root_keeps_its_own_copy_of_the_same_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["cf", "cfe/one", "cfe/two"]);
        let (roots, rejected) =
            WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone(), made[2].clone()]);

        assert!(rejected.is_empty(), "roots beside the configuration are all registered");
        for (root, expected_id) in
            [(&made[0], CONFIGURATION_ROOT_ID), (&made[1], "cfe/one"), (&made[2], "cfe/two")]
        {
            assert_eq!(
                owner(&roots, &root.join(MODULE)),
                Some(FileKey::new(expected_id, MODULE.to_owned())),
                "the same relative path under {root:?} must key to its own root"
            );
        }
    }

    #[test]
    fn the_innermost_root_owns_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["cf", "cfe/outer", "cfe/outer/inner"]);
        let (roots, _) =
            WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone(), made[2].clone()]);

        assert_eq!(
            owner(&roots, &made[2].join(MODULE)),
            Some(FileKey::new("cfe/outer/inner", MODULE.to_owned())),
            "the nested root is the owner, not the one that merely contains it"
        );
        assert_eq!(
            owner(&roots, &made[1].join(MODULE)),
            Some(FileKey::new("cfe/outer", MODULE.to_owned())),
            "a file outside the nested root still belongs to the outer one"
        );
    }

    /// Prefix matching that ignored component boundaries would hand every file
    /// of `cf_ext` to `cf`, keyed by a path starting with `_ext/`.
    #[test]
    fn a_root_never_swallows_a_sibling_whose_name_starts_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["cf", "cf_ext"]);
        let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone()]);

        assert_eq!(
            owner(&roots, &made[1].join(MODULE)),
            Some(FileKey::new("cf_ext", MODULE.to_owned())),
            "`cf` must not claim `cf_ext`"
        );
    }

    /// Declaration order decides nothing: the roots are ranked by how deep they
    /// reach, exactly as the semantics rank them.
    #[test]
    fn attribution_does_not_depend_on_declaration_order() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["cf", "cfe/outer", "cfe/outer/inner"]);
        let file = made[2].join(MODULE);

        let (forward, _) =
            WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone(), made[2].clone()]);
        let (backward, _) =
            WorkspaceRoots::build(dir.path(), &made[0], &[made[2].clone(), made[1].clone()]);

        assert_eq!(owner(&forward, &file), owner(&backward, &file));
    }

    #[test]
    fn an_extension_inside_the_configuration_is_reported_and_left_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["cf", "cf/nested"]);
        let (roots, rejected) = WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone()]);

        assert_eq!(
            rejected,
            vec![RejectedRoot {
                path: made[1].clone(),
                reason: Rejection::InsideConfiguration { root: made[0].clone() },
            }],
            "the overlap must be named, not swallowed"
        );
        // Not lost: the configuration walk is recursive, so the file is still
        // found — as the configuration's, which is what the publisher and the
        // graph enumeration call it too.
        assert_eq!(
            owner(&roots, &made[1].join(MODULE)),
            Some(FileKey::new(CONFIGURATION_ROOT_ID, format!("nested/{MODULE}"))),
        );
    }

    /// The other direction of the same overlap: an extension declared *around*
    /// the configuration. Rejecting it would lose the files outside the
    /// configuration entirely — its walk never reaches them.
    #[test]
    fn an_extension_around_the_configuration_keeps_what_lies_outside_it() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["src/cf", "src/own"]);
        let workspace = dir.path().to_path_buf();
        let (roots, rejected) =
            WorkspaceRoots::build(&workspace, &made[0], std::slice::from_ref(&workspace));

        assert!(rejected.is_empty(), "containing the configuration is not an overlap to reject");
        assert_eq!(
            owner(&roots, &made[0].join(MODULE)),
            Some(FileKey::new(CONFIGURATION_ROOT_ID, MODULE.to_owned())),
            "the configuration's own subtree stays its own"
        );
        assert_eq!(
            owner(&roots, &made[1].join(MODULE)),
            Some(FileKey::new(".", format!("src/own/{MODULE}"))),
            "everything outside it belongs to the surrounding root"
        );
    }

    /// A root equal to the workspace must not take the configuration's reserved
    /// id: the two would then share one key space, and the second write would
    /// overwrite the first.
    #[test]
    fn a_root_spanning_the_workspace_gets_a_non_empty_id() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["src/cf"]);
        let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[dir.path().to_path_buf()]);

        let ids: Vec<&str> = roots.ids().collect();
        assert_eq!(ids, vec![CONFIGURATION_ROOT_ID, "."]);
    }

    #[test]
    fn a_root_equal_to_the_configuration_stays_the_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["cf"]);
        let (roots, rejected) = WorkspaceRoots::build(dir.path(), &made[0], &[made[0].clone()]);

        assert_eq!(roots.ids().collect::<Vec<_>>(), vec![CONFIGURATION_ROOT_ID]);
        assert_eq!(rejected.len(), 1, "the duplicate root is reported");
        assert_eq!(
            owner(&roots, &made[0].join(MODULE)),
            Some(FileKey::new(CONFIGURATION_ROOT_ID, MODULE.to_owned())),
            "one file must not produce both a configuration row and an extension row"
        );
    }

    /// The identifier is a lossy rendering of a path, so two directories that
    /// differ only in bytes no `str` can hold render the same. Registering both
    /// would give one key space to two roots: the second's rows would overwrite
    /// the first's, and `resolve` would hand back the wrong directory for either.
    #[cfg(unix)]
    #[test]
    fn a_root_whose_identifier_is_already_taken_is_rejected() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let workspace = canonical_root(&dir);
        let made = dirs(&workspace, &["cf"]);
        let first = unrepresentable_dir(workspace.join(OsString::from_vec(vec![b'a', 0x80])));
        let second = unrepresentable_dir(workspace.join(OsString::from_vec(vec![b'a', 0x81])));

        let (roots, rejected) =
            WorkspaceRoots::build(&workspace, &made[0], &[first.clone(), second.clone()]);

        let ids: Vec<&str> = roots.ids().collect();
        assert_eq!(ids.len(), 2, "the configuration and exactly one of the two: {ids:?}");
        assert_eq!(
            rejected,
            vec![RejectedRoot {
                path: second,
                reason: Rejection::IdentifierTaken { id: ids[1].to_owned() },
            }],
            "the dropped root is named, not swallowed"
        );
        assert_eq!(
            roots.resolve(&FileKey::new(ids[1], MODULE)).as_deref(),
            Some(first.join(MODULE).as_path()),
            "the surviving root keeps its own directory"
        );
    }

    #[test]
    fn a_key_resolves_back_to_the_file_it_came_from() {
        let dir = tempfile::tempdir().unwrap();
        let made = dirs(dir.path(), &["cf", "cfe/one"]);
        let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone()]);

        for root in [&made[0], &made[1]] {
            let file = root.join(MODULE);
            let key = owner(&roots, &file).unwrap();
            assert_eq!(roots.resolve(&key).as_deref(), Some(file.as_path()));
        }
    }

    #[cfg(unix)]
    mod aliases {
        use super::*;

        fn link(target: &Path, at: &Path) {
            std::os::unix::fs::symlink(target, at).unwrap();
        }

        /// Reached through `one/Linked`, the file physically lives under `two`.
        /// Ranking by the canonical spelling is what makes the answer the same
        /// as the semantics', and the same in both declaration orders.
        #[test]
        fn an_alias_does_not_move_a_file_to_the_root_it_was_reached_through() {
            let dir = tempfile::tempdir().unwrap();
            let made = dirs(dir.path(), &["cf", "cfe/one", "cfe/two"]);
            link(&made[2], &made[1].join("Linked"));

            let (roots, _) =
                WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone(), made[2].clone()]);

            let walked = made[1].join("Linked").join(MODULE);
            // Read off the filesystem, as the enumerator reads it: joining onto the
            // DECLARED root would carry over whatever links lead to the fixture itself
            // (`/var` → `/private/var` on macOS) and hand `root_of` a spelling that is
            // not canonical at all, which is the one question this test does not ask.
            let canonical = std::fs::canonicalize(&made[2]).unwrap().join(MODULE);
            assert_eq!(
                roots.root_of(&walked, &canonical),
                Some(FileKey::new("cfe/two", MODULE.to_owned())),
                "the root the file lives in owns it, not the one that links to it"
            );
        }

        /// A link out of every root. The graph has always followed it, so the
        /// file must stay in the universe; with no canonical owner it is kept by
        /// the root whose walk arrived there, keyed by the walked spelling.
        #[test]
        fn a_file_outside_every_root_is_kept_by_the_root_that_walked_to_it() {
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let made = dirs(dir.path(), &["cf"]);
            std::fs::create_dir_all(outside.path().join("tree")).unwrap();
            link(&outside.path().join("tree"), &made[0].join("Linked"));

            let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[]);

            let walked = made[0].join("Linked").join(MODULE);
            let canonical =
                std::fs::canonicalize(outside.path()).unwrap().join("tree").join(MODULE);
            assert_eq!(
                roots.root_of(&walked, &canonical),
                Some(FileKey::new(CONFIGURATION_ROOT_ID, format!("Linked/{MODULE}"))),
            );
        }

        /// An extension declared through an alias that sits inside the
        /// configuration, while the extension itself lies outside it. Deciding
        /// on the declared spelling would reject the root and lose its files.
        #[test]
        fn an_extension_declared_through_an_alias_inside_the_configuration_is_registered() {
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let made = dirs(dir.path(), &["cf"]);
            let real = outside.path().join("ext");
            std::fs::create_dir_all(&real).unwrap();
            let alias = made[0].join("Linked");
            link(&real, &alias);

            let (roots, rejected) =
                WorkspaceRoots::build(dir.path(), &made[0], std::slice::from_ref(&alias));

            assert!(
                rejected.is_empty(),
                "only the alias is inside the configuration, not the root"
            );
            let file = alias.join(MODULE);
            let canonical = std::fs::canonicalize(&real).unwrap().join(MODULE);
            let key = roots.root_of(&file, &canonical).unwrap();
            assert_ne!(key.root_id, CONFIGURATION_ROOT_ID, "the extension keeps its own identity");
            assert_eq!(key.path, MODULE);
        }
    }

    /// Attribution of a path that is not a stored document: a metadata descriptor asks the
    /// same question a `.bsl` does, and must not get a second answer.
    mod any_path {
        use super::*;

        #[test]
        fn a_descriptor_belongs_to_the_root_it_lies_in() {
            let dir = tempfile::tempdir().unwrap();
            let made = dirs(dir.path(), &["cf", "cfe/one"]);
            let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[made[1].clone()]);

            assert_eq!(
                roots.key_of_path(&made[1].join("Configuration.xml")),
                Some(FileKey::new("cfe/one", "Configuration.xml".to_owned())),
            );
            assert_eq!(
                roots.key_of_path(&made[0].join("Catalogs/Товары.xml")),
                Some(FileKey::configuration("Catalogs/Товары.xml")),
            );
        }

        /// The reading a relative path gets everywhere in the engine: the configuration
        /// root, which is where every stored path with the reserved id is spelled from.
        #[test]
        fn a_relative_path_is_read_against_the_configuration_root() {
            let dir = tempfile::tempdir().unwrap();
            let made = dirs(dir.path(), &["src/cf"]);
            let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[]);

            assert_eq!(
                roots.key_of_path(Path::new("Configuration.xml")),
                Some(FileKey::configuration("Configuration.xml")),
            );
        }

        #[test]
        fn a_path_outside_every_root_has_no_key() {
            let dir = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let made = dirs(dir.path(), &["cf"]);
            let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[]);

            assert_eq!(roots.key_of_path(&outside.path().join("Configuration.xml")), None);
        }
    }

    /// Paths that arrive already rendered, bytes no `str` can carry replaced. Where those
    /// bytes sat decides the answer: below a root the key is the row's own, in a root's own
    /// name there is no answer to give.
    #[cfg(unix)]
    mod rendered_paths {
        use super::*;
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        /// A workspace whose OWN name holds bytes no `str` can carry, holding a
        /// configuration root and one extension.
        fn workspace_named_by_bytes(dir: &tempfile::TempDir) -> (PathBuf, Vec<PathBuf>) {
            let workspace = unrepresentable_dir(
                canonical_root(dir).join(OsString::from_vec(b"ws\xff".to_vec())),
            );
            let made =
                ["cf", "cfe"].iter().map(|rel| unrepresentable_dir(workspace.join(rel))).collect();
            (workspace, made)
        }

        /// Below the root the rendering costs nothing, because the stored key is a rendering
        /// too: `key_of` puts the relative path through the same conversion, so the real
        /// bytes and what came back from a string-keyed store name ONE key — the one the row
        /// lives under. What stays true of that key space is what has always been true of it:
        /// two files whose names differ only in unrepresentable bytes share a key, whichever
        /// spelling reaches it.
        #[test]
        fn a_rendering_below_the_root_names_the_key_the_row_lives_under() {
            let dir = tempfile::tempdir().unwrap();
            let made = dirs(dir.path(), &["cf"]);
            let (roots, _) = WorkspaceRoots::build(dir.path(), &made[0], &[]);

            let file =
                made[0].join(OsString::from_vec(b"CommonModules/M\xff/Ext/Module.bsl".to_vec()));
            let rendered = PathBuf::from(file.to_string_lossy().into_owned());
            assert_eq!(
                owner(&roots, &rendered),
                owner(&roots, &file),
                "both spellings key the same row",
            );
            assert!(owner(&roots, &file).is_some(), "and that row exists to be keyed");
        }

        /// Roots may nest, so a rendering that unseats the inner root can still land on an
        /// ancestor. The file's own row stays keyed by the inner root, so the key this
        /// produces normally carries no row at all. Said plainly here because the shorter
        /// reading — that such a path attributes to nothing — is what a tidier contract
        /// would promise and the code does not deliver.
        #[test]
        fn a_rendering_inside_a_nested_roots_name_lands_on_the_ancestor() {
            let dir = tempfile::tempdir().unwrap();
            let workspace = canonical_root(&dir);
            let made = dirs(&workspace, &["cf", "ext"]);
            let inner =
                unrepresentable_dir(made[1].join(OsString::from_vec(b"d\xff".to_vec())).join("e"));
            let (roots, rejected) =
                WorkspaceRoots::build(&workspace, &made[0], &[made[1].clone(), inner.clone()]);
            assert!(rejected.is_empty(), "an extension inside an extension is a root of its own");

            let file = inner.join(MODULE);
            assert_eq!(
                owner(&roots, &file),
                Some(FileKey::new("ext/d\u{FFFD}/e", MODULE.to_owned())),
                "the real bytes key the file under the innermost root",
            );
            let rendered = PathBuf::from(file.to_string_lossy().into_owned());
            assert_eq!(
                owner(&roots, &rendered),
                Some(FileKey::new("ext", format!("d\u{FFFD}/e/{MODULE}"))),
                "the rendering keeps the ancestor, whose key no row carries",
            );
        }

        /// A rendering of the ROOT's own name fits several roots at once — two that differ
        /// only in those bytes, a root and a neighbour holding a genuine `U+FFFD`, a nesting
        /// that exists only after rendering — so it names no file anyone may act on. Every
        /// caller already reads `None` as "not ours": the graph-sourced context mark is
        /// skipped and the module waits for a wider mark. Marks driven by the watcher carry
        /// the real bytes and are untouched by this.
        #[test]
        fn a_path_that_arrives_rendered_belongs_to_no_root() {
            let dir = tempfile::tempdir().unwrap();
            let (workspace, made) = workspace_named_by_bytes(&dir);
            let (roots, _) = WorkspaceRoots::build(&workspace, &made[0], &[made[1].clone()]);

            let file = made[1].join(MODULE);
            assert_eq!(
                owner(&roots, &file),
                Some(FileKey::new("cfe", MODULE.to_owned())),
                "the real bytes attribute as always",
            );

            let rendered = PathBuf::from(file.to_string_lossy().into_owned());
            assert_eq!(owner(&roots, &rendered), None, "the rendering names no file we can act on");
        }

        /// The identifier of such a root IS a rendering, and that is not the same question:
        /// it names a key space, it is never resolved back to bytes.
        #[test]
        fn the_identifier_of_an_unrepresentable_root_is_still_usable() {
            let dir = tempfile::tempdir().unwrap();
            let (workspace, made) = workspace_named_by_bytes(&dir);
            let (roots, _) = WorkspaceRoots::build(&workspace, &made[0], &[made[1].clone()]);

            let key = owner(&roots, &made[1].join(MODULE)).unwrap();
            assert_eq!(roots.resolve(&key).as_deref(), Some(made[1].join(MODULE).as_path()));
        }
    }

    #[test]
    fn rebound_ids_are_the_kept_ids_whose_directory_changed() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        let old_made = dirs(&workspace, &["old-cf", "ext/kept", "ext/removed"]);
        let new_made = dirs(&workspace, &["new-cf", "ext/kept", "ext/added"]);
        let old = WorkspaceRoots::build(
            &workspace,
            &old_made[0],
            &[old_made[1].clone(), old_made[2].clone()],
        )
        .0;
        let next = WorkspaceRoots::build(
            &workspace,
            &new_made[0],
            &[new_made[1].clone(), new_made[2].clone()],
        )
        .0;

        let rebound = old.rebound_root_ids(&next);
        assert!(rebound.contains(CONFIGURATION_ROOT_ID), "the stable empty id was rebound");
        assert!(!rebound.contains("ext/kept"));
        assert!(!rebound.contains("ext/removed"), "a removed id is not rebound");
        assert!(!rebound.contains("ext/added"), "an added id is not rebound");
    }

    mod containment {
        use super::*;

        #[test]
        fn a_directory_contains_its_files_at_any_depth_but_not_itself() {
            let dir = FileKey::configuration("Dir");
            assert!(FileKey::configuration("Dir/A.bsl").is_under(&dir));
            assert!(FileKey::configuration("Dir/Deep/B.bsl").is_under(&dir));
            assert!(!dir.is_under(&dir), "a directory is not a file inside itself");
        }

        /// The whole reason containment is compared by components: `Dir2` merely starts
        /// with the same text and is an unrelated directory.
        #[test]
        fn a_namesake_directory_is_not_inside() {
            assert!(!FileKey::configuration("Dir2/A.bsl").is_under(&FileKey::configuration("Dir")));
        }

        /// The same relative path under two roots is two different files, so a removal
        /// aimed at one root must not reach the other's.
        #[test]
        fn containment_does_not_cross_roots() {
            let file = FileKey::new("ext-1", "Dir/A.bsl");
            assert!(file.is_under(&FileKey::new("ext-1", "Dir")));
            assert!(!file.is_under(&FileKey::configuration("Dir")));
        }
    }

    #[test]
    fn resolve_rejects_native_and_foreign_absolute_or_parent_paths() {
        let dir = tempfile::tempdir().unwrap();
        let configuration = dirs(dir.path(), &["cf"]).remove(0);
        let (roots, _) = WorkspaceRoots::build(dir.path(), &configuration, &[]);

        for path in [
            "/outside/file.bsl",
            "\\outside\\file.bsl",
            "C:/outside/file.bsl",
            r"C:\\outside\\file.bsl",
            r"\\server\share\file.bsl",
            "nested/../outside/file.bsl",
            r"nested\..\outside\file.bsl",
            "nested/./file.bsl",
            r"nested\.\file.bsl",
        ] {
            assert_eq!(
                roots.resolve(&FileKey::configuration(path)),
                None,
                "malformed persisted path must not be joined to a root: {path:?}",
            );
        }
    }

    #[test]
    fn resolve_accepts_a_relative_path_in_either_persisted_separator_form() {
        let dir = tempfile::tempdir().unwrap();
        let configuration = dirs(dir.path(), &["cf"]).remove(0);
        let (roots, _) = WorkspaceRoots::build(dir.path(), &configuration, &[]);

        assert_eq!(
            roots.resolve(&FileKey::configuration("nested/file.bsl")),
            Some(configuration.join("nested/file.bsl")),
        );
        assert_eq!(
            roots.resolve(&FileKey::configuration(r"nested\file.bsl")),
            Some(configuration.join(r"nested\file.bsl")),
        );
    }
}
