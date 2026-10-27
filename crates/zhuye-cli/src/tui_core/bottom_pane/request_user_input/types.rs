//! 请求用户输入覆盖层的草稿/答案/页脚提示类型。从 mod.rs 抽出。

use super::*;

#[derive(Default, Clone, PartialEq)]
pub(super) struct ComposerDraft {
    pub(super) text: String,
    pub(super) text_elements: Vec<TextElement>,
    pub(super) local_image_paths: Vec<PathBuf>,
    pub(super) pending_pastes: Vec<(String, String)>,
}

impl ComposerDraft {
    pub(super) fn text_with_pending(&self) -> String {
        if self.pending_pastes.is_empty() {
            return self.text.clone();
        }
        debug_assert!(
            !self.text_elements.is_empty(),
            "pending pastes should always have matching text elements"
        );
        let (expanded, _) = ChatComposer::expand_pending_pastes(
            &self.text,
            self.text_elements.clone(),
            &self.pending_pastes,
        );
        expanded
    }
}

pub(super) struct AnswerState {
    // 用于选项导航/高亮的可滚动光标状态。
    pub(super) options_state: ScrollState,
    // 每个问题各自的备注草稿。
    pub(super) draft: ComposerDraft,
    // 该问题的答案是否已被显式提交。
    pub(super) answer_committed: bool,
    // 该问题的备注 UI 是否已被显式打开。
    pub(super) notes_visible: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct FooterTip {
    pub(crate) text: String,
    pub(crate) highlight: bool,
}

impl FooterTip {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            highlight: false,
        }
    }

    pub(crate) fn highlighted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            highlight: true,
        }
    }
}
