use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arch_view::loading::{LoadStatus, LoadingStage, LoadingState};
use arch_view::view::{Viewport, MAX_ZOOM, MIN_ZOOM};
use gpui::{
    div, prelude::*, px, rgb, AnyElement, AnyView, Context, Entity, IntoElement, MouseButton,
    Render, Window,
};

use super::phase_marker;

/// 诊断开关：按需跳过部分绘制，用受控实验的差值定位每帧成本来源。
/// 取值 `bands,edges,nodes,labels` 的逗号子集（`STRATA_RENDER_SKIP`）。
#[derive(Clone, Copy, Default)]
pub(crate) struct RenderSkip {
    pub(crate) bands: bool,
    pub(crate) edges: bool,
    pub(crate) nodes: bool,
    pub(crate) labels: bool,
}

impl RenderSkip {
    pub(crate) fn from_env() -> Self {
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
pub(crate) fn autopan_frames() -> Option<u32> {
    std::env::var("STRATA_AUTOPAN_FRAMES")
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub(crate) fn frame_trace_enabled() -> bool {
    std::env::var_os("STRATA_FRAME_TRACE").is_some()
}

/// 证据抓帧用固定相机：`STRATA_EVIDENCE_VIEWPORT=panX,panY,zoom`。
/// 仅用于跨配置在「同一相机位」抓帧比较，不参与交互逻辑。
pub(crate) fn evidence_viewport() -> Option<Viewport> {
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
pub(crate) struct RenderTuning {
    pub(crate) batch_edges: bool,
    pub(crate) cull_viewport: bool,
}

fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => value.trim() != "0",
        Err(_) => default,
    }
}

impl RenderTuning {
    pub(crate) fn from_env() -> Self {
        Self {
            // A（边批处理）已按票 10 批准默认开启；`STRATA_EDGE_BATCH=0` 可恢复逐边绘制，
            // 用于需要精确栅格（逐条 dash 相位/绘制顺序）的场合。
            batch_edges: env_flag("STRATA_EDGE_BATCH", true),
            // B（视口裁剪）：只剪完全在视口外的元素，静止帧逐像素一致，默认开启。
            cull_viewport: env_flag("STRATA_VIEWPORT_CULL", true),
        }
    }

    pub(crate) fn for_segment(name: &str) -> Self {
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
pub(crate) struct LoadingShell<T: Render> {
    status: LoadStatus,
    child: Option<Entity<T>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl<T: Render> LoadingShell<T> {
    pub(crate) fn new(headline: &str, stage: &str) -> Self {
        Self {
            status: LoadStatus::Loading(LoadingState::for_task(
                headline,
                LoadingStage::Custom(stage.to_string()),
            )),
            child: None,
            cancel: None,
        }
    }

    pub(crate) fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub(crate) fn set_stage(&mut self, stage: &str) {
        if let LoadStatus::Loading(loading) = &mut self.status {
            loading.set_stage(LoadingStage::Custom(stage.to_string()));
        }
    }

    pub(crate) fn fail(&mut self, message: String) {
        self.status = LoadStatus::Failed { message };
    }

    pub(crate) fn ready(&mut self, child: Entity<T>) {
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
