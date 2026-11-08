use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use std::collections::BTreeSet;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arch_core::callgraph::{build_call_graph, CallGraphArtifact};
use arch_core::guided_path::build_path;
use arch_core::history::{build_history, HistoryOptions};
use arch_core::layers::build_layers;
use arch_view::layout::{
    layout_map, BandLayout, LayoutConfig, MapLayout, NodeLayout, Point as LayoutPoint,
};
use arch_view::playback::{PlaybackState, NON_PATH_OPACITY};
use arch_view::timeline::{TimelineEntry, TimelineState};
use arch_view::view::{CanvasState, Level, SymbolLayout, Viewport};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, size, App, Application, Bounds, ClickEvent, Context,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Render,
    ScrollDelta, ScrollWheelEvent, Timer, Window, WindowBounds, WindowOptions,
};

const USAGE: &str = "用法:
  arch-view map <目标仓库> [--auto-close-ms <毫秒>] [--from <入口ID> --to <符号ID>]
  arch-view timeline <目标仓库> [--auto-close-ms <毫秒>]
  arch-view smoke <目标仓库>";

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("map") => match run_map(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(2)
            }
        },
        Some("timeline") => match run_timeline(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(2)
            }
        },
        Some("smoke") => match run_smoke(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(2)
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run_map(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let target = args
        .next()
        .ok_or_else(|| format!("map 缺少目标仓库路径\n{USAGE}"))?;
    if target.starts_with("--") {
        return Err(format!("map 缺少目标仓库路径\n{USAGE}"));
    }

    let mut auto_close_ms: Option<u64> = None;
    let mut from: Option<String> = None;
    let mut to: Option<String> = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--auto-close-ms" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--auto-close-ms 缺少毫秒数".to_string())?;
                auto_close_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("--auto-close-ms 不是有效毫秒数: {value}"))?,
                );
            }
            "--from" => {
                from = Some(
                    args.next()
                        .ok_or_else(|| "--from 缺少入口 ID".to_string())?,
                );
            }
            "--to" => {
                to = Some(args.next().ok_or_else(|| "--to 缺少符号 ID".to_string())?);
            }
            other => return Err(format!("未知参数: {other}\n{USAGE}")),
        }
    }

    let playback = match (from.as_deref(), to.as_deref()) {
        (Some(from), Some(to)) => {
            let path = build_path(&PathBuf::from(&target), env!("CARGO_PKG_VERSION"), from, to)
                .map_err(|error| format!("构建路径失败: {error}"))?;
            Some(PlaybackState::new(&path))
        }
        (None, None) => None,
        _ => return Err(format!("--from 与 --to 必须同时提供\n{USAGE}")),
    };

    let artifact = build_layers(&PathBuf::from(&target), env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建层产物失败: {error}"))?;
    let callgraph = build_call_graph(&PathBuf::from(&target), env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建调用图失败: {error}"))?;
    let layout = layout_map(&artifact, LayoutConfig::default());

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

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| MapView {
                    state: CanvasState::new(layout),
                    callgraph,
                    playback,
                    ticker_running: false,
                    press_origin: None,
                    drag_last: None,
                    dragged: false,
                })
            },
        )
        .expect("open map window");
        cx.activate(true);

        if let Some(milliseconds) = auto_close_ms {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(milliseconds))
                    .await;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });

    Ok(())
}

fn run_timeline(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let target = args
        .next()
        .ok_or_else(|| format!("timeline 缺少目标仓库路径\n{USAGE}"))?;
    if target.starts_with("--") {
        return Err(format!("timeline 缺少目标仓库路径\n{USAGE}"));
    }

    let mut auto_close_ms: Option<u64> = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--auto-close-ms" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--auto-close-ms 缺少毫秒数".to_string())?;
                auto_close_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("--auto-close-ms 不是有效毫秒数: {value}"))?,
                );
            }
            other => return Err(format!("未知参数: {other}\n{USAGE}")),
        }
    }

    let repo = PathBuf::from(&target);
    let entries = build_timeline_entries(&repo)?;
    let artifact = build_layers(&repo, env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建层产物失败: {error}"))?;
    let layout = layout_map(&artifact, LayoutConfig::default());
    let timeline = TimelineState::new(entries);
    println!(
        "timeline: {} ticks, {} modules, {} bands",
        timeline.ticks(),
        layout.nodes.len(),
        layout.bands.len()
    );

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| TimelineView {
                    state: CanvasState::new(layout),
                    timeline,
                })
            },
        )
        .expect("open timeline window");
        cx.activate(true);

        if let Some(milliseconds) = auto_close_ms {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(milliseconds))
                    .await;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });

    Ok(())
}

fn build_timeline_entries(repo: &std::path::Path) -> Result<Vec<TimelineEntry>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "--reverse",
            "--first-parent",
            "--format=%H%x09%ct%x09%s",
        ])
        .output()
        .map_err(|error| format!("运行 git log 失败: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git log 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.splitn(3, '\t');
        let hash = parts.next().unwrap_or_default().to_string();
        let time = parts
            .next()
            .unwrap_or_default()
            .parse::<u64>()
            .map_err(|_| format!("无法解析提交时间: {line}"))?;
        let subject = parts.next().unwrap_or_default().to_string();
        let artifact = build_history(
            repo,
            env!("CARGO_PKG_VERSION"),
            HistoryOptions {
                until: Some(format!("@{}", time + 1)),
                ..Default::default()
            },
        )
        .map_err(|error| format!("构建历史产物失败: {error}"))?;
        entries.push(TimelineEntry {
            hash,
            time,
            subject,
            artifact,
        });
    }
    if entries.is_empty() {
        return Err("没有可用的 first-parent 提交".to_string());
    }
    Ok(entries)
}

fn run_smoke(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let target = args
        .next()
        .ok_or_else(|| format!("smoke 缺少目标仓库路径\n{USAGE}"))?;
    if target.starts_with("--") {
        return Err(format!("smoke 缺少目标仓库路径\n{USAGE}"));
    }
    if args.next().is_some() {
        return Err(format!("smoke 不接受额外参数\n{USAGE}"));
    }

    let artifact = build_layers(&PathBuf::from(&target), env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建层产物失败: {error}"))?;
    let layout = layout_map(&artifact, LayoutConfig::default());
    println!(
        "smoke: layout {} modules, {} edges, {} bands",
        layout.nodes.len(),
        layout.edges.len(),
        layout.bands.len()
    );

    let rendered = Arc::new(AtomicBool::new(false));
    let rendered_in_view = rendered.clone();
    let rendered_timeout = rendered.clone();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| SmokeView {
                    layout,
                    rendered: rendered_in_view,
                    scheduled: false,
                })
            },
        )
        .expect("open smoke window");
        cx.activate(true);

        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2500))
                .await;
            if !rendered_timeout.load(Ordering::SeqCst) {
                println!("smoke: timeout waiting for first frame");
                let _ = cx.update(|cx| cx.quit());
            }
        })
        .detach();
    });

    if !rendered.load(Ordering::SeqCst) {
        return Err("smoke 失败：窗口未渲染一帧（可能缺少可用图形后端）".to_string());
    }
    Ok(())
}

struct SmokeView {
    layout: MapLayout,
    rendered: Arc<AtomicBool>,
    scheduled: bool,
}

impl Render for SmokeView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if !self.scheduled {
            self.scheduled = true;
            let rendered = self.rendered.clone();
            window.on_next_frame(move |_, cx| {
                rendered.store(true, Ordering::SeqCst);
                println!("smoke: frame rendered");
                cx.quit();
            });
        }

        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0d1117));
        for band in &self.layout.bands {
            root = root.child(band_element(band, Viewport::default()));
        }
        for node in &self.layout.nodes {
            root = root.child(
                div()
                    .absolute()
                    .left(px(node.rect.x))
                    .top(px(node.rect.y))
                    .w(px(node.rect.width))
                    .h(px(node.rect.height))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(0x2c3a4a))
                    .bg(rgb(0x2d4f7a))
                    .overflow_hidden()
                    .px(px(6.))
                    .text_xs()
                    .text_color(rgb(0xe6edf3))
                    .child(node.id.clone()),
            );
        }
        root
    }
}

struct TimelineView {
    state: CanvasState,
    timeline: TimelineState,
}

impl Render for TimelineView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = self.state.map().clone();
        let full_scale = self.timeline.heat_is_full_scale();
        let flags = self.timeline.flags();
        let selected = self.timeline.selected().map(ToOwned::to_owned);
        let current = self.timeline.current_index();
        let ticks = self.timeline.ticks();
        let commit = self
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

        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0d1117));

        for band in &layout.bands {
            root = root.child(band_element(band, Viewport::default()));
        }

        for (index, node) in layout.nodes.iter().enumerate() {
            let heat = self.timeline.heat(&node.id);
            let background = rgb(((heat.color[0] as u32) << 16)
                | ((heat.color[1] as u32) << 8)
                | heat.color[2] as u32);
            let border = if selected.as_deref() == Some(node.id.as_str()) {
                rgb(0xf2f5f8)
            } else if heat.neutral {
                rgb(0x8a7a5a)
            } else {
                rgb(0x2c3a4a)
            };
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
                    .rounded(px(4.))
                    .border_1()
                    .border_color(border)
                    .bg(background)
                    .opacity(if full_scale { 1.0 } else { 0.55 })
                    .flex()
                    .items_center()
                    .px(px(6.))
                    .text_xs()
                    .text_color(rgb(0xe6edf3))
                    .child(node.id.clone())
                    .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                        this.timeline.select(&node_id);
                        cx.notify();
                    })),
            );
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
                    .bg(if active { rgb(0xf2f5f8) } else { rgb(0x3d4b5c) })
                    .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                        this.timeline.scrub_to(index);
                        cx.notify();
                    })),
            );
        }

        let mut footer = div()
            .absolute()
            .left(px(12.))
            .right(px(12.))
            .bottom(px(12.))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(rgb(0x161b22))
            .text_xs()
            .text_color(rgb(0xe6edf3))
            .child(commit)
            .child(ticks_row)
            .child("灰=无历史 · 蓝→橙=churn 0→11+");

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
            footer = footer.child(
                div()
                    .text_color(rgb(0xd98a3d))
                    .child(format!("状态: {}", markers.join(" / "))),
            );
        }

        if let Some(selected) = &selected {
            let text = match self.timeline.module_metric(selected) {
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
            footer = footer.child(div().text_color(rgb(0x8b9bb0)).child(text));
        }

        root = root.child(footer);
        root
    }
}

struct MapView {
    state: CanvasState,
    callgraph: CallGraphArtifact,
    playback: Option<PlaybackState>,
    ticker_running: bool,
    press_origin: Option<LayoutPoint>,
    drag_last: Option<LayoutPoint>,
    dragged: bool,
}

impl MapView {
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
        let hit = self.state.select_at(position);
        if matches!(self.state.level(), Level::Modules) {
            if let Some(module) = hit {
                self.state.drill_down(&module, &self.callgraph);
            }
        }
    }
}

impl Render for MapView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = self.state.viewport();
        let labels = self.state.labels_visible();
        let selected = self.state.selected().map(ToOwned::to_owned);
        let highlighted = self.state.highlighted();
        let scene = match self.state.symbol_layout() {
            Some(symbols) => Scene::Symbols(symbols.clone()),
            None => Scene::Modules(self.state.map().clone()),
        };
        let can_go_back = self.state.can_go_back();
        let overlay = self.playback.as_ref().map(PlaybackOverlay::from_state);

        let mut root = div()
            .id("canvas")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0d1117))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, _cx| {
                    let position = Self::screen_position(event.position);
                    this.press_origin = Some(position);
                    this.drag_last = Some(position);
                    this.dragged = false;
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                if !event.dragging() {
                    return;
                }
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
                    .bg(rgb(0x1f2937))
                    .text_xs()
                    .text_color(rgb(0xe6edf3))
                    .child("← 返回地图")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.state.go_back();
                        this.press_origin = None;
                        this.drag_last = None;
                        cx.notify();
                    })),
            );
        }

        match scene {
            Scene::Modules(map) => {
                for band in &map.bands {
                    root = root.child(band_element(band, viewport));
                }
                let edges = map
                    .edges
                    .iter()
                    .map(|edge| EdgeDraw {
                        from: viewport.screen_point(edge.from),
                        to: viewport.screen_point(edge.to),
                        adjusted: edge.direction_adjusted,
                        strong: false,
                    })
                    .collect::<Vec<_>>();
                root = root.child(edge_canvas(
                    edges,
                    map.width * viewport.zoom,
                    map.height * viewport.zoom,
                ));

                if let Some(overlay) = &overlay {
                    let centers = map
                        .nodes
                        .iter()
                        .map(|node| (node.id.clone(), node.rect.center()))
                        .collect::<std::collections::BTreeMap<_, _>>();
                    let mut path_edges = Vec::new();
                    for (index, pair) in overlay.steps.windows(2).enumerate() {
                        let Some(from) = centers.get(&pair[0]) else {
                            continue;
                        };
                        let Some(to) = centers.get(&pair[1]) else {
                            continue;
                        };
                        let strong = overlay.current == index + 2;
                        path_edges.push(EdgeDraw {
                            from: viewport.screen_point(*from),
                            to: viewport.screen_point(*to),
                            adjusted: true,
                            strong,
                        });
                    }
                    root = root.child(edge_canvas(
                        path_edges,
                        map.width * viewport.zoom,
                        map.height * viewport.zoom,
                    ));
                }

                for node in &map.nodes {
                    let opacity = overlay
                        .as_ref()
                        .map(|overlay| {
                            if overlay.path_modules.contains(&node.id) {
                                1.0
                            } else {
                                NON_PATH_OPACITY
                            }
                        })
                        .unwrap_or(1.0);
                    let focus = overlay
                        .as_ref()
                        .and_then(|overlay| overlay.current_module.as_deref());
                    root = root.child(node_element(
                        node,
                        viewport,
                        labels,
                        selected.as_deref(),
                        &highlighted,
                        opacity,
                        focus,
                    ));
                }
            }
            Scene::Symbols(symbols) => {
                let edges = symbols
                    .edges
                    .iter()
                    .map(|edge| EdgeDraw {
                        from: viewport.screen_point(edge.from),
                        to: viewport.screen_point(edge.to),
                        adjusted: false,
                        strong: false,
                    })
                    .collect::<Vec<_>>();
                root = root.child(edge_canvas(
                    edges,
                    symbols.width * viewport.zoom,
                    symbols.height * viewport.zoom,
                ));

                for node in &symbols.nodes {
                    let opacity = self
                        .playback
                        .as_ref()
                        .map(|playback| playback.node_opacity(&node.id))
                        .unwrap_or(1.0);
                    let focus = overlay
                        .as_ref()
                        .and_then(|overlay| overlay.current_symbol.as_deref());
                    root = root.child(symbol_node_element(
                        node,
                        opacity,
                        focus,
                        viewport,
                        labels,
                        selected.as_deref(),
                        &highlighted,
                    ));
                }
            }
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
                .bg(rgb(0x161b22))
                .text_xs()
                .text_color(rgb(0xe6edf3))
                .child(status);

            panel = panel.child(
                div()
                    .id("pb-back")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(rgb(0x1f2937))
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
                    .bg(rgb(0x1f2937))
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
                    .bg(rgb(0x1f2937))
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
                    .bg(rgb(0x1f2937))
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
                    .bg(rgb(0x1f2937))
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
                    .id("pb-exit")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(rgb(0x5f2b2b))
                    .child("退出路径")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.playback = None;
                        this.ticker_running = false;
                        cx.notify();
                    })),
            );
            root = root.child(panel);
        }

        root
    }
}

enum Scene {
    Modules(MapLayout),
    Symbols(SymbolLayout),
}

struct PlaybackOverlay {
    steps: Vec<String>,
    path_modules: BTreeSet<String>,
    current_module: Option<String>,
    current_symbol: Option<String>,
    len: usize,
    current: usize,
    playing: bool,
    animation: bool,
    fly_to: bool,
}

impl PlaybackOverlay {
    fn from_state(playback: &PlaybackState) -> Self {
        let steps = playback.steps();
        let modules = steps
            .iter()
            .map(|step| step.module.clone())
            .collect::<Vec<_>>();
        let current = playback.current();
        Self {
            path_modules: modules.iter().cloned().collect(),
            current_module: if current > 0 {
                Some(modules[current - 1].clone())
            } else {
                None
            },
            current_symbol: if current > 0 {
                Some(steps[current - 1].symbol.clone())
            } else {
                None
            },
            steps: modules,
            len: steps.len(),
            current,
            playing: playback.is_playing(),
            animation: playback.animation_enabled(),
            fly_to: playback.fly_to_enabled(),
        }
    }
}

struct EdgeDraw {
    from: LayoutPoint,
    to: LayoutPoint,
    adjusted: bool,
    strong: bool,
}

fn edge_canvas(edges: Vec<EdgeDraw>, width: f32, height: f32) -> impl IntoElement {
    div()
        .absolute()
        .left(px(0.))
        .top(px(0.))
        .w(px(width))
        .h(px(height))
        .child(canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                for edge in &edges {
                    let color = if edge.strong {
                        rgb(0xf2f5f8)
                    } else if edge.adjusted {
                        rgb(0xd98a3d)
                    } else {
                        rgb(0x3d4b5c)
                    };
                    let mut builder =
                        PathBuilder::stroke(if edge.strong { px(2.) } else { px(1.) });
                    if edge.adjusted {
                        builder = builder.dash_array(&[px(4.), px(2.)]);
                    }
                    builder.move_to(point(
                        bounds.origin.x + px(edge.from.x),
                        bounds.origin.y + px(edge.from.y),
                    ));
                    builder.line_to(point(
                        bounds.origin.x + px(edge.to.x),
                        bounds.origin.y + px(edge.to.y),
                    ));
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        ))
}

fn scaled_rect(rect: arch_view::layout::Rect, viewport: Viewport) -> arch_view::layout::Rect {
    let origin = viewport.screen_point(LayoutPoint {
        x: rect.x,
        y: rect.y,
    });
    arch_view::layout::Rect::new(
        origin.x,
        origin.y,
        rect.width * viewport.zoom,
        rect.height * viewport.zoom,
    )
}

fn selection_border(
    id: &str,
    default: gpui::Rgba,
    selected: Option<&str>,
    highlighted: &std::collections::BTreeSet<String>,
    focus: Option<&str>,
) -> gpui::Rgba {
    if focus == Some(id) || selected == Some(id) {
        rgb(0xf2f5f8)
    } else if highlighted.contains(id) {
        rgb(0xd98a3d)
    } else {
        default
    }
}

fn band_element(band: &BandLayout, viewport: Viewport) -> impl IntoElement {
    let rect = scaled_rect(band.rect, viewport);
    let (background, border, label) = if band.is_unclassified {
        (rgb(0x1f1a12), rgb(0x6b552f), rgb(0xd98a3d))
    } else if band.is_external {
        (rgb(0x101d21), rgb(0x2f5a60), rgb(0x6fb3b8))
    } else {
        (rgb(0x11161c), rgb(0x1c2633), rgb(0x8b9bb0))
    };
    div()
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.width))
        .h(px(rect.height))
        .bg(background)
        .border_b_1()
        .border_color(border)
        .child(
            div()
                .pl(px(8.))
                .pt(px(4.))
                .text_xs()
                .text_color(label)
                .child(format!("{} · {}", band.layer, band.node_count)),
        )
}

fn node_element(
    node: &NodeLayout,
    viewport: Viewport,
    labels: bool,
    selected: Option<&str>,
    highlighted: &std::collections::BTreeSet<String>,
    opacity: f32,
    focus: Option<&str>,
) -> impl IntoElement {
    let rect = scaled_rect(node.rect, viewport);
    let background = layer_color(node.layer, node.is_unclassified);
    let border = if node.is_unclassified {
        rgb(0xd98a3d)
    } else if node.is_external {
        rgb(0x6fb3b8)
    } else {
        rgb(0x2c3a4a)
    };
    let border = selection_border(&node.id, border, selected, highlighted, focus);

    let mut element = div()
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.width))
        .h(px(rect.height))
        .overflow_hidden()
        .rounded(px(4.))
        .border_1()
        .border_color(border)
        .bg(background)
        .opacity(opacity)
        .flex()
        .items_center()
        .px(px(6.));

    if labels {
        element = element.child(
            div()
                .text_xs()
                .text_color(rgb(0xe6edf3))
                .child(node.id.clone()),
        );
    }

    if node.is_unclassified || node.is_external {
        element = element.child(
            div()
                .absolute()
                .top(px(3.))
                .right(px(3.))
                .size(px(6.))
                .rounded_full()
                .bg(if node.is_unclassified {
                    rgb(0xd98a3d)
                } else {
                    rgb(0x6fb3b8)
                }),
        );
    }

    element
}

fn symbol_node_element(
    node: &arch_view::view::SymbolNodeLayout,
    opacity: f32,
    focus: Option<&str>,
    viewport: Viewport,
    labels: bool,
    selected: Option<&str>,
    highlighted: &std::collections::BTreeSet<String>,
) -> impl IntoElement {
    let rect = scaled_rect(node.rect, viewport);
    let background = symbol_kind_color(node.kind);
    let border = selection_border(&node.id, rgb(0x2c3a4a), selected, highlighted, focus);

    let mut element = div()
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.width))
        .h(px(rect.height))
        .overflow_hidden()
        .rounded(px(4.))
        .border_1()
        .border_color(border)
        .bg(background)
        .opacity(opacity)
        .flex()
        .items_center()
        .px(px(6.));

    if labels {
        element = element.child(
            div()
                .text_xs()
                .text_color(rgb(0xe6edf3))
                .child(node.name.clone()),
        );
    }

    element
}

fn symbol_kind_color(kind: &str) -> gpui::Rgba {
    match kind {
        "function" => rgb(0x244a66),
        "component" => rgb(0x4a3768),
        "class" => rgb(0x5f4524),
        "method" => rgb(0x23543d),
        _ => rgb(0x2a3440),
    }
}

fn layer_color(layer: &str, is_unclassified: bool) -> gpui::Rgba {
    if is_unclassified {
        return rgb(0x3a2f22);
    }
    match layer {
        "cli" => rgb(0x244a66),
        "commands" => rgb(0x23543d),
        "components" => rgb(0x4a3768),
        "server" => rgb(0x5f4524),
        "utils" => rgb(0x2f4652),
        "tests" => rgb(0x53303c),
        "external" => rgb(0x243b3d),
        _ => rgb(0x2a3440),
    }
}
