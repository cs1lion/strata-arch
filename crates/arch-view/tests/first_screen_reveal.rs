//! P5 首屏分层揭示（一次性、可跳过、默认关）的 B 档断言清单。
//!
//! 判据先于实现：下列断言必须在实现前给出非零失败读数（断言级，不是编译错误）。
//!
//! 1. 默认关：关闭态不请求任何帧、不改变最终视觉（所有层带不透明度 1、位移 0）。
//! 2. 与加载态衔接：`pending` 时不得推进；未 `start()` 前内容保持最终可见态，
//!    不会出现「加载态还在、揭示已经抢帧」的隐藏首帧。
//! 3. 层序一次性：层带按传入顺序依次淡入/升起，进度单调不减；播完停在最终态，
//!    不循环、不呼吸、不自动重播。
//! 4. 可打断：`skip()` 立即落到最终态且幂等；打断后任何帧推进都不再生效。
//! 5. 进度只由帧计数驱动：相同帧数得到相同读数；源码内无墙钟、随机源与定时器。
//! 6. 与 P1 互斥：有路径播放（P1）时不得启用首屏揭示。

use std::fs;
use std::path::{Path, PathBuf};

use arch_view::reveal::{
    reveal_allowed, RevealState, REVEAL_FADE_FRAMES, REVEAL_GAP_FRAMES, REVEAL_RISE_PX,
};

fn started(stages: usize) -> RevealState {
    let mut state = RevealState::new(true, stages);
    assert!(
        state.start(),
        "数据就绪后 start() 必须成功（stages={stages}）"
    );
    state
}

fn advance(state: &mut RevealState, frames: u32) {
    for _ in 0..frames {
        state.advance_frame();
    }
}

fn all_final(state: &RevealState) {
    for stage in 0..state.stages() {
        assert_eq!(
            state.opacity_for_stage(stage),
            1.0,
            "第 {stage} 层必须落在不透明度 1"
        );
        assert_eq!(
            state.offset_for_stage(stage),
            0.0,
            "第 {stage} 层必须落在位移 0"
        );
    }
    assert_eq!(state.overall_progress(), 1.0, "整体进度必须落到 1");
}

fn source_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = fs::read_dir(root)
        .expect("读取 arch-view/src")
        .map(|entry| entry.expect("读取目录项").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn default_off_requests_no_frames_and_keeps_final_state() {
    let mut reveal = RevealState::new(false, 5);
    assert!(!reveal.is_enabled(), "默认必须关闭");
    assert!(reveal.is_finished(), "关闭态等价于已结束");
    assert!(!reveal.is_pending() && !reveal.is_active());
    assert!(!reveal.start(), "关闭态不能被启动");
    assert!(!reveal.skip(), "关闭态没有可跳过的揭示");
    assert!(!reveal.advance_frame(), "关闭态不得推进帧");
    assert_eq!(reveal.frames(), 0, "关闭态不得消耗帧");
    all_final(&reveal);
}

#[test]
fn reveal_waits_for_data_ready_and_does_not_hide_content() {
    let mut reveal = RevealState::new(true, 4);
    assert!(reveal.is_pending(), "启用后、start 前必须处于 pending");
    assert!(!reveal.is_active(), "pending 不算正在揭示");
    assert!(!reveal.advance_frame(), "pending 不得推进帧");
    assert_eq!(reveal.frames(), 0);
    // 数据未就绪时若已经渲染，内容必须保持最终可见态，不能出现隐藏首帧。
    all_final(&reveal);

    assert!(reveal.start(), "数据就绪后启动");
    assert!(reveal.is_active(), "启动后进入揭示");
    assert!(!reveal.start(), "start 必须幂等");
    assert_eq!(reveal.opacity_for_stage(0), 0.0, "揭示从第 0 层 0 不透明度开始");
    assert_eq!(reveal.offset_for_stage(0), REVEAL_RISE_PX, "揭示从下沉位移开始");
}

#[test]
fn stages_reveal_in_layer_order_without_reordering() {
    let mut reveal = started(5);
    assert_eq!(reveal.opacity_for_stage(0), 0.0);
    assert_eq!(reveal.opacity_for_stage(1), 0.0);
    assert_eq!(reveal.opacity_for_stage(2), 0.0);

    advance(&mut reveal, REVEAL_FADE_FRAMES);
    assert_eq!(reveal.opacity_for_stage(0), 1.0, "第 0 层先完成");
    assert_eq!(
        reveal.opacity_for_stage(1),
        0.0,
        "层间间隙内第 1 层仍未开始，层序不得交错"
    );

    advance(&mut reveal, REVEAL_GAP_FRAMES);
    assert_eq!(reveal.opacity_for_stage(1), 0.0, "第 1 层刚到起点");
    advance(&mut reveal, REVEAL_FADE_FRAMES / 2);
    let second = reveal.opacity_for_stage(1);
    assert!(
        second > 0.0 && second < 1.0,
        "第 1 层应处于中途，实际 {second}"
    );
    assert_eq!(
        reveal.opacity_for_stage(2),
        0.0,
        "第 2 层不得早于第 1 层开始"
    );

    let mut previous = (0..5)
        .map(|stage| reveal.opacity_for_stage(stage))
        .collect::<Vec<_>>();
    while reveal.is_active() {
        reveal.advance_frame();
        for stage in 0..5 {
            let current = reveal.opacity_for_stage(stage);
            assert!(
                current >= previous[stage],
                "第 {stage} 层不透明度必须单调不减: {} -> {current}",
                previous[stage]
            );
            previous[stage] = current;
        }
    }
    all_final(&reveal);
}

#[test]
fn finishes_once_and_never_loops_or_replays() {
    let mut reveal = started(4);
    let total = reveal.total_frames();
    assert!(total > 0, "多层揭示必须有时长");

    while reveal.is_active() {
        reveal.advance_frame();
    }
    assert!(reveal.is_finished(), "播完必须停在最终态");
    assert_eq!(reveal.frames(), total, "帧计数不得回绕");
    all_final(&reveal);

    let frames = reveal.frames();
    assert!(!reveal.advance_frame(), "结束后不得再推进");
    assert_eq!(reveal.frames(), frames, "结束后帧计数必须冻结");
    assert!(!reveal.skip(), "结束后 skip 应为无操作");
    all_final(&reveal);
}

#[test]
fn skip_lands_on_final_state_and_is_idempotent() {
    let mut reveal = started(6);
    advance(&mut reveal, 3);
    assert!(reveal.is_active(), "skip 前必须仍在揭示");

    assert!(reveal.skip(), "交互打断必须生效");
    assert!(reveal.is_finished(), "打断后立即结束");
    assert_eq!(reveal.frames(), reveal.total_frames(), "直接落到最终帧");
    all_final(&reveal);

    assert!(!reveal.skip(), "重复打断必须是无操作");
    let frames = reveal.frames();
    assert!(!reveal.advance_frame(), "打断后不得继续演");
    assert_eq!(reveal.frames(), frames);
    all_final(&reveal);
}

#[test]
fn progress_is_derived_only_from_frame_count() {
    let mut first = started(4);
    let mut second = started(4);
    advance(&mut first, 11);
    advance(&mut second, 11);

    assert_eq!(first.frames(), 11);
    assert_eq!(second.frames(), 11);
    for stage in 0..4 {
        assert_eq!(
            first.opacity_for_stage(stage),
            second.opacity_for_stage(stage),
            "相同帧数必须得到相同第 {stage} 层读数"
        );
        assert_eq!(
            first.offset_for_stage(stage),
            second.offset_for_stage(stage),
            "相同帧数必须得到相同第 {stage} 层位移"
        );
    }
    assert_eq!(first.overall_progress(), second.overall_progress());

    first.advance_frame();
    assert_eq!(first.frames(), 12, "每帧只推进 1");
    assert_eq!(second.frames(), 11, "另一状态不受影响");
}

#[test]
fn reveal_is_mutually_exclusive_with_p1_path_playback() {
    assert!(
        reveal_allowed(true, false),
        "无 P1 路径时允许首屏揭示"
    );
    assert!(
        !reveal_allowed(true, true),
        "存在 P1 路径播放时必须让位，同一时段只能一个主动效"
    );
    assert!(!reveal_allowed(false, false), "未请求时保持默认关");
}

#[test]
fn reveal_source_has_no_wallclock_random_or_timer() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reveal.rs");
    let text = fs::read_to_string(&path).expect("读取 reveal.rs");
    for forbidden in ["Instant", "SystemTime", "rand", "random", "thread_rng", "Timer"] {
        assert!(
            !text.contains(forbidden),
            "reveal.rs 不得出现 {forbidden:?}：进度只能由 GPUI 帧回调驱动"
        );
    }
}

#[test]
fn arch_view_sources_have_no_explicit_wallclock_or_random() {
    for path in source_files() {
        let text = fs::read_to_string(&path).expect("读取 arch-view 源文件");
        for forbidden in [
            "std::time::Instant",
            "Instant::now",
            "SystemTime",
            "rand::",
            "thread_rng",
        ] {
            assert!(
                !text.contains(forbidden),
                "{} 出现墙钟/随机源 {forbidden:?}",
                path.display()
            );
        }
    }
}
