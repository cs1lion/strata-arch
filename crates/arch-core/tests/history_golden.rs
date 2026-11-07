use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use arch_core::history::{build_history, HistoryError, HistoryOptions};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("arch-history-{tag}-{}-{nanos}", std::process::id()));
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

/// 构造固定历史：5 个 first-parent 提交 + 1 个合并提交（第二父分支改动必须被忽略）。
fn build_fixture_repo() -> TempDir {
    let temp = TempDir::new("fixture");
    let dir = temp.path();
    init_repo(dir);

    write(
        dir,
        "src/app.ts",
        "export const a = 1;\nexport const b = 2;\nexport const c = 3;\n",
    );
    write(dir, "src/util.ts", "u1\nu2\nu3\nu4\n");
    commit(dir, "2026-01-01T12:00:00Z", "c1 init");

    write(
        dir,
        "src/app.ts",
        "export const a = 1;\nexport const b = 2;\nexport const c = 4;\n",
    );
    commit(dir, "2026-01-02T12:00:00Z", "c2 app");

    run_git(dir, &["mv", "src/util.ts", "src/helpers.ts"], None);
    commit(dir, "2026-01-03T12:00:00Z", "c3 rename");

    run_git(
        dir,
        &["commit", "-q", "--allow-empty", "-m", "c4 empty"],
        Some("2026-01-04T12:00:00Z"),
    );

    write(dir, "src/helpers.ts", "u1\nu2\nu3x\nu4\n");
    commit(dir, "2026-01-05T12:00:00Z", "c5 helpers");

    let branch = run_git(dir, &["symbolic-ref", "--short", "HEAD"], None);
    run_git(dir, &["checkout", "-q", "-b", "side"], None);
    write(
        dir,
        "src/app.ts",
        "export const a = 1;\nexport const b = 2;\nexport const c = 4;\nexport const d = 5;\n",
    );
    write(
        dir,
        "src/merge-only.ts",
        "export const side = 1;\nexport const other = 2;\n",
    );
    commit(dir, "2026-01-06T12:00:00Z", "side change");
    run_git(dir, &["checkout", "-q", branch.as_str()], None);
    run_git(
        dir,
        &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
        Some("2026-01-07T12:00:00Z"),
    );

    temp
}

fn timestamp_of(dir: &Path, args: &[&str]) -> u64 {
    let text = run_git(dir, args, None);
    text.lines()
        .next()
        .expect("timestamp line")
        .trim()
        .parse()
        .expect("epoch")
}

fn first_seen_oracle(dir: &Path, paths: &[&str]) -> u64 {
    let mut args = vec!["log", "--first-parent", "--no-merges", "--format=%ct", "--"];
    args.extend(paths.iter().copied());
    let output = run_git(dir, &args, None);
    output
        .lines()
        .last()
        .expect("oldest commit")
        .trim()
        .parse()
        .expect("epoch")
}

fn last_modified_oracle(dir: &Path, paths: &[&str]) -> u64 {
    let mut args = vec![
        "log",
        "-1",
        "--first-parent",
        "--no-merges",
        "--format=%ct",
        "--",
    ];
    args.extend(paths.iter().copied());
    timestamp_of(dir, &args)
}

fn commit_count_oracle(dir: &Path, paths: &[&str]) -> usize {
    let mut args = vec![
        "rev-list",
        "--count",
        "--first-parent",
        "--no-merges",
        "HEAD",
        "--",
    ];
    args.extend(paths.iter().copied());
    run_git(dir, &args, None).parse().expect("commit count")
}

fn churn_oracle(dir: &Path, paths: &[&str]) -> (u64, u64) {
    let mut args = vec![
        "log",
        "--first-parent",
        "--no-merges",
        "--numstat",
        "--format=",
        "--",
    ];
    args.extend(paths.iter().copied());
    let output = run_git(dir, &args, None);
    let mut additions = 0u64;
    let mut deletions = 0u64;
    for line in output.lines() {
        let mut parts = line.split('\t');
        let Some(added) = parts.next() else { continue };
        let Some(removed) = parts.next() else {
            continue;
        };
        if parts.next().is_none() {
            continue;
        }
        additions += added.parse::<u64>().unwrap_or(0);
        deletions += removed.parse::<u64>().unwrap_or(0);
    }
    (additions, deletions)
}

#[test]
fn golden_history_metrics_match_git_oracle() {
    let temp = build_fixture_repo();
    let artifact =
        build_history(temp.path(), "test", HistoryOptions::default()).expect("history artifact");

    assert_eq!(artifact.schema_version, 1);
    assert_eq!(artifact.tool_version, "test");
    assert!(artifact.repo.is_git);
    assert_eq!(
        artifact.repo.path,
        run_git(temp.path(), &["rev-parse", "--show-toplevel"], None)
    );
    assert_eq!(
        artifact.repo.head,
        run_git(temp.path(), &["rev-parse", "HEAD"], None)
    );

    assert_eq!(artifact.window.max_commits, 500);
    assert!(artifact.window.first_parent);
    assert!(!artifact.sampled);
    assert_eq!(artifact.sample_step, None);
    assert!(!artifact.truncated);
    assert!(!artifact.shallow);
    assert_eq!(artifact.stats.commits_scanned, 6);
    assert_eq!(artifact.stats.commits_available, 6);

    let ids = artifact
        .modules
        .iter()
        .map(|module| module.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec!["src/app.ts", "src/helpers.ts"],
        "merge-only file must not enter first-parent metrics"
    );

    let app = &artifact.modules[0];
    assert_eq!(
        app.commit_count,
        commit_count_oracle(temp.path(), &["src/app.ts"])
    );
    assert_eq!(
        app.first_seen,
        first_seen_oracle(temp.path(), &["src/app.ts"])
    );
    assert_eq!(
        app.last_modified,
        last_modified_oracle(temp.path(), &["src/app.ts"])
    );
    let (additions, deletions) = churn_oracle(temp.path(), &["src/app.ts"]);
    assert_eq!(app.churn.additions, additions);
    assert_eq!(app.churn.deletions, deletions);
    assert_eq!(app.churn.total, additions + deletions);
    assert_eq!(app.active_span, app.last_modified - app.first_seen);

    let helpers = &artifact.modules[1];
    let lineage = ["src/util.ts", "src/helpers.ts"];
    assert_eq!(
        helpers.commit_count,
        commit_count_oracle(temp.path(), &lineage),
        "rename lineage must cover pre-rename commits"
    );
    assert_eq!(helpers.first_seen, first_seen_oracle(temp.path(), &lineage));
    assert_eq!(
        helpers.last_modified,
        last_modified_oracle(temp.path(), &lineage)
    );
    let (additions, deletions) = churn_oracle(temp.path(), &lineage);
    assert_eq!(helpers.churn.additions, additions);
    assert_eq!(helpers.churn.deletions, deletions);
    assert_eq!(
        helpers.active_span,
        helpers.last_modified - helpers.first_seen
    );

    assert_eq!(artifact.stats.modules, 2);
}

#[test]
fn window_truncation_is_explicit_and_limits_metrics() {
    let temp = build_fixture_repo();
    let artifact = build_history(
        temp.path(),
        "test",
        HistoryOptions {
            max_commits: 2,
            since: None,
            until: None,
        },
    )
    .expect("history artifact");

    assert!(
        artifact.truncated,
        "window smaller than history must be truncated"
    );
    assert_eq!(artifact.stats.commits_scanned, 2);
    assert_eq!(artifact.stats.commits_available, 6);
    assert_eq!(artifact.window.max_commits, 2);

    let ids = artifact
        .modules
        .iter()
        .map(|module| module.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec!["src/helpers.ts"],
        "only c5 and merge fall in window"
    );
    let helpers = &artifact.modules[0];
    assert_eq!(helpers.commit_count, 1, "merge commit must not count");
}

#[test]
fn repo_root_and_head_are_detected_from_subdirectory() {
    let temp = build_fixture_repo();
    let subdir = temp.path().join("src");

    let artifact =
        build_history(&subdir, "test", HistoryOptions::default()).expect("subdirectory history");
    assert!(
        artifact.repo.is_git,
        "subdirectory must be recognized as git"
    );
    assert_eq!(
        artifact.repo.path,
        run_git(temp.path(), &["rev-parse", "--show-toplevel"], None)
    );
    assert_eq!(
        artifact.repo.head,
        run_git(&subdir, &["rev-parse", "HEAD"], None)
    );
    assert!(
        !artifact.repo.head.is_empty(),
        "subdirectory history must carry HEAD"
    );
}

#[test]
fn non_git_and_empty_repositories_report_specific_errors() {
    let plain = TempDir::new("plain");
    assert!(matches!(
        build_history(plain.path(), "test", HistoryOptions::default()),
        Err(HistoryError::NotGit(_))
    ));

    let empty = TempDir::new("empty");
    init_repo(empty.path());
    assert!(matches!(
        build_history(empty.path(), "test", HistoryOptions::default()),
        Err(HistoryError::NoCommits(_))
    ));
}

#[test]
fn shallow_clone_is_rejected() {
    let source = build_fixture_repo();
    let target = TempDir::new("shallow");
    let clone = target.path().join("clone");
    let url = format!(
        "file:///{}",
        source.path().to_string_lossy().replace('\\', "/")
    );
    let output = Command::new("git")
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
        output.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(matches!(
        build_history(&clone, "test", HistoryOptions::default()),
        Err(HistoryError::Shallow(_))
    ));
}

#[test]
fn history_is_deterministic() {
    let temp = build_fixture_repo();
    let first = serde_json::to_vec(
        &build_history(temp.path(), "test", HistoryOptions::default()).expect("first"),
    )
    .expect("serialize first");
    let second = serde_json::to_vec(
        &build_history(temp.path(), "test", HistoryOptions::default()).expect("second"),
    )
    .expect("serialize second");
    assert_eq!(first, second, "history artifact must be byte deterministic");
}
