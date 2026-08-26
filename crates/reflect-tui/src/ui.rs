//! 最小化的稳定 UI 辅助函数。渲染刻意保留在 `tui` 中，而本
//! 模块为未来 Reflect 风格的布局原语提供具名接缝。

use ratatui::layout::Rect;

pub fn content_area(area: Rect) -> Rect {
    area
}
