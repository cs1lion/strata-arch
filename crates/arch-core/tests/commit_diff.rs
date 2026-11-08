//! ticket 17：双 commit 结构 diff 的 A 档测试。
//!
//! 断言只读外部可观察产物（`CommitDiffArtifact`），不触碰私有实现。
//! 数据源固定为测试内建的合成 fixture 仓库与本仓库自身 `2b66deb → 29f8e27`。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use arch_core::commit_diff::{
    diff_commits, diff_commits_cancellable, diff_commits_with_progress, CommitDiffArtifact,
    CommitDiffError, DiffProgress, RENAME_PATH_THRESHOLD, RENAME_SYMBOL_THRESHOLD,
};

const TOOL_VERSION: &str = "test";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        // 时钟在 macOS 上分辨率较粗，并发测试可能拿到相同的 nanos 而复用同一目录，
        // 导致第二个 `git init` 在拷贝模板钩子时报 File exists；用进程内序号兜底。
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arch-commit-diff-{tag}-{}-{nanos}-{seq}",
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

fn init_repo(dir: &Path) {
    run_git(dir, &["init", "-q"], None);
    run_git(dir, &["config", "user.name", "Fixture"], None);
    run_git(dir, &["config", "user.email", "fixture@example.com"], None);
    run_git(dir, &["config", "core.autocrlf", "false"], None);
    run_git(dir, &["config", "commit.gpgsign", "false"], None);
}

fn commit(dir: &Path, date: &str, message: &str) {
    run_git(dir, &["add", "-A"], None);
    run_git(dir, &["commit", "-q", "-m", message], Some(date));
}

struct FixtureRepo {
    temp: TempDir,
    base: String,
    head: String,
}

impl FixtureRepo {
    fn path(&self) -> &Path {
        self.temp.path()
    }
}

/// 合成 fixture：added 3 / removed 2 / modified 1 / 疑似重命名 1。
fn build_fixture_repo() -> FixtureRepo {
    let temp = TempDir::new("fixture");
    let dir = temp.path();
    init_repo(dir);

    write(
        dir,
        "src/a.ts",
        "export function alpha() {\n  return 1;\n}\n",
    );
    write(
        dir,
        "src/b.ts",
        "import { alpha } from \"./a\";\nexport function beta() {\n  return alpha();\n}\n",
    );
    write(
        dir,
        "src/old.ts",
        "export function legacy() {\n  return 3;\n}\n",
    );
    write(
        dir,
        "src/dead.ts",
        "export function dead() {\n  return 4;\n}\n",
    );
    commit(dir, "2026-02-01T12:00:00Z", "base");
    let base = run_git(dir, &["rev-parse", "HEAD"], None);

    write(
        dir,
        "src/a.ts",
        "export function alpha() {\n  return 1;\n}\nexport function gamma() {\n  return 2;\n}\n",
    );
    run_git(dir, &["mv", "src/old.ts", "src/legacy.ts"], None);
    fs::remove_file(dir.join("src/dead.ts")).expect("remove dead.ts");
    write(
        dir,
        "src/new.ts",
        "export function fresh() {\n  return 5;\n}\n",
    );
    write(dir, "src/blank.ts", "");
    commit(dir, "2026-02-02T12:00:00Z", "next");
    let head = run_git(dir, &["rev-parse", "HEAD"], None);

    FixtureRepo { temp, base, head }
}

fn diff(repo: &Path, from: &str, to: &str) -> CommitDiffArtifact {
    diff_commits(repo, from, to, TOOL_VERSION).expect("commit diff")
}

#[test]
fn diff_honours_cancel_flag_before_work() {
    let repo = build_fixture_repo();
    let cancel = AtomicBool::new(true);
    let result = diff_commits_cancellable(
        repo.path(),
        &repo.base,
        &repo.head,
        TOOL_VERSION,
        &cancel,
        |_| {},
    );

    assert!(
        matches!(result, Err(CommitDiffError::Cancelled)),
        "取消标志必须在任何快照导出前生效，实际: {result:?}"
    );
}

#[test]
fn diff_reports_progress_stages_in_order() {
    let repo = build_fixture_repo();
    let mut stages = Vec::new();
    let artifact =
        diff_commits_with_progress(repo.path(), &repo.base, &repo.head, TOOL_VERSION, |stage| {
            stages.push(stage)
        })
        .expect("commit diff with progress");

    assert_eq!(artifact.stats.added, 3, "进度回调不改变 diff 结果");
    assert_eq!(
        stages,
        vec![
            DiffProgress::Resolving,
            DiffProgress::MaterializingFrom,
            DiffProgress::MaterializingTo,
            DiffProgress::FactsFrom,
            DiffProgress::FactsTo,
            DiffProgress::Comparing,
        ],
        "阶段顺序必须覆盖两个快照的导出与事实构建，前后不得跳步"
    );
}

fn assert_counts(artifact: &CommitDiffArtifact, added: usize, removed: usize, modified: usize) {
    println!(
        "counts: added={} removed={} modified={} renames={}",
        artifact.stats.added,
        artifact.stats.removed,
        artifact.stats.modified,
        artifact.stats.renames
    );
    assert_eq!(artifact.stats.added, added, "added 计数");
    assert_eq!(artifact.stats.removed, removed, "removed 计数");
    assert_eq!(artifact.stats.modified, modified, "modified 计数");
    assert_eq!(artifact.stats.added, artifact.added.len(), "added 明细一致");
    assert_eq!(
        artifact.stats.removed,
        artifact.removed.len(),
        "removed 明细一致"
    );
    assert_eq!(
        artifact.stats.modified,
        artifact.modified.len(),
        "modified 明细一致"
    );
    assert_eq!(
        artifact.stats.renames,
        artifact.renames.len(),
        "renames 明细一致"
    );
}

#[test]
fn synthetic_diff_separates_added_removed_modified() {
    let repo = build_fixture_repo();
    let artifact = diff(repo.path(), &repo.base, &repo.head);
    assert_counts(&artifact, 3, 2, 1);

    let added = artifact
        .added
        .iter()
        .map(|change| change.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(added, ["src/blank.ts", "src/legacy.ts", "src/new.ts"]);

    let removed = artifact
        .removed
        .iter()
        .map(|change| change.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(removed, ["src/dead.ts", "src/old.ts"]);

    let modified = &artifact.modified[0];
    assert_eq!(modified.path, "src/a.ts");
    assert_eq!(modified.symbol_count_from, 2);
    assert_eq!(modified.symbol_count_to, 3);
    assert_eq!(modified.reasons, ["symbol-count"]);
}

#[test]
fn synthetic_rename_keeps_both_records_and_hints() {
    let repo = build_fixture_repo();
    let artifact = diff(repo.path(), &repo.base, &repo.head);

    assert_eq!(artifact.stats.renames, 1, "疑似重命名命中数");
    assert_eq!(artifact.renames[0].from, "src/old.ts");
    assert_eq!(artifact.renames[0].to, "src/legacy.ts");
    assert!(
        artifact.renames[0].path_similarity >= RENAME_PATH_THRESHOLD,
        "路径相似度 {} 应达到下限 {}",
        artifact.renames[0].path_similarity,
        RENAME_PATH_THRESHOLD
    );
    assert!(
        artifact.renames[0].symbol_overlap >= RENAME_SYMBOL_THRESHOLD,
        "符号重合度 {} 应达到下限 {}",
        artifact.renames[0].symbol_overlap,
        RENAME_SYMBOL_THRESHOLD
    );

    let added = artifact
        .added
        .iter()
        .find(|change| change.path == "src/legacy.ts")
        .expect("改名目标仍在 added 中");
    assert_eq!(added.rename_hint.as_deref(), Some("src/old.ts"));

    let removed = artifact
        .removed
        .iter()
        .find(|change| change.path == "src/old.ts")
        .expect("改名源仍在 removed 中");
    assert_eq!(removed.rename_hint.as_deref(), Some("src/legacy.ts"));

    for path in ["src/dead.ts", "src/new.ts"] {
        let hinted = artifact
            .added
            .iter()
            .chain(artifact.removed.iter())
            .any(|change| change.path == path && change.rename_hint.is_some());
        assert!(!hinted, "{path} 不应被误判为疑似重命名");
    }
}

#[test]
fn added_empty_file_stays_added_with_one_symbol() {
    let repo = build_fixture_repo();
    let artifact = diff(repo.path(), &repo.base, &repo.head);

    let blank = artifact
        .added
        .iter()
        .find(|change| change.path == "src/blank.ts")
        .expect("空文件应出现在 added");
    assert_eq!(blank.symbol_count, 1, "空文件仍有 module-toplevel");
    assert!(
        artifact
            .modified
            .iter()
            .all(|change| change.path != "src/blank.ts"),
        "空文件不得被记成 modified"
    );
}

#[test]
fn same_commit_has_zero_changes() {
    let repo = build_fixture_repo();
    let artifact = diff(repo.path(), &repo.head, &repo.head);
    assert_counts(&artifact, 0, 0, 0);
    assert_eq!(artifact.stats.renames, 0);
}

#[test]
fn deleting_a_whole_directory_reports_every_file_removed() {
    let temp = TempDir::new("directory");
    let dir = temp.path();
    init_repo(dir);
    write(dir, "src/pkg/first.ts", "export function first() {}\n");
    write(dir, "src/pkg/second.ts", "export function second() {}\n");
    commit(dir, "2026-02-03T12:00:00Z", "base");
    let base = run_git(dir, &["rev-parse", "HEAD"], None);

    fs::remove_dir_all(dir.join("src/pkg")).expect("remove package directory");
    commit(dir, "2026-02-04T12:00:00Z", "remove package");
    let head = run_git(dir, &["rev-parse", "HEAD"], None);

    let artifact = diff(dir, &base, &head);
    assert_counts(&artifact, 0, 2, 0);
    assert_eq!(artifact.removed[0].path, "src/pkg/first.ts");
    assert_eq!(artifact.removed[1].path, "src/pkg/second.ts");
    assert_eq!(artifact.stats.renames, 0);
}

#[test]
fn diff_is_read_only_and_deterministic() {
    let repo = build_fixture_repo();
    let before_head = run_git(repo.path(), &["rev-parse", "HEAD"], None);
    let before_status = run_git(repo.path(), &["status", "--porcelain"], None);

    let first = diff(repo.path(), &repo.base, &repo.head);
    let second = diff(repo.path(), &repo.base, &repo.head);

    let after_head = run_git(repo.path(), &["rev-parse", "HEAD"], None);
    let after_status = run_git(repo.path(), &["status", "--porcelain"], None);
    assert_eq!(before_head, after_head, "HEAD 不得改变");
    assert_eq!(before_status, after_status, "工作区不得被改动");

    let first_json = serde_json::to_string(&first).expect("serialize first");
    let second_json = serde_json::to_string(&second).expect("serialize second");
    assert_eq!(first_json, second_json, "同一输入两次运行必须字节一致");
    assert!(first.stats.added > 0, "diff 必须真的读到提交内容");
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn repository_history_pair_has_expected_counts() {
    let root = workspace_root();
    let artifact = diff(&root, "2b66deb", "29f8e27");
    assert_counts(&artifact, 25, 0, 0);
    assert!(artifact
        .added
        .iter()
        .all(|change| change.path.starts_with("tests/fixtures/layers/")));
    println!(
        "repo pair: {} -> {}",
        artifact.from.commit, artifact.to.commit
    );
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

/// ticket 17 收口：大仓（excalidraw 语料）双 commit 快照的墙钟耗时。
#[test]
#[ignore]
fn large_corpus_commit_diff_wall_clock() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/excalidraw");
    let temp = TempDir::new("large");
    let dir = temp.path();
    copy_tree(&corpus, dir);
    init_repo(dir);
    commit(dir, "2026-02-01T12:00:00Z", "base");
    let base = run_git(dir, &["rev-parse", "HEAD"], None);

    write(
        dir,
        "perf-probe.ts",
        "export function perfProbe() {\n  return 1;\n}\n",
    );
    commit(dir, "2026-02-02T12:00:00Z", "probe");
    let head = run_git(dir, &["rev-parse", "HEAD"], None);

    let started = Instant::now();
    let artifact = diff(dir, &base, &head);
    let elapsed = started.elapsed();
    println!(
        "large corpus commit diff: {} ms; added={} removed={} modified={} renames={}",
        elapsed.as_millis(),
        artifact.stats.added,
        artifact.stats.removed,
        artifact.stats.modified,
        artifact.stats.renames
    );
    assert_counts(&artifact, 1, 0, 0);
}
