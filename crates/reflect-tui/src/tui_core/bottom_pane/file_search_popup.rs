use std::path::PathBuf;

use crate::file_search::FileMatch;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::WidgetRef;

use crate::tui_core::render::Insets;
use crate::tui_core::render::RectExt;

use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::GenericDisplayRow;
use super::selection_popup_common::render_rows;

/// 文件搜索弹出框的视觉状态。
pub(crate) struct FileSearchPopup {
    /// 与当前显示的 `matches` 对应的查询。
    display_query: String,
    /// 用户输入的最新查询。当搜索仍在进行中时，
    /// 可能与 `display_query` 不同。
    pending_query: String,
    /// 当为 `true` 时，表示仍在等待 `pending_query` 的结果。
    waiting: bool,
    /// 缓存的匹配项；路径相对于搜索目录。
    matches: Vec<FileMatch>,
    /// 共享的选中/滚动状态。
    state: ScrollState,
}

impl FileSearchPopup {
    pub(crate) fn new() -> Self {
        Self {
            display_query: String::new(),
            pending_query: String::new(),
            waiting: true,
            matches: Vec::new(),
            state: ScrollState::new(),
        }
    }

    /// 更新查询并将状态重置为*等待中*。
    pub(crate) fn set_query(&mut self, query: &str) {
        if query == self.pending_query {
            return;
        }

        self.pending_query.clear();
        self.pending_query.push_str(query);

        self.waiting = true; // 等待新结果
    }

    /// 将弹出框置为空查询（仅有 "@"）时使用的“空闲”状态。
    /// 在用户输入更多字符之前，显示提示信息而非匹配项。
    pub(crate) fn set_empty_prompt(&mut self) {
        self.display_query.clear();
        self.pending_query.clear();
        self.waiting = false;
        self.matches.clear();
        // 显示空提示时重置选中/滚动状态。
        self.state.reset();
    }

    /// 当 `FileSearchResult` 到达时替换匹配项。
    /// 替换匹配项。仅当 `query` 与 `pending_query` 匹配时才应用。
    pub(crate) fn set_matches(&mut self, query: &str, matches: Vec<FileMatch>) {
        if query != self.pending_query {
            return; // 已过期
        }

        self.display_query = query.to_string();
        self.matches = matches.into_iter().take(MAX_POPUP_ROWS).collect();
        self.waiting = false;
        let len = self.matches.len();
        self.state.clamp_selection(len);
        self.state.ensure_visible(len, len.min(MAX_POPUP_ROWS));
    }

    /// 将选中光标上移。
    pub(crate) fn move_up(&mut self) {
        let len = self.matches.len();
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, len.min(MAX_POPUP_ROWS));
    }

    /// 将选中光标下移。
    pub(crate) fn move_down(&mut self) {
        let len = self.matches.len();
        self.state.move_down_wrap(len);
        self.state.ensure_visible(len, len.min(MAX_POPUP_ROWS));
    }

    pub(crate) fn selected_match(&self) -> Option<&PathBuf> {
        self.state
            .selected_idx
            .and_then(|idx| self.matches.get(idx))
            .map(|file_match| &file_match.path)
    }

    pub(crate) fn calculate_required_height(&self) -> u16 {
        // 行数取决于是否已有匹配项。若尚无匹配项
        // （例如初始搜索或查询无结果），保留一行
        // 使弹出框仍然可见。存在匹配项时，无论等待标志如何，
        // 最多显示 MAX_RESULTS 行，使列表在更新的搜索
        // 进行期间保持稳定。

        self.matches.len().clamp(1, MAX_POPUP_ROWS) as u16
    }
}

impl WidgetRef for &FileSearchPopup {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        // 将匹配项转换为 GenericDisplayRow，并在 UI 边界处将索引转换为 usize。
        let rows_all: Vec<GenericDisplayRow> = if self.matches.is_empty() {
            Vec::new()
        } else {
            self.matches
                .iter()
                .map(|m| GenericDisplayRow {
                    name: m.path.to_string_lossy().to_string(),
                    name_prefix_spans: Vec::new(),
                    match_indices: m
                        .indices
                        .as_ref()
                        .map(|v| v.iter().map(|&i| i as usize).collect()),
                    display_shortcut: None,
                    description: None,
                    category_tag: None,
                    wrap_indent: None,
                    is_disabled: false,
                    disabled_reason: None,
                })
                .collect()
        };

        let empty_message = if self.waiting {
            "loading..."
        } else {
            "no matches"
        };

        render_rows(
            area.inset(Insets::tlbr(
                /*top*/ 0, /*left*/ 2, /*bottom*/ 0, /*right*/ 0,
            )),
            buf,
            &rows_all,
            &self.state,
            MAX_POPUP_ROWS,
            empty_message,
        );
    }
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests {
    use super::*;
    use crate::file_search::MatchType;
    use pretty_assertions::assert_eq;

    fn file_match(index: usize) -> FileMatch {
        FileMatch {
            score: index as u32,
            path: PathBuf::from(format!("src/file_{index:02}.rs")),
            match_type: MatchType::File,
            root: PathBuf::from("/tmp/repo"),
            indices: None,
        }
    }

    #[test]
    fn set_matches_keeps_only_the_first_page_of_results() {
        let mut popup = FileSearchPopup::new();
        popup.set_query("file");
        popup.set_matches("file", (0..(MAX_POPUP_ROWS + 2)).map(file_match).collect());

        assert_eq!(
            popup.matches,
            (0..MAX_POPUP_ROWS).map(file_match).collect::<Vec<_>>()
        );
        assert_eq!(popup.calculate_required_height(), MAX_POPUP_ROWS as u16);
    }
}
