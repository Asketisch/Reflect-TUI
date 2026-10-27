//! 老 TUI 消息标记字形改写层。
//!
//! Reflect 单元用 `› `(user)/`• `(agent) 前缀;用户要求「尽量显示老 TUI 样式」,
//! 这里把渲染后的 `Line` 首位 span 前缀改写成老 TUI 的彩色字形:
//! - user    `› ` → `▶ `(brand 绿)
//! - agent   `• ` → `● `(brand 绿;完整回合)
//!
//! 这是对 Reflect 单元 `display_lines` 输出的纯 `Line`/`Span` 变换,**不**修改
//! Reflect 单元源码(保持上游逐行对应便于 re-sync)。
//!
//! 注意:只改「消息正文首行」的前缀——Reflect 单元会在正文前后各插一条空行,
//! 空行没有前缀 span,不能动。判定方式:找第一条首个 span 内容恰好是
//! `› `/`• ` 的行。

use crate::adapter::UiHistoryItem;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// 老 TUI 品牌色(对齐 `palette::brand()` Dark 主题 = Cyan)。
const BRAND: Color = Color::Cyan;

/// 改写一条历史项渲染出的行:把 Reflect 前缀换成老 TUI 彩色字形。
///
/// 仅处理 `User` / `Agent`;其余变体(Thinking/ToolCall/Separator/...)的
/// 前缀在 [`crate::history_render::cells`] 里已直接用老字形,无需再改。
pub fn apply_legacy_markers(item: &UiHistoryItem, lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    match item {
        UiHistoryItem::User(_) => rewrite_prefix(lines, "› ", "▶ ", BRAND, true),
        UiHistoryItem::Agent(_) => rewrite_prefix(lines, "• ", "● ", BRAND, true),
        _ => lines,
    }
}

/// 在渲染行里找首个内容为 `from` 的前缀 span,换成 `to`(着色),只换第一个。
fn rewrite_prefix(
    lines: Vec<Line<'static>>,
    from: &str,
    to: &str,
    color: Color,
    bold: bool,
) -> Vec<Line<'static>> {
    let mut done = false;
    lines
        .into_iter()
        .map(|mut line| {
            if done || line.spans.is_empty() {
                return line;
            }
            if line.spans[0].content == from {
                let mut style = Style::default().fg(color);
                if bold {
                    style = style.add_modifier(Modifier::BOLD);
                }
                line.spans[0] = Span::styled(to.to_string(), style);
                done = true;
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    fn user_lines() -> Vec<Line<'static>> {
        // 模拟 Reflect UserHistoryCell 输出:空行 / `› hello` / 空行。
        vec![
            Line::from(""),
            Line::from(vec![Span::raw("› "), Span::raw("hello")]),
            Line::from(""),
        ]
    }

    fn agent_lines() -> Vec<Line<'static>> {
        vec![
            Line::from(""),
            Line::from(vec![Span::raw("• "), Span::raw("world")]),
            Line::from(""),
        ]
    }

    #[test]
    fn user_prefix_becomes_arrow() {
        let out = apply_legacy_markers(&UiHistoryItem::User("hello".into()), user_lines());
        assert_eq!(out[1].spans[0].content, "▶ ");
        assert_eq!(out[1].spans[1].content, "hello");
    }

    #[test]
    fn agent_prefix_becomes_dot() {
        let out = apply_legacy_markers(&UiHistoryItem::Agent("world".into()), agent_lines());
        assert_eq!(out[1].spans[0].content, "● ");
    }

    #[test]
    fn only_first_message_line_is_rewritten() {
        // 多行消息:只有首行前缀被改,续行(无 `› ` 前缀)不动。
        let lines = vec![
            Line::from(vec![Span::raw("› "), Span::raw("first")]),
            Line::from(vec![Span::raw("  "), Span::raw("second")]),
        ];
        let out = apply_legacy_markers(&UiHistoryItem::User("m".into()), lines);
        assert_eq!(out[0].spans[0].content, "▶ ");
        assert_eq!(out[1].spans[0].content, "  ", "续行前缀保持");
    }

    #[test]
    fn other_variants_pass_through() {
        let lines = vec![Line::from(vec![Span::raw("… "), Span::raw("hmm")])];
        let out = apply_legacy_markers(&UiHistoryItem::Thinking("hmm".into()), lines.clone());
        assert_eq!(out[0].spans[0].content, "… ");
    }
}
