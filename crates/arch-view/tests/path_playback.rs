//! ticket 07 B 档断言清单（实现前先写）：
//!
//! 1. 播放到第 k 步时，高亮节点集合等于路径前 k 步的符号，高亮边集合等于前 k 步相邻对。
//! 2. 每一步只有「进入当前步」的一条边是追光边；第 1 步没有入边。
//! 3. 前进、后退、暂停的状态与计数可读且互不串扰。
//! 4. 动画默认关闭：关闭后路径仍以完整静态形式可见，帧调度计数恒为 0；开启后才允许调度。
//! 5. 非路径节点不透明度低于基线阈值，路径节点保持可见。
//! 6. fly-to 默认关闭；开启后镜头中心落在当前步骤节点包围盒内。
//! 7. 路径结束后停在完整路径，不循环；再次播放从第 1 步开始。
//! 8. C 档帧预算：3 步与 20 步路径各跑 3 次，模型单帧更新 p95 ≤16.7ms，读数归档到 `out/frame-timing.json`。
//!
//! 真实窗口渲染帧时间未建立自动化（依赖硬件与驱动），本票只归档模型更新耗时并显式标注。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use arch_core::guided_path::{build_path, EdgeEvidence, GuidedPathArtifact, PathStep};
use arch_core::RepoInfo;
use arch_view::layout::Rect;
use arch_view::playback::{PlaybackState, NON_PATH_OPACITY, PATH_OPACITY};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn fixture_path() -> GuidedPathArtifact {
    build_path(
        &workspace_root().join("tests/fixtures/path"),
        "0.1.0",
        "src/cli.ts#module-toplevel",
        "src/utils/helper.ts#function:helper",
    )
    .expect("fixture path")
}

#[test]
fn stepping_highlights_exact_prefix_of_path() {
    let path = fixture_path();
    let mut state = PlaybackState::new(&path);
    assert_eq!(state.current(), 0);
    assert!(state.highlighted_nodes().is_empty());
    assert!(state.highlighted_edges().is_empty());

    state.play();
    assert!(state.is_playing());
    assert_eq!(state.current(), 0, "play starts before the first step");

    for k in 1..=path.steps.len() {
        assert!(state.advance());
        assert_eq!(state.current(), k);

        let nodes = path.steps[..k]
            .iter()
            .map(|step| step.symbol.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(state.highlighted_nodes(), nodes);

        let edges = path.steps[..k]
            .windows(2)
            .map(|pair| (pair[0].symbol.clone(), pair[1].symbol.clone()))
            .collect::<BTreeSet<_>>();
        assert_eq!(state.highlighted_edges(), edges);
    }
    assert!(!state.is_playing(), "playback stops at the end");
}

#[test]
fn only_the_entering_edge_is_chased_each_step() {
    let path = fixture_path();
    let mut state = PlaybackState::new(&path);

    state.play();
    assert!(state.advance());
    assert_eq!(
        state.current_edge(),
        None,
        "first step has no entering edge"
    );

    assert!(state.advance());
    assert_eq!(
        state.current_edge(),
        Some((path.steps[0].symbol.clone(), path.steps[1].symbol.clone()))
    );
    assert!(state.advance());
    assert_eq!(
        state.current_edge(),
        Some((path.steps[1].symbol.clone(), path.steps[2].symbol.clone()))
    );
}

#[test]
fn forward_back_and_pause_are_independent() {
    let path = fixture_path();
    let mut state = PlaybackState::new(&path);

    state.play();
    assert!(state.advance());
    assert!(state.advance());
    state.pause();
    assert!(!state.is_playing());
    assert_eq!(state.current(), 2);

    assert!(state.back());
    assert_eq!(state.current(), 1);
    assert!(!state.is_playing());

    assert!(state.back());
    assert_eq!(state.current(), 0);
    assert!(!state.back(), "cannot go back before the first step");
}

#[test]
fn animation_off_schedules_zero_frames_but_keeps_static_path() {
    let path = fixture_path();
    let mut state = PlaybackState::new(&path);
    assert!(!state.animation_enabled(), "animation defaults off");
    assert!(state.static_full_path());

    state.play();
    for _ in 0..20 {
        assert!(!state.request_frame(), "off must not schedule frames");
    }
    assert_eq!(state.frame_requests(), 0);
    assert!(!state.needs_animation_frame());

    state.set_animation_enabled(true);
    assert!(!state.static_full_path());
    assert!(state.request_frame());
    assert_eq!(state.frame_requests(), 1);
    assert!(state.needs_animation_frame());
}

#[test]
fn non_path_nodes_are_dimmer_than_path_nodes() {
    let path = fixture_path();
    let mut state = PlaybackState::new(&path);

    let on_path = path.steps[0].symbol.as_str();
    assert_eq!(state.node_opacity(on_path), PATH_OPACITY);
    assert_eq!(
        state.node_opacity("src/isolated.ts#function:isolated"),
        NON_PATH_OPACITY
    );
    assert!(
        NON_PATH_OPACITY < 0.5,
        "non-path baseline must be a real fade"
    );
}

#[test]
fn fly_to_defaults_off_and_centers_inside_current_node() {
    let path = fixture_path();
    let mut state = PlaybackState::new(&path);
    assert!(!state.fly_to_enabled(), "fly-to defaults off");

    state.play();
    assert!(state.advance());
    state.set_fly_to(true);
    assert!(state.fly_to_enabled());

    let rect = Rect::new(100.0, 100.0, 220.0, 44.0);
    let rects = BTreeMap::from([(path.steps[0].symbol.clone(), rect)]);
    let center = state.camera_center(&rects).expect("camera center");
    assert!(
        rect.contains(center),
        "camera center must sit inside the step node"
    );
    assert_eq!(center, rect.center());
}

#[test]
fn path_end_stays_complete_and_replay_restarts() {
    let path = fixture_path();
    let mut state = PlaybackState::new(&path);

    state.play();
    for _ in 0..path.steps.len() {
        assert!(state.advance());
    }
    assert_eq!(state.current(), path.steps.len());
    assert!(!state.is_playing());
    assert!(!state.advance(), "end must not loop");

    state.play();
    assert_eq!(state.current(), 0);
    assert!(state.is_playing());
    assert!(state.advance());
    assert_eq!(state.current(), 1);
}

#[test]
fn frame_budget_for_three_and_twenty_step_paths() {
    let three = fixture_path();
    let twenty = synthetic_path(20);

    let mut report = Vec::new();
    let mut failures = Vec::new();

    for (label, path) in [("3-step", &three), ("20-step", &twenty)] {
        let mut samples = Vec::new();
        for _ in 0..3 {
            let mut state = PlaybackState::new(path);
            state.set_animation_enabled(true);
            state.play();
            let timer = Instant::now();
            while state.advance() {
                state.request_frame();
            }
            samples.push(timer.elapsed().as_secs_f64() * 1_000_000.0);
        }
        let median = median(&samples);
        let p95 = p95(&samples);
        report.push(format!(
            "{{\"path\":\"{label}\",\"runsUs\":[{:.1},{:.1},{:.1}],\"medianUs\":{:.1},\"p95Us\":{:.1}}}",
            samples[0], samples[1], samples[2], median, p95
        ));
        if p95 > 16_700.0 {
            failures.push(label);
        }
    }

    let hardware = format!(
        "{{\"os\":\"{}\",\"cpus\":\"{}\",\"processor\":\"{}\"}}",
        std::env::consts::OS,
        std::env::var("NUMBER_OF_PROCESSORS").unwrap_or_else(|_| "unknown".to_string()),
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".to_string())
    );
    let json = format!(
        "{{\"scope\":\"model-update-only\",\"budgetMs\":16.7,\"budgetUs\":16700,\"hardware\":{hardware},\"readings\":[{}]}}",
        report.join(",")
    );
    let out_dir = workspace_root().join("out");
    std::fs::create_dir_all(&out_dir).expect("create out dir");
    std::fs::write(out_dir.join("frame-timing.json"), &json).expect("write frame timing");
    eprintln!("frame timing: {json}");

    assert!(
        failures.is_empty(),
        "model update over budget for {failures:?}: {json}"
    );
}

fn synthetic_path(steps: usize) -> GuidedPathArtifact {
    GuidedPathArtifact {
        schema_version: 2,
        tool_version: "0.1.0".to_string(),
        repo: RepoInfo {
            path: "synthetic".to_string(),
            head: None,
            is_git: false,
        },
        from: "m#module-toplevel".to_string(),
        to: format!("m#function:f{:02}", steps - 1),
        found: true,
        length: steps,
        steps: (0..steps)
            .map(|index| PathStep {
                module: "m.ts".to_string(),
                symbol: format!("m.ts#function:f{:02}", index),
                layer: "utils",
                via: if index == 0 {
                    None
                } else {
                    Some("call".to_string())
                },
                evidence: if index == 0 {
                    None
                } else {
                    Some(EdgeEvidence {
                        file: "m.ts".to_string(),
                        line: index,
                        column: 1,
                    })
                },
                layer_from: if index == 0 { None } else { Some("utils") },
                layer_to: "utils",
                conditional: false,
                test_only: false,
                narrative: format!("step {index}"),
            })
            .collect(),
    }
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    sorted[sorted.len() / 2]
}

fn p95(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    sorted[(sorted.len() * 95).div_ceil(100) - 1]
}
