//! 历史条目类型与方法。从 chat_composer_history/mod.rs 抽出。

use std::path::PathBuf;

use crate::protocol_compat::user_input::TextElement;
use crate::tui_core::bottom_pane::MentionBinding;
use crate::tui_core::mention_codec::decode_history_mentions_with_at_mentions;

/// 一条可用于恢复草稿状态的编辑器历史条目。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistoryEntry {
    /// 历史中存储的原始文本（可能包含占位字符串）。
    pub(crate) text: String,
    /// `text` 内部占位符对应的文本元素范围。
    pub(crate) text_elements: Vec<TextElement>,
    /// 与 `text_elements` 一起捕获的本地图片路径。
    pub(crate) local_image_paths: Vec<PathBuf>,
    /// 随此草稿一并恢复的远程图片 URL。
    pub(crate) remote_image_urls: Vec<String>,
    /// `text` 内部工具/应用/技能引用的提及绑定（mention binding）。
    pub(crate) mention_bindings: Vec<MentionBinding>,
    /// 用于恢复大型粘贴内容的占位符与载荷（payload）配对。
    pub(crate) pending_pastes: Vec<(String, String)>,
}

impl HistoryEntry {
    /// 创建一条纯文本历史条目，并解码持久化的提及绑定。
    ///
    /// 持久化历史不存储附件载荷或文本元素元数据，因此此构造函数有意将这些字段留空。会话内的本地提交
    /// 应使用编辑器构建的完整 `HistoryEntry` 值来记录；若对本地图片或粘贴提交使用 `new`，回忆时会丢失
    /// 占位符归属关系。
    pub(crate) fn new(text: String) -> Self {
        Self::new_with_at_mentions(text, /*at_mentions_enabled*/ true)
    }

    pub(crate) fn new_with_at_mentions(text: String, at_mentions_enabled: bool) -> Self {
        let decoded = decode_history_mentions_with_at_mentions(&text, at_mentions_enabled);
        Self {
            text: decoded.text,
            text_elements: Vec::new(),
            local_image_paths: Vec::new(),
            remote_image_urls: Vec::new(),
            mention_bindings: decoded
                .mentions
                .into_iter()
                .map(|mention| MentionBinding {
                    sigil: mention.sigil,
                    mention: mention.mention,
                    path: mention.path,
                })
                .collect(),
            pending_pastes: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_pending(
        text: String,
        text_elements: Vec<TextElement>,
        local_image_paths: Vec<PathBuf>,
        pending_pastes: Vec<(String, String)>,
    ) -> Self {
        Self {
            text,
            text_elements,
            local_image_paths,
            remote_image_urls: Vec::new(),
            mention_bindings: Vec::new(),
            pending_pastes,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_pending_and_remote(
        text: String,
        text_elements: Vec<TextElement>,
        local_image_paths: Vec<PathBuf>,
        pending_pastes: Vec<(String, String)>,
        remote_image_urls: Vec<String>,
    ) -> Self {
        Self {
            text,
            text_elements,
            local_image_paths,
            remote_image_urls,
            mention_bindings: Vec::new(),
            pending_pastes,
        }
    }
}
