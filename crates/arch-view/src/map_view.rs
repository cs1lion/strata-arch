//! 架构地图视图：模块层带 / 依赖边 / 下钻符号层，外加 HUD、Inspector、Layer Legend 叠层。
//!
//! 从 `main.rs` 抽出（Issue #22 收口）。绘制顺序与动画驱动保持拆分前一致：
//! 点阵背景 → 层带 → 边 → 节点卡片 → 文本叠层；转场与揭示进度只由 GPUI 动画时钟给出。

use std::cell::Cell;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arch_core::callgraph::{build_call_graph, CallGraphArtifact};
use arch_core::guided_path::build_path;
use arch_core::layers::build_layers;
use arch_view::layout::{layout_map, LayoutConfig, MapLayout, Point as LayoutPoint};
use arch_view::playback::{PlaybackState, CONTEXT_OPACITY, NON_PATH_OPACITY};
use arch_view::reveal::{reveal_allowed, RevealState, REVEAL_DURATION_MS};
use arch_view::search::{interpret_key, KeyModifiers, SearchAction, SearchKey, SearchState};
use arch_view::view::{
    find_reachable_symbol_pair, layer_palette, node_visual_geometry, paint_symbol_edges,
    paint_symbol_edges_with_roles, rect_fully_outside, segment_fully_outside, CanvasState,
    EdgeDisplayMode, EdgeVisual as EdgeDraw, Level, SymbolLayout, Viewport, CULL_MARGIN,
};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, Animation, AnimationExt, App, ClickEvent, Context,
    Div, Entity, FocusHandle, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Render, ScrollDelta, ScrollWheelEvent, Stateful, Timer, Window,
};

use super::overlays::{
    edge_canvas, focus_label_element, search_panel_element, selection_border, split_module_label,
    PlaybackOverlay,
};
use super::phase_marker;
use super::shell::{
    autopan_frames, evidence_viewport, frame_trace_enabled, set_shell_stage, LoadingShell,
    RenderSkip, RenderTuning,
};

/// 后台准备架构地图；每完成一个阶段就更新窗口内的阶段文案。
pub(crate) fn spawn_map_loading(
    shell: Entity<LoadingShell<MapView>>,
    repo: PathBuf,
    playback_request: Option<(String, String)>,
    reveal_requested: bool,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        phase_marker("map:prep:start");

        let mut playback = None;
        if let Some((from, to)) = playback_request {
            set_shell_stage(&shell, cx, "构建引导路径");
            let path_repo = repo.clone();
            let built = cx
                .background_executor()
                .spawn(async move {
                    build_path(&path_repo, env!("CARGO_PKG_VERSION"), &from, &to)
                        .map_err(|error| format!("构建路径失败: {error}"))
                })
                .await;
            let path = match built {
                Ok(path) => path,
                Err(message) => return map_load_failed(&shell, cx, message),
            };
            let mut state = PlaybackState::new(&path);
            // 证据钩子（ticket 11）：仅用于本地像素证据链，让 P1 上下文淡出态可在启动时复现；
            // 正常启动不受影响，等价于用户在 UI 里点一次「动画: 关 → 开」。
            if std::env::var_os("STRATA_EVIDENCE_ANIMATION").is_some() {
                state.set_animation_enabled(true);
            }
            playback = Some(state);
        }

        let layers_repo = repo.clone();
        let layered = cx
            .background_executor()
            .spawn(async move { build_layers(&layers_repo, env!("CARGO_PKG_VERSION")) })
            .await;
        phase_marker("map:prep:layers");
        let artifact = match layered {
            Ok(artifact) => artifact,
            Err(error) => return map_load_failed(&shell, cx, format!("构建层产物失败: {error}")),
        };

        set_shell_stage(&shell, cx, "构建调用图");
        let callgraph_repo = repo.clone();
        let graphed = cx
            .background_executor()
            .spawn(async move { build_call_graph(&callgraph_repo, env!("CARGO_PKG_VERSION")) })
            .await;
        phase_marker("map:prep:callgraph");
        let callgraph = match graphed {
            Ok(callgraph) => callgraph,
            Err(error) => return map_load_failed(&shell, cx, format!("构建调用图失败: {error}")),
        };

        set_shell_stage(&shell, cx, "计算布局");
        let layout = cx
            .background_executor()
            .spawn(async move { layout_map(&artifact, LayoutConfig::default()) })
            .await;
        phase_marker("map:prep:layout");

        let band_summary = layout
            .bands
            .iter()
            .map(|band| format!("{}={}", band.layer, band.node_count))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "map: {} modules, {} edges, {} bands [{}], canvas {}x{}",
            layout.nodes.len(),
            layout.edges.len(),
            layout.bands.len(),
            band_summary,
            layout.width,
            layout.height
        );

        let reveal_enabled = reveal_allowed(reveal_requested, playback.is_some());
        let mut reveal = RevealState::new(reveal_enabled, layout.bands.len());
        reveal.start();
        println!(
            "map: reveal={} layers={} durationMs={}",
            if reveal.is_enabled() { "on" } else { "off" },
            layout.bands.len(),
            REVEAL_DURATION_MS
        );

        let built_view = cx.update(|cx| {
            let evidence_camera = evidence_viewport();
            let map_repo = repo.clone();
            cx.new(move |cx| {
                let search = SearchState::new(&layout);
                let focus_handle = cx.focus_handle();
                MapView {
                    state: {
                        let mut state = CanvasState::new(layout);
                        if let Some(viewport) = evidence_camera {
                            state.set_viewport(viewport);
                        }
                        state
                    },
                    repo: map_repo,
                    path_from: None,
                    path_to: None,
                    path_status: None,
                    layer_filter: None,
                    callgraph,
                    playback,
                    ticker_running: false,
                    fly_to_anchor: None,
                    press_origin: None,
                    drag_last: None,
                    dragged: false,
                    transition_generation: 0,
                    transition_delta: Rc::new(Cell::new(0.0)),
                    reveal,
                    reveal_progress: Rc::new(Cell::new(0.0)),
                    reveal_scheduled: false,
                    reveal_start_marked: false,
                    reveal_mid_marked: false,
                    frame_count: 0,
                    autopan_remaining: autopan_frames(),
                    autopan_scheduled: false,
                    ab_segments: std::env::var("STRATA_AB_CYCLE")
                        .map(|value| {
                            value
                                .split(',')
                                .map(|part| part.trim().to_string())
                                .filter(|part| !part.is_empty())
                                .collect()
                        })
                        .unwrap_or_default(),
                    ab_segment_frames: std::env::var("STRATA_AB_SEGMENT_FRAMES")
                        .ok()
                        .and_then(|value| value.trim().parse().ok())
                        .unwrap_or(60)
                        .max(1),
                    search,
                    focus_handle,
                    keyboard_focus_pending: true,
                }
            })
        });
        let map = match built_view {
            Ok(map) => map,
            Err(_) => return,
        };
        phase_marker("map:prep:done");
        let _ = shell.update(cx, |shell, cx| {
            shell.ready(map);
            cx.notify();
        });
    })
    .detach();
}

pub(crate) fn map_load_failed(
    shell: &Entity<LoadingShell<MapView>>,
    cx: &mut gpui::AsyncApp,
    message: String,
) {
    let _ = shell.update(cx, |shell, cx| {
        shell.fail(message);
        cx.notify();
    });
}

pub(crate) const INSPECTOR_MAX_ITEMS: usize = 8;

pub(crate) struct MapView {
    state: CanvasState,
    repo: PathBuf,
    path_from: Option<String>,
    path_to: Option<String>,
    path_status: Option<String>,
    layer_filter: Option<&'static str>,
    callgraph: CallGraphArtifact,
    playback: Option<PlaybackState>,
    ticker_running: bool,
    /// 最近一次 fly-to 已居中的步骤符号；用于只在步骤变化时移动相机。
    fly_to_anchor: Option<String>,
    press_origin: Option<LayoutPoint>,
    drag_last: Option<LayoutPoint>,
    dragged: bool,
    /// 转场代际：每次启动新转场 +1，让 `with_animation` 的元素 id 重新计时。
    transition_generation: u64,
    /// 当前帧的转场进度，由 `with_animation` 的 animator 写入、帧回调读取。
    transition_delta: Rc<Cell<f32>>,
    /// P5 首屏分层揭示：一次性、时长驱动（进度来自 GPUI 动画时钟）、可打断。
    reveal: RevealState,
    /// 当前帧的揭示进度，由 `with_animation` 的 animator 写入、帧回调读取。
    reveal_progress: Rc<Cell<f32>>,
    reveal_scheduled: bool,
    reveal_start_marked: bool,
    reveal_mid_marked: bool,
    /// 诊断：已渲染帧数（仅用于首帧打印绘制计数）。
    frame_count: u32,
    /// 诊断：剩余自动平移帧数（`STRATA_AUTOPAN_FRAMES`）。
    autopan_remaining: Option<u32>,
    autopan_scheduled: bool,
    /// 同进程 A/B：按帧段交替的配置名与每段帧数（`STRATA_AB_CYCLE` / `STRATA_AB_SEGMENT_FRAMES`）。
    ab_segments: Vec<String>,
    ab_segment_frames: u32,
    /// ticket 20：模块搜索/聚焦的纯状态机。
    search: SearchState,
    focus_handle: FocusHandle,
    /// 首帧把键盘焦点交给画布，保证 `/` 与 `Ctrl/Cmd+F` 能到达。
    keyboard_focus_pending: bool,
}

type MapBandDraw = (
    arch_view::layout::Rect,
    String,
    bool,
    bool,
    f32,
    f32,
    usize,
    String,
);

impl MapView {
    /// fly-to 接线：只在启用且当前步骤变化时把相机移到当前节点；
    /// 关闭时清空锚点，避免再次打开时沿用旧状态。
    fn apply_fly_to_if_needed(
        &mut self,
        scene: &Scene,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Viewport {
        if let Some(playback) = &self.playback {
            let anchor = playback
                .steps()
                .get(playback.current().saturating_sub(1))
                .map(|step| step.symbol.clone());
            if playback.fly_to_enabled() && anchor.as_deref() != self.fly_to_anchor.as_deref() {
                let rects = match scene {
                    Scene::Symbols(symbols) => symbols
                        .nodes
                        .iter()
                        .map(|node| (node.id.clone(), node.rect))
                        .collect::<std::collections::BTreeMap<_, _>>(),
                    Scene::Modules(map) => playback
                        .steps()
                        .iter()
                        .filter_map(|step| {
                            map.nodes
                                .iter()
                                .find(|node| node.id == step.module)
                                .map(|node| (step.symbol.clone(), node.rect))
                        })
                        .collect::<std::collections::BTreeMap<_, _>>(),
                };
                let target = playback.camera_center(&rects);
                let viewport_size = (
                    f32::from(window.viewport_size().width),
                    f32::from(window.viewport_size().height),
                );
                if self.state.begin_fly_to(target, viewport_size) {
                    self.transition_generation += 1;
                    self.transition_delta = Rc::new(Cell::new(0.0));
                    Self::schedule_transition_step(self, window, cx);
                }
                self.fly_to_anchor = anchor;
            } else if !playback.fly_to_enabled() {
                self.fly_to_anchor = None;
                self.state.cancel_transition();
            }
        } else if !self.search.is_focused() {
            // 无播放时相机归用户；但搜索聚焦自己发起的转场不能被这里清掉。
            self.fly_to_anchor = None;
            self.state.cancel_transition();
        }
        self.state.viewport()
    }

    /// 每个 GPUI 动画帧把 `with_animation` 给出的进度写入相机；转场未结束时续订下一帧。
    fn schedule_transition_step(
        view: &mut MapView,
        window: &mut Window,
        cx: &mut Context<MapView>,
    ) {
        if view.state.transition().is_none() {
            return;
        }
        cx.on_next_frame(window, |view, window, cx| {
            view.state
                .set_transition_progress(view.transition_delta.get());
            if view.state.transition().is_some() {
                MapView::schedule_transition_step(view, window, cx);
            }
            cx.notify();
        });
    }

    fn screen_position(position: gpui::Point<gpui::Pixels>) -> LayoutPoint {
        LayoutPoint {
            x: f32::from(position.x),
            y: f32::from(position.y),
        }
    }

    fn finish_press(&mut self, position: LayoutPoint) {
        if self.dragged {
            return;
        }
        let prev_selected = self.state.selected().map(str::to_string);
        let hit = self.state.select_at(position);
        if matches!(self.state.level(), Level::Modules) {
            if let Some(module) = hit {
                if prev_selected.as_deref() == Some(module.as_str())
                    && self.state.drill_down(&module, &self.callgraph)
                {
                    self.search.reset();
                }
            }
        }
    }

    fn trigger_interactive_path(&mut self, cx: &mut Context<Self>) {
        let (Some(from_mod), Some(to_mod)) = (self.path_from.clone(), self.path_to.clone()) else {
            self.path_status = None;
            return;
        };
        if from_mod == to_mod {
            self.path_status = Some("起点与终点模块相同，请选择不同模块".to_string());
            return;
        }
        let Some((from_sym, to_sym)) =
            find_reachable_symbol_pair(&from_mod, &to_mod, &self.callgraph)
        else {
            self.path_status = Some("两模块间无可达调用路径".to_string());
            return;
        };
        self.path_status = Some("正在构建调用路径...".to_string());
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let built = cx
                .background_executor()
                .spawn(async move {
                    build_path(&repo, env!("CARGO_PKG_VERSION"), &from_sym, &to_sym)
                        .map_err(|error| format!("构建路径失败: {error}"))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                match built {
                    Ok(path) => {
                        view.playback = Some(PlaybackState::new(&path));
                        view.ticker_running = false;
                        view.fly_to_anchor = None;
                        view.search.reset();
                        view.path_status = None;
                    }
                    Err(message) => {
                        view.path_status = Some(message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 用户任何交互都立即结束揭示并落到最终态；已结束/默认关时无操作。
    fn interrupt_reveal(&mut self) {
        if self.reveal.skip() {
            phase_marker("reveal:skip");
        }
    }

    /// ticket 20：键盘入口。任何按键先结束揭示；有 `--from/--to` 时搜索整体禁用。
    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        phase_marker(&format!("key:{}", event.keystroke.key));
        self.interrupt_reveal();
        if self.playback.is_some() {
            return;
        }
        let keystroke = &event.keystroke;
        let key = interpret_key(
            &keystroke.key,
            keystroke.key_char.as_deref(),
            KeyModifiers {
                secondary: keystroke.modifiers.secondary(),
                alt: keystroke.modifiers.alt,
            },
        );
        let viewport_size = (
            f32::from(window.viewport_size().width),
            f32::from(window.viewport_size().height),
        );
        if !self.search.is_open() && !keystroke.modifiers.secondary() && !keystroke.modifiers.alt {
            match keystroke.key.as_str() {
                "enter" => {
                    if matches!(self.state.level(), Level::Modules) {
                        if let Some(module) = self.state.selected().map(str::to_string) {
                            if self.state.drill_down(&module, &self.callgraph) {
                                self.search.reset();
                            }
                            cx.notify();
                            return;
                        }
                    }
                }
                "escape" => {
                    if self.search.focus_label().is_none()
                        && (self.state.clear_selection() || self.layer_filter.take().is_some())
                    {
                        cx.notify();
                        return;
                    }
                }
                "e" | "E" => {
                    self.state.cycle_edge_mode();
                    cx.notify();
                    return;
                }
                "0" | "r" | "R" => {
                    self.state.fit_to_viewport(viewport_size);
                    cx.notify();
                    return;
                }
                "=" | "+" => {
                    self.state.zoom_at_center(1.25, viewport_size);
                    cx.notify();
                    return;
                }
                "-" => {
                    self.state.zoom_at_center(0.8, viewport_size);
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }
        match self.search.handle(key, self.state.viewport()) {
            SearchAction::None => {}
            SearchAction::Opened => phase_marker("search:open"),
            SearchAction::Closed => phase_marker("search:close"),
            SearchAction::Focused(request) => {
                // 标签计数直接来自一跳集合，供像素证据与人工核对。
                if let Some(label) = self.search.focus_label() {
                    phase_marker(&format!(
                        "search:label module={} upstream={} downstream={}",
                        label.module, label.upstream, label.downstream
                    ));
                }
                phase_marker("search:focus");
                if self.state.begin_fly_to(Some(request.target), viewport_size) {
                    self.transition_generation += 1;
                    self.transition_delta = Rc::new(Cell::new(0.0));
                    Self::schedule_transition_step(self, window, cx);
                }
            }
            SearchAction::Cleared(snapshot) => {
                // 复原是精确的：中断进行中的转场后逐字段写回快照。
                self.state.cancel_transition();
                self.state.set_viewport(snapshot);
                phase_marker("search:clear");
            }
        }
        // 输入、选中移动、开合、聚焦都会改变面板内容，统一要求重绘。
        cx.notify();
    }

    /// 场景元素树构造：只依赖 `self` 与当帧相机参数，不注册事件、不读墙钟。
    /// 单独成方法，供 ticket 21 路线 A 的缓存子视图复用（本提交仍由 render 直接调用）。
    fn add_scene_elements(
        &self,
        mut root: Stateful<Div>,
        scene: &Scene,
        viewport: Viewport,
        viewport_size: (f32, f32),
    ) -> Stateful<Div> {
        let skip = RenderSkip::from_env();
        let labels = self.state.labels_visible() && !skip.labels;
        let selected = self.state.selected().map(ToOwned::to_owned);
        let highlighted = self.state.highlighted();
        let tuning = self.current_tuning();
        let overlay = self.playback.as_ref().map(PlaybackOverlay::from_state);
        let fade_enabled = self
            .playback
            .as_ref()
            .map(|playback| playback.animation_enabled())
            .unwrap_or(false);
        let reveal_overall = self.reveal.overall_progress();
        let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
        let grid_viewport = viewport;
        let grid_theme = theme.clone();
        root = root.child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(0.))
                .size_full()
                .child(canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        arch_view::arch_view_canvas::paint_dot_grid(
                            window,
                            bounds,
                            &grid_viewport,
                            &grid_theme,
                        );
                    },
                )),
        );
        match scene {
            Scene::Modules(map) => {
                // 聚焦态：焦点集合内 1.0，其余沿用 P1 的 0.68；不改变绘制顺序。
                let search_focus = self.search.focus_set();
                let stage_of_layer = map
                    .bands
                    .iter()
                    .enumerate()
                    .map(|(index, band)| (band.layer, index))
                    .collect::<std::collections::BTreeMap<&str, usize>>();
                if !skip.bands {
                    let band_draws: Vec<MapBandDraw> = map
                        .bands
                        .iter()
                        .enumerate()
                        .map(|(index, band)| {
                            (
                                band.rect,
                                band.layer.to_string(),
                                band.is_unclassified,
                                band.is_external,
                                self.reveal.opacity_for_stage(index),
                                self.reveal.offset_for_stage(index),
                                band.node_count,
                                format!("{} · {}", band.layer.to_uppercase(), band.node_count),
                            )
                        })
                        .collect();
                    let band_vp = viewport;
                    let band_vp_size = viewport_size;
                    let band_cull = tuning.cull_viewport;
                    let band_theme = theme.clone();
                    root = root.child(div().absolute().left(px(0.)).top(px(0.)).size_full().child(
                        canvas(
                            move |bounds, _, _| bounds,
                            move |bounds, _, window, _| {
                                use gpui::{quad, size, BorderStyle, Corners, Edges};
                                for (
                                    rect,
                                    layer,
                                    is_unclassified,
                                    is_external,
                                    opacity,
                                    offset_y,
                                    _count,
                                    _label,
                                ) in &band_draws
                                {
                                    let sr = band_vp.screen_rect(*rect);
                                    let sr = arch_view::layout::Rect {
                                        x: sr.x,
                                        y: sr.y + offset_y,
                                        width: sr.width,
                                        height: sr.height,
                                    };
                                    if band_cull
                                        && arch_view::view::rect_fully_outside(
                                            sr,
                                            band_vp_size,
                                            CULL_MARGIN,
                                        )
                                    {
                                        continue;
                                    }
                                    let (fill_c, border_c, header_c) = band_theme.band_style(
                                        layer,
                                        *is_unclassified,
                                        *is_external,
                                        1.0,
                                    );
                                    let b = gpui::Bounds {
                                        origin: point(
                                            bounds.origin.x + px(sr.x),
                                            bounds.origin.y + px(sr.y),
                                        ),
                                        size: size(px(sr.width.max(0.0)), px(sr.height.max(0.0))),
                                    };
                                    // apply opacity by drawing with alpha; gpui quads take colors (Hsla) - adjust alpha
                                    let mut fill_c = fill_c;
                                    fill_c.a *= opacity;
                                    let mut border_c = border_c;
                                    border_c.a *= opacity;
                                    let mut header_c = header_c;
                                    header_c.a *= opacity;
                                    window.paint_quad(quad(
                                        b,
                                        Corners::all(
                                            band_theme.band_radius * band_vp.zoom.clamp(0.6, 1.2),
                                        ),
                                        fill_c,
                                        Edges::all(px(1.0)),
                                        border_c,
                                        BorderStyle::default(),
                                    ));
                                    let header_bar = gpui::Bounds {
                                        origin: point(b.origin.x + px(1.0), b.origin.y + px(6.0)),
                                        size: size(
                                            px(3.5),
                                            px((16.0 * band_vp.zoom).clamp(10.0, 22.0)),
                                        ),
                                    };
                                    window.paint_quad(quad(
                                        header_bar,
                                        Corners::all(px(2.0)),
                                        header_c,
                                        Edges::all(px(0.0)),
                                        gpui::transparent_black(),
                                        BorderStyle::default(),
                                    ));
                                }
                            },
                        ),
                    ));
                    // keep layer labels as lightweight DOM overlay for readability (text not painted in canvas)
                    for (index, band) in map.bands.iter().enumerate() {
                        let sr = viewport.screen_rect(band.rect);
                        let oy = self.reveal.offset_for_stage(index);
                        let op = self.reveal.opacity_for_stage(index);
                        if tuning.cull_viewport
                            && rect_fully_outside(
                                arch_view::layout::Rect {
                                    x: sr.x,
                                    y: sr.y + oy,
                                    width: sr.width,
                                    height: sr.height,
                                },
                                viewport_size,
                                CULL_MARGIN,
                            )
                        {
                            continue;
                        }
                        let header =
                            theme.layer_accent(band.layer, band.is_unclassified, band.is_external);
                        root = root.child(
                            div()
                                .absolute()
                                .left(px(sr.x + 10.))
                                .top(px(sr.y + oy + 4.))
                                .opacity(op)
                                .text_xs()
                                .text_color(header)
                                .child(format!(
                                    "{} · {}",
                                    band.layer.to_uppercase(),
                                    band.node_count
                                )),
                        );
                    }
                }
                let base_opacity =
                    (if fade_enabled { CONTEXT_OPACITY } else { 1.0 }) * reveal_overall;
                let edge_mode = self.state.edge_mode();
                if !skip.edges && edge_mode != EdgeDisplayMode::Hidden {
                    let layer_nodes = self.layer_filter.map(|target_layer| {
                        map.nodes
                            .iter()
                            .filter(|node| node.layer == target_layer)
                            .map(|node| node.id.as_str())
                            .collect::<BTreeSet<_>>()
                    });
                    let edges = map
                        .edges
                        .iter()
                        .filter_map(|edge| {
                            if edge_mode == EdgeDisplayMode::FocusedOnly {
                                let related = match search_focus {
                                    Some(set) => {
                                        set.focus == edge.source || set.focus == edge.target
                                    }
                                    None => {
                                        if let Some(id) = self.state.selected() {
                                            id == edge.source || id == edge.target
                                        } else if let Some(nodes) = &layer_nodes {
                                            nodes.contains(edge.source.as_str())
                                                || nodes.contains(edge.target.as_str())
                                        } else {
                                            false
                                        }
                                    }
                                };
                                if !related {
                                    return None;
                                }
                            }
                            let from = viewport.screen_point(edge.from);
                            let to = viewport.screen_point(edge.to);
                            if tuning.cull_viewport
                                && segment_fully_outside(from, to, viewport_size, CULL_MARGIN)
                            {
                                return None;
                            }
                            let selected_id = self.state.selected();
                            let is_layer_edge = search_focus.is_none()
                                && selected_id.is_none()
                                && layer_nodes.as_ref().is_some_and(|nodes| {
                                    nodes.contains(edge.source.as_str())
                                        || nodes.contains(edge.target.as_str())
                                });
                            let is_selected_edge = search_focus.is_none()
                                && selected_id
                                    .is_some_and(|id| id == edge.source || id == edge.target);
                            let opacity = match search_focus {
                                Some(set) => {
                                    set.edge_opacity(&edge.source, &edge.target) * reveal_overall
                                }
                                None if selected_id.is_some() => {
                                    if is_selected_edge {
                                        reveal_overall
                                    } else {
                                        CONTEXT_OPACITY * reveal_overall
                                    }
                                }
                                None if layer_nodes.is_some() => {
                                    if is_layer_edge {
                                        reveal_overall
                                    } else {
                                        0.18 * reveal_overall
                                    }
                                }
                                None => base_opacity,
                            };
                            Some(EdgeDraw {
                                from,
                                to,
                                adjusted: edge.direction_adjusted,
                                strong: is_selected_edge || is_layer_edge,
                                opacity,
                            })
                        })
                        .collect::<Vec<_>>();
                    root = root.child(edge_canvas(
                        edges,
                        map.width * viewport.zoom,
                        map.height * viewport.zoom,
                        viewport.zoom,
                        tuning.batch_edges,
                    ));
                }

                if !skip.edges {
                    if let Some(playback) = &self.playback {
                        let centers = map
                            .nodes
                            .iter()
                            .map(|node| (node.id.clone(), node.rect.center()))
                            .collect::<std::collections::BTreeMap<_, _>>();
                        let mut path_edges = Vec::new();
                        for (index, pair) in playback.steps().windows(2).enumerate() {
                            let (source_module, target_module) = (&pair[0].module, &pair[1].module);
                            if fade_enabled && source_module == target_module {
                                continue;
                            }
                            let Some(from) = centers.get(source_module) else {
                                continue;
                            };
                            let Some(to) = centers.get(target_module) else {
                                continue;
                            };
                            let (strong, opacity) = if fade_enabled {
                                let role = playback.module_edge_role(source_module, target_module);
                                (role.strong(), role.opacity())
                            } else {
                                (playback.current() == index + 2, 1.0)
                            };
                            let from = viewport.screen_point(*from);
                            let to = viewport.screen_point(*to);
                            if tuning.cull_viewport
                                && segment_fully_outside(from, to, viewport_size, CULL_MARGIN)
                            {
                                continue;
                            }
                            let opacity = opacity * reveal_overall;
                            path_edges.push(EdgeDraw {
                                from,
                                to,
                                adjusted: true,
                                strong,
                                opacity,
                            });
                        }
                        root = root.child(edge_canvas(
                            path_edges,
                            map.width * viewport.zoom,
                            map.height * viewport.zoom,
                            viewport.zoom,
                            tuning.batch_edges,
                        ));
                    }
                }

                let focus = overlay
                    .as_ref()
                    .and_then(|overlay| overlay.current_module.as_deref());
                let indices = if fade_enabled {
                    self.playback
                        .as_ref()
                        .map(|playback| {
                            let ids = map
                                .nodes
                                .iter()
                                .map(|node| node.id.as_str())
                                .collect::<Vec<_>>();
                            playback.module_paint_order(&ids)
                        })
                        .unwrap_or_else(|| (0..map.nodes.len()).collect())
                } else {
                    (0..map.nodes.len()).collect::<Vec<_>>()
                };
                // conservative TASK-2: paint module node cards in canvas, keep text/marker DOM overlays
                #[derive(Clone)]
                struct ModuleNodeDraw {
                    rect: arch_view::layout::Rect,
                    accent: gpui::Hsla,
                    bg: gpui::Hsla,
                    border: gpui::Hsla,
                    opacity: f32,
                }
                #[derive(Clone)]
                struct ModuleNodeOverlay {
                    rect: arch_view::layout::Rect,
                    opacity: f32,
                    label: Option<String>,
                    subtitle: Option<String>,
                    marker: Option<u32>,
                }
                let hovered = self.state.hovered().map(str::to_string);
                let mut node_draws: Vec<ModuleNodeDraw> = Vec::new();
                let mut node_overlays: Vec<ModuleNodeOverlay> = Vec::new();
                for index in indices {
                    if skip.nodes {
                        break;
                    }
                    let node = &map.nodes[index];
                    let opacity = match &self.playback {
                        Some(playback) if fade_enabled => playback.module_context_opacity(&node.id),
                        Some(_) => overlay
                            .as_ref()
                            .map(|overlay| {
                                if overlay.path_modules.contains(&node.id) {
                                    1.0
                                } else {
                                    NON_PATH_OPACITY
                                }
                            })
                            .unwrap_or(1.0),
                        None => match search_focus {
                            Some(set) => set.module_opacity(&node.id),
                            None if selected.is_some() => {
                                if selected.as_deref() == Some(node.id.as_str())
                                    || highlighted.contains(&node.id)
                                {
                                    1.0
                                } else {
                                    CONTEXT_OPACITY
                                }
                            }
                            None if self.layer_filter.is_some() => {
                                if self.layer_filter == Some(node.layer) {
                                    1.0
                                } else {
                                    0.28
                                }
                            }
                            None => 1.0,
                        },
                    };
                    let stage = stage_of_layer.get(node.layer).copied().unwrap_or(0);
                    let opacity = opacity * self.reveal.opacity_for_stage(stage);
                    let offset_y = self.reveal.offset_for_stage(stage);
                    let visual = node_visual_geometry(node, &viewport);
                    let drawn_rect = arch_view::layout::Rect::new(
                        visual.rect.x,
                        visual.rect.y + offset_y,
                        visual.rect.width,
                        visual.rect.height,
                    );
                    if tuning.cull_viewport
                        && rect_fully_outside(drawn_rect, viewport_size, CULL_MARGIN)
                    {
                        continue;
                    }
                    let accent =
                        theme.layer_accent(node.layer, node.is_unclassified, node.is_external);
                    let bg = if focus == Some(node.id.as_str())
                        || selected.as_deref() == Some(node.id.as_str())
                    {
                        theme.card_selected_bg
                    } else if highlighted.contains(&node.id) {
                        theme.card_neighbor_bg
                    } else {
                        let palette_hex = layer_palette(node.layer, node.is_unclassified);
                        let mut tint: gpui::Hsla = rgb(palette_hex).into();
                        tint.s = (tint.s * 0.65).clamp(0.0, 1.0);
                        tint.l = (tint.l * 0.55).clamp(0.08, 0.18);
                        tint
                    };
                    let border = if focus == Some(node.id.as_str())
                        || selected.as_deref() == Some(node.id.as_str())
                    {
                        theme.card_border_selected
                    } else if hovered.as_deref() == Some(node.id.as_str()) {
                        let mut b = accent;
                        b.a = 0.85;
                        b
                    } else if highlighted.contains(&node.id) {
                        let mut b = accent;
                        b.a = 0.65;
                        b
                    } else {
                        theme.card_border_default
                    };
                    node_draws.push(ModuleNodeDraw {
                        rect: drawn_rect,
                        accent,
                        bg,
                        border,
                        opacity,
                    });
                    let marker = if node.is_unclassified {
                        Some(0xd98a3d)
                    } else if node.is_external {
                        Some(0x6fb3b8)
                    } else {
                        None
                    };
                    let (label, subtitle) = if labels {
                        let (title, dir) = split_module_label(&node.id);
                        (Some(title), if viewport.zoom >= 0.85 { dir } else { None })
                    } else {
                        (None, None)
                    };
                    node_overlays.push(ModuleNodeOverlay {
                        rect: drawn_rect,
                        opacity,
                        label,
                        subtitle,
                        marker,
                    });
                }
                if !node_draws.is_empty() {
                    let nd = node_draws.clone();
                    let node_theme = theme.clone();
                    root = root.child(div().absolute().left(px(0.)).top(px(0.)).size_full().child(
                        canvas(
                            move |bounds, _, _| bounds,
                            move |bounds, _, window, _| {
                                use gpui::{quad, size, BorderStyle, Corners, Edges};
                                for item in &nd {
                                    let mut bg = item.bg;
                                    bg.a *= item.opacity;
                                    let mut border = item.border;
                                    border.a *= item.opacity;
                                    let mut accent = item.accent;
                                    accent.a *= item.opacity;
                                    let b = gpui::Bounds {
                                        origin: point(
                                            bounds.origin.x + px(item.rect.x),
                                            bounds.origin.y + px(item.rect.y),
                                        ),
                                        size: size(
                                            px(item.rect.width.max(0.0)),
                                            px(item.rect.height.max(0.0)),
                                        ),
                                    };
                                    window.paint_quad(quad(
                                        b,
                                        Corners::all(node_theme.card_radius),
                                        bg,
                                        Edges::all(px(1.0)),
                                        border,
                                        BorderStyle::default(),
                                    ));
                                    let accent_bar = gpui::Bounds {
                                        origin: point(b.origin.x + px(1.0), b.origin.y + px(4.0)),
                                        size: size(
                                            node_theme.accent_bar_width,
                                            px((f32::from(b.size.height) - 8.0).max(0.0)),
                                        ),
                                    };
                                    window.paint_quad(quad(
                                        accent_bar,
                                        Corners::all(px(2.0)),
                                        accent,
                                        Edges::all(px(0.0)),
                                        gpui::transparent_black(),
                                        BorderStyle::default(),
                                    ));
                                }
                            },
                        ),
                    ));
                }
                for overlay_item in &node_overlays {
                    if let Some(label) = &overlay_item.label {
                        let mut text_box = div()
                            .absolute()
                            .left(px(overlay_item.rect.x + 10.))
                            .top(px(overlay_item.rect.y))
                            .w(px((overlay_item.rect.width - 18.0).max(0.0)))
                            .h(px(overlay_item.rect.height))
                            .opacity(overlay_item.opacity)
                            .flex()
                            .flex_col()
                            .justify_center()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_bright)
                                    .child(label.clone()),
                            );
                        if let Some(subtitle) = &overlay_item.subtitle {
                            text_box = text_box.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(theme.text_secondary)
                                    .child(subtitle.clone()),
                            );
                        }
                        root = root.child(text_box);
                    }
                    if let Some(color) = overlay_item.marker {
                        root = root.child(
                            div()
                                .absolute()
                                .left(px(overlay_item.rect.x + overlay_item.rect.width - 9.))
                                .top(px(overlay_item.rect.y + 3.))
                                .size(px(6.))
                                .rounded_full()
                                .opacity(overlay_item.opacity)
                                .bg(rgb(color)),
                        );
                    }
                }
            }
            Scene::Symbols(symbols) => {
                let current_edge = overlay
                    .as_ref()
                    .and_then(|overlay| overlay.current_edge.as_ref());
                let painted = match (&self.playback, fade_enabled) {
                    (Some(playback), true) => {
                        paint_symbol_edges_with_roles(symbols, &viewport, playback)
                    }
                    _ => paint_symbol_edges(symbols, &viewport, current_edge),
                };
                let edges = painted
                    .into_iter()
                    .filter(|edge| {
                        !tuning.cull_viewport
                            || !segment_fully_outside(
                                edge.from,
                                edge.to,
                                viewport_size,
                                CULL_MARGIN,
                            )
                    })
                    .map(|edge| EdgeDraw {
                        from: edge.from,
                        to: edge.to,
                        adjusted: false,
                        strong: edge.strong,
                        opacity: edge.opacity * reveal_overall,
                    })
                    .collect::<Vec<_>>();
                root = root.child(edge_canvas(
                    edges,
                    symbols.width * viewport.zoom,
                    symbols.height * viewport.zoom,
                    viewport.zoom,
                    tuning.batch_edges,
                ));

                let focus = overlay
                    .as_ref()
                    .and_then(|overlay| overlay.current_symbol.as_deref());
                let indices = if fade_enabled {
                    self.playback
                        .as_ref()
                        .map(|playback| {
                            let ids = symbols
                                .nodes
                                .iter()
                                .map(|node| node.id.as_str())
                                .collect::<Vec<_>>();
                            playback.paint_order(&ids)
                        })
                        .unwrap_or_else(|| (0..symbols.nodes.len()).collect())
                } else {
                    (0..symbols.nodes.len()).collect::<Vec<_>>()
                };
                #[derive(Clone)]
                struct SymbolNodeDraw {
                    rect: arch_view::layout::Rect,
                    bg: gpui::Hsla,
                    border: gpui::Hsla,
                    accent: gpui::Hsla,
                    opacity: f32,
                }
                #[derive(Clone)]
                struct SymbolNodeOverlay {
                    rect: arch_view::layout::Rect,
                    opacity: f32,
                    badge_bg: gpui::Hsla,
                    badge_fg: gpui::Hsla,
                    label_color: gpui::Hsla,
                    kind: &'static str,
                    name: String,
                }
                let mut draw_items: Vec<SymbolNodeDraw> = Vec::new();
                let mut overlays: Vec<SymbolNodeOverlay> = Vec::new();
                for index in indices {
                    let node = &symbols.nodes[index];
                    let opacity = match &self.playback {
                        Some(playback) if fade_enabled => playback.node_context_opacity(&node.id),
                        Some(playback) => playback.node_opacity(&node.id),
                        None => 1.0,
                    };
                    let opacity = opacity * reveal_overall;
                    let rect = viewport.screen_rect(node.rect);
                    if tuning.cull_viewport && rect_fully_outside(rect, viewport_size, CULL_MARGIN)
                    {
                        continue;
                    }
                    let is_selected = selected.as_deref() == Some(node.id.as_str())
                        || focus == Some(node.id.as_str());
                    let is_highlighted = highlighted.contains(&node.id);
                    let style = theme.resolve_symbol_node_style(
                        node,
                        is_selected,
                        false,
                        is_highlighted,
                        is_selected || is_highlighted,
                    );
                    let border = selection_border(
                        &node.id,
                        &theme,
                        theme.card_border_default,
                        selected.as_deref(),
                        &highlighted,
                        focus,
                    );
                    draw_items.push(SymbolNodeDraw {
                        rect,
                        bg: style.bg,
                        border,
                        accent: style.accent_bar,
                        opacity,
                    });
                    if labels {
                        overlays.push(SymbolNodeOverlay {
                            rect,
                            opacity,
                            badge_bg: style.badge_bg,
                            badge_fg: style.badge_fg,
                            label_color: style.label_color,
                            kind: node.kind,
                            name: node.name.clone(),
                        });
                    }
                }
                if !draw_items.is_empty() {
                    let items = draw_items.clone();
                    let sym_theme = theme.clone();
                    root = root.child(div().absolute().left(px(0.)).top(px(0.)).size_full().child(
                        canvas(
                            move |bounds, _, _| bounds,
                            move |bounds, _, window, _| {
                                use gpui::{quad, size, BorderStyle, Corners, Edges};
                                for item in &items {
                                    let mut bg = item.bg;
                                    bg.a *= item.opacity;
                                    let mut border = item.border;
                                    border.a *= item.opacity;
                                    let mut accent = item.accent;
                                    accent.a *= item.opacity;
                                    let b = gpui::Bounds {
                                        origin: point(
                                            bounds.origin.x + px(item.rect.x),
                                            bounds.origin.y + px(item.rect.y),
                                        ),
                                        size: size(
                                            px(item.rect.width.max(0.0)),
                                            px(item.rect.height.max(0.0)),
                                        ),
                                    };
                                    window.paint_quad(quad(
                                        b,
                                        Corners::all(sym_theme.card_radius),
                                        bg,
                                        Edges::all(px(1.0)),
                                        border,
                                        BorderStyle::default(),
                                    ));
                                    let accent_bar = gpui::Bounds {
                                        origin: point(b.origin.x + px(1.0), b.origin.y + px(4.0)),
                                        size: size(
                                            sym_theme.accent_bar_width,
                                            px((f32::from(b.size.height) - 8.0).max(0.0)),
                                        ),
                                    };
                                    window.paint_quad(quad(
                                        accent_bar,
                                        Corners::all(px(2.0)),
                                        accent,
                                        Edges::all(px(0.0)),
                                        gpui::transparent_black(),
                                        BorderStyle::default(),
                                    ));
                                }
                            },
                        ),
                    ));
                }
                for item in overlays {
                    root = root.child(
                        div()
                            .absolute()
                            .left(px(item.rect.x + 10.))
                            .top(px(item.rect.y))
                            .w(px((item.rect.width - 16.0).max(0.0)))
                            .h(px(item.rect.height))
                            .opacity(item.opacity)
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .overflow_hidden()
                            .child(
                                div()
                                    .px(px(4.))
                                    .py(px(1.))
                                    .rounded(px(3.))
                                    .bg(item.badge_bg)
                                    .text_xs()
                                    .text_color(item.badge_fg)
                                    .child(item.kind),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(item.label_color)
                                    .child(item.name),
                            ),
                    );
                }
            }
        }
        root
    }

    /// 当前帧生效的渲染调优：无 A/B 周期时读环境变量，有周期时按帧段轮换。
    fn current_tuning(&self) -> RenderTuning {
        if self.ab_segments.is_empty() {
            return RenderTuning::from_env();
        }
        let ordinal = self.frame_count.saturating_sub(1);
        let segment = (ordinal / self.ab_segment_frames) as usize % self.ab_segments.len();
        RenderTuning::for_segment(&self.ab_segments[segment])
    }

    /// 诊断：逐帧平移，用于在外部测持续重绘的帧间隔；播完即停。
    fn schedule_autopan_step(view: &mut MapView, window: &mut Window, cx: &mut Context<MapView>) {
        if view.autopan_scheduled {
            return;
        }
        let Some(remaining) = view.autopan_remaining else {
            return;
        };
        if remaining == 0 {
            view.autopan_remaining = None;
            return;
        }
        view.autopan_scheduled = true;
        cx.on_next_frame(window, |view, window, cx| {
            view.autopan_scheduled = false;
            // 往返平移：位置在 0 / 8px 间交替，避免单向平移把内容移出屏幕后
            // 让后段帧成本因可见内容变少而虚假下降（A/B 交替对照要求空间条件一致）。
            let remaining = view.autopan_remaining.unwrap_or(0);
            let step = if remaining % 2 == 0 { 8.0 } else { -8.0 };
            view.state.pan_by(step, 0.0);
            phase_marker("pan:frame");
            let left = view.autopan_remaining.unwrap_or(1).saturating_sub(1);
            if left == 0 {
                view.autopan_remaining = None;
                phase_marker("pan:done");
            } else {
                view.autopan_remaining = Some(left);
                MapView::schedule_autopan_step(view, window, cx);
            }
            cx.notify();
        });
    }

    /// 每个 GPUI 帧回调读取 `with_animation` 写下的进度；仍在揭示时续订下一帧，
    /// 播完即停（不回绕）。进度来自框架动画时钟，帧数只影响采样疏密。
    fn schedule_reveal_frame(view: &mut MapView, window: &mut Window, cx: &mut Context<MapView>) {
        if !view.reveal.is_active() || view.reveal_scheduled {
            return;
        }
        if !view.reveal_start_marked {
            view.reveal_start_marked = true;
            phase_marker("reveal:start");
        }
        view.reveal_scheduled = true;
        cx.on_next_frame(window, |view, window, cx| {
            view.reveal_scheduled = false;
            let was_active = view.reveal.is_active();
            view.reveal.set_progress(view.reveal_progress.get());
            if was_active && view.reveal.is_finished() {
                phase_marker("reveal:done");
            } else if was_active && !view.reveal_mid_marked && view.reveal.overall_progress() >= 0.5
            {
                view.reveal_mid_marked = true;
                phase_marker("reveal:mid");
            }
            if view.reveal.is_active() {
                MapView::schedule_reveal_frame(view, window, cx);
            }
            cx.notify();
        });
    }
}

static MAP_FIRST_FRAME: AtomicBool = AtomicBool::new(false);

impl Render for MapView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
        let skip = RenderSkip::from_env();
        let trace = frame_trace_enabled();
        if trace {
            phase_marker("frame");
        }
        self.frame_count = self.frame_count.saturating_add(1);
        if self.keyboard_focus_pending {
            self.keyboard_focus_pending = false;
            // 等首帧把 `track_focus` 挂上后再交焦点，避免句柄尚未注册时被忽略。
            let handle = self.focus_handle.clone();
            window.on_next_frame(move |window, _cx| window.focus(&handle));
        }
        if trace && !self.ab_segments.is_empty() {
            let ordinal = self.frame_count - 1;
            if ordinal.is_multiple_of(self.ab_segment_frames) {
                let segment = (ordinal / self.ab_segment_frames) as usize % self.ab_segments.len();
                phase_marker(&format!("ab:segment={}", self.ab_segments[segment]));
            }
        }
        if !MAP_FIRST_FRAME.swap(true, Ordering::SeqCst) {
            phase_marker("content:render");
            window.on_next_frame(|_, _cx| phase_marker("content:presented"));
        }
        let labels = self.state.labels_visible() && !skip.labels;
        let scene = match self.state.symbol_layout() {
            Some(symbols) => Scene::Symbols(symbols.clone()),
            None => Scene::Modules(self.state.map().clone()),
        };
        let can_go_back = self.state.can_go_back();
        if trace && self.frame_count == 1 {
            if let Scene::Modules(map) = &scene {
                println!(
                    "frame: nodes={} edges={} bands={} labels={}",
                    map.nodes.len(),
                    map.edges.len(),
                    map.bands.len(),
                    if labels { map.nodes.len() } else { 0 }
                );
                let _ = std::io::stdout().flush();
            }
        }
        // 诊断平移：在取 viewport 前由帧回调推进，保证每帧都有真实位移。
        if self.autopan_remaining.is_some() {
            Self::schedule_autopan_step(self, window, cx);
        }
        let overlay = self.playback.as_ref().map(PlaybackOverlay::from_state);
        let transition_enabled = self.state.transition_enabled();
        let viewport = self.apply_fly_to_if_needed(&scene, window, cx);
        let viewport_size = (
            f32::from(window.viewport_size().width),
            f32::from(window.viewport_size().height),
        );
        if self.reveal.is_active() {
            Self::schedule_reveal_frame(self, window, cx);
        }

        let mut root = div()
            .id("canvas")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key(event, window, cx)
            }))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.canvas_bg)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, _cx| {
                    this.interrupt_reveal();
                    let position = Self::screen_position(event.position);
                    this.press_origin = Some(position);
                    this.drag_last = Some(position);
                    this.dragged = false;
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                if !event.dragging() {
                    if this.state.hover_at(Self::screen_position(event.position)) {
                        cx.notify();
                    }
                    return;
                }
                this.interrupt_reveal();
                let position = Self::screen_position(event.position);
                if let Some(last) = this.drag_last {
                    let dx = position.x - last.x;
                    let dy = position.y - last.y;
                    this.state.pan_by(dx, dy);
                    if let Some(origin) = this.press_origin {
                        if (position.x - origin.x).abs() + (position.y - origin.y).abs() > 3.0 {
                            this.dragged = true;
                        }
                    }
                    cx.notify();
                }
                this.drag_last = Some(position);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    this.finish_press(Self::screen_position(event.position));
                    this.press_origin = None;
                    this.drag_last = None;
                    this.dragged = false;
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, cx| {
                this.interrupt_reveal();
                let anchor = Self::screen_position(event.position);
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => f32::from(delta.y),
                    ScrollDelta::Lines(delta) => delta.y * 20.0,
                };
                if delta.abs() > f32::EPSILON {
                    let factor = if delta > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    this.state.zoom_at(anchor, factor);
                    cx.notify();
                }
            }));

        if self.reveal.is_active() {
            root = root.child(
                div()
                    .id("reveal-hint")
                    .absolute()
                    .right(px(12.))
                    .top(px(12.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(theme.card_bg)
                    .text_xs()
                    .text_color(theme.text_slate)
                    .child("层归属为推断结果 · 点击/拖动立即跳过揭示"),
            );
        }

        if can_go_back {
            root = root.child(
                div()
                    .id("back")
                    .absolute()
                    .left(px(12.))
                    .top(px(12.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .text_xs()
                    .text_color(theme.text_primary)
                    .child("← 返回地图")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.interrupt_reveal();
                        this.state.go_back();
                        this.press_origin = None;
                        this.drag_last = None;
                        cx.notify();
                    })),
            );
        }

        root = self.add_scene_elements(root, &scene, viewport, viewport_size);

        if self.search.is_open() {
            root = root.child(search_panel_element(&self.search));
        }
        // 聚焦标签与淡出同属一个状态：`Esc` 清除时同一帧一起消失。
        if let Some(label) = self.search.focus_label() {
            root = root.child(focus_label_element(&label));
        }

        if let Some(overlay) = &overlay {
            let status = format!(
                "路径 {}/{} · {}",
                overlay.current,
                overlay.len,
                overlay
                    .current_symbol
                    .clone()
                    .unwrap_or_else(|| "未开始".to_string())
            );
            let mut panel = div()
                .id("playback")
                .absolute()
                .left(px(12.))
                .bottom(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(6.))
                .bg(theme.card_bg)
                .text_xs()
                .text_color(theme.text_primary)
                .child(status);

            panel = panel.child(
                div()
                    .id("pb-back")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child("◀")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if let Some(playback) = this.playback.as_mut() {
                            playback.back();
                        }
                        cx.notify();
                    })),
            );
            panel = panel.child(
                div()
                    .id("pb-play")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child(if overlay.playing { "暂停" } else { "播放" })
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if let Some(playback) = this.playback.as_mut() {
                            if playback.is_playing() {
                                playback.pause();
                            } else {
                                playback.play();
                                if !this.ticker_running {
                                    this.ticker_running = true;
                                    cx.spawn(async move |this, cx| loop {
                                        Timer::after(Duration::from_millis(180)).await;
                                        let keep = this
                                            .update(cx, |view, cx| {
                                                let Some(playback) = view.playback.as_mut() else {
                                                    view.ticker_running = false;
                                                    return false;
                                                };
                                                if !playback.is_playing() {
                                                    view.ticker_running = false;
                                                    return false;
                                                }
                                                let advanced = playback.advance();
                                                let still_playing =
                                                    advanced && playback.is_playing();
                                                if !still_playing {
                                                    view.ticker_running = false;
                                                }
                                                cx.notify();
                                                still_playing
                                            })
                                            .unwrap_or(false);
                                        if !keep {
                                            break;
                                        }
                                    })
                                    .detach();
                                }
                            }
                        }
                        cx.notify();
                    })),
            );
            panel = panel.child(
                div()
                    .id("pb-next")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child("▶")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if let Some(playback) = this.playback.as_mut() {
                            playback.advance();
                        }
                        cx.notify();
                    })),
            );
            panel = panel.child(
                div()
                    .id("pb-animation")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child(if overlay.animation {
                        "动画: 开"
                    } else {
                        "动画: 关"
                    })
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if let Some(playback) = this.playback.as_mut() {
                            let enabled = playback.animation_enabled();
                            playback.set_animation_enabled(!enabled);
                        }
                        cx.notify();
                    })),
            );
            panel = panel.child(
                div()
                    .id("pb-fly-to")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child(if overlay.fly_to {
                        "fly-to: 开"
                    } else {
                        "fly-to: 关"
                    })
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if let Some(playback) = this.playback.as_mut() {
                            let enabled = playback.fly_to_enabled();
                            playback.set_fly_to(!enabled);
                        }
                        cx.notify();
                    })),
            );
            panel = panel.child(
                div()
                    .id("pb-transition")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child(if transition_enabled {
                        "转场: 开"
                    } else {
                        "转场: 关"
                    })
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        let enabled = this.state.transition_enabled();
                        this.state.set_transition_enabled(!enabled);
                        cx.notify();
                    })),
            );
            panel = panel.child(
                div()
                    .id("pb-exit")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(theme.chip_danger_bg)
                    .child("退出路径")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.playback = None;
                        this.ticker_running = false;
                        cx.notify();
                    })),
            );
            root = root.child(panel);
        }
        if matches!(self.state.level(), Level::Modules) {
            if let Some(selected_id) = self.state.selected().map(str::to_string) {
                if let Some(node) = self.state.map().nodes.iter().find(|n| n.id == selected_id) {
                    let short_name = selected_id
                        .rsplit('/')
                        .next()
                        .unwrap_or(&selected_id)
                        .to_string();
                    let layer_name = node.layer;
                    let symbol_count = self
                        .state
                        .module_symbol_count(&selected_id, &self.callgraph);
                    let (upstream, downstream) = self.state.module_neighbors(&selected_id);
                    let up_total = upstream.len();
                    let down_total = downstream.len();

                    let mut inspector = div()
                        .id("module-inspector")
                        .absolute()
                        .right(px(12.))
                        .top(px(52.))
                        .w(px(280.))
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.hud_border)
                        .bg(theme.card_bg)
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .text_xs()
                        .text_color(theme.text_primary)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(8.))
                                .child(div().text_color(theme.text_bright).child(short_name))
                                .child(
                                    div()
                                        .id("inspector-close")
                                        .px(px(6.))
                                        .py(px(2.))
                                        .rounded(px(4.))
                                        .bg(theme.chip_bg)
                                        .child("×")
                                        .on_click(cx.listener(
                                            |this, _event: &ClickEvent, _window, cx| {
                                                this.state.clear_selection();
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .text_color(theme.text_secondary)
                                .child(selected_id.clone()),
                        )
                        .child(
                            div()
                                .text_color(theme.info_blue)
                                .child(format!("层: {layer_name} · {symbol_count} 个符号")),
                        );

                    if symbol_count > 0 {
                        let drill_id = selected_id.clone();
                        inspector = inspector.child(
                            div()
                                .id("inspector-drill")
                                .px(px(8.))
                                .py(px(4.))
                                .rounded(px(4.))
                                .bg(theme.chip_active_bg)
                                .child("下钻符号视图 (双击 / Enter)")
                                .on_click(cx.listener(
                                    move |this, _event: &ClickEvent, _window, cx| {
                                        this.interrupt_reveal();
                                        if this.state.drill_down(&drill_id, &this.callgraph) {
                                            this.search.reset();
                                        }
                                        cx.notify();
                                    },
                                )),
                        );
                    }

                    let is_from = self.path_from.as_deref() == Some(selected_id.as_str());
                    let is_to = self.path_to.as_deref() == Some(selected_id.as_str());
                    let from_id = selected_id.clone();
                    let to_id = selected_id.clone();
                    inspector = inspector.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .id("inspector-set-from")
                                    .px(px(8.))
                                    .py(px(4.))
                                    .rounded(px(4.))
                                    .bg(if is_from {
                                        theme.chip_active_bg
                                    } else {
                                        theme.chip_bg
                                    })
                                    .child(if is_from {
                                        "✓ 路径起点"
                                    } else {
                                        "设为路径起点"
                                    })
                                    .on_click(cx.listener(
                                        move |this, _event: &ClickEvent, _window, cx| {
                                            this.interrupt_reveal();
                                            this.path_from = Some(from_id.clone());
                                            this.trigger_interactive_path(cx);
                                            cx.notify();
                                        },
                                    )),
                            )
                            .child(
                                div()
                                    .id("inspector-set-to")
                                    .px(px(8.))
                                    .py(px(4.))
                                    .rounded(px(4.))
                                    .bg(if is_to {
                                        theme.chip_active_bg
                                    } else {
                                        theme.chip_bg
                                    })
                                    .child(if is_to {
                                        "✓ 路径终点"
                                    } else {
                                        "设为路径终点"
                                    })
                                    .on_click(cx.listener(
                                        move |this, _event: &ClickEvent, _window, cx| {
                                            this.interrupt_reveal();
                                            this.path_to = Some(to_id.clone());
                                            this.trigger_interactive_path(cx);
                                            cx.notify();
                                        },
                                    )),
                            ),
                    );
                    if self.path_from.is_some()
                        || self.path_to.is_some()
                        || self.path_status.is_some()
                    {
                        let from_label = self
                            .path_from
                            .as_deref()
                            .and_then(|id| id.rsplit('/').next())
                            .unwrap_or("未选");
                        let to_label = self
                            .path_to
                            .as_deref()
                            .and_then(|id| id.rsplit('/').next())
                            .unwrap_or("未选");
                        let summary = match &self.path_status {
                            Some(status) => status.clone(),
                            None => format!("路径: {from_label} -> {to_label}"),
                        };
                        inspector = inspector.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(6.))
                                .child(div().text_color(theme.hint_amber).child(summary))
                                .child(
                                    div()
                                        .id("inspector-clear-path")
                                        .px(px(6.))
                                        .py(px(2.))
                                        .rounded(px(4.))
                                        .bg(theme.chip_bg)
                                        .child("清除")
                                        .on_click(cx.listener(
                                            |this, _event: &ClickEvent, _window, cx| {
                                                this.path_from = None;
                                                this.path_to = None;
                                                this.path_status = None;
                                                cx.notify();
                                            },
                                        )),
                                ),
                        );
                    }
                    inspector = inspector.child(
                        div()
                            .text_color(theme.text_secondary)
                            .child(format!("上游依赖 ({up_total})")),
                    );
                    if upstream.is_empty() {
                        inspector =
                            inspector.child(div().text_color(theme.text_muted).child("无上游依赖"));
                    } else {
                        for (idx, dep) in upstream.into_iter().take(INSPECTOR_MAX_ITEMS).enumerate()
                        {
                            let target = dep.clone();
                            inspector = inspector.child(
                                div()
                                    .id(("insp-up", idx as u64))
                                    .px(px(6.))
                                    .py(px(3.))
                                    .rounded(px(4.))
                                    .bg(theme.canvas_bg)
                                    .child(dep)
                                    .on_click(cx.listener(
                                        move |this, _event: &ClickEvent, window, cx| {
                                            this.interrupt_reveal();
                                            let size = (
                                                f32::from(window.viewport_size().width),
                                                f32::from(window.viewport_size().height),
                                            );
                                            if let Some(center) = this.state.select_module(&target)
                                            {
                                                if this.state.begin_fly_to(Some(center), size) {
                                                    this.transition_generation += 1;
                                                    this.transition_delta = Rc::new(Cell::new(0.0));
                                                    Self::schedule_transition_step(
                                                        this, window, cx,
                                                    );
                                                }
                                            }
                                            cx.notify();
                                        },
                                    )),
                            );
                        }
                        if up_total > INSPECTOR_MAX_ITEMS {
                            inspector =
                                inspector.child(div().text_color(theme.text_muted).child(format!(
                                    "... 另有 {} 项",
                                    up_total - INSPECTOR_MAX_ITEMS
                                )));
                        }
                    }

                    inspector = inspector.child(
                        div()
                            .text_color(theme.text_secondary)
                            .child(format!("下游依赖 ({down_total})")),
                    );
                    if downstream.is_empty() {
                        inspector =
                            inspector.child(div().text_color(theme.text_muted).child("无下游依赖"));
                    } else {
                        for (idx, dep) in
                            downstream.into_iter().take(INSPECTOR_MAX_ITEMS).enumerate()
                        {
                            let target = dep.clone();
                            inspector = inspector.child(
                                div()
                                    .id(("insp-down", idx as u64))
                                    .px(px(6.))
                                    .py(px(3.))
                                    .rounded(px(4.))
                                    .bg(theme.canvas_bg)
                                    .child(dep)
                                    .on_click(cx.listener(
                                        move |this, _event: &ClickEvent, window, cx| {
                                            this.interrupt_reveal();
                                            let size = (
                                                f32::from(window.viewport_size().width),
                                                f32::from(window.viewport_size().height),
                                            );
                                            if let Some(center) = this.state.select_module(&target)
                                            {
                                                if this.state.begin_fly_to(Some(center), size) {
                                                    this.transition_generation += 1;
                                                    this.transition_delta = Rc::new(Cell::new(0.0));
                                                    Self::schedule_transition_step(
                                                        this, window, cx,
                                                    );
                                                }
                                            }
                                            cx.notify();
                                        },
                                    )),
                            );
                        }
                        if down_total > INSPECTOR_MAX_ITEMS {
                            inspector = inspector.child(div().text_color(theme.text_muted).child(
                                format!("... 另有 {} 项", down_total - INSPECTOR_MAX_ITEMS),
                            ));
                        }
                    }

                    root = root.child(inspector);
                }
            }
        }
        let edge_mode = self.state.edge_mode();
        let module_count = self.state.map().nodes.len();
        let edge_count = self.state.map().edges.len();
        let zoom_pct = (viewport.zoom * 100.0).round() as i32;
        let mut hud = div()
            .id("map-hud-toolbar")
            .absolute()
            .right(px(12.))
            .bottom(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.hud_border)
            .bg(theme.card_bg)
            .text_xs()
            .text_color(theme.text_primary)
            .child(
                div()
                    .text_color(theme.text_secondary)
                    .child(format!("{module_count} 模块 · {edge_count} 边")),
            );

        if self.playback.is_none() {
            hud = hud.child(
                div()
                    .id("hud-search")
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child("搜索 /")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.interrupt_reveal();
                        let _ = this.search.handle(SearchKey::Open, this.state.viewport());
                        cx.notify();
                    })),
            );
        }

        hud = hud
            .child(
                div()
                    .id("hud-edge-mode")
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(if edge_mode == EdgeDisplayMode::All {
                        theme.chip_bg
                    } else {
                        theme.chip_active_bg
                    })
                    .child(format!("{} (E)", edge_mode.label()))
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.interrupt_reveal();
                        this.state.cycle_edge_mode();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("hud-zoom-out")
                    .px(px(6.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child("−")
                    .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                        this.interrupt_reveal();
                        let size = (
                            f32::from(window.viewport_size().width),
                            f32::from(window.viewport_size().height),
                        );
                        this.state.zoom_at_center(0.8, size);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_color(theme.text_contrast)
                    .child(format!("{zoom_pct}%")),
            )
            .child(
                div()
                    .id("hud-zoom-in")
                    .px(px(6.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child("+")
                    .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                        this.interrupt_reveal();
                        let size = (
                            f32::from(window.viewport_size().width),
                            f32::from(window.viewport_size().height),
                        );
                        this.state.zoom_at_center(1.25, size);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("hud-fit")
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(theme.chip_bg)
                    .child("适应 0")
                    .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                        this.interrupt_reveal();
                        let size = (
                            f32::from(window.viewport_size().width),
                            f32::from(window.viewport_size().height),
                        );
                        this.state.fit_to_viewport(size);
                        cx.notify();
                    })),
            );
        root = root.child(hud);

        if matches!(self.state.level(), Level::Modules)
            && overlay.is_none()
            && !self.state.map().bands.is_empty()
        {
            let active_layer = self.layer_filter;
            let mut legend = div()
                .id("map-layer-legend")
                .absolute()
                .left(px(12.))
                .bottom(px(12.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .border_1()
                .border_color(theme.hud_border)
                .bg(theme.card_bg)
                .text_xs()
                .text_color(theme.text_primary);

            for (idx, band) in self.state.map().bands.iter().enumerate() {
                let layer_name = band.layer;
                let swatch_hex = layer_palette(band.layer, band.is_unclassified);
                let is_active = active_layer == Some(layer_name);
                legend = legend.child(
                    div()
                        .id(("legend-layer", idx as u64))
                        .px(px(7.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .bg(if is_active {
                            theme.chip_active_bg
                        } else {
                            theme.chip_bg
                        })
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            div()
                                .w(px(8.))
                                .h(px(8.))
                                .rounded(px(4.))
                                .bg(rgb(swatch_hex)),
                        )
                        .child(format!("{} ({})", band.layer, band.node_count))
                        .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                            this.interrupt_reveal();
                            this.layer_filter = if this.layer_filter == Some(layer_name) {
                                None
                            } else {
                                Some(layer_name)
                            };
                            cx.notify();
                        })),
                );
            }

            if active_layer.is_some() {
                legend = legend.child(
                    div()
                        .id("legend-clear")
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .bg(theme.chip_muted_bg)
                        .child("全部")
                        .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                            this.layer_filter = None;
                            cx.notify();
                        })),
                );
            }

            root = root.child(legend);
        }

        // P5：揭示进度只由 GPUI 动画时钟驱动（固定时长），帧率只决定中间帧疏密。
        let mut element = if self.reveal.is_active() {
            let progress = self.reveal_progress.clone();
            root.with_animation(
                ("reveal", 0u64),
                Animation::new(Duration::from_millis(REVEAL_DURATION_MS)),
                move |element, value| {
                    progress.set(value);
                    element
                },
            )
            .into_any_element()
        } else {
            root.into_any_element()
        };

        if self.state.transition().is_some() {
            let delta = self.transition_delta.clone();
            let generation = self.transition_generation;
            element = element
                .with_animation(
                    ("camera-transition", generation),
                    Animation::new(Duration::from_millis(200)),
                    move |element, progress| {
                        delta.set(progress);
                        element
                    },
                )
                .into_any_element();
        }
        element
    }
}

pub(crate) enum Scene {
    Modules(MapLayout),
    Symbols(SymbolLayout),
}
