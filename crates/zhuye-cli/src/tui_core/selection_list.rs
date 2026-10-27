//! 选择项渲染辅助函数。
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;

/// 将单个选择项渲染为一行。
pub(crate) fn selection_option_row(
    _index: usize,
    label: impl std::fmt::Display,
    selected: bool,
) -> Line<'static> {
    let style = if selected {
        Style::default().bold()
    } else {
        Style::default()
    };
    Line::from(Span::styled(label.to_string(), style))
}

/// 使用暗淡样式渲染选择项。
pub(crate) fn selection_option_row_with_dim(label: &str, selected: bool, dim: bool) -> Line<'_> {
    let mut style = Style::default();
    if selected {
        style = style.bold();
    }
    if dim {
        style = style.dim();
    }
    Line::from(Span::styled(label.to_string(), style))
}
