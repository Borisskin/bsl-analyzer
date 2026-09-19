mod common;

use common::*;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn partitioned_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for source in ["src/cf", "src/cfe/A", "src/cfe/B"] {
        std::fs::create_dir_all(dir.path().join(source)).unwrap();
        std::fs::write(dir.path().join(source).join("Configuration.xml"), "<Configuration/>")
            .unwrap();
    }
    for path in ["src/cf/Main.bsl", "src/cfe/A/A.bsl", "src/cfe/B/B.bsl"] {
        std::fs::write(dir.path().join(path), BROKEN).unwrap();
    }
    std::fs::write(
        dir.path().join("bsl-analyzer.toml"),
        r#"[source]
root = "src/cf"
extensions = [{ name = "A", path = "src/cfe/A" }, { name = "B", path = "src/cfe/B" }]

[diagnostics.baseline]
directory = "baselines"

[[diagnostics.baseline.groups]]
name = "vendor"
extensions = ["A"]
"#,
    )
    .unwrap();
    let created = Command::new(env!("CARGO_BIN_EXE_bsl-analyzer-app"))
        .current_dir(dir.path())
        .args(["diagnostics", "baseline", "create", "-s", "."])
        .output()
        .unwrap();
    assert!(created.status.success(), "{}", String::from_utf8_lossy(&created.stderr));
    dir
}

/// The first publication of each path, which must be empty or not as `empty` says; returned by
/// uri so a later phase can compare against it.
fn expect_publications(
    lsp: &mut Lsp,
    root: &Path,
    paths: &[&str],
    empty: bool,
) -> std::collections::BTreeMap<String, Value> {
    let mut seen = std::collections::BTreeMap::new();
    let mut remaining: std::collections::BTreeSet<_> = paths
        .iter()
        .map(|path| lsp_types::Url::from_file_path(root.join(path)).unwrap().to_string())
        .collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !remaining.is_empty() {
        // Poked, for the reason every wait in this file is: the baseline change these
        // publications answer may have landed before the watcher was armed and raised no
        // event, and the server notices that only when the client asks it for something.
        let Some(published) = lsp.wait_for_within(std::time::Duration::from_secs(1), |message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"].as_str().is_some_and(|uri| remaining.contains(uri))
        }) else {
            assert!(std::time::Instant::now() < deadline, "the server never republished");
            lsp.poke();
            continue;
        };
        let uri = published["params"]["uri"].as_str().unwrap();
        assert_eq!(
            published["params"]["diagnostics"].as_array().unwrap().is_empty(),
            empty,
            "unexpected publication for {uri}: {published}"
        );
        seen.insert(uri.to_owned(), published["params"]["diagnostics"].clone());
        remaining.remove(uri);
    }
    seen
}

/// After the repair every path has to come back clean — eventually, not necessarily at once.
///
/// The server asks whether the baseline moved at most once per 250 ms, and says so where it
/// asks (`Workspace::refresh_diagnostics_baseline`): a change landing inside that window can be
/// answered once from the snapshot it already holds. Nothing tells a client when the server has
/// seen the repair, so a publication computed just before that — by anything that schedules
/// diagnostics, the workspace load included — may still arrive after the files were fixed.
/// That publication is allowed to be exactly the broken state this stand already saw for the
/// path, and nothing else. Once a path has been published clean it has to stay clean, and a
/// server that never comes back fails on the deadline.
fn expect_recovery(
    lsp: &mut Lsp,
    root: &Path,
    paths: &[&str],
    broken: &std::collections::BTreeMap<String, Value>,
) {
    let watched: std::collections::BTreeSet<_> = paths
        .iter()
        .map(|path| lsp_types::Url::from_file_path(root.join(path)).unwrap().to_string())
        .collect();
    let mut remaining = watched.clone();
    let mut recovered = std::collections::BTreeSet::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !remaining.is_empty() {
        // Checked before every wait, not only after a silent one: a server repeating the
        // broken state it is allowed to repeat is never silent, and must not outlive the bound.
        let left =
            deadline.checked_duration_since(std::time::Instant::now()).unwrap_or_else(|| {
                panic!("the server never recovered from the repaired baseline: {remaining:?}")
            });
        // Poked, as every wait in this file is: the repair may raise no event, and the server
        // notices it only when the client asks for something.
        let Some(published) =
            lsp.wait_for_within(left.min(std::time::Duration::from_secs(1)), |message| {
                message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["uri"].as_str().is_some_and(|uri| watched.contains(uri))
            })
        else {
            lsp.poke();
            continue;
        };
        let uri = published["params"]["uri"].as_str().unwrap().to_owned();
        let diagnostics = &published["params"]["diagnostics"];
        if diagnostics.as_array().unwrap().is_empty() {
            remaining.remove(&uri);
            recovered.insert(uri);
            continue;
        }
        assert!(
            !recovered.contains(&uri),
            "{uri} was published unsuppressed after it had recovered: {published}"
        );
        assert_eq!(
            Some(diagnostics),
            broken.get(&uri),
            "before the server saw the repair, {uri} may only repeat the broken state it had: \
             {published}"
        );
    }
}

#[test]
fn partitioned_baseline_lsp_main_extension_group_partial_and_recovery() {
    let dir = partitioned_project();
    let root = dir.path();
    let mut lsp = Lsp::start(root);
    let paths = ["src/cf/Main.bsl", "src/cfe/A/A.bsl", "src/cfe/B/B.bsl"];
    for path in paths {
        let published = lsp.open(&root.join(path), BROKEN);
        assert!(
            published["params"]["diagnostics"].as_array().unwrap().is_empty(),
            "owner baseline must suppress {path}: {published}"
        );
    }

    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("baselines/manifest.json")).unwrap())
            .unwrap();
    let object = manifest["partitions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["partition_id"] == "group:vendor")
        .unwrap()["file"]
        .as_str()
        .unwrap();
    let object_relative = object.to_owned();
    let object = root.join("baselines").join(&object_relative);
    let valid = std::fs::read(&object).unwrap();
    // Broken ONCE. The watcher is armed asynchronously, after the loader has already
    // announced the load finished, so this write may raise no event at all — and the
    // server has to notice it anyway, the next time the client asks it for anything.
    std::fs::write(&object, b"{broken").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while lsp
        .wait_for_within(std::time::Duration::from_secs(1), |message| {
            message["method"] == "window/showMessage"
        })
        .is_none()
    {
        assert!(std::time::Instant::now() < deadline, "the server never saw the broken object");
        lsp.poke();
    }
    let broken = expect_publications(&mut lsp, root, &paths, false);

    let directory =
        project_model::ManagedBaselineDirectory::open(root, "baselines", false).unwrap();
    directory.create_file_new("replacement.tmp").unwrap().write_all(&valid).unwrap();
    directory.replace_file("replacement.tmp", &object_relative).unwrap();
    expect_recovery(&mut lsp, root, &paths, &broken);
}
