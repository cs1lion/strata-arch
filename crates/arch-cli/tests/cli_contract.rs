use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const FIXTURE_HASHES: &[(&str, &str)] = &[
    (
        "package.json",
        "a9ebd72385371f07f88316b33d01a0b08084acb04a589d4e78f4824f4a629b4f",
    ),
    (
        "src/main.ts",
        "5569e2a87576c3a91ae73e6a0969aff25e6e0f1be6f9f6d67fa0cf18b4c1a7a0",
    ),
    (
        "src/types.ts",
        "692dbb03be25192ecca3be2c32e480ccd7b80e057b00732ff59fe89328fc0e74",
    ),
    (
        "src/util.ts",
        "6db345a069918bc67336752bc0fe6024f416b9ec4509080915c4b9f61a7f4540",
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("tests/fixtures/tiny")
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
            "arch-cli-{tag}-{}-{nanos}",
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

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tree_hashes(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, current: &Path, output: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(current).expect("read fixture dir") {
            let entry = entry.expect("fixture entry");
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, output);
            } else if path.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .expect("relative path")
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().replace('\\', "/"))
                    .collect::<Vec<_>>()
                    .join("/");
                let bytes = fs::read(&path).expect("read fixture file");
                output.insert(relative, sha256_hex(&bytes));
            }
        }
    }

    let mut output = BTreeMap::new();
    walk(root, root, &mut output);
    output
}

#[test]
fn fixture_contents_are_pinned() {
    let hashes = tree_hashes(&fixture_dir());
    for (path, expected) in FIXTURE_HASHES {
        let actual = hashes
            .get(*path)
            .unwrap_or_else(|| panic!("fixture file missing: {path}"));
        assert_eq!(actual, expected, "fixture file changed: {path}");
    }
}

#[test]
fn schema_flag_matches_artifact_schema_version() {
    let schema = run_cli(&["--schema"]);
    assert!(schema.status.success());
    assert_eq!(String::from_utf8_lossy(&schema.stdout).trim(), "1");

    let index = run_cli(&["index", fixture_dir().to_str().expect("fixture path")]);
    assert!(index.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&index.stdout).expect("stdout must be pure JSON");
    assert_eq!(json["schemaVersion"], 1);
}

#[test]
fn success_writes_contract_and_clean_stdout() {
    let output = run_cli(&["index", fixture_dir().to_str().expect("fixture path")]);
    assert_eq!(output.status.code(), Some(0));

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be pure JSON");
    assert!(json["toolVersion"].is_string());
    assert!(json["repo"]["path"].is_string());
    assert!(json["repo"]["head"].is_null());
    assert_eq!(json["repo"]["isGit"], false);
    assert_eq!(json["stats"]["files"], 3);
    assert_eq!(json["stats"]["modules"], 3);
    assert_eq!(json["stats"]["edges"], 0);
    assert_eq!(json["stats"]["unresolved"], 0);

    let modules = json["modules"].as_array().expect("modules array");
    assert_eq!(modules.len(), 3);
    let mut ids = modules
        .iter()
        .map(|module| module["id"].as_str().expect("module id").to_string())
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(
        ids,
        vec!["src/main.ts", "src/types.ts", "src/util.ts"]
    );
}

#[test]
fn input_errors_exit_with_code_two() {
    let missing = run_cli(&["index", "does-not-exist"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty(), "errors must not write stdout");

    let unknown = run_cli(&["index", fixture_dir().to_str().expect("fixture path"), "--bogus"]);
    assert_eq!(unknown.status.code(), Some(2));

    let no_target = run_cli(&["index"]);
    assert_eq!(no_target.status.code(), Some(2));

    let unknown_command = run_cli(&["bogus"]);
    assert_eq!(unknown_command.status.code(), Some(2));
}

#[test]
fn repeated_runs_are_byte_identical() {
    let temp = TempDir::new("determinism");
    let first = temp.path().join("a.json");
    let second = temp.path().join("b.json");

    let first_output = run_cli(&[
        "index",
        fixture_dir().to_str().expect("fixture path"),
        "--out",
        first.to_str().expect("output path"),
    ]);
    assert_eq!(first_output.status.code(), Some(0));
    assert!(first_output.stdout.is_empty(), "--out must not print JSON");

    let second_output = run_cli(&[
        "index",
        fixture_dir().to_str().expect("fixture path"),
        "--out",
        second.to_str().expect("output path"),
    ]);
    assert_eq!(second_output.status.code(), Some(0));

    let first_bytes = fs::read(&first).expect("read first artifact");
    let second_bytes = fs::read(&second).expect("read second artifact");
    assert_eq!(first_bytes, second_bytes, "artifacts must be byte-identical");
}

#[test]
fn target_repository_is_not_modified() {
    let before = tree_hashes(&fixture_dir());
    let output = run_cli(&["index", fixture_dir().to_str().expect("fixture path")]);
    assert!(output.status.success());
    let after = tree_hashes(&fixture_dir());
    assert_eq!(before, after, "index must not modify the target repository");
}

#[test]
fn oversize_repository_is_rejected_with_counts() {
    let temp = TempDir::new("oversize");
    for index in 0..5001 {
        fs::write(
            temp.path().join(format!("file-{index:05}.ts")),
            format!("export const value{index} = {index};\n"),
        )
        .expect("create synthetic source file");
    }

    let output = run_cli(&["index", temp.path().to_str().expect("temp path")]);
    assert!(
        !output.status.success(),
        "oversize repository must not exit successfully"
    );
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("5001"), "stderr must report actual file count: {stderr}");
    assert!(stderr.contains("5000"), "stderr must report the limit: {stderr}");
}
