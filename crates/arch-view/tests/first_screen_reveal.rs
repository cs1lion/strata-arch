//! P5 首屏分层揭示（一次性、可跳过、关闭态等价最终态）的 B 档断言清单。
//!
//! 判据先于实现：下列断言必须在实现前给出非零失败读数（断言级，不是编译错误）。
//!
//! 1. 关闭态：不请求任何动画、不改变最终视觉（所有层带不透明度 1、位移 0）。
//! 2. 与加载态衔接：`pending` 时不得推进；未 `start()` 前内容保持最终可见态，
//!    不会出现「加载态还在、揭示已经抢帧」的隐藏首帧。
//! 3. 层序一次性：层带按传入顺序依次淡入/升起，进度单调不减；播完停在最终态，
//!    不循环、不呼吸、不自动重播。
//! 4. 可打断：`skip()` 立即落到最终态且幂等；打断后任何进度写入都不再生效。
//! 5. 进度只由框架动画时钟给出的「已播时长占比」驱动：相同 progress 得到相同读数；
//!    总时长是常量，不随帧数/更新次数变化；源码内无墙钟、随机源与定时器。
//! 6. 与 P1 互斥：有路径播放（P1）时不得启用首屏揭示。

use std::fs;
use std::path::{Path, PathBuf};

use arch_view::reveal::{reveal_allowed, RevealState, REVEAL_DURATION_MS, REVEAL_RISE_PX};

fn started(stages: usize) -> RevealState {
    let mut state = RevealState::new(true, stages);
    assert!(
        state.start(),
        "数据就绪后 start() 必须成功（stages={stages}）"
    );
    state
}

/// 把 `0..=1` 的已播时长占比按 `steps` 段喂给状态机；返回最终状态。
fn play(state: &mut RevealState, steps: u32) {
    for step in 0..=steps {
        state.set_progress(step as f32 / steps as f32);
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
fn disabled_state_ignores_progress_and_keeps_final_state() {
    let mut reveal = RevealState::new(false, 5);
    assert!(!reveal.is_enabled(), "关闭态必须不可见");
    assert!(reveal.is_finished(), "关闭态等价于已结束");
    assert!(!reveal.is_pending() && !reveal.is_active());
    assert!(!reveal.start(), "关闭态不能被启动");
    assert!(!reveal.skip(), "关闭态没有可跳过的揭示");
    reveal.set_progress(0.25);
    assert_eq!(
        reveal.overall_progress(),
        1.0,
        "关闭态的进度不得被迟到值拉回中途"
    );
    all_final(&reveal);
}

#[test]
fn reveal_waits_for_data_ready_and_does_not_hide_content() {
    let mut reveal = RevealState::new(true, 4);
    assert!(reveal.is_pending(), "启用后、start 前必须处于 pending");
    assert!(!reveal.is_active(), "pending 不算正在揭示");
    reveal.set_progress(0.4);
    assert_eq!(reveal.progress(), 1.0, "pending 不得推进进度");
    // 数据未就绪时若已经渲染，内容必须保持最终可见态，不能出现隐藏首帧。
    all_final(&reveal);

    assert!(reveal.start(), "数据就绪后启动");
    assert!(reveal.is_active(), "启动后进入揭示");
    assert!(!reveal.start(), "start 必须幂等");
    assert_eq!(
        reveal.opacity_for_stage(0),
        0.0,
        "揭示从第 0 层 0 不透明度开始"
    );
    assert_eq!(
        reveal.offset_for_stage(0),
        REVEAL_RISE_PX,
        "揭示从下沉位移开始"
    );
}

#[test]
fn stages_reveal_in_layer_order_without_reordering() {
    let mut reveal = started(5);
    for stage in 0..5 {
        assert_eq!(reveal.opacity_for_stage(stage), 0.0);
    }

    // 第 0 层用整段时长的 1/5 完成；此时第 1 层刚到起点。
    reveal.set_progress(0.2);
    assert_eq!(reveal.opacity_for_stage(0), 1.0, "第 0 层先完成");
    assert_eq!(
        reveal.opacity_for_stage(1),
        0.0,
        "第 1 层刚到起点，层序不得交错"
    );

    // 走到第 1 层的中途：它是半透明，第 2 层仍未开始。
    reveal.set_progress(0.3);
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
    for step in 31..=100 {
        reveal.set_progress(step as f32 / 100.0);
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
    play(&mut reveal, 40);
    assert!(reveal.is_finished(), "播完必须停在最终态");
    assert_eq!(reveal.progress(), 1.0, "进度不得回绕");
    all_final(&reveal);

    // 迟到帧给 0.2 也不得把最终态拖回中途。
    reveal.set_progress(0.2);
    assert_eq!(reveal.progress(), 1.0, "结束后进度必须冻结");
    assert!(!reveal.skip(), "结束后 skip 应为无操作");
    all_final(&reveal);
}

#[test]
fn skip_lands_on_final_state_and_is_idempotent() {
    let mut reveal = started(6);
    reveal.set_progress(0.3);
    assert!(reveal.is_active(), "skip 前必须仍在揭示");

    assert!(reveal.skip(), "交互打断必须生效");
    assert!(reveal.is_finished(), "打断后立即结束");
    assert_eq!(reveal.progress(), 1.0, "直接落到最终进度");
    all_final(&reveal);

    assert!(!reveal.skip(), "重复打断必须是无操作");
    reveal.set_progress(0.1);
    assert_eq!(reveal.progress(), 1.0);
    all_final(&reveal);
}

#[test]
fn progress_is_derived_only_from_framework_progress() {
    let mut first = started(4);
    let mut second = started(4);
    // 帧数不同：11 段 vs 100 段，但同一 progress 读数必须完全一致。
    for step in 0..=11 {
        first.set_progress(step as f32 / 11.0);
    }
    for step in 0..=100 {
        second.set_progress(step as f32 / 100.0);
    }
    assert_eq!(first.progress(), 1.0);
    assert_eq!(second.progress(), 1.0, "总进度只取决于已播时长占比");

    let mut a = started(4);
    let mut b = started(4);
    a.set_progress(0.375);
    b.set_progress(0.375);
    for stage in 0..4 {
        assert_eq!(
            a.opacity_for_stage(stage),
            b.opacity_for_stage(stage),
            "相同 progress 必须得到相同第 {stage} 层读数"
        );
        assert_eq!(
            a.offset_for_stage(stage),
            b.offset_for_stage(stage),
            "相同 progress 必须得到相同第 {stage} 层位移"
        );
    }
    assert_eq!(a.overall_progress(), b.overall_progress());

    // 进度单调：回喂更小的值不得回退。
    a.set_progress(0.1);
    assert_eq!(a.progress(), 0.375, "进度必须单调不减");
}

#[test]
fn duration_is_fixed_and_frame_independent() {
    // 判据：总时长是常量（约 900ms），与帧数/更新次数无关；各层按层数均分时长。
    assert_eq!(
        REVEAL_DURATION_MS, 900,
        "揭示总时长必须是固定 900ms，慢机器只减帧不改时长"
    );
    for steps in [2u32, 7, 60, 600] {
        let mut reveal = started(6);
        play(&mut reveal, steps);
        assert!(
            reveal.is_finished(),
            "{steps} 段更新后必须播完（总时长不由更新次数决定）"
        );
        // 第 i 层恰好在 (i+1)/6 的已播占比处完成。
        for stage in 0..6 {
            let mut probe = started(6);
            probe.set_progress((stage + 1) as f32 / 6.0);
            assert_eq!(
                probe.opacity_for_stage(stage),
                1.0,
                "第 {stage} 层必须在 {}ms 处完成",
                (stage + 1) as u64 * REVEAL_DURATION_MS / 6
            );
        }
    }
}

#[test]
fn reveal_is_mutually_exclusive_with_p1_path_playback() {
    assert!(reveal_allowed(true, false), "无 P1 路径时允许首屏揭示");
    assert!(
        !reveal_allowed(true, true),
        "存在 P1 路径播放时必须让位，同一时段只能一个主动效"
    );
    assert!(!reveal_allowed(false, false), "未请求时保持关闭态");
}

#[test]
fn reveal_state_is_not_frame_counted() {
    // 旧判据（静态帧预算 total_frames ≤ 16）已按新判据改写为固定时长；
    // 这里守住「状态机里不再有帧计数」这一结构性事实。
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reveal.rs");
    let text = fs::read_to_string(&path).expect("读取 reveal.rs");
    for forbidden in [
        "advance_frame",
        "total_frames",
        "REVEAL_FRAME_BUDGET",
        "frames:",
    ] {
        assert!(
            !text.contains(forbidden),
            "reveal.rs 不得再出现 {forbidden:?}：进度只能来自框架动画时钟的时长占比"
        );
    }
    assert!(
        text.contains("REVEAL_DURATION_MS"),
        "reveal.rs 必须保留固定时长常量"
    );
}

#[test]
fn reveal_source_has_no_wallclock_random_or_timer() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reveal.rs");
    let text = fs::read_to_string(&path).expect("读取 reveal.rs");
    for forbidden in [
        "Instant",
        "SystemTime",
        "rand",
        "random",
        "thread_rng",
        "Timer",
    ] {
        assert!(
            !text.contains(forbidden),
            "reveal.rs 不得出现 {forbidden:?}：进度只能由 GPUI 帧回调给的时间驱动"
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
