//! B 档（断言先行）：P2 短镜头转场的结构断言。
//!
//! 断言清单与对应可见行为：
//! 1. 转场必须多于一步：中间进度下相机中心落在起终点之间且不等于任一端点（区分「瞬移」与「转场」）。
//! 2. 转场关闭时行为与改动前的 `apply_fly_to` 完全一致（回归判据）。
//! 3. 起终点相同 / 目标缺失时不启动转场。
//! 4. 手动平移或缩放中断转场：相机停在当前插值位置，不弹回起点、不跳到终点。
//! 5. 转场途中点击按当前插值相机换算世界坐标（命中测试一致性）。
//! 6. 单步路径转场完整走完后精确落在终点，无上冲、无 NaN。
//!
//! 实现前本文件应真实失败：`begin_fly_to` / `set_transition_progress` 尚不存在。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use arch_core::callgraph::build_call_graph;
use arch_core::guided_path::{build_path, GuidedPathArtifact};
use arch_core::layers::build_layers;
use arch_view::layout::{layout_map, LayoutConfig, Point, Rect};
use arch_view::playback::PlaybackState;
use arch_view::view::{CanvasState, Viewport, SYMBOL_CANVAS_HEIGHT, SYMBOL_CANVAS_WIDTH};

const SIZE: (f32, f32) = (SYMBOL_CANVAS_WIDTH, SYMBOL_CANVAS_HEIGHT);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("tests/fixtures/callgraph")
}

fn drilled_state() -> CanvasState {
    let layers = build_layers(&fixture(), "test").expect("layers fixture");
    let map = layout_map(&layers, LayoutConfig::default());
    let mut state = CanvasState::new(map);
    assert!(
        state.drill_down("src/a.ts", &build_call_graph(&fixture(), "test").unwrap()),
        "fixture must drill into src/a.ts"
    );
    state
}

fn beta_to_alpha() -> GuidedPathArtifact {
    build_path(
        &fixture(),
        "test",
        "src/a.ts#function:beta",
        "src/a.ts#function:alpha",
    )
    .expect("fixture path beta -> alpha")
}

fn screen_center() -> Point {
    Point {
        x: SIZE.0 / 2.0,
        y: SIZE.1 / 2.0,
    }
}

fn rects(state: &CanvasState) -> BTreeMap<String, Rect> {
    state
        .symbol_layout()
        .expect("drilled symbol layout")
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.rect))
        .collect()
}

fn camera_target(state: &CanvasState, path: &GuidedPathArtifact) -> Point {
    let mut playback = PlaybackState::new(path);
    playback.play();
    playback.set_fly_to(true);
    while playback.advance() {}
    playback
        .camera_center(&rects(state))
        .expect("current step must have a node rect")
}

fn center_under(viewport: Viewport) -> Point {
    viewport.world_point(screen_center())
}

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn target_viewport(from: Viewport, target: Point) -> Viewport {
    Viewport {
        pan_x: screen_center().x - target.x * from.zoom,
        pan_y: screen_center().y - target.y * from.zoom,
        zoom: from.zoom,
    }
}

#[test]
fn transition_passes_through_intermediate_camera_centers() {
    let mut state = drilled_state();
    let path = beta_to_alpha();
    let target = camera_target(&state, &path);
    assert!(
        state.transition_enabled(),
        "transition must be enabled by default (P2 is the shipped behavior)"
    );

    let before = state.viewport();
    let before_center = center_under(before);
    assert!(
        state.begin_fly_to(Some(target), SIZE),
        "fly-to must start a transition instead of teleporting"
    );

    let samples = [0.0_f32, 0.25, 0.5, 0.75, 1.0];
    let mut centers = Vec::new();
    for progress in samples {
        state.set_transition_progress(progress);
        centers.push(center_under(state.viewport()));
    }

    let full = distance(before_center, target);
    assert!(
        full > 100.0,
        "fixture must produce a measurable displacement: {full}px"
    );

    let mut line = String::new();
    for (progress, center) in samples.iter().zip(centers.iter()) {
        line.push_str(&format!(
            "p={progress:.2} ({:.2},{:.2}) ",
            center.x, center.y
        ));
    }
    eprintln!("transition centers: {line}");

    assert!(
        distance(centers[0], before_center) < 0.01,
        "p=0 must equal the start center: {:?} vs {:?}",
        centers[0],
        before_center
    );
    assert!(
        distance(centers[4], target) < 0.01,
        "p=1 must land exactly on the target: {:?} vs {:?}",
        centers[4],
        target
    );

    // 反瞬移判据：中间帧必须严格位于起终点之间，且不等于任一端点。
    for index in [1, 2, 3] {
        let center = centers[index];
        assert!(
            distance(center, before_center) > 0.5,
            "intermediate frame {index} must not equal the start: {center:?}"
        );
        assert!(
            distance(center, target) > 0.5,
            "intermediate frame {index} must not equal the target: {center:?}"
        );
        assert!(
            distance(center, target) < full,
            "intermediate frame {index} must lie strictly between endpoints: {center:?}"
        );
    }
    assert!(
        distance(centers[2], target) < distance(centers[1], target)
            && distance(centers[3], target) < distance(centers[2], target),
        "camera must approach the target monotonically: {line}"
    );
    let midpoint = Point {
        x: (before_center.x + target.x) / 2.0,
        y: (before_center.y + target.y) / 2.0,
    };
    assert!(
        distance(centers[2], midpoint) < 0.5,
        "p=0.5 must be the geometric midpoint (linear tween): {:?} vs {:?}",
        centers[2],
        midpoint
    );
    assert!(
        state.transition().is_none(),
        "transition must finish at p=1 and clear its state"
    );
    eprintln!(
        "transition terminal error: {:.6}px, displacement: {:.4}px",
        distance(centers[4], target),
        full
    );
}

#[test]
fn transition_off_is_identical_to_instant_fly_to() {
    let mut instant = drilled_state();
    let mut animated = drilled_state();
    let path = beta_to_alpha();
    let target = camera_target(&instant, &path);

    let moved = instant.apply_fly_to(Some(target), SIZE);
    assert!(
        moved,
        "the legacy instant path must still move the viewport"
    );

    animated.set_transition_enabled(false);
    let started = animated.begin_fly_to(Some(target), SIZE);
    assert!(
        !started,
        "with the transition switched off no transition may start"
    );
    assert!(
        animated.transition().is_none(),
        "with the transition switched off no transition state may remain"
    );
    assert_eq!(
        animated.viewport(),
        instant.viewport(),
        "switched-off behavior must match the legacy instant fly-to exactly"
    );
    eprintln!(
        "transition-off viewport: leg={:?} off={:?}",
        instant.viewport(),
        animated.viewport()
    );
}

#[test]
fn same_endpoints_and_missing_target_start_no_transition() {
    let mut state = drilled_state();
    let before = state.viewport();
    let current_center = center_under(before);

    assert!(
        !state.begin_fly_to(Some(current_center), SIZE),
        "when the camera already centers the target, no transition may start"
    );
    assert_eq!(
        state.viewport(),
        before,
        "same endpoints must leave the viewport untouched"
    );
    assert!(state.transition().is_none());

    assert!(
        !state.begin_fly_to(None, SIZE),
        "a missing target must not start a transition"
    );
    assert_eq!(state.viewport(), before, "missing target must not move");
    assert!(state.transition().is_none());
    eprintln!("same-endpoints viewport unchanged: {:?}", before);
}

#[test]
fn manual_pan_interrupts_and_keeps_the_interpolated_camera() {
    let mut state = drilled_state();
    let path = beta_to_alpha();
    let target = camera_target(&state, &path);
    let from = state.viewport();

    assert!(state.begin_fly_to(Some(target), SIZE));
    state.set_transition_progress(0.5);
    let mid = state.viewport();

    state.pan_by(15.0, -25.0);
    let after = state.viewport();
    assert!(
        state.transition().is_none(),
        "manual pan must cancel the running transition"
    );
    assert!(
        (after.pan_x - (mid.pan_x + 15.0)).abs() < 0.01
            && (after.pan_y - (mid.pan_y - 25.0)).abs() < 0.01,
        "after interrupt the camera must keep the interpolated position plus the drag: {after:?} vs {mid:?}"
    );
    let after_center = center_under(after);
    assert!(
        distance(after_center, center_under(from)) > 100.0
            && distance(after_center, target) > 100.0,
        "interrupted camera must not snap back to the start or jump to the target: {after_center:?}"
    );
    eprintln!(
        "pan interrupt: from {:?} -> mid {:?} -> after {:?}",
        center_under(from),
        center_under(mid),
        after_center
    );
}

#[test]
fn manual_zoom_interrupts_and_keeps_the_interpolated_camera() {
    let mut state = drilled_state();
    let path = beta_to_alpha();
    let target = camera_target(&state, &path);
    let from = state.viewport();

    assert!(state.begin_fly_to(Some(target), SIZE));
    state.set_transition_progress(0.5);
    let mid = state.viewport();

    state.zoom_at(screen_center(), 1.2);
    let after = state.viewport();
    assert!(
        state.transition().is_none(),
        "manual zoom must cancel the running transition"
    );
    assert!(
        (after.zoom - mid.zoom * 1.2).abs() < 0.01,
        "zoom must apply on top of the interpolated camera: {after:?} vs {mid:?}"
    );
    assert!(
        distance(center_under(after), center_under(mid)) < 0.5,
        "zooming at the screen center must keep the interpolated camera center: {after:?} vs {mid:?}"
    );
    assert!(
        distance(center_under(after), center_under(from)) > 100.0
            && distance(center_under(after), target) > 100.0,
        "interrupted camera must not snap back to the start or jump to the target"
    );
}

#[test]
fn clicks_during_transition_use_the_interpolated_camera() {
    let mut state = drilled_state();
    let path = beta_to_alpha();
    let target = camera_target(&state, &path);
    let layout = state.symbol_layout().expect("drilled symbol layout");
    let beta = layout
        .nodes
        .iter()
        .find(|node| node.id == "src/a.ts#function:beta")
        .expect("fixture must expose beta")
        .clone();

    let from = state.viewport();
    assert!(state.begin_fly_to(Some(target), SIZE));
    state.set_transition_progress(0.5);
    let mid = state.viewport();

    let world = beta.rect.center();
    let click = mid.screen_point(world);
    let hit = state.select_at(click);

    let from_world = from.world_point(click);
    let to_world = target_viewport(from, target).world_point(click);
    assert!(
        distance(from_world, world) > 100.0 && distance(to_world, world) > 100.0,
        "the probe must not accidentally match the start or end camera"
    );
    assert_eq!(
        hit.as_deref(),
        Some(beta.id.as_str()),
        "click during transition must map through the interpolated camera: click {click:?}, from-world {from_world:?}, to-world {to_world:?}, hit {hit:?}"
    );
    eprintln!(
        "click consistency: click ({:.2},{:.2}) world ({:.2},{:.2}) from-world ({:.2},{:.2}) to-world ({:.2},{:.2}) hit {:?}",
        click.x, click.y, world.x, world.y, from_world.x, from_world.y, to_world.x, to_world.y, hit
    );
}

#[test]
fn single_step_path_transition_lands_without_overshoot() {
    let mut state = drilled_state();
    let single = build_path(
        &fixture(),
        "test",
        "src/a.ts#function:alpha",
        "src/a.ts#function:alpha",
    )
    .expect("single step path");
    assert_eq!(single.steps.len(), 1, "fixture must stay a one-step path");

    let target = camera_target(&state, &single);
    let from = state.viewport();
    assert!(state.begin_fly_to(Some(target), SIZE));

    let mut previous = distance(center_under(from), target);
    for step in 1..=4 {
        state.set_transition_progress(step as f32 / 4.0);
        let center = center_under(state.viewport());
        assert!(
            center.x.is_finite() && center.y.is_finite(),
            "single-step transition must not produce NaN/Inf: {center:?}"
        );
        let current = distance(center, target);
        assert!(
            current <= previous + 0.5,
            "single-step transition must not overshoot: step {step}, {current}px vs {previous}px"
        );
        previous = current;
    }
    assert!(
        state.transition().is_none(),
        "single-step transition must finish and clear its state"
    );
    assert!(
        distance(center_under(state.viewport()), target) < 0.01,
        "single-step transition must land exactly on the target"
    );
}
