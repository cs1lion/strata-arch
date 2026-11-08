use std::cell::Cell;
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use arch_core::callgraph::{build_call_graph, CallGraphArtifact};
use arch_core::commit_diff::{diff_commits_cancellable, CommitDiffError, DiffProgress};
use arch_core::guided_path::build_path;
use arch_core::layers::build_layers;
use arch_view::diff::{DiffFade, DiffModel, DIFF_LABEL, FADE_DURATION_MS};
use arch_view::layout::{
    layout_map, BandLayout, LayoutConfig, MapLayout, NodeLayout, Point as LayoutPoint,
};
use arch_view::loading::{
    build_layer_artifact, compute_layout, list_first_parent_commits, sample_history, LoadStatus,
    LoadingStage, LoadingState,
};
use arch_view::playback::{opacity_alpha_byte, PlaybackState, CONTEXT_OPACITY, NON_PATH_OPACITY};
use arch_view::reveal::{reveal_allowed, RevealState};
use arch_view::timeline::{degradation_label, TimelinePlayback, TimelineState};
use arch_view::view::{
    band_background_palette, layer_palette, paint_symbol_edges, paint_symbol_edges_with_roles,
    symbol_kind_palette, CanvasState, Level, SymbolLayout, Viewport,
};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, rgba, size, Animation, AnimationExt, AnyElement,
    AnyView, App, Application, Bounds, ClickEvent, Context, Entity, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Render, ScrollDelta,
    ScrollWheelEvent, Timer, Window, WindowBounds, WindowOptions,
};

const USAGE: &str = "用法:
  arch-view map <目标仓库> [--auto-close-ms <毫秒>] [--reveal] [--from <入口ID> --to <符号ID>]
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

/// 「先开窗、后加载」的通用外壳：窗口先呈现阶段文案，数据就绪后挂上真实子视图。
///
/// 长操作期间窗口必须可见且显示当前阶段；用户可关闭窗口或点「取消」立即退出。
struct LoadingShell<T: Render> {
    status: LoadStatus,
    child: Option<Entity<T>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl<T: Render> LoadingShell<T> {
    fn new(headline: &str, stage: &str) -> Self {
        Self {
            status: LoadStatus::Loading(LoadingState::for_task(
                headline,
                LoadingStage::Custom(stage.to_string()),
            )),
            child: None,
            cancel: None,
        }
    }

    fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    fn set_stage(&mut self, stage: &str) {
        if let LoadStatus::Loading(loading) = &mut self.status {
            loading.set_stage(LoadingStage::Custom(stage.to_string()));
        }
    }

    fn fail(&mut self, message: String) {
        self.status = LoadStatus::Failed { message };
    }

    fn ready(&mut self, child: Entity<T>) {
        self.status = LoadStatus::Ready;
        self.child = Some(child);
    }

    fn render_status_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let failed = matches!(self.status, LoadStatus::Failed { .. });
        let color = if failed { rgb(0xe07b39) } else { rgb(0xe6edf3) };
        let mut panel = div()
            .size_full()
            .bg(rgb(0x0d1117))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .px(px(24.));
        for line in self.status.visible_lines() {
            panel = panel.child(div().text_sm().text_color(color).child(line));
        }
        if !failed {
            panel = panel.child(
                div()
                    .mt(px(6.))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(0x2c3a4a))
                    .bg(rgb(0x161b22))
                    .text_color(rgb(0xe6edf3))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if let Some(cancel) = &this.cancel {
                                cancel.store(true, Ordering::SeqCst);
                                if let LoadStatus::Loading(loading) = &mut this.status {
                                    loading.set_stage(LoadingStage::Custom(
                                        "正在取消：等待当前阶段收尾".to_string(),
                                    ));
                                }
                            } else {
                                cx.quit();
                            }
                            cx.notify();
                        }),
                    )
                    .child("取消准备（也可直接关闭窗口）"),
            );
        }
        panel.into_any_element()
    }
}

impl<T: Render> Render for LoadingShell<T> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        static SHELL_FIRST_FRAME: AtomicBool = AtomicBool::new(false);
        if !SHELL_FIRST_FRAME.swap(true, Ordering::SeqCst) {
            phase_marker("frame:render");
            window.on_next_frame(|_, _cx| phase_marker("frame:presented"));
        }
        if let Some(child) = &self.child {
            return div()
                .size_full()
                .child(AnyView::from(child.clone()))
                .into_any_element();
        }
        self.render_status_panel(cx)
    }
}

/// 冒烟诊断：`STRATA_SMOKE_LOG=1` 时把 gpui 的 log 输出到 stderr，
/// 用于在无 GPU 的 CI 上定位帧请求循环为什么没启动（不改变正常运行的输出）。
fn install_smoke_logger() {
    static LOGGER: StderrLogger = StderrLogger;
    if std::env::var_os("STRATA_SMOKE_LOG").is_none() {
        return;
    }
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "log[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
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
    let mut reveal_requested = false;
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
            "--reveal" => reveal_requested = true,
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

    let playback_request = match (from, to) {
        (Some(from), Some(to)) => Some((from, to)),
        (None, None) => None,
        _ => return Err(format!("--from 与 --to 必须同时提供\n{USAGE}")),
    };

    let repo = PathBuf::from(&target);
    let reveal_requested = reveal_requested || std::env::var_os("STRATA_EVIDENCE_REVEAL").is_some();

    // 先开窗：分层/调用图/布局都在窗口打开后的后台任务里完成，
    // 长操作期间窗口显示阶段文案，用户可随时关闭或点取消。
    phase_marker("app:created");
    Application::new().run(move |cx: &mut App| {
        phase_marker("app:entry");
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let window_repo = repo;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_window, cx| {
                phase_marker("window:callback");
                let shell =
                    cx.new(|_| LoadingShell::<MapView>::new("正在准备架构地图", "构建分层"));
                spawn_map_loading(
                    shell.clone(),
                    window_repo,
                    playback_request,
                    reveal_requested,
                    cx,
                );
                shell
            },
        )
        .expect("open map window");
        phase_marker("window:created");
        cx.activate(true);
        phase_marker("window:activated");

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

/// 后台准备架构地图；每完成一个阶段就更新窗口内的阶段文案。
fn spawn_map_loading(
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
            "map: reveal={} layers={} frames={}",
            if reveal.is_enabled() { "on" } else { "off" },
            layout.bands.len(),
            reveal.total_frames()
        );

        let built_view = cx.update(|cx| {
            cx.new(move |_| MapView {
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
                reveal,
                reveal_scheduled: false,
                reveal_start_marked: false,
                reveal_mid_marked: false,
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

fn set_shell_stage<T: Render>(
    shell: &Entity<LoadingShell<T>>,
    cx: &mut gpui::AsyncApp,
    stage: &str,
) {
    let _ = shell.update(cx, |shell, cx| {
        shell.set_stage(stage);
        cx.notify();
    });
}

fn map_load_failed(
    shell: &Entity<LoadingShell<MapView>>,
    cx: &mut gpui::AsyncApp,
    message: String,
) {
    let _ = shell.update(cx, |shell, cx| {
        shell.fail(message);
        cx.notify();
    });
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

    // 先开窗：数据准备移到窗口打开后的后台任务里，首帧只呈现加载态。
    let application = Application::new();
    phase_marker("app:created");
    application.run(move |cx: &mut App| {
        phase_marker("app:entry");
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let window_repo = repo;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_window, cx| {
                phase_marker("window:callback");
                let view = cx.new(|_| TimelineView::loading(autoplay));
                spawn_timeline_loading(view.clone(), window_repo, autoplay, cx);
                view
            },
        )
        .expect("open timeline window");
        phase_marker("window:created");
        cx.activate(true);
        phase_marker("window:activated");

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

/// 后台分阶段准备数据；每段完成即更新加载态文案，全部完成后切换到真实视图。
///
/// 自动关闭早于本任务完成时，`view.update` 返回 `Err`（app 已释放），这里只忽略返回值：
/// 进程仍按 `quit:auto` 干净退出，不延迟、不 panic。
fn spawn_timeline_loading(view: Entity<TimelineView>, repo: PathBuf, autoplay: bool, cx: &mut App) {
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

fn set_loading_stage(view: &Entity<TimelineView>, cx: &mut gpui::AsyncApp, stage: LoadingStage) {
    let _ = view.update(cx, |view, cx| {
        if let TimelineContent::Loading(loading) = &mut view.content {
            loading.set_stage(stage);
        }
        cx.notify();
    });
}

fn timeline_load_failed(view: &Entity<TimelineView>, cx: &mut gpui::AsyncApp, message: String) {
    phase_marker("load:failed");
    let _ = view.update(cx, |view, cx| {
        view.content = TimelineContent::Failed(message);
        cx.notify();
    });
}

fn run_diff(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    phase_marker("diff:prep:start");
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
    let cancel = Arc::new(AtomicBool::new(false));

    // 先开窗：`diff_commits` 在窗口打开后的后台任务里执行，并在每个阶段更新文案。
    // 超过 2 秒的路径不能再让用户面对「没有窗口」的静止画面。
    phase_marker("app:created");
    Application::new().run(move |cx: &mut App| {
        phase_marker("app:entry");
        let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
        let window_repo = repo;
        let window_cancel = cancel.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_window, cx| {
                phase_marker("window:callback");
                let shell_cancel = window_cancel.clone();
                let shell = cx.new(move |_| {
                    LoadingShell::<DiffView>::new("正在计算结构 diff", "解析提交")
                        .with_cancel(shell_cancel)
                });
                spawn_diff_loading(
                    shell.clone(),
                    window_repo,
                    from,
                    to,
                    fade_enabled,
                    window_cancel,
                    cx,
                );
                shell
            },
        )
        .expect("open diff window");
        phase_marker("window:created");
        cx.activate(true);
        phase_marker("window:activated");

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

/// 后台计算结构 diff；阶段进度经共享槽转给前台，取消标志在每个可中断边界检查。
fn spawn_diff_loading(
    shell: Entity<LoadingShell<DiffView>>,
    repo: PathBuf,
    from: String,
    to: String,
    fade_enabled: bool,
    cancel: Arc<AtomicBool>,
    cx: &mut App,
) {
    let progress = Arc::new(Mutex::new(Some(DiffProgress::Resolving)));
    let done = Arc::new(AtomicBool::new(false));
    let progress_task = progress.clone();
    let done_task = done.clone();
    let cancel_task = cancel.clone();
    let task = cx.background_executor().spawn(async move {
        phase_marker("diff:prep:start");
        let mut report = |stage: DiffProgress| {
            if let Ok(mut slot) = progress_task.lock() {
                *slot = Some(stage);
            }
        };
        let result = diff_commits_cancellable(
            &repo,
            &from,
            &to,
            env!("CARGO_PKG_VERSION"),
            &cancel_task,
            &mut report,
        );
        done_task.store(true, Ordering::SeqCst);
        result
    });

    cx.spawn(async move |cx| {
        loop {
            if cancel.load(Ordering::SeqCst) {
                set_shell_stage(&shell, cx, "正在取消：等待当前阶段收尾");
            } else if let Ok(slot) = progress.lock() {
                if let Some(stage) = *slot {
                    set_shell_stage(&shell, cx, stage.label());
                }
            }
            if done.load(Ordering::SeqCst) {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
        }

        match task.await {
            Ok(artifact) => {
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
                let built = cx.update(|cx| {
                    cx.new(move |_| DiffView {
                        model,
                        fade_enabled,
                        fade_generation: 0,
                    })
                });
                let view = match built {
                    Ok(view) => view,
                    Err(_) => return,
                };
                phase_marker("diff:prep:done");
                let _ = shell.update(cx, |shell, cx| {
                    shell.ready(view);
                    cx.notify();
                });
            }
            Err(CommitDiffError::Cancelled) => {
                phase_marker("diff:cancelled");
                let _ = cx.update(|cx| cx.quit());
            }
            Err(error) => {
                let _ = shell.update(cx, |shell, cx| {
                    shell.fail(format!("构建结构 diff 失败: {error}"));
                    cx.notify();
                });
            }
        }
    })
    .detach();
}

static DIFF_FIRST_FRAME: AtomicBool = AtomicBool::new(false);

struct DiffView {
    model: DiffModel,
    fade_enabled: bool,
    fade_generation: u64,
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !DIFF_FIRST_FRAME.swap(true, Ordering::SeqCst) {
            phase_marker("content:render");
            window.on_next_frame(|_, _cx| phase_marker("content:presented"));
        }
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
    // 看门狗预算默认与 ticket 14 保持一致（12 秒）；`STRATA_SMOKE_TIMEOUT_SECS`
    // 只用于诊断/软件渲染环境下确认「慢」还是「卡死」，不放宽断言。
    let timeout = Duration::from_secs(
        std::env::var("STRATA_SMOKE_TIMEOUT_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(12),
    );
    println!("smoke: watchdog {}s", timeout.as_secs());
    // 看门狗必须是独立 OS 线程：窗口创建/首次渲染阻塞时 GPUI 执行器无法运行，
    // 原来的 `cx.spawn` 定时器无法触发（见 ticket 14 取证）。
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        if !rendered_watchdog.load(Ordering::SeqCst) {
            println!("smoke: timeout waiting for first frame");
            let _ = std::io::stdout().flush();
            std::process::exit(2);
        }
    });

    phase_marker("smoke:app:before-new");
    install_smoke_logger();
    let application = Application::new();
    phase_marker("smoke:app:built");
    application.run(move |cx: &mut App| {
        phase_marker("smoke:run:enter");
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let _window = cx
            .open_window(
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
        phase_marker("smoke:window:opened");
        cx.activate(true);
        phase_marker("smoke:activate:done");
    });
    phase_marker("smoke:run:exited");

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
        phase_marker("smoke:render:enter");
        if !self.scheduled {
            self.scheduled = true;
            let rendered = self.rendered.clone();
            window.on_next_frame(move |_, cx| {
                phase_marker("smoke:frame:callback");
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
            root = root.child(band_element(band, Viewport::default(), 1.0, 0.0));
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

struct ReadyTimeline {
    state: CanvasState,
    timeline: TimelineState,
}

/// 视图内容：先呈现加载态，后台数据就绪后整体切换；失败时在窗口内显示错误。
enum TimelineContent {
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

struct TimelineView {
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
    fn loading(trace: bool) -> Self {
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
    fn render_status_panel(&self) -> impl IntoElement {
        let failed = matches!(self.content, TimelineContent::Failed(_));
        let mut panel = div()
            .size_full()
            .bg(rgb(0x0d1117))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .px(px(24.));
        for line in self.content.status_lines() {
            let color = if failed { rgb(0xe07b39) } else { rgb(0xe6edf3) };
            panel = panel.child(div().text_sm().text_color(color).child(line));
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

        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0d1117));

        for band in &layout.bands {
            root = root.child(band_element(band, Viewport::default(), 1.0, 0.0));
        }

        for (index, node) in layout.nodes.iter().enumerate() {
            let heat = ready.timeline.heat(&node.id);
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
                        if let Some(timeline) = this.timeline_mut() {
                            timeline.select(&node_id);
                        }
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
                    .bg(rgb(0x2c3a4a))
                    .child("下一步")
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
                    .bg(rgb(0x2c3a4a))
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
                    .bg(rgb(0x2c3a4a))
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
            .child(
                div()
                    .text_color(rgb(0x8b9bb0))
                    .child(ready.timeline.sampling_label()),
            )
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

        if let Some(notice) = ready.timeline.gap_notice() {
            footer = footer.child(
                div()
                    .text_color(rgb(0xe07b39))
                    .child(format!("采样空洞: {notice}")),
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
    /// P5 首屏分层揭示：一次性、帧驱动、可打断。
    reveal: RevealState,
    reveal_scheduled: bool,
    reveal_start_marked: bool,
    reveal_mid_marked: bool,
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

    /// 用户任何交互都立即结束揭示并落到最终态；已结束/默认关时无操作。
    fn interrupt_reveal(&mut self) {
        if self.reveal.skip() {
            phase_marker("reveal:skip");
        }
    }

    /// 每个 GPUI 帧回调推进一格；仍在揭示时续订下一帧，播完即停（不回绕）。
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
            view.reveal.advance_frame();
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
        if !MAP_FIRST_FRAME.swap(true, Ordering::SeqCst) {
            phase_marker("content:render");
            window.on_next_frame(|_, _cx| phase_marker("content:presented"));
        }
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
        if self.reveal.is_active() {
            Self::schedule_reveal_frame(self, window, cx);
        }
        let reveal_overall = self.reveal.overall_progress();

        let mut root = div()
            .id("canvas")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0d1117))
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
                    .bg(rgb(0x161b22))
                    .text_xs()
                    .text_color(rgb(0x8b9bb0))
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
                    .bg(rgb(0x1f2937))
                    .text_xs()
                    .text_color(rgb(0xe6edf3))
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

        match scene {
            Scene::Modules(map) => {
                let stage_of_layer = map
                    .bands
                    .iter()
                    .enumerate()
                    .map(|(index, band)| (band.layer, index))
                    .collect::<std::collections::BTreeMap<&str, usize>>();
                for (index, band) in map.bands.iter().enumerate() {
                    root = root.child(band_element(
                        band,
                        viewport,
                        self.reveal.opacity_for_stage(index),
                        self.reveal.offset_for_stage(index),
                    ));
                }
                let base_opacity =
                    (if fade_enabled { CONTEXT_OPACITY } else { 1.0 }) * reveal_overall;
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
                        let opacity = opacity * reveal_overall;
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
                    let stage = stage_of_layer.get(node.layer).copied().unwrap_or(0);
                    let opacity = opacity * self.reveal.opacity_for_stage(stage);
                    let offset_y = self.reveal.offset_for_stage(stage);
                    root = root.child(node_element(
                        node,
                        viewport,
                        labels,
                        selected.as_deref(),
                        &highlighted,
                        opacity,
                        focus,
                        offset_y,
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
                        opacity: edge.opacity * reveal_overall,
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
                    let opacity = opacity * reveal_overall;
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

fn band_element(
    band: &BandLayout,
    viewport: Viewport,
    opacity: f32,
    offset_y: f32,
) -> impl IntoElement {
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
        .top(px(rect.y + offset_y))
        .w(px(rect.width))
        .h(px(rect.height))
        .opacity(opacity)
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
    offset_y: f32,
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
        .top(px(rect.y + offset_y))
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
