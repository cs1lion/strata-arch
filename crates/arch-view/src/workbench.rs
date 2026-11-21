use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use gpui::{div, prelude::*, px, rgb, ClickEvent, Context, Entity, IntoElement, Render, Window};

use super::diff_view::DiffView;
use super::shell::LoadingShell;
use super::{
    spawn_diff_loading, spawn_map_loading, spawn_timeline_loading, MapView, TimelineView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkbenchTab {
    Map,
    Timeline,
    Diff,
}

/// 顶层统一工作台：在单窗口顶部提供 Map / Timeline / Diff 切换，按需懒加载其他视图。
pub(crate) struct WorkbenchView {
    pub(crate) repo: PathBuf,
    pub(crate) active: WorkbenchTab,
    pub(crate) map: Option<Entity<LoadingShell<MapView>>>,
    pub(crate) timeline: Option<Entity<TimelineView>>,
    pub(crate) diff: Option<Entity<LoadingShell<DiffView>>>,
    pub(crate) reveal_requested: bool,
    pub(crate) autoplay: bool,
    pub(crate) fade_enabled: bool,
}

impl WorkbenchView {
    fn switch_to(&mut self, tab: WorkbenchTab, cx: &mut Context<Self>) {
        if self.active == tab {
            return;
        }
        match tab {
            WorkbenchTab::Map => {
                if self.map.is_none() {
                    let shell =
                        cx.new(|_| LoadingShell::<MapView>::new("正在准备架构地图", "构建分层"));
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
                        )
                        .with_cancel(shell_cancel)
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
                    ),
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
                if let Some(diff) = &self.diff {
                    content = content.child(diff.clone());
                }
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
