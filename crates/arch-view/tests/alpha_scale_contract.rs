//! 票 29：锁定 `main.rs` 搬出来的 α 乘法点（`scale_alpha` / `scale_alpha3`）。
//!
//! 背景：`49c5432` 之前，`main.rs` 里有 8 条内联 `*.a *= opacity`（TimelineView 节点 2 条、
//! `map` 层带 3 条、符号节点 3 条），全部无测试锁定；已锁的只有矩形模块节点那一个调用点
//! （见 `fade_alpha_contract.rs`）。本文件补齐其余两形的语义。
//!
//! 判别性设计（沿用 `fade_alpha_contract.rs` T1 与 T2/T3 的教训）：所有喂入的 `a` 都**不等于 1.0**，
//! 因为当 `a == 1.0` 时「乘法」与「替换」恰好同值，测试会失去判别力。每条断言除了核对
//! 乘法应得值，还显式核对它**不等于** `opacity` 本身（替换语义的产物）。
//!
//! 注：`hsla_with_alpha` / `max_channel_delta` 与 `fade_alpha_contract.rs` 各自持有一份副本——
//! 集成测试之间无法共享私有 helper（除非再开一个 test-support crate）。此处刻意保持两份独立实现，
//! 避免为了复用而引入新的构建面。

use arch_view::arch_view_canvas::{apply_fade_alpha, scale_alpha, scale_alpha3};
use gpui::{Hsla, Rgba};

fn hsla_with_alpha(hex: u32, a: f32) -> Hsla {
    let mut c: Hsla = gpui::rgb(hex).into();
    c.a = a;
    c
}

fn max_channel_delta(a: Rgba, b: Rgba) -> f32 {
    ((a.r - b.r) * 255.0)
        .abs()
        .max(((a.g - b.g) * 255.0).abs())
        .max(((a.b - b.b) * 255.0).abs())
}

/// T1（判别性）：TimelineView 节点色带的 α 必须是乘法。
/// 两个输入都取产品实际值：`main.rs` 的 `node_opacity = if full_scale { 1.0 } else { 0.55 }`，
/// 边框色本身带非 1 的 alpha ⇒ 取 a=0.55、opacity=0.55，应得 0.3025；替换语义会得 0.55。
#[test]
fn timeline_alpha_multiplies_not_replaces() {
    let src = hsla_with_alpha(0x30363f, 0.55);
    let got = scale_alpha(src, 0.55);
    let expected = 0.55_f32 * 0.55_f32;
    assert!(
        (got.a - expected).abs() < 1e-6,
        "α 应为乘法结果 {expected:.6}，实得 {:.6}",
        got.a
    );
    assert!(
        (got.a - 0.55_f32).abs() > 0.1,
        "退化为替换语义（a = opacity）了：{}",
        got.a
    );
}

/// T2：层带三级色（fill / border / header）各自乘以 reveal α，且**三级都不许被同一个 opacity 覆盖**。
/// 输入取 `arch_view_theme.rs` 的 band_style 实际 alpha：fill 0.78、border 0.28、header 0.90；
/// reveal 取中值 0.5。若实现退化为替换，三者会全部变成 0.5——header 一侧只差 0.05，
/// 故该通道用 0.04 的独立余量（其余两通道余量为 0.11 / 0.36）。
#[test]
fn band_tiers_multiply_not_replace() {
    let fill = hsla_with_alpha(0x21262b, 0.78);
    let border = hsla_with_alpha(0x58a6ff, 0.28);
    let header = hsla_with_alpha(0x58a6ff, 0.90);
    let reveal = 0.5_f32;
    let (f, b, h) = scale_alpha3((fill, border, header), reveal);

    for (name, got, expected, margin) in [
        ("fill", f.a, 0.78_f32 * reveal, 0.04_f32),
        ("border", b.a, 0.28_f32 * reveal, 0.04),
        ("header", h.a, 0.90_f32 * reveal, 0.04),
    ] {
        assert!(
            (got - expected).abs() < 1e-6,
            "{name} α 应为 {expected:.6}，实得 {got:.6}"
        );
        assert!(
            (got - reveal).abs() > margin,
            "{name} 退化为替换语义（a = reveal）了：{got}"
        );
    }
    // 三级色必须仍彼此不同——替换语义会把它们压成同一个值（这正是"层级塌陷"的早期信号）。
    assert!(
        (f.a - b.a).abs() > 0.1 && (h.a - b.a).abs() > 0.1,
        "三级 alpha 被压平了：fill={f_a} border={b_a} header={h_a}",
        f_a = f.a,
        b_a = b.a,
        h_a = h.a
    );
}

/// T3：`scale_alpha3` 与已锁定的 `apply_fade_alpha` 语义必须一致（两者都是逐通道乘法）。
/// 钉的是"两个 helper 将来不许各自漂移"——符号节点走后者、层带走前者。
/// 输入 a 取非 1 值以保证判别性。
#[test]
fn scale_alpha3_agrees_with_apply_fade_alpha() {
    let (a, b, c) = (
        hsla_with_alpha(0x4a3768, 0.78),
        hsla_with_alpha(0x30363f, 0.28),
        hsla_with_alpha(0xa371f7, 0.90),
    );
    let opacity = 0.68_f32;
    let via_fade = apply_fade_alpha(a, b, c, opacity);
    let via_scale = scale_alpha3((a, b, c), opacity);
    for (name, x, y) in [
        ("first", via_fade.0, via_scale.0),
        ("second", via_fade.1, via_scale.1),
        ("third", via_fade.2, via_scale.2),
    ] {
        assert!(
            (x.a - y.a).abs() < 1e-6,
            "{name} 两个 helper 的 α 不一致：apply_fade_alpha={xa} scale_alpha3={ya}",
            xa = x.a,
            ya = y.a
        );
        assert!(
            max_channel_delta(x.into(), y.into()) < 1e-3,
            "{name} 两个 helper 的 RGB 不一致"
        );
    }
}

/// T4：α 缩放只许动 alpha，不得顺带改 h/s/l 或 RGB 通道。
/// 防的是"在合成处顺手调色"——那会同时改变 P1 像素差的语义（与 fade 测试同一防线）。
#[test]
fn scale_alpha_preserves_color_channels() {
    let src = hsla_with_alpha(0x4a3768, 0.8);
    let before: Rgba = src.into();
    let got = scale_alpha(src, 0.68);
    let after: Rgba = got.into();
    assert!(
        max_channel_delta(before, after) < 1e-3,
        "RGB 通道被 α 缩放改变了：before=({},{},{}) after=({},{},{})",
        before.r * 255.0,
        before.g * 255.0,
        before.b * 255.0,
        after.r * 255.0,
        after.g * 255.0,
        after.b * 255.0,
    );
    assert!(
        (got.h - src.h).abs() < 1e-6
            && (got.s - src.s).abs() < 1e-6
            && (got.l - src.l).abs() < 1e-6,
        "h/s/l 不应被 α 缩放改动"
    );
    assert!(
        (got.a - 0.8_f32 * 0.68_f32).abs() < 1e-6,
        "α 应为 0.544，实得 {}",
        got.a
    );
}

/// T5：两次缩放必须复合成 α·o1·o2（锁死"不夹带 clamp / 不重置"）。
/// 层带实际就是这种叠加：`band_style` 内部先按 0.78·reveal 写一次 alpha，
/// 外层再乘一次 reveal——本断言只锁乘法的可复合性，不主张该叠加是否多余（见票 29 §未验证）。
#[test]
fn scale_alpha_composes_multiplicatively() {
    let src = hsla_with_alpha(0x89b4fa, 1.0);
    let once = scale_alpha(src, 0.5);
    let twice = scale_alpha(once, 0.5);
    assert!(
        (twice.a - 0.25_f32).abs() < 1e-6,
        "两次 0.5 缩放应得 0.25，实得 {}",
        twice.a
    );
    assert!(
        (once.a - 0.5_f32).abs() < 1e-6,
        "单次应得 0.5，实得 {}",
        once.a
    );
}

/// T6（由 M3 反证逼出，不是凭空加的）：α 缩放不得夹带地板或钳制。
/// 实测过程：先把实现改成 `(a * opacity).max(0.05)`，T1–T5 **全部仍绿**（5 passed / 0 failed）——
/// 因为那五条的输入都落在钳制区间之外。因此必须专门喂"乘完小于地板"的输入才咬得住。
/// 产品语义上这里不允许任何下限：`opacity = 0.0` 就是"完全不可见"，票 27 的播放态正是靠它抹掉上下文边。
#[test]
fn scale_alpha_has_no_floor_or_clamp() {
    let tiny = scale_alpha(hsla_with_alpha(0x89b4fa, 0.02), 0.5);
    assert!(
        (tiny.a - 0.01_f32).abs() < 1e-7,
        "0.02 × 0.5 应得 0.01，实得 {}（若得 0.05 量级说明被加了下限钳制）",
        tiny.a
    );
    let zero = scale_alpha(hsla_with_alpha(0x89b4fa, 1.0), 0.0);
    assert!(
        zero.a.abs() < 1e-7,
        "opacity = 0 必须得 0，实得 {}（若非 0 说明有地板）",
        zero.a
    );
    let sub_floor = scale_alpha(hsla_with_alpha(0x89b4fa, 0.04), 1.0);
    assert!(
        (sub_floor.a - 0.04_f32).abs() < 1e-7,
        "0.04 原样保留应得 0.04，实得 {}",
        sub_floor.a
    );
}
