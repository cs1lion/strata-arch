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
use arch_view::diff::{DiffFade, DiffModel, FADE_DURATION_MS};
use arch_view::layout::{layout_map, LayoutConfig, MapLayout, Point as LayoutPoint};
use arch_view::loading::{
    build_layer_artifact, compute_layout, list_first_parent_commits, sample_history, LoadStatus,
    LoadingStage, LoadingState,
};
use arch_view::playback::{PlaybackState, CONTEXT_OPACITY, NON_PATH_OPACITY};
use arch_view::reveal::{reveal_allowed, RevealState, REVEAL_DURATION_MS};
use arch_view::search::{interpret_key, FocusLabel, KeyModifiers, SearchAction, SearchState, SearchKey};
use arch_view::timeline::{degradation_label, TimelinePlayback, TimelineState};
use arch_view::view::{
    find_reachable_symbol_pair, group_edges, layer_palette, node_visual_geometry, paint_symbol_edges,
    paint_symbol_edges_with_roles, rect_fully_outside, segment_fully_outside, CanvasState, EdgeDisplayMode,
    EdgeVisual as EdgeDraw, Level, SymbolLayout, Viewport, CULL_MARGIN, MAX_ZOOM, MIN_ZOOM,
};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, size, Animation, AnimationExt, AnyElement, AnyView,
    App, Application, Bounds, ClickEvent, Context, Div, Entity, FocusHandle, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Render,
    ScrollDelta, ScrollWheelEvent, Stateful, Timer, Window, WindowBounds, WindowOptions,
};

const USAGE: &str = "用法:
  arch-view map <目标仓库> [--auto-close-ms <毫秒>] [--reveal|--no-reveal] [--from <入口ID> --to <符号ID>]
  arch-view timeline <目标仓库> [--auto-close-ms <毫秒>] [--autoplay]
  arch-view diff <目标仓库> --from <commit> --to <commit> [--no-fade] [--auto-close-ms <毫秒>]
  arch-view scene <目标仓库> [--out <场景文件>]
  arch-view smoke <目标仓库>
  首屏分层揭示默认开启；--no-reveal 关闭。两者同时出现时以 --no-reveal 为准（与顺序无关）。
  模块搜索：/ 或 Ctrl/Cmd+F 打开，↑/↓ 选择，Enter 聚焦，Esc 清除；有 --from/--to 时禁用搜索。";

/// 阶段标记：仅在 `STRATA_PHASE_MARKERS=1` 时输出，供外部计时脚本记录到达时间。
/// 不在视图源码里取墙钟（`arch-view/src` 禁用时间源判据不变），时间戳由外部进程提供。
fn phase_marker(label: &str) {
    if std::env::var_os("STRATA_PHASE_MARKERS").is_some() {
        println!("phase: {label}");
        let _ = std::io::stdout().flush();
    }
}

/// 诊断开关：按需跳过部分绘制，用受控实验的差值定位每帧成本来源。
/// 取值 `bands,edges,nodes,labels` 的逗号子集（`STRATA_RENDER_SKIP`）。
#[derive(Clone, Copy, Default)]
struct RenderSkip {
    bands: bool,
    edges: bool,
    nodes: bool,
    labels: bool,
}

impl RenderSkip {
    fn from_env() -> Self {
        let mut skip = Self::default();
        let Ok(spec) = std::env::var("STRATA_RENDER_SKIP") else {
            return skip;
        };
        for part in spec.split(',') {
            match part.trim() {
                "bands" => skip.bands = true,
                "edges" => skip.edges = true,
                "nodes" => skip.nodes = true,
                "labels" => skip.labels = true,
                _ => {}
            }
        }
        skip
    }
}

/// 诊断用确定性平移帧数：每渲染帧平移 8px 并续订下一帧（`STRATA_AUTOPAN_FRAMES`）。
fn autopan_frames() -> Option<u32> {
    std::env::var("STRATA_AUTOPAN_FRAMES")
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn frame_trace_enabled() -> bool {
    std::env::var_os("STRATA_FRAME_TRACE").is_some()
}

/// 证据抓帧用固定相机：`STRATA_EVIDENCE_VIEWPORT=panX,panY,zoom`。
/// 仅用于跨配置在「同一相机位」抓帧比较，不参与交互逻辑。
fn evidence_viewport() -> Option<Viewport> {
    let spec = std::env::var("STRATA_EVIDENCE_VIEWPORT").ok()?;
    let parts = spec
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if parts.len() != 3 {
        return None;
    }
    Some(Viewport {
        pan_x: parts[0],
        pan_y: parts[1],
        zoom: parts[2].clamp(MIN_ZOOM, MAX_ZOOM),
    })
}

/// 渲染调优开关（A/B）。`STRATA_AB_CYCLE` 存在时按帧段交替覆盖，用于同进程 A/B。
#[derive(Clone, Copy)]
struct RenderTuning {
    batch_edges: bool,
    cull_viewport: bool,
}

fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => value.trim() != "0",
        Err(_) => default,
    }
}

impl RenderTuning {
    fn from_env() -> Self {
        Self {
            // A（边批处理）已按票 10 批准默认开启；`STRATA_EDGE_BATCH=0` 可恢复逐边绘制，
            // 用于需要精确栅格（逐条 dash 相位/绘制顺序）的场合。
            batch_edges: env_flag("STRATA_EDGE_BATCH", true),
            // B（视口裁剪）：只剪完全在视口外的元素，静止帧逐像素一致，默认开启。
            cull_viewport: env_flag("STRATA_VIEWPORT_CULL", true),
        }
    }

    fn for_segment(name: &str) -> Self {
        match name {
            "base" => Self {
                batch_edges: false,
                cull_viewport: false,
            },
            "A" => Self {
                batch_edges: true,
                cull_viewport: false,
            },
            "B" => Self {
                batch_edges: false,
                cull_viewport: true,
            },
            "BA" | "AB" => Self {
                batch_edges: true,
                cull_viewport: true,
            },
            _ => Self::from_env(),
        }
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
        if let LoadStatus::Loading(loading) = &self.status {
            let progress = match loading.stage() {
                LoadingStage::ReadingCommits => 0.2,
                LoadingStage::SamplingHistory { .. } => 0.5,
                LoadingStage::BuildingLayers => 0.75,
                LoadingStage::ComputingLayout => 0.9,
                LoadingStage::Custom(_) => 0.6,
            };
            panel = panel.child(
                div()
                    .w(px(280.))
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(rgb(0x2c3a4a))
                    .child(
                        div()
                            .w(px(280.0 * progress))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(rgb(0x58a6ff)),
                    ),
            );
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
                                        "正在取消，等待当前阶段收尾".to_string(),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkbenchTab {
    Map,
    Timeline, 
    Diff,
}

/// 顶层统一工作台：在单窗口顶部提供 Map / Timeline 切换，按需懒加载另一个视图。
struct WorkbenchView {
    repo: PathBuf,
    active: WorkbenchTab,
    map: Option<Entity<LoadingShell<MapView>>>,
    timeline: Option<Entity<TimelineView>>,
    diff: Option<Entity<LoadingShell<DiffView>>>,
    reveal_requested: bool,
    autoplay: bool,
    fade_enabled: bool,
}

impl WorkbenchView {
    fn switch_to(&mut self, tab: WorkbenchTab, cx: &mut Context<Self>) {
        if self.active == tab {
            return;
        }
        match tab {
            WorkbenchTab::Map => {
                if self.map.is_none() {
                    let shell = cx
                        .new(|_| LoadingShell::<MapView>::new("正在准备架构地图", "构建分层"));
                    spawn_map_loading(
                        shell.clone(),
                        self.repo.clone(),
                        None,
                        self.reveal_requested,
                        cx,
                    );
                    self.map = Some(shell);
                }
            }
            WorkbenchTab::Timeline => {
                if self.timeline.is_none() {
                    let view = cx.new(|_| TimelineView::loading(self.autoplay));
                    spawn_timeline_loading(view.clone(), self.repo.clone(), self.autoplay, cx);
                    self.timeline = Some(view);
                }
            }
            WorkbenchTab::Diff => {
                if self.diff.is_none() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    let shell_cancel = cancel.clone();
                    let shell = cx.new(move |_| {
                        LoadingShell::<DiffView>::new(
                            "正在计算最近提交结构 diff",
                            "解析 HEAD~1..HEAD",
                            ).with_cancel(shell_cancel)
                        });
                    spawn_diff_loading(
                        shell.clone(),
                        self.repo.clone(),
                        "HEAD~1".to_string(),
                        "HEAD".to_string(),
                        self.fade_enabled,
                        cancel,
                        cx,
                        );
                    self.diff = Some(shell);
                }
            }
        }
        self.active = tab;
        cx.notify();
    }
}

impl Render for WorkbenchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active;
        let repo_name = self
            .repo
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("strata-arch")
            .to_string();

        let top_bar = div()
            .id("workbench-tabs")
            .w_full()
            .h(px(36.))
            .px(px(12.))
            .bg(rgb(0x0d1117))
            .border_b_1()
            .border_color(rgb(0x21262d))
            .flex()
            .items_center()
            .justify_between()
            .text_xs()
            .text_color(rgb(0xe6edf3))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().text_color(rgb(0x8b949e)).child("Strata"))
                    .child(
                        div()
                            .id("tab-map")
                            .px(px(10.))
                            .py(px(4.))
                            .rounded(px(5.))
                            .bg(if active == WorkbenchTab::Map {
                                rgb(0x1f3a5f)
                            } else {
                                rgb(0x161b22)
                            })
                            .child("架构地图 Map")
                            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                                this.switch_to(WorkbenchTab::Map, cx);
                            })),
                    )
                    .child(
                        div()
                            .id("tab-timeline")
                            .px(px(10.))
                            .py(px(4.))
                            .rounded(px(5.))
                            .bg(if active == WorkbenchTab::Timeline {
                                rgb(0x1f3a5f)
                            } else {
                                rgb(0x161b22)
                            })
                            .child("演化时间线 Timeline")
                            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                                this.switch_to(WorkbenchTab::Timeline, cx);
                            })),
                    )
                .child(
                div()
                .id("tab-diff")
                .px(px(10.))
                .py(px(4.))
                .rounded(px(5.))
                .bg(if active == WorkbenchTab::Diff {
                    rgb(0x1f3a5f)
                    } else {
                    rgb(0x161b22)
                    })
                .child("结构差异 Diff")
                .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                    this.switch_to(WorkbenchTab::Diff, cx);
                    })),
                )
            )
            .child(div().text_color(rgb(0x8b949e)).child(repo_name));

        let mut content = div().flex_1().w_full().relative().overflow_hidden();
        match self.active {
            WorkbenchTab::Map => {
                if let Some(map) = &self.map {
                    content = content.child(map.clone());
                }
            }
            WorkbenchTab::Timeline => {
                if let Some(timeline) = &self.timeline {
                    content = content.child(timeline.clone());
                }
            }
            WorkbenchTab::Diff => {
                if let Some(diff) = &self.diff { content = content.child(diff.clone()); }
            }
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x0d1117))
            .child(top_bar)
            .child(content)
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

/// 首屏揭示的 CLI 语义：默认开启；只要出现 `--no-reveal` 就关闭。
///
/// `--reveal` 保留为显式兼容参数，不参与决定，因此两者同时出现时以显式关闭为准，
/// 且与参数顺序无关。
fn reveal_default_on(explicit_off: bool) -> bool {
    !explicit_off
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
    let mut reveal_off = false;
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
            // 默认已开启，`--reveal` 只保留为显式兼容参数；是否关闭只看 `--no-reveal`，
            // 因此两者同时出现、无论顺序，都以显式关闭为准。
            "--reveal" => {}
            "--no-reveal" => reveal_off = true,
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
    // 默认开启；`--no-reveal` 显式关闭，且优先于同批出现的 `--reveal`。
    let reveal_requested = reveal_default_on(reveal_off);

    // 先开窗：分层/调用图/布局都在窗口打开后的后台任务里完成，
    // 长操作期间窗口显示阶段文案，用户可随时关闭或点取消。
    phase_marker("app:created");
    Application::new().run(move |cx: &mut App| {
        arch_view::arch_view_theme::StrataTheme::init_global(cx);
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
                    window_repo.clone(),
                    playback_request,
                    reveal_requested,
                    cx,
                );
                cx.new(|_| WorkbenchView {
                    repo: window_repo,
                    active: WorkbenchTab::Map,
                    map: Some(shell),
                    timeline: None,
                    reveal_requested,
                    autoplay: false, diff: None, fade_enabled: true,
                })
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
        arch_view::arch_view_theme::StrataTheme::init_global(cx);
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
                spawn_timeline_loading(view.clone(), window_repo.clone(), autoplay, cx);
                cx.new(|_| WorkbenchView {
                    repo: window_repo,
                    active: WorkbenchTab::Timeline,
                    map: None,
                    timeline: Some(view),
                    reveal_requested: true,
                    autoplay, diff: None, fade_enabled: true,
                })
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
        arch_view::arch_view_theme::StrataTheme::init_global(cx);
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
                    window_repo.clone(),
                    from,
                    to,
                    fade_enabled,
                    window_cancel,
                    cx,
                );
                cx.new(|_| WorkbenchView { repo: window_repo, active: WorkbenchTab::Diff, map: None, timeline: None, diff: Some(shell), reveal_requested: true, autoplay: false, fade_enabled }) // shell
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
        let theme = cx
            .try_global::<arch_view::arch_view_theme::StrataTheme>()
            .cloned()
            .unwrap_or_default();

        let header = format!(
            "{} → {} · 新增 {} / 删除 {} / 修改 {} · 疑似重命名 {}",
            short_commit(&self.model.from_commit),
            short_commit(&self.model.to_commit),
            self.model.added,
            self.model.removed,
            self.model.modified,
            self.model.renames
        );

        let badge = |label: String, color: u32| {
            div()
                .px(px(8.))
                .py(px(2.))
                .rounded(px(999.))
                .bg(rgb(color))
                .text_xs()
                .text_color(rgb(0xe6edf3))
                .child(label)
        };

        let mut root = div()
            .id("diff-root")
            .flex()
            .flex_col()
            .gap(px(8.))
            .size_full()
            .p(px(14.))
            .bg(theme.canvas_bg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().text_sm().text_color(rgb(0xe6edf3)).child(header))
                    .child(
                        div()
                            .id("diff-fade")
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .bg(theme.hud_panel_bg)
                            .border_1()
                            .border_color(theme.hud_border)
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
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(badge(format!("+{}", self.model.added), 0x2f6b3f))
                    .child(badge(format!("-{}", self.model.removed), 0x7a3138))
                    .child(badge(format!("~{}", self.model.modified), 0x7a5b24))
                    .child(badge(format!("renames {}", self.model.renames), 0x1f4e79)),
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
            let (bg, accent, text_color) = theme.resolve_diff_row_colors(row.kind, 1.0);
            let kind_label = match row.kind {
                arch_view::diff::DiffKind::Added => "新增",
                arch_view::diff::DiffKind::Removed => "删除",
                arch_view::diff::DiffKind::Modified => "修改",
            };
            let index = row.z_index;
            let mut middle = div()
                .flex()
                .flex_1()
                .items_center()
                .gap(px(8.))
                .min_w(px(0.));
            middle = middle.child(
                div()
                    .text_xs()
                    .text_color(text_color)
                    .child(row.path.clone()),
            );
            if let Some(hint) = &row.rename_hint {
                middle = middle.child(
                    div()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(999.))
                        .bg(rgb(0x1f2937))
                        .text_xs()
                        .text_color(rgb(0x8b9bb0))
                        .child(format!("← {}", hint)),
                );
            }

            let element = div()
                .id(("diff-row", index as usize))
                .flex()
                .items_center()
                .gap(px(8.))
                .w_full()
                .px(px(10.))
                .py(px(6.))
                .rounded(px(6.))
                .bg(bg)
                .child(div().w(px(3.)).h(px(18.)).rounded(px(2.)).bg(accent))
                .child(
                    div()
                        .w(px(44.))
                        .flex()
                        .justify_center()
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(999.))
                        .bg(rgb(0x161b22))
                        .text_xs()
                        .text_color(text_color)
                        .child(kind_label),
                )
                .child(middle)
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x8b9bb0))
                        .child(row.detail.clone()),
                );

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
        {
            let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
            let band_draws: Vec<(arch_view::layout::Rect, String, bool, bool, usize)> = self
                .layout
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
            for band in &self.layout.bands {
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
            let color = if failed { rgb(0xe07b39) } else { rgb(0xe6edf3) };
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
                    .bg(rgb(0x2c3a4a))
                    .child(
                        div()
                            .w(px(280.0 * progress))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(rgb(0x58a6ff)),
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
                    rgb(0x8a7a5a).into()
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
                    .bg(if active { rgb(0x58a6ff) } else { rgb(0x3d4b5c) })
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
                        rgb(0x2c3a4a)
                    } else {
                        rgb(0x50241f)
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
                    .bg(rgb(0x2c3a4a))
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
            .child(div().child(format!("共 {position}/{ticks} 刻度")));

        let mut badges = div().flex().items_center().gap(px(6.));
        if let Some(label) = degradation_label(flags) {
            badges = badges.child(
                div()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(999.))
                    .bg(rgb(0x5a2d23))
                    .text_color(rgb(0xffd0c2))
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
                    .bg(rgb(0x3a2f1a))
                    .text_color(rgb(0xd98a3d))
                    .child(format!("状态: {}", markers.join(" / "))),
            );
        }
        if let Some(notice) = ready.timeline.gap_notice() {
            badges = badges.child(
                div()
                    .text_color(rgb(0xe07b39))
                    .child(format!("采样空洞: {notice}")),
            );
        }
        if ended {
            badges = badges.child(div().text_color(rgb(0xd98a3d)).child("播放结束"));
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
            .text_color(rgb(0xe6edf3))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(div().child(commit).child(format!("  [{position}/{ticks}]")))
                    .child(
                        div()
                            .text_color(rgb(0x8b9bb0))
                            .child(ready.timeline.sampling_label()),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(rgb(0x2c3a4a))
                    .child(
                        div()
                            .w(gpui::relative(progress_ratio))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(rgb(0x58a6ff)),
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
const INSPECTOR_MAX_ITEMS: usize = 8;

struct MapView {
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
        if !self.search.is_open()
            && !keystroke.modifiers.secondary()
            && !keystroke.modifiers.alt
        {
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
                            Some(set) => set.focus == edge.source || set.focus == edge.target,
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
                    .child(div().text_xs().text_color(rgb(0xf0f6fc)).child(label.clone()));
                    if let Some(subtitle) = &overlay_item.subtitle {
                        text_box = text_box.child(div().text_size(px(10.)).text_color(rgb(0x8b949e)).child(subtitle.clone()));
                    }root = root.child(text_box);
                        
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
                        .border_color(rgb(0x30363d))
                        .bg(rgb(0x161b22))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .text_xs()
                        .text_color(rgb(0xe6edf3))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(8.))
                                .child(div().text_color(rgb(0xf0f6fc)).child(short_name))
                                .child(
                                    div()
                                        .id("inspector-close")
                                        .px(px(6.))
                                        .py(px(2.))
                                        .rounded(px(4.))
                                        .bg(rgb(0x1f2937))
                                        .child("×")
                                        .on_click(cx.listener(
                                            |this, _event: &ClickEvent, _window, cx| {
                                                this.state.clear_selection();
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .child(div().text_color(rgb(0x8b949e)).child(selected_id.clone()))
                        .child(
                            div()
                                .text_color(rgb(0x79c0ff))
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
                                .bg(rgb(0x1f3a5f))
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
                                        rgb(0x1f3a5f)
                                    } else {
                                        rgb(0x1f2937)
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
                                        rgb(0x1f3a5f)
                                    } else {
                                        rgb(0x1f2937)
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
                                .child(div().text_color(rgb(0xd29922)).child(summary))
                                .child(
                                    div()
                                        .id("inspector-clear-path")
                                        .px(px(6.))
                                        .py(px(2.))
                                        .rounded(px(4.))
                                        .bg(rgb(0x1f2937))
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
                            .text_color(rgb(0x8b949e))
                            .child(format!("上游依赖 ({up_total})")),
                    );
                    if upstream.is_empty() {
                        inspector = inspector
                            .child(div().text_color(rgb(0x6e7681)).child("无上游依赖"));
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
                                    .bg(rgb(0x0d1117))
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
                            inspector = inspector.child(div().text_color(rgb(0x6e7681)).child(
                                format!("... 另有 {} 项", up_total - INSPECTOR_MAX_ITEMS),
                            ));
                        }
                    }

                    inspector = inspector.child(
                        div()
                            .text_color(rgb(0x8b949e))
                            .child(format!("下游依赖 ({down_total})")),
                    );
                    if downstream.is_empty() {
                        inspector = inspector
                            .child(div().text_color(rgb(0x6e7681)).child("无下游依赖"));
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
                                    .bg(rgb(0x0d1117))
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
                            inspector = inspector.child(div().text_color(rgb(0x6e7681)).child(
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
            .border_color(rgb(0x30363d))
            .bg(rgb(0x161b22))
            .text_xs()
            .text_color(rgb(0xe6edf3))
            .child(
                div()
                    .text_color(rgb(0x8b949e))
                    .child(format!("{module_count} 模块 · {edge_count} 边")),
            );

        if self.playback.is_none() {
            hud = hud.child(
                div()
                    .id("hud-search")
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(rgb(0x1f2937))
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
                        rgb(0x1f2937)
                    } else {
                        rgb(0x1f3a5f)
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
                    .bg(rgb(0x1f2937))
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
                    .text_color(rgb(0xc9d1d9))
                    .child(format!("{zoom_pct}%")),
            )
            .child(
                div()
                    .id("hud-zoom-in")
                    .px(px(6.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(rgb(0x1f2937))
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
                    .bg(rgb(0x1f2937))
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
                .border_color(rgb(0x30363d))
                .bg(rgb(0x161b22))
                .text_xs()
                .text_color(rgb(0xe6edf3));

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
                            rgb(0x1f3a5f)
                        } else {
                            rgb(0x1f2937)
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
                        .on_click(cx.listener(
                            move |this, _event: &ClickEvent, _window, cx| {
                                this.interrupt_reveal();
                                this.layer_filter = if this.layer_filter == Some(layer_name) {
                                    None
                                } else {
                                    Some(layer_name)
                                };
                                cx.notify();
                            },
                        )),
                );
            }

            if active_layer.is_some() {
                legend = legend.child(
                    div()
                        .id("legend-clear")
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .bg(rgb(0x30363d))
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

enum Scene {
    Modules(MapLayout),
    Symbols(SymbolLayout),
}

/// 输入态面板：query、`前 6/共 N`（或 `无匹配 · 0/0`）与可见命中列表。
fn search_panel_element(search: &SearchState) -> AnyElement {
    let query = if search.query().is_empty() {
        "输入以筛选模块".to_string()
    } else {
        search.query().to_string()
    };
    let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
    let mut panel = div()
        .id("search-panel")
        .absolute()
        .left(px(12.))
        .top(px(12.))
        .w(px(420.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.hud_border)
        .bg(theme.hud_panel_bg)
        .text_xs()
        .text_color(rgb(0xe6edf3))
        .child(format!("搜索模块 · {query}"))
        .child(div().text_color(rgb(0x8b9bb0)).child(search.summary()));
    let start = search.list_start();
    for (offset, hit) in search.visible_hits().iter().enumerate() {
        let selected = start + offset == search.selected();
        panel = panel.child(
            div()
                .px(px(6.))
                .py(px(2.))
                .rounded(px(4.))
                .bg(if selected {
                    theme.search_hit_bg
                } else {
                    theme.canvas_bg
                })
                .child(hit.id.clone()),
        );
    }
    panel.into_any_element()
}

/// 聚焦态角落标签：聚焦模块 id + 上游 / 下游一跳计数。
fn focus_label_element(label: &FocusLabel) -> AnyElement {
    let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
    div()
        .id("focus-label")
        .absolute()
        .right(px(12.))
        .bottom(px(12.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.hud_border)
        .bg(theme.hud_panel_bg)
        .text_xs()
        .text_color(rgb(0xe6edf3))
        .child(label.text())
        .into_any_element()
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

fn edge_canvas(
    edges: Vec<EdgeDraw>,
    width: f32,
    height: f32,
    zoom: f32,
    batch_edges: bool,
) -> impl IntoElement {
    div()
        .absolute()
        .left(px(0.))
        .top(px(0.))
        .w(px(width))
        .h(px(height))
        .child(canvas(
            |_, _, _| (),
            move |bounds, _, window, _cx| {
                let theme = arch_view::arch_view_theme::StrataTheme::obsidian_dark();
                for run in group_edges(&edges, batch_edges) {
                    let (color, stroke_w) =
                        theme.resolve_edge_style(run.strong, run.adjusted, run.opacity, zoom);
                    let mut batched = PathBuilder::stroke(stroke_w);
                    let mut has_batched = false;
                    for index in &run.edges {
                        let edge = &edges[*index];
                        let start = point(
                            bounds.origin.x + px(edge.from.x),
                            bounds.origin.y + px(edge.from.y),
                        );
                        let end = point(
                            bounds.origin.x + px(edge.to.x),
                            bounds.origin.y + px(edge.to.y),
                        );
                        let dy = f32::from(end.y) - f32::from(start.y);
                        let offset_y = (dy.abs() * 0.45).clamp(24.0 * zoom, 140.0 * zoom);
                        let sign_y = if dy >= 0.0 { 1.0 } else { -1.0 };
                        let ctrl1 = point(start.x, start.y + px(offset_y * sign_y));
                        let ctrl2 = point(end.x, end.y - px(offset_y * sign_y));
                        if run.strong || run.adjusted || !batch_edges {
                            let mut b = PathBuilder::stroke(stroke_w);
                            b.move_to(start);
                            b.cubic_bezier_to(end, ctrl1, ctrl2);
                            if let Ok(path) = b.build() {
                                window.paint_path(path, color);
                            }
                            let vx = f32::from(end.x) - f32::from(ctrl2.x);
                            let vy = f32::from(end.y) - f32::from(ctrl2.y);
                            let len = vx.hypot(vy).max(0.001);
                            let ux = vx / len;
                            let uy = vy / len;
                            let arrow_len = 7.0 * zoom.clamp(0.65, 1.3);
                            let arrow_half_w = 3.5 * zoom.clamp(0.65, 1.3);
                            let base_x = f32::from(end.x) - ux * arrow_len;
                            let base_y = f32::from(end.y) - uy * arrow_len;
                            let left = point(
                                px(base_x - uy * arrow_half_w),
                                px(base_y + ux * arrow_half_w),
                            );
                            let right = point(
                                px(base_x + uy * arrow_half_w),
                                px(base_y - ux * arrow_half_w),
                            );
                            let mut arrow = PathBuilder::fill();
                            arrow.move_to(end);
                            arrow.line_to(left);
                            arrow.line_to(right);
                            arrow.close();
                            if let Ok(ap) = arrow.build() {
                                window.paint_path(ap, color);
                            }
                        } else {
                            batched.move_to(start);
                            batched.cubic_bezier_to(end, ctrl1, ctrl2);
                            has_batched = true;
                        }
                    }
                    if has_batched {
                        if let Ok(path) = batched.build() {
                            window.paint_path(path, color);
                        }
                    }
                }
            },
        ))
}
fn selection_border(
    id: &str,
    theme: &arch_view::arch_view_theme::StrataTheme,
    default: gpui::Hsla,
    selected: Option<&str>,
    highlighted: &std::collections::BTreeSet<String>,
    focus: Option<&str>,
) -> gpui::Hsla {
    if focus == Some(id) || selected == Some(id) {
        theme.selection_border
    } else if highlighted.contains(id) {
        theme.selection_highlight_border
    } else {
        default
    }
}
                /// 将模块路径拆分为（文件名主标题, 精简目录副标题），超长目录左侧省略。
fn split_module_label(id: &str) -> (String, Option<String>) {
    let Some((dir, file)) = id.rsplit_once('/') else { return (id.to_string(), None); };
    if dir.is_empty() { return (file.to_string(), None); } const MAX_DIR_CHARS: usize = 22;
    let subtitle = if dir.chars().count() > MAX_DIR_CHARS { let tail = dir.chars().rev().take(MAX_DIR_CHARS - 2).collect::<Vec<_>>().into_iter().rev().collect::<String>(); format!("…{tail}") } else { dir.to_string() }; (file.to_string(), Some(subtitle)) }
                #[cfg(test)]
mod tests {
    use super::{reveal_default_on, split_module_label};

    #[test] fn split_module_label_extracts_file_and_dir() { assert_eq!(split_module_label("src/components/Button.tsx"), ("Button.tsx".to_string(), Some("src/components".to_string()))); assert_eq!(split_module_label("index.ts"), ("index.ts".to_string(), None)); }
    #[test] fn split_module_label_truncates_long_directory_safely() { let (title, sub) = split_module_label("packages/core/src/features/editor/toolbar/Button.tsx"); assert_eq!(title, "Button.tsx"); let sub = sub.expect("subtitle"); assert!(sub.starts_with('…')); assert!(sub.chars().count() <= 22); }
    #[test]
    fn reveal_is_on_by_default() {
        assert!(reveal_default_on(false), "未给开关时必须默认开启");
    }

    #[test]
    fn explicit_no_reveal_turns_reveal_off() {
        // `--reveal` 不参与决定，所以与 `--no-reveal` 同时出现时、无论顺序都以关闭为准。
        assert!(!reveal_default_on(true), "--no-reveal 必须关闭揭示");
    }
}
