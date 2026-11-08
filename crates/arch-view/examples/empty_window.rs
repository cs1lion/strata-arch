//! Measurement-only GPUI empty-window probe.
//!
//! Opens a window with no data preparation and no product view, emits the same
//! `phase:` markers as `arch-view timeline`, and exits after `--auto-close-ms`.
//! Used to separate environment-level startup drift from the timeline data path.
//!
//! Run:
//! cargo run --release -p arch-view --example empty_window -- empty <repo> --auto-close-ms 500
//!
//! The probe never runs in the product path and does not touch `arch-view/src`.

use std::env;
use std::io::Write;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, IntoElement, Render, Window,
    WindowBounds, WindowOptions,
};

static EMPTY_FIRST_FRAME: AtomicBool = AtomicBool::new(false);

fn phase_marker(label: &str) {
    if std::env::var_os("STRATA_PHASE_MARKERS").is_some() {
        println!("phase: {label}");
        let _ = std::io::stdout().flush();
    }
}

struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if !EMPTY_FIRST_FRAME.swap(true, Ordering::SeqCst) {
            phase_marker("frame:render");
            window.on_next_frame(|_, _cx| phase_marker("frame:presented"));
        }
        div().size_full().bg(rgb(0x111827))
    }
}

fn main() -> ExitCode {
    phase_marker("main:start");

    let mut auto_close_ms: Option<u64> = None;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--auto-close-ms" {
            auto_close_ms = args.next().and_then(|value| value.parse::<u64>().ok());
        }
    }

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
            move |_, cx| {
                phase_marker("window:callback");
                cx.new(|_| EmptyView)
            },
        )
        .expect("open empty window");
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

    ExitCode::SUCCESS
}
