//! Blue-Topaz 风格的标题渐变色板，根据终端背景明暗自适应。
//!
//! 参考 Blue-Topaz Obsidian CSS 的 HSL 彩虹渐变方案（黄绿 → 绿 → 青 → 蓝 → 紫 → 粉紫），
//! 在暗色亮色背景下分别提供两套配色，保证可读性。颜色通过 `best_color()` 自动
//! 降级到终端支持的颜色级别（TrueColor / Ansi256 / Ansi16）。

use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::style::Stylize;

use crate::tui_core::color::is_light;
use crate::tui_core::terminal_palette::best_color_for_level;
use crate::tui_core::terminal_palette::default_bg;
use crate::tui_core::terminal_palette::effective_stdout_color_level;
use crate::tui_core::terminal_palette::StdoutColorLevel;

/// HSL 颜色到 RGB 颜色的转换。
///
/// 输入：H 为度数（0-360，超出取模），S 和 L 为 0.0-1.0 的浮点数（自动裁剪）。
/// 输出：对应的 8 位 RGB 三元组。
fn hsl_to_rgb(h: u16, s: f32, l: f32) -> (u8, u8, u8) {
    let h = (h % 360) as f32;
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r_prime, g_prime, b_prime) = match (h / 60.0) as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        5 => (c, 0.0, x),
        _ => (0.0, 0.0, 0.0),
    };
    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r_prime), to_u8(g_prime), to_u8(b_prime))
}

/// 返回 H1-H6 对应的 RGB 颜色，依据终端背景明暗选择亮/暗两套配色。
///
/// 色相沿 Blue-Topaz 方案渐变：黄绿(78°) → 绿(118°) → 青(180°) → 蓝(216°) → 紫(258°) → 粉紫(290°)
/// 暗色背景的 lightness 提到 57% 保证在黑底上可读；亮色背景降到 38% 在白底上有足够对比度。
pub(crate) fn heading_rgb_levels(is_light_bg: bool) -> [(u8, u8, u8); 6] {
    // 暗色背景：饱和度稍降避免刺眼，亮度提升到 57%
    // 亮色背景：饱和度保持 0.62，亮度降到 38% 保证对比度
    let (s, l) = if is_light_bg { (0.62, 0.38) } else { (0.55, 0.57) };
    [
        hsl_to_rgb(78, s, l),  // H1 黄绿
        hsl_to_rgb(118, s, l), // H2 绿
        hsl_to_rgb(180, s, l), // H3 青
        hsl_to_rgb(216, s, l), // H4 蓝
        hsl_to_rgb(258, s, l), // H5 紫
        hsl_to_rgb(290, s, l), // H6 粉紫
    ]
}

/// 根据当前终端背景返回 H1-H6 的 `Style`，已应用 Blue-Topaz 渐变色和粗体/斜体修饰。
///
/// H1 加粗下划线，H2-H4 加粗，H5-H6 斜体。颜色按终端能力分级：
/// - TrueColor：直接使用 HSL 渐变 RGB；
/// - Ansi256：经 `best_color_for_level` 映射到最接近的 xterm 索引色；
/// - Ansi16 / Unknown：退化到 6 个标准 ANSI 亮色（红/绿/黄/蓝/品红/青），
///   保证标题级别在任何终端都彼此可区分。此前低颜色环境下标题全部退化为无前景色，
///   各级标题颜色完全相同，本分支修复该回归。
pub(crate) fn heading_styles_for_terminal() -> [Style; 6] {
    let bg = default_bg();
    let is_light_bg = bg.is_some_and(is_light);
    let levels = heading_rgb_levels(is_light_bg);
    let color_level = effective_stdout_color_level();

    let modifiers: [Style; 6] = [
        Style::new().bold().underlined(),
        Style::new().bold(),
        Style::new().bold().italic(),
        Style::new().italic(),
        Style::new().italic(),
        Style::new().italic(),
    ];

    match color_level {
        StdoutColorLevel::TrueColor | StdoutColorLevel::Ansi256 => {
            let apply_fg = |base: Style, rgb: (u8, u8, u8)| match best_color_for_level(rgb, color_level) {
                Color::Reset => base,
                color => base.fg(color),
            };
            std::array::from_fn(|idx| apply_fg(modifiers[idx], levels[idx]))
        }
        // 16 色（或探测不到颜色能力）的终端下，best_color_for_level 会返回 Color::Reset，
        // 导致 H1-H6 颜色全部相同。这里直接用 6 个标准 ANSI 亮色，保证各级标题始终可区分。
        StdoutColorLevel::Ansi16 | StdoutColorLevel::Unknown => {
            const ANSI16_FALLBACK: [Color; 6] = [
                Color::LightRed,
                Color::LightGreen,
                Color::LightYellow,
                Color::LightBlue,
                Color::LightMagenta,
                Color::LightCyan,
            ];
            std::array::from_fn(|idx| modifiers[idx].fg(ANSI16_FALLBACK[idx]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn hsl_to_rgb_white_at_lightness_one() {
        // L=1.0 应该是白色（任何色相/饱和度）
        assert_eq!(hsl_to_rgb(0, 0.0, 1.0), (255, 255, 255));
        assert_eq!(hsl_to_rgb(180, 0.5, 1.0), (255, 255, 255));
    }

    #[test]
    fn hsl_to_rgb_black_at_lightness_zero() {
        // L=0.0 应该是黑色
        assert_eq!(hsl_to_rgb(0, 1.0, 0.0), (0, 0, 0));
        assert_eq!(hsl_to_rgb(240, 1.0, 0.0), (0, 0, 0));
    }

    #[test]
    fn hsl_to_rgb_gray_at_zero_saturation() {
        // S=0.0 任何 H 都不影响颜色，只看 L
        assert_eq!(hsl_to_rgb(0, 0.0, 0.5), (128, 128, 128));
        assert_eq!(hsl_to_rgb(180, 0.0, 0.25), (64, 64, 64));
    }

    #[test]
    fn hsl_to_rgb_primary_hues() {
        // H=0 红，H=120 绿，H=240 蓝
        assert_eq!(hsl_to_rgb(0, 1.0, 0.5), (255, 0, 0));
        assert_eq!(hsl_to_rgb(120, 1.0, 0.5), (0, 255, 0));
        assert_eq!(hsl_to_rgb(240, 1.0, 0.5), (0, 0, 255));
    }

    #[test]
    fn heading_rgb_levels_dark_is_brighter_than_light() {
        // 暗色背景配色整体亮度应高于亮色背景配色
        let dark = heading_rgb_levels(false);
        let light = heading_rgb_levels(true);
        for (idx, (d, l)) in dark.iter().zip(light.iter()).enumerate() {
            let dark_lum = (d.0 as u16 + d.1 as u16 + d.2 as u16) / 3;
            let light_lum = (l.0 as u16 + l.1 as u16 + l.2 as u16) / 3;
            assert!(
                dark_lum > light_lum,
                "H{idx} 暗色背景亮度 {dark_lum} 应大于亮色背景亮度 {light_lum}"
            );
        }
    }

    #[test]
    fn heading_rgb_levels_returns_six_distinct_colors() {
        let dark = heading_rgb_levels(false);
        let unique: HashSet<_> = dark.iter().collect();
        assert_eq!(unique.len(), 6, "六个标题颜色应该都不同");
    }

    #[test]
    fn heading_styles_for_terminal_yields_distinct_fg_in_low_color_env() {
        // 在测试/CI 环境下颜色探测解析为 Ansi16 或 Unknown；色板仍必须分配六个不同的前景色
        // （标准 ANSI 亮色），保证各级标题在视觉上彼此可区分，避免退化为单一无色样式。
        let styles = super::heading_styles_for_terminal();
        let fgs: HashSet<_> = styles.iter().map(|style| style.fg).collect();
        assert_eq!(fgs.len(), 6, "H1-H6 must have six distinct foreground colors, got {:#?}", styles);
        for style in &styles {
            assert!(style.fg.is_some(), "each heading must carry a foreground color: {:#?}", style);
        }
    }
}