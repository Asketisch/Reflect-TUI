/// 用于垂直列表菜单的通用滚动/选中状态。
///
/// 封装可选列表的通用行为，支持：
/// - 可选的选中项（列表为空时为 None）
/// - 上/下移动时循环导航
/// - 维护滚动窗口（`scroll_top`）以保持选中行可见
///
/// 调用方负责持有过滤后的行数和可见窗口大小。每个
/// 变更方法都接收这些值而不是在此处缓存，这样列表
/// 视图就能应用过滤、分页或密度调整，而无需此辅助类型
/// 了解其数据模型。过滤后若传入过期的长度，会
/// 导致选中项指向错误的行，因此调用方在更改其可见行集后
/// 应立即通过此类型进行钳制或移动。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ScrollState {
    pub selected_idx: Option<usize>,
    pub scroll_top: usize,
}

impl ScrollState {
    pub fn new() -> Self {
        Self {
            selected_idx: None,
            scroll_top: 0,
        }
    }

    /// 重置选中项和滚动位置。
    pub fn reset(&mut self) {
        self.selected_idx = None;
        self.scroll_top = 0;
    }

    /// 将选中项钳制到 [0, len-1] 范围内，列表为空时置为 None。
    pub fn clamp_selection(&mut self, len: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        self.selected_idx = Some(self.selected_idx.unwrap_or(0).min(len - 1));
    }

    /// 将选中项上移一行，必要时循环到底部。
    pub fn move_up_wrap(&mut self, len: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        self.selected_idx = Some(match self.selected_idx {
            Some(idx) if idx > 0 => idx - 1,
            Some(_) => len - 1,
            None => 0,
        });
    }

    /// 将选中项下移一行，必要时循环到顶部。
    pub fn move_down_wrap(&mut self, len: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        self.selected_idx = Some(match self.selected_idx {
            Some(idx) if idx + 1 < len => idx + 1,
            _ => 0,
        });
    }

    /// 将选中项上移一个可见页面，在第一行处钳制。
    ///
    /// 翻页移动有意不做循环。它仿照终端列表的
    /// 行为：连续的向上/向下翻页会收敛到最近的边缘，
    /// 同时保持选中行可见。
    pub fn page_up_clamped(&mut self, len: usize, visible_rows: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        let step = visible_rows.max(1);
        let current = self.selected_idx.unwrap_or(0).min(len - 1);
        self.selected_idx = Some(current.saturating_sub(step));
        self.ensure_visible(len, visible_rows);
    }

    /// 将选中项下移一个可见页面，在最后一行处钳制。
    ///
    /// 翻页移动有意不做循环。它仿照终端列表的
    /// 行为：连续的向上/向下翻页会收敛到最近的边缘，
    /// 同时保持选中行可见。
    pub fn page_down_clamped(&mut self, len: usize, visible_rows: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        let step = visible_rows.max(1);
        let current = self.selected_idx.unwrap_or(0).min(len - 1);
        self.selected_idx = Some(current.saturating_add(step).min(len - 1));
        self.ensure_visible(len, visible_rows);
    }

    /// 将选中项跳转到第一行。
    pub fn jump_top(&mut self, len: usize, visible_rows: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        self.selected_idx = Some(0);
        self.ensure_visible(len, visible_rows);
    }

    /// 将选中项跳转到最后一行。
    pub fn jump_bottom(&mut self, len: usize, visible_rows: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        self.selected_idx = Some(len - 1);
        self.ensure_visible(len, visible_rows);
    }

    fn clear_if_empty(&mut self, len: usize) -> bool {
        if len != 0 {
            return false;
        }
        self.selected_idx = None;
        self.scroll_top = 0;
        true
    }

    /// 调整 `scroll_top`，使当前 `selected_idx` 在
    /// `visible_rows` 大小的窗口内保持可见。
    pub fn ensure_visible(&mut self, len: usize, visible_rows: usize) {
        if len == 0 || visible_rows == 0 {
            self.scroll_top = 0;
            return;
        }
        if let Some(sel) = self.selected_idx {
            if sel < self.scroll_top {
                self.scroll_top = sel;
            } else {
                let bottom = self.scroll_top + visible_rows - 1;
                if sel > bottom {
                    self.scroll_top = sel + 1 - visible_rows;
                }
            }
        } else {
            self.scroll_top = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScrollState;

    #[test]
    fn wrap_navigation_and_visibility() {
        let mut s = ScrollState::new();
        let len = 10;
        let vis = 5;

        s.clamp_selection(len);
        assert_eq!(s.selected_idx, Some(0));
        s.ensure_visible(len, vis);
        assert_eq!(s.scroll_top, 0);

        s.move_up_wrap(len);
        s.ensure_visible(len, vis);
        assert_eq!(s.selected_idx, Some(len - 1));
        match s.selected_idx {
            Some(sel) => assert!(s.scroll_top <= sel),
            None => panic!("expected Some(selected_idx) after wrap"),
        }

        s.move_down_wrap(len);
        s.ensure_visible(len, vis);
        assert_eq!(s.selected_idx, Some(0));
        assert_eq!(s.scroll_top, 0);
    }

    #[test]
    fn page_and_jump_navigation_clamps() {
        let mut s = ScrollState::new();
        let len = 10;
        let vis = 4;

        s.clamp_selection(len);
        s.page_down_clamped(len, vis);
        assert_eq!(s.selected_idx, Some(4));
        assert_eq!(s.scroll_top, 1);

        s.page_down_clamped(len, vis);
        assert_eq!(s.selected_idx, Some(8));
        assert_eq!(s.scroll_top, 5);

        s.page_down_clamped(len, vis);
        assert_eq!(s.selected_idx, Some(9));
        assert_eq!(s.scroll_top, 6);

        s.page_up_clamped(len, vis);
        assert_eq!(s.selected_idx, Some(5));
        assert_eq!(s.scroll_top, 5);

        s.jump_top(len, vis);
        assert_eq!(s.selected_idx, Some(0));
        assert_eq!(s.scroll_top, 0);

        s.jump_bottom(len, vis);
        assert_eq!(s.selected_idx, Some(9));
        assert_eq!(s.scroll_top, 6);
    }
}
