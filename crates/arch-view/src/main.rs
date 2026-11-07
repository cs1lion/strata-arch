use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use arch_core::layers::build_layers;
use arch_view::layout::{layout_map, BandLayout, LayoutConfig, MapLayout, NodeLayout};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, size, App, Application, Bounds, Context, IntoElement,
    PathBuilder, Render, Window, WindowBounds, WindowOptions,
};

const USAGE: &str = "用法:
  arch-view map <目标仓库> [--auto-close-ms <毫秒>]";

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

    let artifact = build_layers(&PathBuf::from(&target), env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("构建层产物失败: {error}"))?;
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
            move |_, cx| cx.new(|_| MapView { layout }),
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

struct MapView {
    layout: MapLayout,
}

impl Render for MapView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let layout = self.layout.clone();
        let mut root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0d1117));

        for band in &layout.bands {
            root = root.child(band_element(band));
        }

        let edges = layout.edges.clone();
        root = root.child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(0.))
                .w(px(layout.width))
                .h(px(layout.height))
                .child(canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        for edge in &edges {
                            let color = if edge.direction_adjusted {
                                rgb(0xd98a3d)
                            } else {
                                rgb(0x3d4b5c)
                            };
                            let mut builder = PathBuilder::stroke(px(1.));
                            if edge.direction_adjusted {
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
                )),
        );

        for node in &layout.nodes {
            root = root.child(node_element(node));
        }

        root
    }
}

fn band_element(band: &BandLayout) -> impl IntoElement {
    let (background, border, label) = if band.is_unclassified {
        (rgb(0x1f1a12), rgb(0x6b552f), rgb(0xd98a3d))
    } else if band.is_external {
        (rgb(0x101d21), rgb(0x2f5a60), rgb(0x6fb3b8))
    } else {
        (rgb(0x11161c), rgb(0x1c2633), rgb(0x8b9bb0))
    };
    div()
        .absolute()
        .left(px(band.rect.x))
        .top(px(band.rect.y))
        .w(px(band.rect.width))
        .h(px(band.rect.height))
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

fn node_element(node: &NodeLayout) -> impl IntoElement {
    let background = layer_color(node.layer, node.is_unclassified);
    let border = if node.is_unclassified {
        rgb(0xd98a3d)
    } else if node.is_external {
        rgb(0x6fb3b8)
    } else {
        rgb(0x2c3a4a)
    };

    let mut element = div()
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
        .flex()
        .items_center()
        .px(px(6.))
        .child(
            div()
                .text_xs()
                .text_color(rgb(0xe6edf3))
                .child(node.id.clone()),
        );

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
