//! 为 token 活动图表构建感知终端环境的样式与字形选择。
//!
//! 该调色板在将主题颜色适配到当前终端色深的同时，
//! 把图表专属的字形策略保持在 token 活动渲染器内部。

use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::style::Stylize;

use super::TokenActivityView;
use crate::tui_core::color::blend;
use crate::tui_core::render::highlight::foreground_style_for_scopes;
use crate::tui_core::style::accent_style;
use crate::tui_core::terminal_palette::StdoutColorLevel;
use crate::tui_core::terminal_palette::best_color_for_level;
use crate::tui_core::terminal_palette::default_bg;
use crate::tui_core::terminal_palette::default_fg;
use crate::tui_core::terminal_palette::stdout_color_level;

// 在低色深终端中，我们靠字形区分空单元与活跃单元（宽度匹配的实心/空心对）。
// 在 truecolor 终端中，网格使用单一字形并由颜色承载强度（GitHub 风格），
// 这能让网格保持完美对齐且没有纹理噪点。
const EMPTY_CELL_GLYPH: &str = "□";
const ACTIVE_CELL_GLYPH: &str = "■";
const BAR_CELL_GLYPH: &str = "█";

/// 存储 token 活动单元格的终端专属样式与字形策略。
pub(super) struct TokenActivityPalette {
    styles: [Style; 5],
    bar_style: Style,
    /// 当终端支持 truecolor 渐变时为 true，此时网格可纯靠颜色编码强度，
    /// 每个单元用单一字形渲染。在低色深终端上为 false，
    /// 此时回退到实心/空心字形对，让空单元与活跃单元仍可区分。
    uses_color: bool,
}

impl TokenActivityPalette {
    pub(super) fn current() -> Self {
        Self::from_parts(
            default_fg(),
            default_bg(),
            stdout_color_level(),
            theme_activity_style(),
        )
    }

    fn from_parts(
        default_fg: Option<(u8, u8, u8)>,
        default_bg: Option<(u8, u8, u8)>,
        color_level: StdoutColorLevel,
        active_style: Style,
    ) -> Self {
        let fallback_palette = || Self::fallback(active_style);
        let (Some(fg), Some(bg), Some(anchor)) =
            (default_fg, default_bg, activity_anchor_rgb(active_style))
        else {
            return fallback_palette();
        };
        if matches!(
            color_level,
            StdoutColorLevel::Ansi16 | StdoutColorLevel::Unknown
        ) {
            return fallback_palette();
        }

        let empty_alpha = if crate::tui_core::color::is_light(bg) {
            0.18
        } else {
            0.14
        };
        let alphas = [empty_alpha, 0.22, 0.42, 0.68, 1.00];
        let styles = std::array::from_fn(|index| {
            let color = if index == 0 {
                blend(fg, bg, alphas[index])
            } else {
                blend(anchor, bg, alphas[index])
            };
            Style::default().fg(best_color_for_level(color, color_level))
        });
        let bar_style = Style::default().fg(best_color_for_level(
            blend(anchor, bg, /*alpha*/ 0.78),
            color_level,
        ));
        Self {
            styles,
            bar_style,
            uses_color: true,
        }
    }

    fn fallback(active_style: Style) -> Self {
        let empty_style = Style::default().dim();
        Self {
            styles: [
                empty_style,
                active_style,
                active_style,
                active_style,
                active_style,
            ],
            bar_style: active_style,
            uses_color: false,
        }
    }

    pub(super) fn for_level(&self, level: usize) -> Style {
        self.styles[level.min(/*other*/ 4)]
    }

    pub(super) fn for_bar_level(&self, level: usize) -> Style {
        if level == 0 {
            self.for_level(/*level*/ 0)
        } else {
            self.bar_style
        }
    }

    /// `level` 单元的 glyph。Daily truecolor 用相同的方块字形渲染每个可见单元，
    /// 由颜色承载强度；在低色深终端中，空单元用空心字形，使其在
    /// 没有颜色渐变时依然可见。柱状视图对已填充高度用整块、空高度用空格，
    /// 使轮廓呈柱状图效果。
    pub(super) fn glyph(&self, view: TokenActivityView, level: usize) -> &'static str {
        if view != TokenActivityView::Daily {
            return if level == 0 { " " } else { BAR_CELL_GLYPH };
        }
        if self.uses_color || level > 0 {
            ACTIVE_CELL_GLYPH
        } else {
            EMPTY_CELL_GLYPH
        }
    }
}

fn theme_activity_style() -> Style {
    foreground_style_for_scopes(&["entity.name.type", "support.type", "variable"])
        .unwrap_or_else(accent_style)
        .bold()
}

fn activity_anchor_rgb(style: Style) -> Option<(u8, u8, u8)> {
    match style.fg? {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod tests;
