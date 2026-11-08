//! N1 漂移检测的 CLI 契约测试（A 档）。
//!
//! 覆盖：基线录制、无漂移退出 0、命中门禁退出 1、输入错误退出 2、
//! 字节确定性与基线可移植性（不写机器绝对路径）。

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
    workspace_root().join("tests/fixtures/drift")
}

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arch-cli"))
        .args(arguments)
        .output()
        .expect("run arch-cli")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arch-drift-cli-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create target dir");
    for entry in fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("fixture entry");
        let source = entry.path();
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&source, &target);
        } else {
            fs::copy(&source, &target).expect("copy fixture file");
        }
    }
}

fn fixture_copy(tag: &str) -> TempDir {
    let temp = TempDir::new(tag);
    copy_dir(&fixture_dir(), temp.path());
    temp
}

fn record_baseline(output: &Path) -> Output {
    run_cli(&[
        "drift",
        fixture_dir().to_str().expect("fixture"),
        "--record",
        "--out",
        output.to_str().expect("baseline out"),
    ])
}

#[test]
fn drift_record_writes_a_portable_deterministic_baseline() {
    let temp = TempDir::new("record");
    let first_path = temp.path().join("first.json");
    let second_path = temp.path().join("second.json");

    let first = record_baseline(&first_path);
    assert_eq!(
        first.status.code(),
        Some(0),
        "drift --record failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(first.stdout.is_empty(), "stdout must stay clean with --out");
    assert!(first.stderr.is_empty(), "stderr must stay clean");

    let second = record_baseline(&second_path);
    assert_eq!(second.status.code(), Some(0));
    let first_bytes = fs::read(&first_path).expect("read baseline");
    let second_bytes = fs::read(&second_path).expect("read baseline");
    assert_eq!(first_bytes, second_bytes, "baseline bytes must be stable");

    let text = String::from_utf8(first_bytes.clone()).expect("baseline is UTF-8");
    let json: serde_json::Value = serde_json::from_str(&text).expect("baseline is JSON");
    assert_eq!(json["schemaVersion"].as_u64(), Some(1));
    assert_eq!(json["populationTolerance"].as_u64(), Some(0));
    assert_eq!(
        json["recordedHead"].as_str(),
        None,
        "fixture is not a git repo"
    );
    assert_eq!(
        json["layers"].as_array().expect("layers array").len(),
        8,
        "all fixed layers must be recorded, including empty ones"
    );
    assert!(
        !text.contains(fixture_dir().to_str().expect("fixture")),
        "the recorded baseline must not embed a machine-specific repo path"
    );

    let ids = json["modules"]
        .as_array()
        .expect("modules array")
        .iter()
        .map(|module| module["id"].as_str().expect("module id").to_string())
        .collect::<Vec<_>>();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "module detail must be sorted by id");
}

#[test]
fn drift_with_matching_baseline_exits_zero_and_reports_empty() {
    let temp = TempDir::new("match");
    let baseline = temp.path().join("drift-baseline.json");
    assert_eq!(record_baseline(&baseline).status.code(), Some(0));

    let output = run_cli(&[
        "drift",
        fixture_dir().to_str().expect("fixture"),
        "--base",
        baseline.to_str().expect("baseline"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "drift failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    assert_eq!(json["gate"]["passed"].as_bool(), Some(true));
    assert_eq!(
        json["gate"]["checks"]["layerPopulation"]["newCount"].as_u64(),
        Some(0)
    );
    assert_eq!(
        json["gate"]["checks"]["reverseEdges"]["newCount"].as_u64(),
        Some(0)
    );
    assert_eq!(
        json["gate"]["checks"]["cycles"]["newCount"].as_u64(),
        Some(0)
    );
    assert_eq!(
        json["gate"]["checks"]["externalLeaks"]["newCount"].as_u64(),
        Some(0)
    );
    assert!(json["newViolations"]
        .as_array()
        .expect("newViolations array")
        .is_empty());
    assert!(json["modules"]["added"]
        .as_array()
        .expect("added array")
        .is_empty());
    assert!(json["modules"]["removed"]
        .as_array()
        .expect("removed array")
        .is_empty());
    assert!(json["modules"]["moved"]
        .as_array()
        .expect("moved array")
        .is_empty());
}

#[test]
fn drift_gate_failure_exits_one_with_a_full_report() {
    let temp = TempDir::new("gate");
    let baseline = temp.path().join("drift-baseline.json");
    assert_eq!(record_baseline(&baseline).status.code(), Some(0));

    let repo = fixture_copy("gate");
    let target = repo.path().join("utils/text.ts");
    let mut source = fs::read_to_string(&target).expect("read text module");
    source.push_str("\nimport { run } from \"../commands/run.js\";\n");
    fs::write(&target, source).expect("append reverse import");

    let output = run_cli(&[
        "drift",
        repo.path().to_str().expect("repo"),
        "--base",
        baseline.to_str().expect("baseline"),
    ]);
    assert_eq!(output.status.code(), Some(1), "gate failure must exit 1");
    assert!(
        !output.stderr.is_empty(),
        "gate failure must explain itself on stderr"
    );

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must still carry the full report");
    assert_eq!(json["gate"]["passed"].as_bool(), Some(false));
    assert_eq!(
        json["gate"]["checks"]["reverseEdges"]["newCount"].as_u64(),
        Some(1)
    );
    let violations = json["newViolations"].as_array().expect("violations array");
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["kind"].as_str(), Some("reverseEdge"));
    assert_eq!(violations[0]["source"].as_str(), Some("utils/text.ts"));
    assert_eq!(violations[0]["target"].as_str(), Some("commands/run.ts"));
}

#[test]
fn missing_or_invalid_baseline_exits_two_without_partial_json() {
    let temp = TempDir::new("invalid");
    let missing = temp.path().join("missing.json");

    let output = run_cli(&[
        "drift",
        fixture_dir().to_str().expect("fixture"),
        "--base",
        missing.to_str().expect("missing baseline"),
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "no partial JSON on input errors");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("基线"),
        "stderr must name the missing baseline"
    );

    let baseline = temp.path().join("drift-baseline.json");
    assert_eq!(record_baseline(&baseline).status.code(), Some(0));
    let text = fs::read_to_string(&baseline).expect("read baseline");
    let mut json: serde_json::Value = serde_json::from_str(&text).expect("baseline JSON");
    json["schemaVersion"] = serde_json::json!(2);
    fs::write(
        &baseline,
        serde_json::to_string_pretty(&json).expect("serialize"),
    )
    .expect("write");

    let output = run_cli(&[
        "drift",
        fixture_dir().to_str().expect("fixture"),
        "--base",
        baseline.to_str().expect("baseline"),
    ]);
    assert_eq!(output.status.code(), Some(2), "schema mismatch must exit 2");
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("schemaVersion"),
        "stderr must explain the schema mismatch"
    );
}

#[test]
fn drift_argument_contract_rejects_missing_or_conflicting_flags() {
    let output = run_cli(&["drift", fixture_dir().to_str().expect("fixture")]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "drift needs either --record or --base"
    );
    assert!(output.stdout.is_empty());

    let temp = TempDir::new("conflict");
    let baseline = temp.path().join("baseline.json");
    let output = run_cli(&[
        "drift",
        fixture_dir().to_str().expect("fixture"),
        "--record",
        "--base",
        baseline.to_str().expect("baseline"),
        "--out",
        baseline.to_str().expect("baseline"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "--record and --base must be mutually exclusive"
    );
    assert!(output.stdout.is_empty());
}
