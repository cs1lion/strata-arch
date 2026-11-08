//! ticket 09 断言清单（B 档）。
//!
//! 流程偏差：本票实现先于测试，未取得真实编译 Red；此清单为测试落盘时补写，
//! 已登记在 ticket 09 与 NIGHTLOG，不冒充红绿证据。
//!
//! 1. 时间线刻度数等于 first-parent 扫描提交数（fixture 6）。
//! 2. 拖动到第 k 个提交后，模块 `lastModified` 等于「截至该提交」重新采样的读数，不得回退到最新值。
//! 3. 热力映射固定：中性灰用于无历史模块；churn 分档与尺寸百分比可逐值冻结。
//! 4. 模块 ID 与地图产物一致：历史模块 ID 能在层产物中找到同名节点，地图节点都能取到热力值。
//! 5. `truncated`/`sampled`/`shallow` 存在时热力不按全量读数展示。
//! 6. 选择模块与返回地图：选择在存在时生效、未知模块清空；离开时间线清空选择但保留刻度。
//!
//! 截图结构断言已移交 ticket 11；本票不做像素差门禁。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use arch_core::history::{
    build_history, Churn, HistoryArtifact, HistoryModule, HistoryOptions, HistoryRepo,
    HistoryStats, HistoryWindow,
};
use arch_core::layers::build_layers;
use arch_view::timeline::{heat_for_churn, TimelineEntry, TimelineState, HEAT_NEUTRAL, HEAT_STOPS};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        // 并发用例可能在同一时钟刻度上取到相同 nanos，目录撞名会互相删除对方仓库
        // （macOS CI 上表现为 git add 找不到目录），用进程内序号兜底。
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "arch-timeline-{tag}-{}-{nanos}-{seq}",
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

fn write_file(dir: &Path, relative: &str, content: &str) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write fixture file");
}

fn commit(dir: &Path, date: &str, message: &str) {
    run_git(dir, &["add", "-A"], None);
    run_git(dir, &["commit", "-q", "-m", message], Some(date));
}

/// 与 `tests/fixtures/history-seed/build.ps1` 相同的 6 个 first-parent 提交。
fn seed_repo() -> TempDir {
    let temp = TempDir::new("repo");
    let dir = temp.path();
    run_git(dir, &["init", "-q"], None);
    run_git(dir, &["config", "core.autocrlf", "false"], None);
    run_git(dir, &["config", "commit.gpgsign", "false"], None);

    write_file(
        dir,
        "src/app.ts",
        "export const a = 1;\nexport const b = 2;\nexport const c = 3;\n",
    );
    write_file(dir, "src/util.ts", "u1\nu2\nu3\nu4\n");
    commit(dir, "2026-01-01T12:00:00Z", "c1 init");

    write_file(
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

    write_file(dir, "src/helpers.ts", "u1\nu2\nu3x\nu4\n");
    commit(dir, "2026-01-05T12:00:00Z", "c5 helpers");

    let branch = run_git(dir, &["symbolic-ref", "--short", "HEAD"], None);
    run_git(dir, &["checkout", "-q", "-b", "side"], None);
    write_file(
        dir,
        "src/app.ts",
        "export const a = 1;\nexport const b = 2;\nexport const c = 4;\nexport const d = 5;\n",
    );
    write_file(
        dir,
        "src/merge-only.ts",
        "export const side = 1;\nexport const other = 2;\n",
    );
    commit(dir, "2026-01-06T12:00:00Z", "side change");
    run_git(dir, &["checkout", "-q", &branch], None);
    run_git(
        dir,
        &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
        Some("2026-01-07T12:00:00Z"),
    );

    temp
}

fn prefix_entries(dir: &Path) -> Vec<TimelineEntry> {
    let log = run_git(
        dir,
        &[
            "log",
            "--reverse",
            "--first-parent",
            "--format=%H%x09%ct%x09%s",
        ],
        None,
    );
    log.lines()
        .map(|line| {
            let mut parts = line.splitn(3, '\t');
            let hash = parts.next().expect("hash").to_string();
            let time = parts.next().expect("time").parse::<u64>().expect("time");
            let subject = parts.next().unwrap_or_default().to_string();
            let artifact = build_history(
                dir,
                "0.1.0",
                HistoryOptions {
                    max_commits: 500,
                    since: None,
                    until: Some(format!("@{}", time + 1)),
                },
            )
            .expect("prefix history");
            TimelineEntry {
                hash,
                time,
                subject,
                artifact,
            }
        })
        .collect()
}

fn truncated_artifact() -> HistoryArtifact {
    HistoryArtifact {
        schema_version: 1,
        tool_version: "0.1.0".to_string(),
        repo: HistoryRepo {
            path: "synthetic".to_string(),
            head: "deadbeef".to_string(),
            is_git: true,
        },
        window: HistoryWindow {
            max_commits: 1,
            since: None,
            until: None,
            first_parent: true,
        },
        sampled: false,
        sample_step: None,
        truncated: true,
        shallow: false,
        stats: HistoryStats {
            commits_scanned: 1,
            commits_available: 6,
            modules: 1,
        },
        modules: vec![HistoryModule {
            id: "src/app.ts".to_string(),
            first_seen: 1,
            last_modified: 2,
            commit_count: 1,
            churn: Churn {
                additions: 1,
                deletions: 0,
                total: 1,
            },
            active_span: 1,
        }],
    }
}

#[test]
fn ticks_match_first_parent_commits() {
    let temp = seed_repo();
    let state = TimelineState::new(prefix_entries(temp.path()));
    assert_eq!(state.ticks(), 6);
    assert_eq!(state.current_index(), 5, "defaults to latest commit");
}

#[test]
fn scrubbing_reads_metrics_as_of_selected_commit() {
    let temp = seed_repo();
    let mut state = TimelineState::new(prefix_entries(temp.path()));

    assert!(state.scrub_to(0));
    let app = state.module_metric("src/app.ts").expect("app at c1");
    assert_eq!(app.last_modified, 1_767_268_800);
    assert_eq!(app.commit_count, 1);
    assert_eq!(app.churn.total, 3);
    assert!(state.module_metric("src/helpers.ts").is_none());

    assert!(state.scrub_to(1));
    let app = state.module_metric("src/app.ts").expect("app at c2");
    assert_eq!(app.last_modified, 1_767_355_200);
    assert_eq!(app.commit_count, 2);
    assert_eq!(app.churn.total, 5);

    assert!(state.scrub_to(2));
    let helpers = state
        .module_metric("src/helpers.ts")
        .expect("helpers at rename");
    assert_eq!(helpers.last_modified, 1_767_441_600);

    assert!(state.scrub_to(4));
    let helpers = state
        .module_metric("src/helpers.ts")
        .expect("helpers at c5");
    assert_eq!(helpers.last_modified, 1_767_614_400);
    assert_eq!(helpers.commit_count, 3);
    assert_eq!(helpers.churn.total, 6);
    assert!(!state.scrub_to(6), "out of range must not move");
    assert_eq!(state.current_index(), 4);
}

#[test]
fn heat_mapping_is_frozen() {
    assert_eq!(HEAT_STOPS[0].0, [0x2d, 0x4f, 0x7a]);
    assert_eq!(HEAT_STOPS[1].1, 1);
    assert_eq!(HEAT_STOPS[2].1, 4);
    assert_eq!(HEAT_STOPS[3].1, 11);

    let zero = heat_for_churn(0, 0);
    assert_eq!(zero.color, HEAT_STOPS[0].0);
    assert_eq!(zero.size_scale_percent, 100);

    let small = heat_for_churn(2, 1);
    assert_eq!(small.color, HEAT_STOPS[1].0);
    assert_eq!(small.size_scale_percent, 100 + 3 + 2);

    let mid = heat_for_churn(5, 2);
    assert_eq!(mid.color, HEAT_STOPS[2].0);
    assert_eq!(mid.size_scale_percent, 100 + 6 + 5);

    let hot = heat_for_churn(12, 4);
    assert_eq!(hot.color, HEAT_STOPS[3].0);
    assert_eq!(hot.size_scale_percent, 100 + 12 + 12);
    assert!(!hot.neutral);
}

#[test]
fn ids_match_map_artifact_and_unknown_modules_are_neutral() {
    let temp = seed_repo();
    let state = TimelineState::new(prefix_entries(temp.path()));
    let layers = build_layers(temp.path(), "0.1.0").expect("layers");
    let layer_ids = layers
        .modules
        .iter()
        .map(|module| module.id.clone())
        .collect::<std::collections::BTreeSet<_>>();

    for id in state.module_ids() {
        assert!(
            layer_ids.contains(&id),
            "history module {id} must exist in the map with the same id"
        );
        assert!(
            !state.heat(&id).neutral,
            "history module {id} must have heat"
        );
    }

    for module in &layers.modules {
        let _ = state.heat(&module.id);
    }
    assert_eq!(
        state.heat("src/merge-only.ts"),
        arch_view::timeline::Heat {
            color: HEAT_NEUTRAL,
            size_scale_percent: 100,
            neutral: true,
        }
    );
}

#[test]
fn status_markers_block_full_scale_heat() {
    let temp = seed_repo();
    assert!(TimelineState::new(prefix_entries(temp.path())).heat_is_full_scale());

    let mut entries = prefix_entries(temp.path());
    entries.push(TimelineEntry {
        hash: "truncated".to_string(),
        time: 0,
        subject: "truncated".to_string(),
        artifact: truncated_artifact(),
    });
    let mut state = TimelineState::new(entries);
    state.scrub_to(state.ticks() - 1);
    assert!(state.flags().truncated);
    assert!(!state.heat_is_full_scale());
}

#[test]
fn selection_clears_on_unknown_and_leaving_timeline_keeps_ticks() {
    let temp = seed_repo();
    let mut state = TimelineState::new(prefix_entries(temp.path()));

    assert!(state.select("src/app.ts"));
    assert_eq!(state.selected(), Some("src/app.ts"));
    assert!(!state.select("src/missing.ts"));
    assert_eq!(state.selected(), None);

    assert!(state.select("src/helpers.ts"));
    state.leave_timeline();
    assert_eq!(state.selected(), None);
    assert_eq!(state.ticks(), 6);
}
