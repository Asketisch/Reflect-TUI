//! 聊天 widget 的用户消息模型与辅助函数。
//!
//! app 服务端把用户输入保留为结构化块，而聊天历史渲染为单行提示。
//! 本模块负责草稿/消息数据模型、合并/重映射行为、显示投影，
//! 以及用于抑制待转向重复行的小型比较键。

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::ops::Deref;
use std::path::PathBuf;

use crate::app_server_protocol::TextElement as AppServerTextElement;
use crate::app_server_protocol::UserInput;
use crate::protocol_compat::config_types::CollaborationMode;
use crate::protocol_compat::config_types::CollaborationModeMask;
use crate::protocol_compat::models::local_image_label_text;
use crate::protocol_compat::user_input::ByteRange;
use crate::protocol_compat::user_input::TextElement;
use crate::tui_core::bottom_pane::LocalImageAttachment;
use crate::tui_core::bottom_pane::MentionBinding;
use crate::tui_core::bottom_pane::QueuedInputAction;
use crate::utils_plugins::mention_syntax::PLUGIN_TEXT_MENTION_SIGIL;
use crate::utils_plugins::mention_syntax::TOOL_MENTION_SIGIL;

use super::ChatWidget;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UserMessage {
    pub(crate) text: String,
    pub(crate) local_images: Vec<LocalImageAttachment>,
    /// 以 URL 表示的远程图片附件（例如 data URL），
    /// 由 app-server 客户端提供。
    pub(crate) remote_image_urls: Vec<String>,
    pub(crate) text_elements: Vec<TextElement>,
    pub(crate) mention_bindings: Vec<MentionBinding>,
}

impl std::fmt::Display for UserMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum UserMessageHistoryRecord {
    UserMessageText,
    Override(UserMessageHistoryOverride),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct UserMessageHistoryOverride {
    pub(super) text: String,
    pub(super) text_elements: Vec<TextElement>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShellEscapePolicy {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct QueuedUserMessage {
    pub(super) user_message: UserMessage,
    pub(super) action: QueuedInputAction,
    pub(super) pending_pastes: Vec<(String, String)>,
}

impl QueuedUserMessage {
    pub(super) fn new(user_message: UserMessage, action: QueuedInputAction) -> Self {
        Self {
            user_message,
            action,
            pending_pastes: Vec::new(),
        }
    }

    pub(super) fn into_user_message(self) -> UserMessage {
        self.user_message
    }
}

impl From<UserMessage> for QueuedUserMessage {
    fn from(user_message: UserMessage) -> Self {
        Self::new(user_message, QueuedInputAction::Plain)
    }
}

impl Deref for QueuedUserMessage {
    type Target = UserMessage;

    fn deref(&self) -> &Self::Target {
        &self.user_message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueueDrain {
    Continue,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct ThreadComposerState {
    pub(super) text: String,
    pub(super) local_images: Vec<LocalImageAttachment>,
    pub(super) remote_image_urls: Vec<String>,
    pub(super) text_elements: Vec<TextElement>,
    pub(super) mention_bindings: Vec<MentionBinding>,
    pub(super) pending_pastes: Vec<(String, String)>,
}

impl ThreadComposerState {
    pub(super) fn has_content(&self) -> bool {
        !self.text.is_empty()
            || !self.local_images.is_empty()
            || !self.remote_image_urls.is_empty()
            || !self.text_elements.is_empty()
            || !self.mention_bindings.is_empty()
            || !self.pending_pastes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ThreadInputState {
    pub(super) composer: Option<ThreadComposerState>,
    pub(super) safety_buffering_prompt: Option<UserMessage>,
    pub(super) pending_steers: VecDeque<UserMessage>,
    pub(super) pending_steer_history_records: VecDeque<UserMessageHistoryRecord>,
    pub(super) pending_steer_compare_keys: VecDeque<PendingSteerCompareKey>,
    pub(super) rejected_steers_queue: VecDeque<UserMessage>,
    pub(super) rejected_steer_history_records: VecDeque<UserMessageHistoryRecord>,
    pub(super) queued_user_messages: VecDeque<QueuedUserMessage>,
    pub(super) queued_user_message_history_records: VecDeque<UserMessageHistoryRecord>,
    pub(super) user_turn_pending_start: bool,
    pub(super) submit_pending_steers_after_interrupt: bool,
    pub(super) current_collaboration_mode: CollaborationMode,
    pub(super) active_collaboration_mask: Option<CollaborationModeMask>,
    pub(super) task_running: bool,
    pub(super) agent_turn_running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ThreadInputStateRestoreMode {
    pub(crate) preserve_in_flight_turn: bool,
}

impl From<String> for UserMessage {
    fn from(text: String) -> Self {
        Self {
            text,
            local_images: Vec::new(),
            remote_image_urls: Vec::new(),
            // 纯文本转换没有 UI 元素范围。
            text_elements: Vec::new(),
            mention_bindings: Vec::new(),
        }
    }
}

impl From<&str> for UserMessage {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_string(),
            local_images: Vec::new(),
            remote_image_urls: Vec::new(),
            // 纯文本转换没有 UI 元素范围。
            text_elements: Vec::new(),
            mention_bindings: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(super) struct PendingSteer {
    pub(super) user_message: UserMessage,
    pub(super) history_record: UserMessageHistoryRecord,
    pub(super) compare_key: PendingSteerCompareKey,
}

pub(crate) fn create_initial_user_message(
    text: Option<String>,
    local_image_paths: Vec<PathBuf>,
    text_elements: Vec<TextElement>,
) -> Option<UserMessage> {
    let text = text.unwrap_or_default();
    if text.is_empty() && local_image_paths.is_empty() {
        None
    } else {
        let local_images = local_image_paths
            .into_iter()
            .enumerate()
            .map(|(idx, path)| LocalImageAttachment {
                placeholder: local_image_label_text(idx + 1),
                path,
            })
            .collect();
        Some(UserMessage {
            text,
            local_images,
            remote_image_urls: Vec::new(),
            text_elements,
            mention_bindings: Vec::new(),
        })
    }
}

fn append_text_with_rebased_elements(
    target_text: &mut String,
    target_text_elements: &mut Vec<TextElement>,
    text: &str,
    text_elements: impl IntoIterator<Item = TextElement>,
) {
    let offset = target_text.len();
    target_text.push_str(text);
    target_text_elements.extend(text_elements.into_iter().map(|mut element| {
        element.byte_range.start += offset;
        element.byte_range.end += offset;
        element
    }));
}

pub(super) fn app_server_text_elements(elements: &[TextElement]) -> Vec<AppServerTextElement> {
    elements.iter().cloned().map(Into::into).collect()
}

fn build_placeholder_mapping(
    local_images: Vec<LocalImageAttachment>,
    next_label: &mut usize,
) -> (HashMap<String, String>, Vec<LocalImageAttachment>) {
    let mut mapping: HashMap<String, String> = HashMap::new();
    let mut remapped_images = Vec::new();
    for attachment in local_images {
        let new_placeholder = local_image_label_text(*next_label);
        *next_label += 1;
        mapping.insert(attachment.placeholder.clone(), new_placeholder.clone());
        remapped_images.push(LocalImageAttachment {
            placeholder: new_placeholder,
            path: attachment.path,
        });
    }
    (mapping, remapped_images)
}

fn remap_placeholders_in_text(
    text: String,
    text_elements: Vec<TextElement>,
    mapping: &HashMap<String, String>,
) -> (String, Vec<TextElement>) {
    if mapping.is_empty() {
        return (text, text_elements);
    }

    let mut elements = text_elements;
    elements.sort_by_key(|elem| elem.byte_range.start);

    let mut cursor = 0usize;
    let mut rebuilt = String::new();
    let mut rebuilt_elements = Vec::new();
    for mut elem in elements {
        let start = elem.byte_range.start.min(text.len());
        let end = elem.byte_range.end.min(text.len());
        if let Some(segment) = text.get(cursor..start) {
            rebuilt.push_str(segment);
        }

        let original = text.get(start..end).unwrap_or("");
        let placeholder = elem.placeholder(&text);
        let replacement = placeholder
            .and_then(|ph| mapping.get(ph))
            .map(String::as_str)
            .unwrap_or(original);

        let elem_start = rebuilt.len();
        rebuilt.push_str(replacement);
        let elem_end = rebuilt.len();

        if let Some(remapped) = placeholder.and_then(|ph| mapping.get(ph)) {
            elem.set_placeholder(Some(remapped.clone()));
        }
        elem.byte_range = (elem_start..elem_end).into();
        rebuilt_elements.push(elem);
        cursor = end;
    }
    if let Some(segment) = text.get(cursor..) {
        rebuilt.push_str(segment);
    }

    (rebuilt, rebuilt_elements)
}

pub(super) fn remap_colliding_paste_placeholders(
    mut message: UserMessage,
    mut pending_pastes: Vec<(String, String)>,
    used: &mut HashSet<String>,
) -> (UserMessage, Vec<(String, String)>) {
    let mut mapping = HashMap::new();
    for (placeholder, text) in &mut pending_pastes {
        if used.insert(placeholder.clone()) {
            continue;
        }

        let base = format!("[Pasted Content {} chars]", text.chars().count());
        let mut suffix = 2;
        let replacement = loop {
            let candidate = format!("{base} #{suffix}");
            if used.insert(candidate.clone()) {
                break candidate;
            }
            suffix += 1;
        };
        mapping.insert(placeholder.clone(), replacement.clone());
        *placeholder = replacement;
    }
    (message.text, message.text_elements) =
        remap_placeholders_in_text(message.text, message.text_elements, &mapping);
    (message, pending_pastes)
}

// 合并多个排队草稿时（例如中断之后），每个草稿的附件
// 从 [Image #1] 开始编号。根据附件列表重新分配占位符标签，使
// 组合后的 local_image_paths 顺序与标签一致——即使占位符在文本中被移动
// 过（例如 [Image #2] 出现在 [Image #1] 之前）。对
// 历史覆盖项应用相同的重映射，使恢复的草稿与渲染的 transcript 条目一致。
fn remap_placeholders_for_message_and_history_record(
    message: UserMessage,
    history_record: UserMessageHistoryRecord,
    next_label: &mut usize,
) -> (UserMessage, UserMessageHistoryRecord) {
    let UserMessage {
        text,
        text_elements,
        local_images,
        remote_image_urls,
        mention_bindings,
    } = message;
    let (mapping, remapped_images) = build_placeholder_mapping(local_images, next_label);
    let (text, text_elements) = remap_placeholders_in_text(text, text_elements, &mapping);
    let history_record = match history_record {
        UserMessageHistoryRecord::Override(history) if !history.text.is_empty() => {
            let (text, text_elements) =
                remap_placeholders_in_text(history.text, history.text_elements, &mapping);
            UserMessageHistoryRecord::Override(UserMessageHistoryOverride {
                text,
                text_elements,
            })
        }
        record => record,
    };

    (
        UserMessage {
            text,
            local_images: remapped_images,
            remote_image_urls,
            text_elements,
            mention_bindings,
        },
        history_record,
    )
}

#[cfg(test)]
pub(super) fn remap_placeholders_for_message(
    message: UserMessage,
    next_label: &mut usize,
) -> UserMessage {
    remap_placeholders_for_message_and_history_record(
        message,
        UserMessageHistoryRecord::UserMessageText,
        next_label,
    )
    .0
}

fn remap_user_messages_with_history_records(
    messages: Vec<(UserMessage, UserMessageHistoryRecord)>,
) -> Vec<(UserMessage, UserMessageHistoryRecord)> {
    let total_remote_images = messages
        .iter()
        .map(|(message, _)| message.remote_image_urls.len())
        .sum::<usize>();
    let mut next_image_label = total_remote_images + 1;
    messages
        .into_iter()
        .map(|(message, history_record)| {
            remap_placeholders_for_message_and_history_record(
                message,
                history_record,
                &mut next_image_label,
            )
        })
        .collect()
}

pub(super) fn merge_user_messages(messages: Vec<UserMessage>) -> UserMessage {
    let messages = remap_user_messages_with_history_records(
        messages
            .into_iter()
            .map(|message| (message, UserMessageHistoryRecord::UserMessageText))
            .collect(),
    );
    merge_remapped_user_messages(messages.into_iter().map(|(message, _)| message))
}

fn merge_remapped_user_messages(messages: impl IntoIterator<Item = UserMessage>) -> UserMessage {
    let mut combined = UserMessage {
        text: String::new(),
        text_elements: Vec::new(),
        local_images: Vec::new(),
        remote_image_urls: Vec::new(),
        mention_bindings: Vec::new(),
    };

    for (idx, message) in messages.into_iter().enumerate() {
        if idx > 0 {
            combined.text.push('\n');
        }
        let UserMessage {
            text,
            text_elements,
            local_images,
            remote_image_urls,
            mention_bindings,
        } = message;
        append_text_with_rebased_elements(
            &mut combined.text,
            &mut combined.text_elements,
            &text,
            text_elements,
        );
        combined.local_images.extend(local_images);
        combined.remote_image_urls.extend(remote_image_urls);
        combined.mention_bindings.extend(mention_bindings);
    }

    combined
}

pub(super) fn user_message_for_restore(
    message: UserMessage,
    history_record: &UserMessageHistoryRecord,
) -> UserMessage {
    match history_record {
        UserMessageHistoryRecord::Override(history) if !history.text.is_empty() => UserMessage {
            text: history.text.clone(),
            text_elements: history.text_elements.clone(),
            ..message
        },
        UserMessageHistoryRecord::Override(_) | UserMessageHistoryRecord::UserMessageText => {
            message
        }
    }
}

pub(super) fn user_message_preview_text(
    message: &UserMessage,
    history_record: Option<&UserMessageHistoryRecord>,
) -> String {
    match history_record {
        Some(UserMessageHistoryRecord::Override(history)) if !history.text.is_empty() => {
            history.text.clone()
        }
        Some(UserMessageHistoryRecord::Override(_))
        | Some(UserMessageHistoryRecord::UserMessageText)
        | None => message.text.clone(),
    }
}

pub(super) fn user_message_display_for_history(
    message: UserMessage,
    history_record: &UserMessageHistoryRecord,
) -> UserMessageDisplay {
    let message = user_message_for_restore(message, history_record);
    ChatWidget::user_message_display_from_parts(
        message.text,
        message.text_elements,
        message
            .local_images
            .into_iter()
            .map(|image| image.path)
            .collect(),
        message.remote_image_urls,
    )
}

pub(super) fn merge_user_messages_with_history_record(
    messages: Vec<(UserMessage, UserMessageHistoryRecord)>,
) -> (UserMessage, UserMessageHistoryRecord) {
    let messages = remap_user_messages_with_history_records(messages);
    let history_record = if messages
        .iter()
        .all(|(_, record)| *record == UserMessageHistoryRecord::UserMessageText)
    {
        UserMessageHistoryRecord::UserMessageText
    } else {
        let mut history_text = String::new();
        let mut history_text_elements = Vec::new();
        let mut history_segment_count = 0usize;
        let mut append_history_segment = |text: &str, text_elements: Vec<TextElement>| {
            if history_segment_count > 0 {
                history_text.push('\n');
            }
            append_text_with_rebased_elements(
                &mut history_text,
                &mut history_text_elements,
                text,
                text_elements,
            );
            history_segment_count += 1;
        };
        for (message, record) in &messages {
            match record {
                UserMessageHistoryRecord::Override(history) if !history.text.is_empty() => {
                    append_history_segment(&history.text, history.text_elements.clone());
                }
                UserMessageHistoryRecord::Override(_) if message.text.is_empty() => {}
                UserMessageHistoryRecord::Override(_)
                | UserMessageHistoryRecord::UserMessageText => {
                    append_history_segment(&message.text, message.text_elements.clone());
                }
            }
        }
        UserMessageHistoryRecord::Override(UserMessageHistoryOverride {
            text: history_text,
            text_elements: history_text_elements,
        })
    };

    (
        merge_remapped_user_messages(messages.into_iter().map(|(message, _)| message)),
        history_record,
    )
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserMessageDisplay {
    pub(crate) message: String,
    pub(crate) remote_image_urls: Vec<String>,
    pub(crate) local_images: Vec<PathBuf>,
    pub(crate) text_elements: Vec<TextElement>,
}

pub(crate) fn mention_bindings_from_user_inputs(
    items: &[UserInput],
    message: &str,
) -> Vec<MentionBinding> {
    let mention_start = |sigil: char, mention: &str| {
        let token = format!("{sigil}{mention}");
        message.match_indices(&token).find_map(|(start, _)| {
            let end = start + token.len();
            message
                .as_bytes()
                .get(end)
                .is_none_or(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-'))
                .then_some(start)
        })
    };
    let mut mention_bindings: Vec<MentionBinding> = items
        .iter()
        .filter_map(|item| match item {
            UserInput::Skill { name, path } => Some(MentionBinding {
                sigil: TOOL_MENTION_SIGIL,
                mention: name.clone(),
                path: path.to_string_lossy().into_owned(),
            }),
            UserInput::Mention { name, path } => {
                let plugin_id = path.strip_prefix("plugin://");
                let mention = if let Some(plugin_id) = plugin_id {
                    plugin_id
                        .split_once('@')
                        .map(|(plugin_name, _)| plugin_name)
                        .unwrap_or(plugin_id)
                        .to_string()
                } else if path.starts_with("app://") {
                    crate::connectors::metadata::connector_mention_slug_from_name(name)
                } else {
                    name.clone()
                };
                let sigil = if plugin_id.is_some()
                    && mention_start(PLUGIN_TEXT_MENTION_SIGIL, &mention).is_some()
                {
                    PLUGIN_TEXT_MENTION_SIGIL
                } else {
                    TOOL_MENTION_SIGIL
                };
                Some(MentionBinding {
                    sigil,
                    mention,
                    path: path.clone(),
                })
            }
            UserInput::Text { .. }
            | UserInput::Image { .. }
            | UserInput::LocalImage { .. }
            | UserInput::Audio { .. }
            | UserInput::LocalAudio { .. } => None,
        })
        .collect();
    mention_bindings.sort_by_key(|binding| {
        mention_start(binding.sigil, &binding.mention).unwrap_or(usize::MAX)
    });
    mention_bindings
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingSteerCompareKey {
    pub(super) message: String,
    pub(super) image_count: usize,
}

impl ChatWidget {
    pub(super) fn user_message_display_from_parts(
        message: String,
        text_elements: Vec<TextElement>,
        local_images: Vec<PathBuf>,
        remote_image_urls: Vec<String>,
    ) -> UserMessageDisplay {
        let (message, prompt_request_offset) =
            crate::tui_core::ide_context::extract_prompt_request_with_offset(&message);
        let prompt_request_end = prompt_request_offset + message.len();
        // Prompt 上下文使用与桌面端和 IDE 扩展相同的分隔符与剥离行为。
        // 原始用户消息会发给 agent，但所有界面只渲染该分隔符之后的
        // 请求部分，因此把元素保留在可见请求内，并将其
        // 字节范围随之偏移。
        let text_elements = text_elements
            .into_iter()
            .filter_map(|element| {
                let range = element.byte_range;
                if range.start < prompt_request_offset || range.end > prompt_request_end {
                    return None;
                }

                Some(element.map_range(|range| ByteRange {
                    start: range.start - prompt_request_offset,
                    end: range.end - prompt_request_offset,
                }))
            })
            .collect();

        UserMessageDisplay {
            message: message.to_string(),
            remote_image_urls,
            local_images,
            text_elements,
        }
    }

    /// 在不走昂贵的请求序列化路径的情况下，构建已提交待处理转向的
    /// 比较键。待处理转向只需匹配输入排空后提交的
    /// app-server `UserMessage` 项，
    /// 该过程保留扁平化文本与图片总数。
    pub(super) fn pending_steer_compare_key_from_items(
        items: &[UserInput],
    ) -> PendingSteerCompareKey {
        let mut message = String::new();
        let mut image_count = 0;

        for item in items {
            match item {
                UserInput::Text { text, .. } => message.push_str(text),
                UserInput::Image { .. } | UserInput::LocalImage { .. } => image_count += 1,
                UserInput::Audio { .. } // TODO: Include audio inputs in pending steer comparison.
                | UserInput::LocalAudio { .. } // TODO: Include audio inputs in pending steer comparison.
                | UserInput::Skill { .. }
                | UserInput::Mention { .. } => {}
            }
        }

        PendingSteerCompareKey {
            message,
            image_count,
        }
    }

    pub(crate) fn user_message_display_from_inputs(items: &[UserInput]) -> UserMessageDisplay {
        if items
            .iter()
            .any(|item| matches!(item, UserInput::Audio { .. } | UserInput::LocalAudio { .. }))
        {
            tracing::warn!("audio user inputs are not supported by the TUI and will be omitted");
        }
        let mut message = String::new();
        let mut remote_image_urls = Vec::new();
        let mut local_images = Vec::new();
        let mut text_elements = Vec::new();

        for item in items {
            match item {
                UserInput::Text {
                    text,
                    text_elements: current_text_elements,
                    ..
                } => append_text_with_rebased_elements(
                    &mut message,
                    &mut text_elements,
                    text,
                    current_text_elements.iter().map(|element| {
                        let range = element.byte_range.clone();
                        TextElement::new(
                            range.clone().into(),
                            element
                                .placeholder()
                                .or_else(|| text.get(range.start..range.end))
                                .map(str::to_string),
                        )
                    }),
                ),
                UserInput::Image { url, .. } => remote_image_urls.push(url.clone()),
                UserInput::LocalImage { path, .. } => local_images.push(path.clone()),
                UserInput::Audio { .. } // TODO: Include audio inputs in the user message display.
                | UserInput::LocalAudio { .. } // TODO: Include audio inputs in the user message display.
                | UserInput::Skill { .. }
                | UserInput::Mention { .. } => {}
            }
        }

        Self::user_message_display_from_parts(
            message,
            text_elements,
            local_images,
            remote_image_urls,
        )
    }
}
