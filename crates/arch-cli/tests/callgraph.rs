use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("tests/fixtures/callgraph")
}

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arch-cli"))
        .args(arguments)
        .output()
        .expect("run arch-cli")
}

fn callgraph(arguments: &[&str]) -> serde_json::Value {
    let output = run_cli(arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "callgraph failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr must be empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout must be JSON")
}

fn unresolved(json: &serde_json::Value) -> &Vec<serde_json::Value> {
    json["unresolved"].as_array().expect("unresolved array")
}

fn reasons(json: &serde_json::Value) -> Vec<String> {
    unresolved(json)
        .iter()
        .filter_map(|entry| entry["reason"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn default_artifact_aggregates_unattributed_calls() {
    let json = callgraph(&["callgraph", fixture_dir().to_str().expect("path")]);

    assert_eq!(json["schemaVersion"], 2, "call graph schema version");
    assert_eq!(json["stats"]["unattributedCalls"], 1);
    assert_eq!(json["stats"]["unresolvedImports"], 1);
    assert_eq!(json["stats"]["externalCalls"], 6);
    assert_eq!(
        unresolved(&json).len(),
        7,
        "default artifact must not expose call-unattributed detail"
    );
    assert!(
        !reasons(&json)
            .iter()
            .any(|reason| reason == "call-unattributed"),
        "call-unattributed must be aggregated: {:?}",
        reasons(&json)
    );
    assert!(reasons(&json)
        .iter()
        .any(|reason| reason == "dynamic-import"));
    assert!(reasons(&json)
        .iter()
        .any(|reason| reason == "external-call"));
}

#[test]
fn include_unattributed_restores_full_detail() {
    let default = callgraph(&["callgraph", fixture_dir().to_str().expect("path")]);
    let full = callgraph(&[
        "callgraph",
        fixture_dir().to_str().expect("path"),
        "--include-unattributed",
    ]);

    assert_eq!(
        unresolved(&default).len(),
        7,
        "default artifact aggregates call-unattributed"
    );
    assert_eq!(
        unresolved(&full).len(),
        8,
        "diagnostic artifact keeps full detail"
    );
    let detail = unresolved(&full)
        .iter()
        .find(|entry| entry["reason"] == "call-unattributed")
        .expect("unattributed detail is present");
    assert_eq!(detail["module"], "src/b.ts");
    assert!(detail["expression"]
        .as_str()
        .unwrap_or_default()
        .contains("unknownMethod"));

    assert_eq!(
        default["stats"]["unattributedCalls"], full["stats"]["unattributedCalls"],
        "aggregate count must not change with the flag"
    );
    assert_eq!(default["stats"]["edges"], full["stats"]["edges"]);
}

#[test]
fn out_file_matches_default_artifact() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("arch-callgraph-{unique}.json"));

    let output = run_cli(&[
        "callgraph",
        fixture_dir().to_str().expect("path"),
        "--out",
        path.to_str().expect("path"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "callgraph failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "--out must not also print to stdout"
    );
    let written: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("read artifact")).expect("artifact JSON");

    assert_eq!(written["schemaVersion"], 2);
    assert_eq!(unresolved(&written).len(), 7);
    assert_eq!(written["stats"]["unattributedCalls"], 1);

    let _ = fs::remove_file(path);
}
