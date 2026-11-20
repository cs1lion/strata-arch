use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arch_core::layers::build_layers;
use arch_view::layout::{layout_map, LayoutConfig, MapLayout};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, size, App, Application, Bounds, Context, IntoElement,
    Render, Window, WindowBounds, WindowOptions,
};

use super::{install_smoke_logger, phase_marker, USAGE};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneArtifact {
    schema_version: u32,
    tool_version: String,
    repo: arch_core::RepoInfo,
    layout: MapLayout,
}

pub(crate) fn run_scene(mut args: impl Iterator<Item = String>) -> Result<(), String> {
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

pub(crate) fn run_smoke(mut args: impl Iterator<Item = String>) -> Result<(), String> {
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
