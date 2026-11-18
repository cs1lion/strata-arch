//! 票 24 D 节：锁定 map 实际绘制步的淡出 α 语义。
//!
//! 背景：`bg.a *= item.opacity` 此前内联在 `main.rs` 的 canvas 绘制闭包里且无测试锁定；
//! `visual_theme_regression.rs` 里锁 `bg.a = 0.22` 的那条测试走的是 canvas 路径，
//! 而 `map` 命令不调用 canvas 渲染入口。本文件测的 `apply_fade_alpha` 正是 `map`
//! 现在实际调用的函数（`main.rs` 模块节点绘制闭包），因此构成对 live 路径的约束。
//!
//! 判别性说明：`fade_multiplies_not_replaces`（T1）是唯一能区分「乘法」与「替换」的断言
//! ——若实现被写成 `a = opacity`，T2/T3 恰好仍然通过（因为它们的输入 `a` 本就是 1.0），
//! 只有 T1 会红。这是本文件存在的核心理由。

use arch_view::arch_view_canvas::apply_fade_alpha;
use gpui::{Hsla, Rgba};

/// 以 `map` 实际使用的取值方式构造一个带指定 alpha 的颜色：先由十六进制转 `Hsla`，
/// 再只改 `a`（与 `main.rs` 中 `rgb(hex).into()` 后调 alpha 的写法同源）。
fn hsla_with_alpha(hex: u32, a: f32) -> Hsla {
    let mut c: Hsla = gpui::rgb(hex).into();
    c.a = a;
    c
}

fn max_channel_delta(a: Rgba, b: Rgba) -> f32 {
    let dr = ((a.r - b.r) * 255.0).abs();
    let dg = ((a.g - b.g) * 255.0).abs();
    let db = ((a.b - b.b) * 255.0).abs();
    dr.max(dg).max(db)
}

/// T1（判别性）：压暗态 alpha 与播放透明度必须相乘，不得被替换。
/// 0.22 × 0.68 = 0.1496；若实现改成 `a = opacity` 则得 0.68。
#[test]
fn fade_multiplies_not_replaces() {
    // 0.22 = arch_view_theme::non_neighbor_dim_factor，0.68 = playback::CONTEXT_OPACITY
    let (bg, border, accent) = apply_fade_alpha(
        hsla_with_alpha(0x4a3768, 0.22),
        hsla_with_alpha(0x30363f, 0.22),
        hsla_with_alpha(0xa371f7, 0.22),
        0.68,
    );
    let expected = 0.22_f32 * 0.68_f32;
    for (name, got) in [("bg", bg.a), ("border", border.a), ("accent", accent.a)] {
        assert!(
            (got - expected).abs() < 1e-6,
            "{name} 应为乘法结果 {expected:.6}，实得 {got:.6}"
        );
        // 显式钉住「不是替换」：替换语义会给出 0.68，与正确值差 0.53。
        assert!(
            (got - 0.68_f32).abs() > 0.1,
            "{name} 退化为替换语义（a = opacity）了：{got}"
        );
    }
}

/// T2：live 上下文透明度 0.68 对完全不透明颜色的作用。
#[test]
fn fade_at_context_opacity() {
    let (bg, border, accent) = apply_fade_alpha(
        hsla_with_alpha(0x4a3768, 1.0),
        hsla_with_alpha(0x30363f, 1.0),
        hsla_with_alpha(0xa371f7, 1.0),
        0.68,
    );
    assert!((bg.a - 0.68).abs() < 1e-6, "bg.a = {}", bg.a);
    assert!((border.a - 0.68).abs() < 1e-6, "border.a = {}", border.a);
    assert!((accent.a - 0.68).abs() < 1e-6, "accent.a = {}", accent.a);
}

/// T3：live 非路径透明度 0.25（p1-off 关闭态实际用的值）的作用。
#[test]
fn fade_at_non_path_opacity() {
    let (bg, border, accent) = apply_fade_alpha(
        hsla_with_alpha(0x4a3768, 1.0),
        hsla_with_alpha(0x30363f, 1.0),
        hsla_with_alpha(0xa371f7, 1.0),
        0.25,
    );
    assert!((bg.a - 0.25).abs() < 1e-6, "bg.a = {}", bg.a);
    assert!((border.a - 0.25).abs() < 1e-6, "border.a = {}", border.a);
    assert!((accent.a - 0.25).abs() < 1e-6, "accent.a = {}", accent.a);
}

/// T4：淡出只允许动 alpha，不得顺带改色相/饱和度/亮度或 RGB 通道。
/// 防的是「顺手在合成处调色」这类越界改动——它会同时悄悄改变 P1 的像素差语义。
#[test]
fn fade_preserves_rgb_channels() {
    let src = hsla_with_alpha(0x4a3768, 1.0);
    let (bg, border, accent) = apply_fade_alpha(
        src,
        hsla_with_alpha(0x4a3768, 1.0),
        hsla_with_alpha(0x4a3768, 1.0),
        0.68,
    );
    let before: Rgba = src.into();
    for (name, got) in [("bg", bg), ("border", border), ("accent", accent)] {
        let after: Rgba = got.into();
        assert!(
            max_channel_delta(before, after) < 1e-3,
            "{name} 的 RGB 通道被淡出合成改变了：before=({},{},{}) after=({},{},{})",
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
            "{name} 的 h/s/l 不应被淡出改动"
        );
    }
}

/// T5（停靠守卫，S1 采纳）：把票 24 D 节量化出的 A 方案目标写成可执行断言。
///
/// 断言内容：p1-on 与 p1-off 之间，节点填充色的逐像素最大位移必须跨过像素判据的容差，
/// 位移 = `|CONTEXT_OPACITY − NON_PATH_OPACITY| × max|fill − 层带有效 base|`。
/// 现状（`99fc678` 之后的暗卡片 palette）主力组合 unclassified-on-unclassified 仅 ≈1.3，
/// 因此本断言**当前必失败** ⇒ 标 `#[ignore]` 停靠，不作为常驻红。
///
/// 使用方式：A（修渲染）落地后移除 `#[ignore]`，它即转为常驻守卫；若届时仍失败，
/// 说明 palette 修复没真正恢复可感位移。
///
/// 注意：`6.0` 镜像 `eval/pixel-evidence.ps1` 的 `ChangeTolerance`（本 Rust 测试与之
/// 无自动联动）。若那边容差变更，这里必须同步重推，否则断言语义失真。
#[test]
#[ignore = "票 24 D 节已知不足：当前 palette 的淡出位移跨不过容差；A（修渲染）落地后移除本 ignore 即转为常驻守卫"]
fn salience_parked_guard_fade_displacement_exceeds_change_tolerance() {
    use arch_view::playback::{CONTEXT_OPACITY, NON_PATH_OPACITY};
    use arch_view::view::{band_background_palette, layer_palette, CANVAS_BACKGROUND};

    const CHANGE_TOLERANCE: f32 = 6.0;

    // 层带有效 base = band_style 以 alpha=0.78 叠在画布底色上
    let band = band_background_palette(true, false);
    let canvas = CANVAS_BACKGROUND;
    let chan = |hex: u32, shift: u32| ((hex >> shift) & 0xFF) as f32;
    let base = [
        0.78 * chan(band, 16) + 0.22 * chan(canvas, 16),
        0.78 * chan(band, 8) + 0.22 * chan(canvas, 8),
        0.78 * chan(band, 0) + 0.22 * chan(canvas, 0),
    ];

    // 当前 map 节点填充色：与 main.rs 同源的两步调色（饱和度 ×0.65，亮度 ×0.55 后 clamp）
    let pal = layer_palette("components", true);
    let mut tint: Hsla = gpui::rgb(pal).into();
    tint.s = (tint.s * 0.65).clamp(0.0, 1.0);
    tint.l = (tint.l * 0.55).clamp(0.08, 0.18);
    let fill: Rgba = tint.into();
    let fill255 = [fill.r * 255.0, fill.g * 255.0, fill.b * 255.0];

    let max_abs_diff = (fill255[0] - base[0])
        .abs()
        .max((fill255[1] - base[1]).abs())
        .max((fill255[2] - base[2]).abs());
    let alpha_gap = (CONTEXT_OPACITY - NON_PATH_OPACITY).abs();
    let displacement = alpha_gap * max_abs_diff;

    assert!(
        displacement > CHANGE_TOLERANCE,
        "淡出位移 {displacement:.3} 跨不过容差 {CHANGE_TOLERANCE}（max|fill−base|={max_abs_diff:.1}, α 差={alpha_gap:.2}）；\
         等价要求：max|fill−base| ≥ {:.1}",
        CHANGE_TOLERANCE / alpha_gap
    );
}
