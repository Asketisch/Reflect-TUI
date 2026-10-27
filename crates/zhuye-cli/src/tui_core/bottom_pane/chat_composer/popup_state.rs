//! 聊天编辑器的弹窗生命周期状态。
//! 追踪当前唯一的活动弹窗,以及用于与之同步的关闭/查询状态。

use crate::tui_core::bottom_pane::command_popup::CommandPopup;
use crate::tui_core::bottom_pane::file_search_popup::FileSearchPopup;
use crate::tui_core::bottom_pane::mentions_v2::MentionV2Popup;
use crate::tui_core::bottom_pane::skill_popup::SkillPopup;
use crate::tui_core::bottom_pane::textarea::TextArea;
use std::ops::Range;

/// 一个其自动补全弹窗应保持隐藏的 token 出现项。
pub(super) struct DismissedToken {
    /// 该 token 的弹窗查询文本,不含前导 sigil 符号。
    query: String,
    /// 弹窗被关闭时捕获的完整 token 文本(包含其 sigil 符号)。
    token: String,
    /// 关闭时刻草稿中相同 token 字符串之间的零基序号。
    occurrence: usize,
}

impl DismissedToken {
    /// 捕获位于 `range` 处 token 的稳定标识。
    pub(super) fn new(textarea: &TextArea, range: Range<usize>, query: String) -> Self {
        let text = textarea.text();
        let token = text[range.clone()].to_string();
        let occurrence = complete_token_occurrences_before(textarea, &token, range.start);
        Self {
            query,
            token,
            occurrence,
        }
    }

    /// 返回 `range` 在当前草稿中是否标识同一个 token 出现项。
    ///
    /// 纯偏移编辑可能使字节偏移发生移动,而 token 文本及其序号能够区分后续相同的出现项。
    pub(super) fn matches(&self, textarea: &TextArea, range: &Range<usize>, query: &str) -> bool {
        let text = textarea.text();
        if self.query != query || text.get(range.clone()) != Some(self.token.as_str()) {
            return false;
        }
        complete_token_occurrences_before(textarea, &self.token, range.start) == self.occurrence
    }
}

/// 以单次有序遍历统计 `before` 之前完整的可编辑 token 出现次数。
///
/// 出现项由空白符与原子元素边界界定;裸露的嵌套 sigil 不作数。这样既能让关闭标识与补全
/// 解析器保持一致,又无需为每次匹配重新扫描所有元素。
fn complete_token_occurrences_before(textarea: &TextArea, token: &str, before: usize) -> usize {
    let text = textarea.text();
    let mut elements_ending_before_matches = textarea.text_element_ranges().peekable();
    let mut elements_starting_after_matches = textarea.text_element_ranges().peekable();
    text[..before]
        .match_indices(token)
        .filter(|(start, _)| {
            let end = start + token.len();
            while elements_ending_before_matches
                .peek()
                .is_some_and(|element| element.end < *start)
            {
                elements_ending_before_matches.next();
            }
            while elements_starting_after_matches
                .peek()
                .is_some_and(|element| element.start < end)
            {
                elements_starting_after_matches.next();
            }
            let ends_at_boundary = text[end..].chars().next().is_none_or(char::is_whitespace)
                || elements_starting_after_matches
                    .peek()
                    .is_some_and(|element| element.start == end);
            let starts_at_boundary = *start == 0
                || text[..*start]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
                || elements_ending_before_matches
                    .peek()
                    .is_some_and(|element| element.end == *start);
            starts_at_boundary && ends_at_boundary
        })
        .count()
}

#[derive(Default)]
pub(super) struct PopupState {
    pub(super) active: ActivePopup,
    pub(super) dismissed_command_token: Option<String>,
    pub(super) dismissed_file_token: Option<DismissedToken>,
    pub(super) current_file_query: Option<String>,
    pub(super) dismissed_mention_token: Option<DismissedToken>,
}

impl PopupState {
    pub(super) fn active(&self) -> bool {
        !matches!(self.active, ActivePopup::None)
    }
}

/// 弹窗状态——同一时刻最多只有一个弹窗可见。
#[derive(Default)]
pub(super) enum ActivePopup {
    #[default]
    None,
    Command(CommandPopup),
    File(FileSearchPopup),
    Skill(SkillPopup),
    MentionV2(MentionV2Popup),
}

#[cfg(test)]
#[path = "popup_state_tests.rs"]
mod tests;
