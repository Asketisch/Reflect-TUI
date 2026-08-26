//! Reflect 对齐的视口扩缩与清屏语义。
//!
//! 视口变化时的处理语义：
//! 1. 按 desired height 调整底部锚定区域；
//! 2. 若底部超出屏幕则 `scroll_region_up` 腾出空间；
//! 3. 区域变化时 `clear_for_viewport_change`，避免旧 live 像素残留成「双份」历史。
//!
//! Reflect 额外处理 live 消失时的 shrink：
//! 1. clear + 删掉 `[old_top, bottom_anchor)` 空洞（避免回合间空行）；
//! 2. 再 `scroll_region_down(0..screen_h, gap)` 把历史 + composer 推回屏底
//!    （保持输入框贴底，避免后续 `insert_history` 因未贴底而整块下推造成「整屏上滚」）。

use crate::tui_core::custom_terminal::Terminal as ReflectTerminal;
use ratatui::backend::Backend;
use ratatui::layout::{Position, Rect};
use std::io::{self, Write};

fn rect_origin(area: Rect) -> Position {
    Position {
        x: area.x,
        y: area.y,
    }
}

/// 视口区域变化时，从旧/新区域顶部起清到屏幕末尾，防止残留 cell 进入 scrollback 可见区。
pub fn clear_for_viewport_change<B>(
    terminal: &mut ReflectTerminal<B>,
    new_area: Rect,
) -> io::Result<()>
where
    B: Backend + Write,
{
    let clear_position = if terminal.viewport_area.is_empty() {
        rect_origin(new_area)
    } else {
        rect_origin(terminal.viewport_area)
    };
    terminal.clear_after_position(clear_position)
}

/// 删除 `[old_top, …)` 空隙：从 `old_top` 起向上滚 `gap` 行，composer 上移到 `old_top`。
fn delete_viewport_gap_rows<B>(
    terminal: &mut ReflectTerminal<B>,
    old_top: u16,
    gap: u16,
    screen_height: u16,
) -> io::Result<()>
where
    B: Backend + Write,
{
    if gap == 0 || screen_height == 0 || old_top >= screen_height {
        return Ok(());
    }
    let _ = terminal
        .backend_mut()
        .scroll_region_up(old_top..screen_height, gap);
    Ok(())
}

/// 删行后内容在 `old_top`、下方留空；整屏下推 `gap` 行以贴回屏底。
///
/// 必须滚 `0..screen_height`：若只滚到 `old_top+height`，下方空洞不在区内，
/// 下推会把历史/composer 从区底挤丢，空洞仍留在屏底。
fn reanchor_content_to_bottom<B>(
    terminal: &mut ReflectTerminal<B>,
    gap: u16,
    screen_height: u16,
) -> io::Result<()>
where
    B: Backend + Write,
{
    if gap == 0 || screen_height == 0 {
        return Ok(());
    }
    // 整屏下推：屏底原空洞滚出，历史 + composer 下移贴底，顶部补等量空行。
    let _ = terminal
        .backend_mut()
        .scroll_region_down(0..screen_height, gap);
    Ok(())
}

/// 按 Reflect `Tui::draw` 语义准备底部锚定 viewport，返回最终区域。
///
/// `desired_height` 为 live + composer + status 的合计行数（已 clamp 到屏幕高度）。
/// shrink 时：clear → 删空洞 → 下推贴底。
pub fn prepare_bottom_viewport<B>(
    terminal: &mut ReflectTerminal<B>,
    screen_width: u16,
    screen_height: u16,
    desired_height: u16,
) -> io::Result<Rect>
where
    B: Backend + Write,
{
    let height = desired_height.min(screen_height).max(1);
    let old_area = terminal.viewport_area;
    let mut area = old_area;
    if area.width == 0 && area.height == 0 {
        // 首帧：从屏幕底部起算。
        area = Rect::new(
            0,
            screen_height.saturating_sub(height),
            screen_width,
            height,
        );
    } else {
        area.height = height;
        area.width = screen_width;
        // 保持底部锚定：y = screen_height - height（若尚未越界）。
        if area.bottom() > screen_height || area.y + height != screen_height {
            // 视口增高且超出屏幕底：只滚视口上方区域腾地方（与 Reflect 一致）。
            if area.bottom() > screen_height {
                let scroll_by = area.bottom() - screen_height;
                let top = area.top();
                if top > 0 && scroll_by > 0 {
                    let _ = terminal.backend_mut().scroll_region_up(0..top, scroll_by);
                }
            }
            area.y = screen_height.saturating_sub(height);
        }
    }

    if area != terminal.viewport_area {
        let shrinking = !old_area.is_empty() && area.y > old_area.y;
        let old_top = old_area.y;
        let gap = area.y.saturating_sub(old_top);
        let bottom_y = area.y; // screen_height - height
        clear_for_viewport_change(terminal, area)?;
        if shrinking && gap > 0 {
            // 1) 删掉 live 空洞，内容上移到 old_top
            delete_viewport_gap_rows(terminal, old_top, gap, screen_height)?;
            // 2) 整屏下推 gap，贴回屏底（避免中部视口触发 insert_history 大滚动）
            reanchor_content_to_bottom(terminal, gap, screen_height)?;
            area.y = bottom_y;
        }
        terminal.set_viewport_area(area);
    } else {
        terminal.set_viewport_area(area);
    }
    Ok(area)
}

/// 计算 live 区所需行数：按 markdown 渲染结果与可用宽度折行估算。
///
/// 输入应已做过 `sanitize_agent_display_text`（由调用方保证），避免把
/// `FINAL ANSWER:` 行算进高度。
pub fn live_region_height(live: &str, width: u16) -> u16 {
    if live.is_empty() {
        return 0;
    }
    let text = crate::tui_core::markdown_render::render_markdown_text(live);
    let wrap_w = usize::from(width.max(1));
    let mut rows: u16 = 0;
    for line in &text.lines {
        let w = line.width().max(1);
        let line_rows = w.div_ceil(wrap_w);
        rows = rows.saturating_add(u16::try_from(line_rows).unwrap_or(u16::MAX).max(1));
    }
    rows.max(1)
}

#[cfg(test)]
mod viewport_tests {
    use super::*;
    use crate::tui_core::custom_terminal::Terminal as ReflectTerminal;
    use crate::tui_core::test_backend::VT100Backend;

    #[test]
    fn shrink_keeps_bottom_anchor_without_gap() {
        let width: u16 = 40;
        let height: u16 = 16;
        let backend = VT100Backend::new(width, height);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();

        // 高 live 视口：y=6, h=10。
        let tall = Rect::new(0, height - 10, width, 10);
        terminal.set_viewport_area(tall);

        // 收缩到 3 行：删空洞后再贴底。
        let desired = 3u16;
        let area = prepare_bottom_viewport(&mut terminal, width, height, desired).unwrap();
        assert_eq!(area.height, desired);
        assert_eq!(
            area.y,
            height - desired,
            "viewport must remain bottom-anchored after shrink"
        );
        assert_eq!(terminal.viewport_area.y, height - desired);
        assert_eq!(terminal.viewport_area.bottom(), height);
    }
}
