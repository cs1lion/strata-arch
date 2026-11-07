use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use arch_core::callgraph::build_call_graph;
use arch_core::guided_path::{build_path, list_entries, PathError};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture() -> PathBuf {
    workspace_root().join("tests/fixtures/path")
}

#[test]
fn entries_come_from_manifest_and_public_exports() {
    let artifact = list_entries(&fixture(), "test").expect("entries");
    assert_eq!(artifact.schema_version, 1);

    let ids = artifact
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<BTreeSet<_>>();
    assert!(ids.contains("src/cli.ts#module-toplevel"));
    assert!(ids.contains("src/index.ts#module-toplevel"));
    assert!(ids.contains("src/utils/helper.ts#function:helper"));

    let cli = artifact
        .entries
        .iter()
        .find(|entry| entry.id == "src/cli.ts#module-toplevel")
        .expect("cli entry");
    assert_eq!(cli.source, "package.json:bin");
    assert_eq!(cli.layer, "unclassified");
}

#[test]
fn path_steps_are_connected_and_ordered() {
    let artifact = build_path(
        &fixture(),
        "test",
        "src/cli.ts#module-toplevel",
        "src/utils/helper.ts#function:helper",
    )
    .expect("path");

    assert_eq!(artifact.length, 3);
    assert_eq!(artifact.from, "src/cli.ts#module-toplevel");
    assert_eq!(artifact.to, "src/utils/helper.ts#function:helper");

    let sequence = artifact
        .steps
        .iter()
        .map(|step| step.symbol.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        sequence,
        vec![
            "src/cli.ts#module-toplevel",
            "src/alt.ts#function:alt",
            "src/utils/helper.ts#function:helper",
        ],
        "equal-length paths must break ties on the lexicographically first target"
    );

    let mut unique = BTreeSet::new();
    for step in &artifact.steps {
        assert!(unique.insert(step.symbol.clone()), "duplicate step");
    }

    let graph = build_call_graph(&fixture(), "test").expect("call graph");
    for pair in artifact.steps.windows(2) {
        let source = &pair[0].symbol;
        let target = &pair[1].symbol;
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| &edge.source == source && &edge.target == target),
            "adjacent steps must share an edge: {source} -> {target}"
        );
        assert_eq!(
            pair[1].via.as_deref(),
            graph
                .edges
                .iter()
                .find(|edge| &edge.source == source && &edge.target == target)
                .map(|edge| edge.kind.as_str())
        );
    }
}

#[test]
fn module_id_is_normalized_to_module_toplevel() {
    let artifact = build_path(
        &fixture(),
        "test",
        "src/cli.ts",
        "src/utils/helper.ts#function:helper",
    )
    .expect("path");
    assert_eq!(artifact.from, "src/cli.ts#module-toplevel");
    assert_eq!(artifact.steps[0].symbol, "src/cli.ts#module-toplevel");
}

#[test]
fn unreachable_target_is_reported() {
    let error = build_path(
        &fixture(),
        "test",
        "src/cli.ts#module-toplevel",
        "src/isolated.ts#function:isolated",
    )
    .expect_err("isolated symbol must be unreachable");
    assert!(matches!(error, PathError::Unreachable { .. }), "{error:?}");
}

#[test]
fn unknown_target_is_reported() {
    let error = build_path(
        &fixture(),
        "test",
        "src/cli.ts#module-toplevel",
        "src/isolated.ts#function:missing",
    )
    .expect_err("missing symbol");
    assert!(matches!(error, PathError::UnknownSymbol(_)), "{error:?}");
}

#[test]
fn path_artifact_is_deterministic() {
    let first = build_path(
        &fixture(),
        "test",
        "src/cli.ts#module-toplevel",
        "src/utils/helper.ts#function:helper",
    )
    .expect("path");
    let second = build_path(
        &fixture(),
        "test",
        "src/cli.ts#module-toplevel",
        "src/utils/helper.ts#function:helper",
    )
    .expect("path");
    assert_eq!(
        serde_json::to_string(&first).expect("json"),
        serde_json::to_string(&second).expect("json")
    );
}

#[test]
fn zero_entries_is_reported() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-path-core-{unique}"));
    fs::create_dir_all(directory.join("src")).expect("create temp dir");
    fs::write(directory.join("src/only.ts"), "export const value = 1;\n").expect("write source");

    let error = list_entries(&directory, "test").expect_err("no entries");
    assert!(matches!(error, PathError::NoEntries(_)), "{error:?}");

    fs::remove_dir_all(&directory).ok();
}
