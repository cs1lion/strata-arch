use std::cell::Cell;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use std::collections::BTreeSet;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arch_core::callgraph::{build_call_graph, CallGraphArtifact};
use arch_core::commit_diff::diff_commits;
use arch_core::guided_path::build_path;
use arch_core::history::{build_history_prefixes, HistoryOptions};
use arch_core::layers::build_layers;
use arch_view::diff::{DiffFade, DiffModel, DIFF_LABEL, FADE_DURATION_MS};
use arch_view::layout::{
    layout_map, BandLayout, LayoutConfig, MapLayout, NodeLayout, Point as LayoutPoint,
};
use arch_view::playback::{opacity_alpha_byte, PlaybackState, CONTEXT_OPACITY, NON_PATH_OPACITY};
use arch_view::timeline::{degradation_label, TimelineEntry, TimelinePlayback, TimelineState};
use arch_view::view::{
    band_background_palette, layer_palette, paint_symbol_edges, paint_symbol_edges_with_roles,
    symbol_kind_palette, CanvasState, Level, SymbolLayout, Viewport,
};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, rgba, size, Animation, AnimationExt, App, Application,
    Bounds, ClickEvent, Context, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PathBuilder, Render, ScrollDelta, ScrollWheelEvent, Timer, Window, WindowBounds,
    WindowOptions,
};

const USAGE: &str = "用法:
  arch-view map <目标仓库> [--auto-close-ms <毫秒>] [--from <入口ID> --to <符号ID>]
  arch-view timeline <目标仓库> [--auto-close-ms <毫秒>] [--autoplay]
  arch-view diff <目标仓库> --from <commit> --to <commit> [--no-fade] [--auto-close-ms <毫秒>]
  arch-view scene <目标仓库> [--out <场景文件>]
  arch-view smoke <目标仓库>";

/// 阶段标记：仅在 `STRATA_PHASE_MARKERS=1` 时输出，供外部计时脚本记录到达时间。
/// 不在视图源码里取墙钟（`arch-view/src` 禁用时间源判据不变），时间戳由外部进程提供。
fn phase_marker(label: &str) {
    if std::env::var_os("STRATA_PHASE_MARKERS").is_some() {
        println!("phase: {label}");
        let _ = std::io::stdout().flush();
    }
}

fn main() -> ExitCode {
    phase_marker("main:start");
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
        Some("diff") => match run_diff(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(2)
            }
        },
        Some("scene") => match run_scene(args) {
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
                    fly_to_anchor: None,
                    press_origin: None,
                    drag_last: None,
                    dragged: false,
                    transition_generation: 0,
                    transition_delta: Rc::new(Cell::new(0.0)),
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
    let mut autoplay = false;
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
            "--autoplay" => autoplay = true,
            other => return Err(format!("未知参数: {other}\n{USAGE}")),
        }
    }

    let repo = PathBuf::from(&target);
    let entries = build_timeline_entries(&repo)?;
    let artifact = build_layers(&repo, env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建层产物失败: {error}"))?;
    phase_marker("prep:layers");
    let layout = layout_map(&artifact, LayoutConfig::default());
    phase_marker("prep:layout");
    let timeline = TimelineState::new(entries);
    println!(
        "timeline: {} ticks, {} modules, {} bands",
        timeline.ticks(),
        layout.nodes.len(),
        layout.bands.len()
    );

    let application = Application::new();
    phase_marker("app:created");
    application.run(move |cx: &mut App| {
        phase_marker("app:entry");
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let ticks = timeline.ticks();
                let view = cx.new(|_| TimelineView {
                    state: CanvasState::new(layout),
                    playback: TimelinePlayback::new(ticks),
                    timeline,
                    trace: autoplay,
                    playback_progress: Rc::new(Cell::new(0.0)),
                    playback_last_progress: Rc::new(Cell::new(-1.0)),
                    playback_generation: 0,
                });
                if autoplay {
                    view.update(cx, |view, cx| view.start_playback(window, cx));
                }
                view
            },
        )
        .expect("open timeline window");
        cx.activate(true);
        phase_marker("window:opened");

        if let Some(milliseconds) = auto_close_ms {
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(milliseconds))
                    .await;
                phase_marker("quit:auto");
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
    phase_marker("main:end");

    Ok(())
}

fn run_diff(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let target = args
        .next()
        .ok_or_else(|| format!("diff 缺少目标仓库路径\n{USAGE}"))?;
    if target.starts_with("--") {
        return Err(format!("diff 缺少目标仓库路径\n{USAGE}"));
    }

    let mut from: Option<String> = None;
    let mut to: Option<String> = None;
    let mut auto_close_ms: Option<u64> = None;
    let mut fade_enabled = true;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--from" => {
                from = Some(
                    args.next()
                        .ok_or_else(|| "--from 缺少 commit".to_string())?,
                );
            }
            "--to" => {
                to = Some(args.next().ok_or_else(|| "--to 缺少 commit".to_string())?);
            }
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
            "--no-fade" => fade_enabled = false,
            other => return Err(format!("未知参数: {other}\n{USAGE}")),
        }
    }

    let from = from.ok_or_else(|| format!("diff 缺少 --from\n{USAGE}"))?;
    let to = to.ok_or_else(|| format!("diff 缺少 --to\n{USAGE}"))?;
    let repo = PathBuf::from(&target);
    let artifact = diff_commits(&repo, &from, &to, env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建结构 diff 失败: {error}"))?;
    println!(
        "diff: from={} to={} added={} removed={} modified={} renames={}",
        artifact.from.commit,
        artifact.to.commit,
        artifact.stats.added,
        artifact.stats.removed,
        artifact.stats.modified,
        artifact.stats.renames
    );
    let model = DiffModel::from_artifact(&artifact);

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| DiffView {
                    model,
                    fade_enabled,
                    fade_generation: 0,
                })
            },
        )
        .expect("open diff window");
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

struct DiffView {
    model: DiffModel,
    fade_enabled: bool,
    fade_generation: u64,
}

impl Render for DiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fade = DiffFade::new(self.fade_enabled);
        let generation = self.fade_generation;
        let header = format!(
            "{} → {} · 新增 {} / 删除 {} / 修改 {} · 疑似重命名 {}",
            short_commit(&self.model.from_commit),
            short_commit(&self.model.to_commit),
            self.model.added,
            self.model.removed,
            self.model.modified,
            self.model.renames
        );

        let mut root = div()
            .id("diff-root")
            .flex()
            .flex_col()
            .gap(px(6.))
            .size_full()
            .p(px(14.))
            .bg(rgb(0x0d1117))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().text_sm().text_color(rgb(0xe6edf3)).child(header))
                    .child(
                        div()
                            .id("diff-fade")
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .bg(rgb(0x1f2937))
                            .text_xs()
                            .text_color(rgb(0xe6edf3))
                            .child(if self.fade_enabled {
                                "淡入: 开"
                            } else {
                                "淡入: 关"
                            })
                            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                                this.fade_enabled = !this.fade_enabled;
                                if this.fade_enabled {
                                    this.fade_generation += 1;
                                }
                                cx.notify();
                            })),
                    ),
            );

        if self.model.rows.is_empty() {
            root = root.child(
                div()
                    .text_sm()
                    .text_color(rgb(0x8b9bb0))
                    .child("两个 commit 没有结构变化"),
            );
            return root;
        }

        for row in &self.model.rows {
            let text = match &row.rename_hint {
                Some(hint) => format!(
                    "{} {} · {} · 疑似重命名 ↔ {}",
                    row.kind.label(),
                    row.path,
                    row.detail,
                    hint
                ),
                None => format!("{} {} · {}", row.kind.label(), row.path, row.detail),
            };
            let index = row.z_index;
            let element = div()
                .id(("diff-row", index as usize))
                .flex()
                .items_center()
                .px(px(10.))
                .py(px(5.))
                .rounded(px(4.))
                .bg(rgb(row.kind.tint()))
                .text_xs()
                .text_color(rgb(DIFF_LABEL))
                .child(text);
            if self.fade_enabled {
                root = root.child(element.with_animation(
                    ("diff-fade-row", (generation << 16) | index as u64),
                    Animation::new(Duration::from_millis(FADE_DURATION_MS)),
                    move |element, progress| element.opacity(fade.alpha_at(progress)),
                ));
            } else {
                root = root.child(element);
            }
        }

        root
    }
}

fn short_commit(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
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

    let mut commits = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.splitn(3, '\t');
        let hash = parts.next().unwrap_or_default().to_string();
        let time = parts
            .next()
            .unwrap_or_default()
            .parse::<u64>()
            .map_err(|_| format!("无法解析提交时间: {line}"))?;
        let subject = parts.next().unwrap_or_default().to_string();
        commits.push((hash, time, subject));
    }
    phase_marker("prep:git-log");

    // 一次 git 遍历生成所有刻度前缀，替代逐刻度重跑 build_history 的 O(刻度×遍历) 采样。
    let tick_times = commits.iter().map(|(_, time, _)| *time).collect::<Vec<_>>();
    let artifacts = build_history_prefixes(
        repo,
        env!("CARGO_PKG_VERSION"),
        HistoryOptions::default(),
        &tick_times,
    )
    .map_err(|error| format!("构建历史产物失败: {error}"))?;
    phase_marker("prep:history");

    let mut entries = Vec::new();
    for ((hash, time, subject), artifact) in commits.into_iter().zip(artifacts) {
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneArtifact {
    schema_version: u32,
    tool_version: String,
    repo: arch_core::RepoInfo,
    layout: MapLayout,
}

fn run_scene(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let target = args
        .next()
        .ok_or_else(|| format!("scene 缺少目标仓库路径\n{USAGE}"))?;
    if target.starts_with("--") {
        return Err(format!("scene 缺少目标仓库路径\n{USAGE}"));
    }

    let mut out: Option<PathBuf> = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--out" => {
                out = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--out 缺少文件路径".to_string())?,
                ));
            }
            other => return Err(format!("未知参数: {other}\n{USAGE}")),
        }
    }

    let artifact = build_layers(&PathBuf::from(&target), env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建层产物失败: {error}"))?;
    let layout = layout_map(&artifact, LayoutConfig::default());
    let scene = SceneArtifact {
        schema_version: 1,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        repo: artifact.repo,
        layout,
    };
    let json =
        serde_json::to_string_pretty(&scene).map_err(|error| format!("序列化场景失败: {error}"))?;

    match out {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("创建输出目录失败 {}: {error}", parent.display())
                    })?;
                }
            }
            fs::write(&path, format!("{json}\n"))
                .map_err(|error| format!("写入场景失败 {}: {error}", path.display()))?;
        }
        None => println!("{json}"),
    }
    Ok(())
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
    let rendered_watchdog = rendered.clone();
    // 看门狗必须是独立 OS 线程：窗口创建/首次渲染阻塞时 GPUI 执行器无法运行，
    // 原来的 `cx.spawn` 定时器无法触发（见 ticket 14 取证）。
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(12));
        if !rendered_watchdog.load(Ordering::SeqCst) {
            println!("smoke: timeout waiting for first frame");
            let _ = std::io::stdout().flush();
            std::process::exit(2);
        }
    });

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

/// `with_animation` 的循环周期；进度回绕后按差值换算为毫秒喂给播放模型。
const PLAYBACK_ANIMATION_MS: f32 = 10_000.0;

struct TimelineView {
    state: CanvasState,
    timeline: TimelineState,
    playback: TimelinePlayback,
    /// `--autoplay` 冒烟模式：把帧回调驱动的刻度切换打印出来。
    trace: bool,
    /// 当前帧的动画进度，由 `with_animation` animator 写入、帧回调读取。
    playback_progress: Rc<Cell<f32>>,
    /// 上一帧进度；`-1.0` 表示尚未收到帧。
    playback_last_progress: Rc<Cell<f32>>,
    /// 播放代际：每次开始播放 +1，让动画元素 id 重新计时。
    playback_generation: u64,
}

impl TimelineView {
    fn start_playback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.playback.play() {
            return;
        }
        self.timeline.scrub_to(self.playback.current());
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
                view.timeline.scrub_to(view.playback.current());
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
}

static TIMELINE_FIRST_FRAME: AtomicBool = AtomicBool::new(false);

impl Render for TimelineView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !TIMELINE_FIRST_FRAME.swap(true, Ordering::SeqCst) {
            phase_marker("frame:render");
            window.on_next_frame(|_, _cx| phase_marker("frame:presented"));
        }
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
                        this.playback.on_scrub(index);
                        cx.notify();
                    })),
            );
        }

        let playing = self.playback.is_playing();
        let playback_enabled = self.playback.enabled();
        let ended = self.playback.is_ended();
        let speed = self.playback.speed();
        let position = if ticks > 0 { current + 1 } else { 0 };

        // 离散播放控制：只在刻度间跳变，不提供刻度内进度条（避免暗示连续变化）。
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
                        rgb(0x2c3a4a)
                    } else {
                        rgb(0x50241f)
                    })
                    .child(if playback_enabled {
                        "播放:开"
                    } else {
                        "播放:关"
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
                    .bg(rgb(0x2c3a4a))
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
                    .bg(rgb(0x2c3a4a))
                    .child("上一步")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if this.playback.step_back() {
                            this.timeline.scrub_to(this.playback.current());
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
                    .bg(rgb(0x2c3a4a))
                    .child("下一步")
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        if this.playback.step_forward() {
                            this.timeline.scrub_to(this.playback.current());
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
                    .bg(rgb(0x2c3a4a))
                    .child(format!("速度 {speed}x"))
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.playback.cycle_speed();
                        cx.notify();
                    })),
            )
            .child(div().child(format!("第 {position}/{ticks} 刻度")));

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
            .child(controls)
            .child("灰=无历史 · 蓝→橙=churn 0→11+")
            .child(if ended {
                div().text_color(rgb(0xd98a3d)).child("播放结束")
            } else {
                div()
            });

        if let Some(label) = degradation_label(flags) {
            footer = footer.child(
                div()
                    .text_color(rgb(0xe07b39))
                    .child(format!("退化标记: {label}")),
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

struct MapView {
    state: CanvasState,
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
}

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
        } else {
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
        let hit = self.state.select_at(position);
        if matches!(self.state.level(), Level::Modules) {
            if let Some(module) = hit {
                self.state.drill_down(&module, &self.callgraph);
            }
        }
    }
}

impl Render for MapView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels = self.state.labels_visible();
        let selected = self.state.selected().map(ToOwned::to_owned);
        let highlighted = self.state.highlighted();
        let scene = match self.state.symbol_layout() {
            Some(symbols) => Scene::Symbols(symbols.clone()),
            None => Scene::Modules(self.state.map().clone()),
        };
        let can_go_back = self.state.can_go_back();
        let overlay = self.playback.as_ref().map(PlaybackOverlay::from_state);
        let fade_enabled = self
            .playback
            .as_ref()
            .map(|playback| playback.animation_enabled())
            .unwrap_or(false);
        let transition_enabled = self.state.transition_enabled();
        let viewport = self.apply_fly_to_if_needed(&scene, window, cx);

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
                let base_opacity = if fade_enabled { CONTEXT_OPACITY } else { 1.0 };
                let edges = map
                    .edges
                    .iter()
                    .map(|edge| EdgeDraw {
                        from: viewport.screen_point(edge.from),
                        to: viewport.screen_point(edge.to),
                        adjusted: edge.direction_adjusted,
                        strong: false,
                        opacity: base_opacity,
                    })
                    .collect::<Vec<_>>();
                root = root.child(edge_canvas(
                    edges,
                    map.width * viewport.zoom,
                    map.height * viewport.zoom,
                ));

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
                        path_edges.push(EdgeDraw {
                            from: viewport.screen_point(*from),
                            to: viewport.screen_point(*to),
                            adjusted: true,
                            strong,
                            opacity,
                        });
                    }
                    root = root.child(edge_canvas(
                        path_edges,
                        map.width * viewport.zoom,
                        map.height * viewport.zoom,
                    ));
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
                for index in indices {
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
                        None => 1.0,
                    };
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
                let current_edge = overlay
                    .as_ref()
                    .and_then(|overlay| overlay.current_edge.as_ref());
                let painted = match (&self.playback, fade_enabled) {
                    (Some(playback), true) => {
                        paint_symbol_edges_with_roles(&symbols, &viewport, playback)
                    }
                    _ => paint_symbol_edges(&symbols, &viewport, current_edge),
                };
                let edges = painted
                    .into_iter()
                    .map(|edge| EdgeDraw {
                        from: edge.from,
                        to: edge.to,
                        adjusted: false,
                        strong: edge.strong,
                        opacity: edge.opacity,
                    })
                    .collect::<Vec<_>>();
                root = root.child(edge_canvas(
                    edges,
                    symbols.width * viewport.zoom,
                    symbols.height * viewport.zoom,
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
                for index in indices {
                    let node = &symbols.nodes[index];
                    let opacity = match &self.playback {
                        Some(playback) if fade_enabled => playback.node_context_opacity(&node.id),
                        Some(playback) => playback.node_opacity(&node.id),
                        None => 1.0,
                    };
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
                    .id("pb-transition")
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(rgb(0x1f2937))
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

        if self.state.transition().is_some() {
            let delta = self.transition_delta.clone();
            let generation = self.transition_generation;
            return root
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
        root.into_any_element()
    }
}

enum Scene {
    Modules(MapLayout),
    Symbols(SymbolLayout),
}

struct PlaybackOverlay {
    path_modules: BTreeSet<String>,
    current_module: Option<String>,
    current_symbol: Option<String>,
    current_edge: Option<(String, String)>,
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
        Self {
            path_modules: modules.iter().cloned().collect(),
            current_module: playback.current_module().map(ToOwned::to_owned),
            current_symbol: playback.current_symbol().map(ToOwned::to_owned),
            current_edge: playback.current_edge(),
            len: steps.len(),
            current: playback.current(),
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
    opacity: f32,
}

fn edge_color_at(base: u32, opacity: f32) -> gpui::Rgba {
    if opacity >= 1.0 {
        rgb(base)
    } else {
        rgba((base << 8) | u32::from(opacity_alpha_byte(opacity)))
    }
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
                    let base = if edge.strong {
                        0xf2f5f8
                    } else if edge.adjusted {
                        0xd98a3d
                    } else {
                        0x3d4b5c
                    };
                    let color = edge_color_at(base, edge.opacity);
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
    let background = rgb(band_background_palette(
        band.is_unclassified,
        band.is_external,
    ));
    let (border, label) = if band.is_unclassified {
        (rgb(0x6b552f), rgb(0xd98a3d))
    } else if band.is_external {
        (rgb(0x2f5a60), rgb(0x6fb3b8))
    } else {
        (rgb(0x1c2633), rgb(0x8b9bb0))
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
    rgb(symbol_kind_palette(kind))
}

fn layer_color(layer: &str, is_unclassified: bool) -> gpui::Rgba {
    rgb(layer_palette(layer, is_unclassified))
}
