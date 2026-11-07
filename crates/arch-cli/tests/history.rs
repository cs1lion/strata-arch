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

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arch-cli"))
        .args(arguments)
        .output()
        .expect("run arch-cli")
}

fn run_git(dir: &Path, args: &[&str], date: Option<&str>) -> String {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args);
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(date) = date {
        command
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com");
    }
    let output = command.output().expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write(dir: &Path, relative: &str, content: &str) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arch-cli-history-{tag}-{}-{nanos}",
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

fn build_repo(tag: &str) -> TempDir {
    let temp = TempDir::new(tag);
    let dir = temp.path();
    run_git(dir, &["init", "-q"], None);
    run_git(dir, &["config", "user.name", "Fixture"], None);
    run_git(dir, &["config", "user.email", "fixture@example.com"], None);
    run_git(dir, &["config", "core.autocrlf", "false"], None);
    run_git(dir, &["config", "commit.gpgsign", "false"], None);

    write(dir, "src/app.ts", "export const a = 1;\n");
    run_git(dir, &["add", "-A"], None);
    run_git(
        dir,
        &["commit", "-q", "-m", "c1"],
        Some("2026-02-01T12:00:00Z"),
    );

    write(
        dir,
        "src/app.ts",
        "export const a = 1;\nexport const b = 2;\n",
    );
    run_git(dir, &["add", "-A"], None);
    run_git(
        dir,
        &["commit", "-q", "-m", "c2"],
        Some("2026-02-02T12:00:00Z"),
    );

    temp
}

fn history_json(arguments: &[&str]) -> serde_json::Value {
    let output = run_cli(arguments);
    assert_eq!(
        output.status.code(),
        Some(0),
        "history failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr must be empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout must be JSON")
}

#[test]
fn history_emits_windowed_artifact() {
    let temp = build_repo("artifact");
    let path = temp.path().to_str().expect("path");
    let json = history_json(&["history", path]);

    assert_eq!(json["schemaVersion"], 1);
    assert!(json["toolVersion"].is_string());
    assert_eq!(json["repo"]["isGit"], true);
    assert_eq!(
        json["repo"]["head"],
        run_git(temp.path(), &["rev-parse", "HEAD"], None)
    );
    assert_eq!(json["window"]["maxCommits"], 500);
    assert_eq!(json["window"]["firstParent"], true);
    assert_eq!(json["sampled"], false);
    assert!(json["sampleStep"].is_null());
    assert_eq!(json["truncated"], false);
    assert_eq!(json["shallow"], false);
    assert_eq!(json["stats"]["commitsScanned"], 2);
    assert_eq!(json["stats"]["commitsAvailable"], 2);

    let modules = json["modules"].as_array().expect("modules array");
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0]["id"], "src/app.ts");
    assert_eq!(modules[0]["commitCount"], 2);
    assert_eq!(modules[0]["churn"]["additions"], 2);
    assert_eq!(modules[0]["churn"]["deletions"], 0);
    assert_eq!(modules[0]["churn"]["total"], 2);
    assert_eq!(
        modules[0]["activeSpan"],
        modules[0]["lastModified"].as_u64().expect("last")
            - modules[0]["firstSeen"].as_u64().expect("first")
    );
}

#[test]
fn history_accepts_repo_flag_and_out_file() {
    let temp = build_repo("out");
    let out = temp.path().join("history.json");
    let output = run_cli(&[
        "history",
        "--repo",
        temp.path().to_str().expect("repo"),
        "--out",
        out.to_str().expect("out"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "history failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "--out must not print to stdout");
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(&out).expect("read out")).expect("JSON");
    assert_eq!(json["modules"].as_array().expect("modules").len(), 1);
}

#[test]
fn history_detects_repo_from_subdirectory() {
    let temp = build_repo("subdir");
    let subdir = temp.path().join("src");
    let json = history_json(&["history", subdir.to_str().expect("subdir")]);

    assert_eq!(json["repo"]["isGit"], true);
    assert_eq!(
        json["repo"]["path"],
        run_git(temp.path(), &["rev-parse", "--show-toplevel"], None)
    );
    assert_eq!(
        json["repo"]["head"],
        run_git(&subdir, &["rev-parse", "HEAD"], None)
    );
    assert!(!json["repo"]["head"].is_null());
}

#[test]
fn history_is_deterministic() {
    let temp = build_repo("determinism");
    let path = temp.path().to_str().expect("path");
    let first = run_cli(&["history", path]);
    let second = run_cli(&["history", path]);
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(
        first.stdout, second.stdout,
        "history artifact must be byte deterministic"
    );
}

#[test]
fn history_does_not_modify_target() {
    let temp = build_repo("readonly");
    let dir = temp.path();
    let status_before = run_git(dir, &["status", "--porcelain"], None);
    let head_before = run_git(dir, &["rev-parse", "HEAD"], None);
    let worktrees_before = run_git(dir, &["worktree", "list"], None).lines().count();

    let output = run_cli(&["history", dir.to_str().expect("path")]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "history failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        run_git(dir, &["status", "--porcelain"], None),
        status_before
    );
    assert_eq!(run_git(dir, &["rev-parse", "HEAD"], None), head_before);
    assert_eq!(
        run_git(dir, &["worktree", "list"], None).lines().count(),
        worktrees_before
    );
}

#[test]
fn history_degrades_with_exit_code_five() {
    let plain = TempDir::new("plain");
    let output = run_cli(&["history", plain.path().to_str().expect("plain")]);
    assert_eq!(output.status.code(), Some(5));
    assert!(!output.stderr.is_empty(), "reason must be on stderr");
    assert!(output.stdout.is_empty(), "no artifact on failure");

    let empty = TempDir::new("empty");
    run_git(empty.path(), &["init", "-q"], None);
    let output = run_cli(&["history", empty.path().to_str().expect("empty")]);
    assert_eq!(output.status.code(), Some(5));

    let source = build_repo("shallow-source");
    let target = TempDir::new("shallow-target");
    let clone = target.path().join("clone");
    let url = format!(
        "file:///{}",
        source.path().to_string_lossy().replace('\\', "/")
    );
    let clone_output = Command::new("git")
        .args([
            "clone",
            "-q",
            "--depth",
            "1",
            &url,
            clone.to_str().expect("path"),
        ])
        .output()
        .expect("git clone");
    assert!(
        clone_output.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&clone_output.stderr)
    );
    let output = run_cli(&["history", clone.to_str().expect("clone")]);
    assert_eq!(output.status.code(), Some(5), "shallow clone must exit 5");
    assert!(output.stdout.is_empty());
}

#[test]
fn history_workspace_fixture_path_is_stable() {
    // tests/fixtures/tiny 位于本仓工作树内：git 视角下属于本仓，必须识别到仓库根而不是误报非 git。
    let fixture = workspace_root().join("tests/fixtures/tiny");
    let json = history_json(&["history", fixture.to_str().expect("fixture")]);
    assert_eq!(json["repo"]["isGit"], true);
    assert_eq!(
        json["repo"]["path"],
        run_git(&fixture, &["rev-parse", "--show-toplevel"], None)
    );
    assert!(!json["repo"]["head"].is_null());
}
