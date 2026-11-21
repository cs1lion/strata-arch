//! 结构差异视图：把两次提交间的模块变化渲染为可淡入的清单。
//!
//! 从 `main.rs` 抽出（Issue #21 收尾），本票再把后台装配流程 `spawn_diff_loading`
//! 一并迁入，使 `main.rs` 只剩 CLI 解析与子命令路由。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arch_core::commit_diff::{diff_commits_cancellable, CommitDiffError, DiffProgress};
use arch_view::diff::{DiffFade, DiffModel, FADE_DURATION_MS};
use gpui::{
    div, prelude::*, px, rgb, Animation, AnimationExt, App, ClickEvent, Context, Entity,
    IntoElement, Render, Window,
};

use super::phase_marker;
use super::shell::{set_shell_stage, LoadingShell};

/// 进程级首帧标记：保证 `content:render` 只在真正首次渲染时打印一次。
static DIFF_FIRST_FRAME: AtomicBool = AtomicBool::new(false);

/// 后台计算结构 diff；阶段进度经共享槽转给前台，取消标志在每个可中断边界检查。
pub(crate) fn spawn_diff_loading(
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

pub(crate) struct DiffView {
    pub(crate) model: DiffModel,
    pub(crate) fade_enabled: bool,
    pub(crate) fade_generation: u64,
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
                .text_color(theme.text_primary)
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
                    .child(div().text_sm().text_color(theme.text_primary).child(header))
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
                            .text_color(theme.text_primary)
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
                    .text_color(theme.text_slate)
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
                        .bg(theme.chip_bg)
                        .text_xs()
                        .text_color(theme.text_slate)
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
                        .bg(theme.card_bg)
                        .text_xs()
                        .text_color(text_color)
                        .child(kind_label),
                )
                .child(middle)
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_slate)
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
