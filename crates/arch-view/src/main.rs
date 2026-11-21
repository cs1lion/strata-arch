mod diff_view;
mod map_view;
mod overlays;
mod shell;
mod smoke_view;
mod timeline_view;
mod workbench;
mod insights;

use diff_view::{spawn_diff_loading, DiffView};
use map_view::{spawn_map_loading, MapView};
use shell::LoadingShell;
use smoke_view::{run_scene, run_smoke};
use timeline_view::{spawn_timeline_loading, TimelineView};
use workbench::{WorkbenchTab, WorkbenchView};

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use gpui::{prelude::*, px, size, App, Application, Bounds, WindowBounds, WindowOptions};

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
                    autoplay: false,
                    diff: None,
                    fade_enabled: true,
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
                    autoplay,
                    diff: None,
                    fade_enabled: true,
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
                cx.new(|_| WorkbenchView {
                    repo: window_repo,
                    active: WorkbenchTab::Diff,
                    map: None,
                    timeline: None,
                    diff: Some(shell),
                    reveal_requested: true,
                    autoplay: false,
                    fade_enabled,
                }) // shell
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

#[cfg(test)]
mod tests {
    use super::reveal_default_on;

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
