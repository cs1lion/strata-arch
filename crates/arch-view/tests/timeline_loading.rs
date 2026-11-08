//! ticket 09「先开窗、后加载」断言清单（B 档，先写测试）。
//!
//! 1. 加载态不是白屏：窗口内文案非空且写明当前阶段（读取提交历史 / 刻度数 / 构建分层 / 计算布局）。
//! 2. 失败可见：数据准备失败时，错误文本出现在窗口文案里，不能只打到 stderr。
//! 3. 动效语义走帧计数：`LoadingState` 的相位只由帧数派生。
//! 4. 计数一致：后台准备完成后的刻度/模块/分层数与同步路径固定读数逐项相等
//!    （fixture `6 ticks, 3 modules, 1 bands`）。
//! 5. `--auto-close-ms` 早于数据就绪：进程干净退出（exit 0、无 panic、不挂起）。
//!
//! 未验证项：加载态的像素呈现（gpui 0.2.2 无离屏导出，见 ticket 11）。

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use arch_view::loading::{prepare_timeline, LoadStatus, LoadingStage, LoadingState};

const AUTO_CLOSE_BUDGET: Duration = Duration::from_secs(15);

fn fixture_history() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/history")
}

fn assert_visible(lines: &[String], needle: &str, context: &str) {
    assert!(
        lines.iter().any(|line| line.contains(needle)),
        "{context}: 期望窗口文案包含 {needle:?}，实际 {lines:?}"
    );
}

#[test]
fn loading_state_is_never_blank_and_names_the_stage() {
    let status = LoadStatus::Loading(LoadingState::new());
    let lines = status.visible_lines();
    assert!(!lines.is_empty(), "加载态不能是空白: {lines:?}");
    assert!(
        !lines.iter().all(|line| line.trim().is_empty()),
        "加载态文案不能全为空行: {lines:?}"
    );
    assert_visible(&lines, "正在准备演化时间线", "加载态标题");
    assert_visible(&lines, "读取提交历史", "首阶段文案");
    assert!(
        status.is_loading() && !status.is_ready(),
        "新建状态必须是 Loading"
    );
}

#[test]
fn loading_state_reports_tick_count_after_git_log() {
    let status = LoadStatus::Loading(LoadingState::with_stage(LoadingStage::SamplingHistory {
        ticks: 617,
    }));
    let lines = status.visible_lines();
    assert_visible(&lines, "617", "历史刻度总数");
    assert_visible(&lines, "准备历史", "采样阶段文案");
}

#[test]
fn loading_state_names_each_background_stage() {
    let building = LoadingState::with_stage(LoadingStage::BuildingLayers);
    assert_visible(&building.visible_lines(), "构建分层", "分层阶段");

    let layout = LoadingState::with_stage(LoadingStage::ComputingLayout);
    assert_visible(&layout.visible_lines(), "计算布局", "布局阶段");
}

#[test]
fn failed_state_shows_the_error_in_the_window() {
    let status = LoadStatus::Failed {
        message: "git log 失败: bad revision".to_string(),
    };
    let lines = status.visible_lines();
    assert!(!lines.is_empty(), "失败态不能是空白: {lines:?}");
    assert_visible(&lines, "失败", "失败态标题");
    assert_visible(&lines, "git log 失败: bad revision", "失败原因");
    assert!(!status.is_ready(), "失败态不能当成就绪");
}

#[test]
fn ready_state_has_no_status_panel() {
    assert!(LoadStatus::Ready.is_ready());
    assert!(
        LoadStatus::Ready.visible_lines().is_empty(),
        "就绪后不应残留加载文案"
    );
}

#[test]
fn loading_animation_phase_is_derived_from_frame_count() {
    let mut loading = LoadingState::new();
    assert_eq!(loading.frames(), 0);
    assert_eq!(loading.phase(), 0);
    loading.advance_frame();
    assert_eq!(loading.frames(), 1);
    assert_eq!(loading.phase(), 1);
    for _ in 0..3 {
        loading.advance_frame();
    }
    assert_eq!(loading.frames(), 4);
    assert_eq!(loading.phase(), 0, "相位按帧数 4 取模，不读墙钟");
}

#[test]
fn prepared_counts_match_the_sync_path_golden() {
    let prepared = prepare_timeline(&fixture_history());
    assert!(prepared.is_ok(), "fixture 准备失败: {:?}", prepared.err());
    let prepared = prepared.unwrap();
    // 同步路径固定读数：`timeline: 6 ticks, 3 modules, 1 bands`。
    assert_eq!(prepared.timeline.ticks(), 6, "刻度数必须与同步路径一致");
    assert_eq!(prepared.layout.nodes.len(), 3, "模块数必须与同步路径一致");
    assert_eq!(prepared.layout.bands.len(), 1, "分层数必须与同步路径一致");
}

#[test]
fn auto_close_before_ready_exits_cleanly() {
    for attempt in 1..=3 {
        let mut child = Command::new(env!("CARGO_BIN_EXE_arch-view"))
            .arg("timeline")
            .arg(fixture_history())
            .args(["--auto-close-ms", "20"])
            .env("STRATA_PHASE_MARKERS", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn arch-view timeline");

        let deadline = Instant::now() + AUTO_CLOSE_BUDGET;
        let output = loop {
            match child.try_wait().expect("poll timeline process") {
                Some(_) => break child.wait_with_output().expect("collect output"),
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let output = child.wait_with_output().expect("collect killed output");
                    panic!(
                        "第 {attempt} 次自动关闭未在 {} 秒内退出（挂起窗口）: stdout={} stderr={}",
                        AUTO_CLOSE_BUDGET.as_secs(),
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "第 {attempt} 次自动关闭退出码 {:?}: stdout={stdout} stderr={stderr}",
            output.status.code()
        );
        assert!(
            !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
            "第 {attempt} 次自动关闭出现 panic: {stderr}"
        );
        assert!(
            stdout.contains("phase: load:start"),
            "第 {attempt} 次未进入加载路径: {stdout}"
        );
        assert!(
            stdout.contains("phase: quit:auto"),
            "第 {attempt} 次未触发自动关闭: {stdout}"
        );
    }
}
