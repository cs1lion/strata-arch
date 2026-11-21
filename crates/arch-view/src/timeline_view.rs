//! 时间线视图：把 first-parent 历史按刻度呈现为模块热度地图。
//!
//! 从 `main.rs` 抽出（Issue #22 收口）。绘制顺序、首帧标记、播放刻度驱动与
//! 加载态面板保持拆分前一致；播放进度只来自 `with_animation` 的框架动画时钟。

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arch_view::loading::{
    build_layer_artifact, compute_layout, list_first_parent_commits, sample_history, LoadStatus,
    LoadingStage, LoadingState,
};
use arch_view::timeline::{degradation_label, TimelinePlayback, TimelineState};
use arch_view::view::{CanvasState, Viewport};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, Animation, AnimationExt, App, ClickEvent, Context,
    Entity, IntoElement, Render, Window,
};

use super::phase_marker;

/// 后台分阶段准备数据；每段完成即更新加载态文案，全部完成后切换到真实视图。
///
/// 自动关闭早于本任务完成时，`view.update` 返回 `Err`（app 已释放），这里只忽略返回值：
/// 进程仍按 `quit:auto` 干净退出，不延迟、不 panic。
pub(crate) fn spawn_timeline_loading(
    view: Entity<TimelineView>,
    repo: PathBuf,
    autoplay: bool,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        phase_marker("load:start");

        let list_repo = repo.clone();
        let listed = cx
            .background_executor()
            .spawn(async move { list_first_parent_commits(&list_repo) });
        let commits = match listed.await {
            Ok(commits) => commits,
            Err(message) => return timeline_load_failed(&view, cx, message),
        };
        phase_marker("load:stage:git-log");
        phase_marker("prep:git-log");
        if commits.is_empty() {
            return timeline_load_failed(&view, cx, "没有可用的 first-parent 提交".to_string());
        }

        let total = commits.len();
        set_loading_stage(&view, cx, LoadingStage::SamplingHistory { ticks: total });

        let sample_repo = repo.clone();
        let sampled = cx
            .background_executor()
            .spawn(async move { sample_history(&sample_repo, &commits) });
        let entries = match sampled.await {
            Ok(entries) => entries,
            Err(message) => return timeline_load_failed(&view, cx, message),
        };
        phase_marker("load:stage:history");
        phase_marker("prep:history");

        set_loading_stage(&view, cx, LoadingStage::BuildingLayers);
        let layers_repo = repo.clone();
        let layered = cx
            .background_executor()
            .spawn(async move { build_layer_artifact(&layers_repo) });
        let artifact = match layered.await {
            Ok(artifact) => artifact,
            Err(message) => return timeline_load_failed(&view, cx, message),
        };
        phase_marker("load:stage:layers");
        phase_marker("prep:layers");

        set_loading_stage(&view, cx, LoadingStage::ComputingLayout);
        let layout = cx
            .background_executor()
            .spawn(async move { compute_layout(&artifact) })
            .await;
        phase_marker("load:stage:layout");
        phase_marker("prep:layout");

        let timeline = TimelineState::new(entries);
        let ticks = timeline.ticks();
        if autoplay {
            println!("playback: {}", timeline.sampling_label());
        }
        println!(
            "timeline: {} ticks, {} modules, {} bands",
            ticks,
            layout.nodes.len(),
            layout.bands.len()
        );
        let _ = view.update(cx, |view, cx| {
            view.playback = TimelinePlayback::new(ticks);
            view.autoplay_pending = autoplay;
            view.content = TimelineContent::Ready(Box::new(ReadyTimeline {
                state: CanvasState::new(layout),
                timeline,
            }));
            cx.notify();
        });
        phase_marker("load:done");
    })
    .detach();
}

pub(crate) fn set_loading_stage(
    view: &Entity<TimelineView>,
    cx: &mut gpui::AsyncApp,
    stage: LoadingStage,
) {
    let _ = view.update(cx, |view, cx| {
        if let TimelineContent::Loading(loading) = &mut view.content {
            loading.set_stage(stage);
        }
        cx.notify();
    });
}

pub(crate) fn timeline_load_failed(
    view: &Entity<TimelineView>,
    cx: &mut gpui::AsyncApp,
    message: String,
) {
    phase_marker("load:failed");
    let _ = view.update(cx, |view, cx| {
        view.content = TimelineContent::Failed(message);
        cx.notify();
    });
}

/// `with_animation` 的循环周期；进度回绕后按差值换算为毫秒喂给播放模型。
const PLAYBACK_ANIMATION_MS: f32 = 10_000.0;

pub(crate) struct ReadyTimeline {
    state: CanvasState,
    timeline: TimelineState,
}

/// 视图内容：先呈现加载态，后台数据就绪后整体切换；失败时在窗口内显示错误。
pub(crate) enum TimelineContent {
    Loading(LoadingState),
    Failed(String),
    Ready(Box<ReadyTimeline>),
}

impl TimelineContent {
    fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    fn status_lines(&self) -> Vec<String> {
        match self {
            Self::Loading(loading) => LoadStatus::Loading(loading.clone()).visible_lines(),
            Self::Failed(message) => LoadStatus::Failed {
                message: message.clone(),
            }
            .visible_lines(),
            Self::Ready(_) => Vec::new(),
        }
    }
}

pub(crate) struct TimelineView {
    content: TimelineContent,
    playback: TimelinePlayback,
    /// `--autoplay` 冒烟模式：把帧回调驱动的刻度切换打印出来。
    trace: bool,
    /// 数据就绪后、首个渲染帧启动一次播放（`start_playback` 需要 `Window`）。
    autoplay_pending: bool,
    /// 当前帧的动画进度，由 `with_animation` animator 写入、帧回调读取。
    playback_progress: Rc<Cell<f32>>,
    /// 上一帧进度；`-1.0` 表示尚未收到帧。
    playback_last_progress: Rc<Cell<f32>>,
    /// 播放代际：每次开始播放 +1，让动画元素 id 重新计时。
    playback_generation: u64,
}

impl TimelineView {
    pub(crate) fn loading(trace: bool) -> Self {
        Self {
            content: TimelineContent::Loading(LoadingState::new()),
            playback: TimelinePlayback::new(1),
            trace,
            autoplay_pending: trace,
            playback_progress: Rc::new(Cell::new(0.0)),
            playback_last_progress: Rc::new(Cell::new(-1.0)),
            playback_generation: 0,
        }
    }

    fn timeline_mut(&mut self) -> Option<&mut TimelineState> {
        match &mut self.content {
            TimelineContent::Ready(ready) => Some(&mut ready.timeline),
            _ => None,
        }
    }

    fn start_playback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.playback.play() {
            let index = self.playback.current();
            if let Some(timeline) = self.timeline_mut() {
                timeline.scrub_to(index);
            }
        } else {
            return;
        }
        self.playback_generation += 1;
        self.playback_progress.set(0.0);
        self.playback_last_progress.set(-1.0);
        TimelineView::schedule_playback_frame(self, window, cx);
        cx.notify();
    }

    /// 每个动画帧读取 `with_animation` 进度并推进离散播放；仍在播放时续订下一帧。
    fn schedule_playback_frame(
        view: &mut TimelineView,
        window: &mut Window,
        cx: &mut Context<TimelineView>,
    ) {
        if !view.playback.should_drive_frames() {
            return;
        }
        cx.on_next_frame(window, |view, window, cx| {
            let progress = view.playback_progress.get();
            let last = view.playback_last_progress.get();
            let delta = if last < 0.0 {
                0.0
            } else if progress >= last {
                progress - last
            } else {
                // 循环动画回绕：补上上一周期尾部。
                1.0 - last + progress
            };
            view.playback_last_progress.set(progress);
            if view.playback.on_frame(delta * PLAYBACK_ANIMATION_MS) {
                let index = view.playback.current();
                if let Some(timeline) = view.timeline_mut() {
                    timeline.scrub_to(index);
                }
                if view.trace {
                    println!(
                        "playback: frame tick {} speed {}x",
                        view.playback.current() + 1,
                        view.playback.speed()
                    );
                }
            }
            if view.playback.should_drive_frames() {
                TimelineView::schedule_playback_frame(view, window, cx);
            } else if view.trace && view.playback.is_ended() {
                println!("playback: ended at tick {}", view.playback.current() + 1);
            }
            cx.notify();
        });
    }

    /// 加载/失败态：整窗居中显示非空文案，避免白屏或静默。
    /// 加载/失败态：保留原有状态文案，并在加载时显示进度条
    fn render_status_panel(&self) -> impl IntoElement {
        let failed = matches!(self.content, TimelineContent::Failed(_));
        let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
        let mut panel = div()
            .size_full()
            .bg(theme.canvas_bg)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .px(px(24.));
        for line in self.content.status_lines() {
            let color = if failed {
                theme.warning_fg
            } else {
                theme.text_primary
            };
            panel = panel.child(div().text_sm().text_color(color).child(line));
        }
        if let TimelineContent::Loading(loading) = &self.content {
            let progress = match loading.stage() {
                LoadingStage::ReadingCommits => 0.2,
                LoadingStage::SamplingHistory { .. } => 0.55,
                LoadingStage::BuildingLayers => 0.75,
                LoadingStage::ComputingLayout => 0.9,
                LoadingStage::Custom(_) => 0.6,
            };
            panel = panel.child(
                div()
                    .w(px(280.))
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(theme.control_bg)
                    .child(
                        div()
                            .w(px(280.0 * progress))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(theme.accent_blue),
                    ),
            );
        }
        panel
    }
}

static TIMELINE_FIRST_FRAME: AtomicBool = AtomicBool::new(false);
static TIMELINE_CONTENT_FIRST_FRAME: AtomicBool = AtomicBool::new(false);

impl Render for TimelineView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !TIMELINE_FIRST_FRAME.swap(true, Ordering::SeqCst) {
            phase_marker("frame:render");
            window.on_next_frame(|_, _cx| phase_marker("frame:presented"));
        }
        if !self.content.is_ready() {
            return self.render_status_panel().into_any_element();
        }
        if self.autoplay_pending {
            self.autoplay_pending = false;
            self.start_playback(window, cx);
        }
        if !TIMELINE_CONTENT_FIRST_FRAME.swap(true, Ordering::SeqCst) {
            phase_marker("content:render");
            window.on_next_frame(|_, _cx| phase_marker("content:presented"));
        }
        let TimelineContent::Ready(ready) = &self.content else {
            unreachable!("加载态已在上面提前返回");
        };
        let layout = ready.state.map().clone();
        let full_scale = ready.timeline.heat_is_full_scale();
        let flags = ready.timeline.flags();
        let selected = ready.timeline.selected().map(ToOwned::to_owned);
        let current = ready.timeline.current_index();
        let ticks = ready.timeline.ticks();
        let commit = ready
            .timeline
            .current_entry()
            .map(|entry| {
                format!(
                    "{}/{} {} {}",
                    current + 1,
                    ticks,
                    &entry.hash[..7.min(entry.hash.len())],
                    entry.subject
                )
            })
            .unwrap_or_else(|| "无提交".to_string());

        let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.canvas_bg);

        let grid_viewport = Viewport::default();
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

        {
            let band_draws: Vec<(arch_view::layout::Rect, String, bool, bool, usize)> = layout
                .bands
                .iter()
                .map(|band| {
                    (
                        band.rect,
                        band.layer.to_string(),
                        band.is_unclassified,
                        band.is_external,
                        band.node_count,
                    )
                })
                .collect();
            let band_theme = theme.clone();
            root = root.child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .size_full()
                    .child(canvas(
                        move |bounds, _, _| bounds,
                        move |bounds, _, window, _| {
                            use gpui::{quad, size, BorderStyle, Corners, Edges};
                            for (rect, layer, is_unclassified, is_external, _count) in &band_draws {
                                let (fill_c, border_c, header_c) = band_theme.band_style(
                                    layer,
                                    *is_unclassified,
                                    *is_external,
                                    1.0,
                                );
                                let b = gpui::Bounds {
                                    origin: point(
                                        bounds.origin.x + px(rect.x),
                                        bounds.origin.y + px(rect.y),
                                    ),
                                    size: size(px(rect.width.max(0.0)), px(rect.height.max(0.0))),
                                };
                                window.paint_quad(quad(
                                    b,
                                    Corners::all(band_theme.band_radius),
                                    fill_c,
                                    Edges::all(px(1.0)),
                                    border_c,
                                    BorderStyle::default(),
                                ));
                                let header_bar = gpui::Bounds {
                                    origin: point(b.origin.x + px(1.0), b.origin.y + px(6.0)),
                                    size: size(px(3.5), px(16.0)),
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
                    )),
            );
            for band in &layout.bands {
                let header = theme.layer_accent(band.layer, band.is_unclassified, band.is_external);
                root = root.child(
                    div()
                        .absolute()
                        .left(px(band.rect.x + 10.))
                        .top(px(band.rect.y + 4.))
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

        {
            #[derive(Clone)]
            struct TimelineNodeDraw {
                rect: arch_view::layout::Rect,
                bg: gpui::Hsla,
                border: gpui::Hsla,
                opacity: f32,
            }
            let node_opacity = if full_scale { 1.0 } else { 0.55 };
            let mut draws: Vec<TimelineNodeDraw> = Vec::with_capacity(layout.nodes.len());
            for node in &layout.nodes {
                let heat = ready.timeline.heat(&node.id);
                let background: gpui::Hsla = rgb(((heat.color[0] as u32) << 16)
                    | ((heat.color[1] as u32) << 8)
                    | heat.color[2] as u32)
                .into();
                let border = if selected.as_deref() == Some(node.id.as_str()) {
                    theme.card_border_selected
                } else if heat.neutral {
                    theme.heat_neutral_border
                } else {
                    theme.card_border_default
                };
                draws.push(TimelineNodeDraw {
                    rect: node.rect,
                    bg: background,
                    border,
                    opacity: node_opacity,
                });
            }
            let tl_theme = theme.clone();
            root = root.child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .size_full()
                    .child(canvas(
                        move |bounds, _, _| bounds,
                        move |bounds, _, window, _| {
                            use gpui::{quad, size, BorderStyle, Corners, Edges};
                            for item in &draws {
                                let mut bg = item.bg;
                                bg.a *= item.opacity;
                                let mut border = item.border;
                                border.a *= item.opacity;
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
                                    Corners::all(tl_theme.card_radius),
                                    bg,
                                    Edges::all(px(1.0)),
                                    border,
                                    BorderStyle::default(),
                                ));
                            }
                        },
                    )),
            );
            for (index, node) in layout.nodes.iter().enumerate() {
                let node_id = node.id.clone();
                root = root.child(
                    div()
                        .id(("timeline-node", index))
                        .absolute()
                        .left(px(node.rect.x))
                        .top(px(node.rect.y))
                        .w(px(node.rect.width))
                        .h(px(node.rect.height))
                        .overflow_hidden()
                        .rounded(theme.card_radius)
                        .opacity(node_opacity)
                        .flex()
                        .items_center()
                        .px(px(8.))
                        .text_xs()
                        .text_color(theme.text_primary)
                        .child(node.id.clone())
                        .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                            if let Some(timeline) = this.timeline_mut() {
                                timeline.select(&node_id);
                            }
                            cx.notify();
                        })),
                );
            }
        }

        let mut ticks_row = div().flex().items_center().gap(px(2.));
        for index in 0..ticks {
            let active = index == current;
            ticks_row = ticks_row.child(
                div()
                    .id(("timeline-tick", index))
                    .w(px(10.))
                    .h(px(if active { 12. } else { 8. }))
                    .rounded(px(2.))
                    .bg(if active {
                        theme.accent_blue
                    } else {
                        theme.tick_inactive
                    })
                    .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                        let accepted = this
                            .timeline_mut()
                            .map(|timeline| timeline.scrub_to(index))
                            .unwrap_or(false);
                        if accepted {
                            this.playback.on_scrub(index);
                        }
                        cx.notify();
                    })),
            );
        }

        let playing = self.playback.is_playing();
        let playback_enabled = self.playback.enabled();
        let ended = self.playback.is_ended();
        let speed = self.playback.speed();
        let position = if ticks > 0 { current + 1 } else { 0 };
        let progress_ratio = if ticks > 1 {
            current as f32 / (ticks.saturating_sub(1)) as f32
        } else if ticks == 1 {
            1.0
        } else {
            0.0
        };

        let controls = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .id("timeline-playback-toggle")
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .bg(if playback_enabled {
                        theme.control_bg
                    } else {
                        theme.toggle_off_bg
                    })
                    .child(if playback_enabled {
                        "动画:开"
                    } else {
                        "动画:关"
                    })
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.playback.set_enabled(!this.playback.enabled());
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("timeline-playback-play")
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .bg(theme.control_bg)
                    .child(if playing { "暂停" } else { "播放" })
                    .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                        if this.playback.is_playing() {
                            this.playback.pause();
                        } else {
                            this.start_playback(window, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("timeline-playback-back")
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .bg(theme.control_bg)
                    .child("前一步")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if this.playback.step_back() {
                            let index = this.playback.current();
                            if let Some(timeline) = this.timeline_mut() {
                                timeline.scrub_to(index);
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("timeline-playback-forward")
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .bg(theme.control_bg)
                    .child("后一步")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if this.playback.step_forward() {
                            let index = this.playback.current();
                            if let Some(timeline) = this.timeline_mut() {
                                timeline.scrub_to(index);
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("timeline-playback-first")
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .bg(theme.control_bg)
                    .child("首")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if this.playback.jump_to_first() {
                            let index = this.playback.current();
                            if let Some(timeline) = this.timeline_mut() {
                                timeline.scrub_to(index);
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("timeline-playback-last")
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .bg(theme.control_bg)
                    .child("尾")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if this.playback.jump_to_last() {
                            let index = this.playback.current();
                            if let Some(timeline) = this.timeline_mut() {
                                timeline.scrub_to(index);
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("timeline-playback-speed")
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(3.))
                    .bg(theme.control_bg)
                    .child(format!("速度 {speed}x"))
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.playback.cycle_speed();
                        cx.notify();
                    })),
            )
            .child(div().child(format!("共 {position}/{ticks} 刻度")));

        let mut badges = div().flex().items_center().gap(px(6.));
        if let Some(label) = degradation_label(flags) {
            badges = badges.child(
                div()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(999.))
                    .bg(theme.badge_danger_bg)
                    .text_color(theme.badge_danger_fg)
                    .child(format!("降级: {label}")),
            );
        }
        let mut markers = Vec::new();
        if flags.sampled {
            markers.push("sampled");
        }
        if flags.truncated {
            markers.push("truncated");
        }
        if flags.shallow {
            markers.push("shallow");
        }
        if !markers.is_empty() {
            badges = badges.child(
                div()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(999.))
                    .bg(theme.badge_warn_bg)
                    .text_color(theme.amber_fg)
                    .child(format!("状态: {}", markers.join(" / "))),
            );
        }
        if let Some(notice) = ready.timeline.gap_notice() {
            badges = badges.child(
                div()
                    .text_color(theme.warning_fg)
                    .child(format!("采样空洞: {notice}")),
            );
        }
        if ended {
            badges = badges.child(div().text_color(theme.amber_fg).child("播放结束"));
        }

        let mut footer = div()
            .absolute()
            .left(px(12.))
            .right(px(12.))
            .bottom(px(12.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(10.))
            .py(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.hud_border)
            .bg(theme.hud_panel_bg)
            .text_xs()
            .text_color(theme.text_primary)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(div().child(commit).child(format!("  [{position}/{ticks}]")))
                    .child(
                        div()
                            .text_color(theme.text_slate)
                            .child(ready.timeline.sampling_label()),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(theme.control_bg)
                    .child(
                        div()
                            .w(gpui::relative(progress_ratio))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(theme.accent_blue),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(ticks_row)
                    .child(controls),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child("蓝=无历史 · 热度=churn 0→11+")
                    .child(badges),
            );

        if let Some(selected) = &selected {
            let text = match ready.timeline.module_metric(selected) {
                Some(module) => format!(
                    "{} · lastModified {} · count {} · churn {} · span {}s",
                    module.id,
                    module.last_modified,
                    module.commit_count,
                    module.churn.total,
                    module.active_span
                ),
                None => format!("{selected} · 无历史"),
            };
            footer = footer.child(div().text_color(theme.text_slate).child(text));
        }

        root = root.child(footer);

        if self.playback.is_playing() {
            let progress = self.playback_progress.clone();
            let generation = self.playback_generation;
            root.with_animation(
                ("timeline-playback", generation),
                Animation::new(Duration::from_millis(PLAYBACK_ANIMATION_MS as u64)).repeat(),
                move |element, value| {
                    progress.set(value);
                    element
                },
            )
            .into_any_element()
        } else {
            root.into_any_element()
        }
    }
}
