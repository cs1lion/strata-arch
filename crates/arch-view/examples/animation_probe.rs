// Measurement-only probe for GPUI `with_animation` frame cadence (ticket 15).
//
// This does NOT drive the camera transition and is not part of the fly-to path;
// `Instant` here is only used to report the measured frame schedule. The
// transition progress itself comes from GPUI's animation delta, and
// `crates/arch-view/src` contains no wall-clock reads.
//
// Run: cargo run -p arch-view --example animation_probe

use std::cell::Cell;
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, rgb, size, Animation, AnimationExt, App, Application, Bounds, Context,
    IntoElement, Render, Window, WindowBounds, WindowOptions,
};

struct ProbeView {
    frames: Rc<Cell<u32>>,
    delta: Rc<Cell<f32>>,
    start: Rc<Cell<Option<Instant>>>,
    printed: Rc<Cell<bool>>,
}

impl Render for ProbeView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.start.get().is_none() {
            self.start.set(Some(Instant::now()));
        }

        let callback_frames = self.frames.clone();
        let callback_delta = self.delta.clone();
        let callback_start = self.start.clone();
        let callback_printed = self.printed.clone();
        if !self.printed.get() {
            window.on_next_frame(move |_, cx| {
                let delta = callback_delta.get();
                let elapsed = callback_start.get().expect("start").elapsed();
                println!(
                    "probe frame {:>2} delta {:.4} elapsed {}ms",
                    callback_frames.get(),
                    delta,
                    elapsed.as_millis()
                );
                if delta >= 1.0 {
                    callback_printed.set(true);
                    println!(
                        "probe done: frames={} elapsed_ms={}",
                        callback_frames.get(),
                        elapsed.as_millis()
                    );
                    let _ = std::io::stdout().flush();
                    cx.quit();
                }
            });
        }

        let animator_frames = self.frames.clone();
        let animator_delta = self.delta.clone();
        div().size_full().bg(rgb(0x111827)).with_animation(
            "animation-probe",
            Animation::new(Duration::from_millis(200)),
            move |element, progress| {
                animator_frames.set(animator_frames.get() + 1);
                animator_delta.set(progress);
                element
            },
        )
    }
}

fn main() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(12));
        println!("probe timeout waiting for the animation to finish");
        let _ = std::io::stdout().flush();
        std::process::exit(2);
    });

    let frames = Rc::new(Cell::new(0));
    let delta = Rc::new(Cell::new(0.0));
    let start = Rc::new(Cell::new(None));
    let printed = Rc::new(Cell::new(false));

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(400.), px(300.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| ProbeView {
                    frames,
                    delta,
                    start,
                    printed,
                })
            },
        )
        .expect("open probe window");
        cx.activate(true);
    });
}
