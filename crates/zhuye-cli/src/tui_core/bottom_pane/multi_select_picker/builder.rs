//! 多选 picker 构建器与匹配辅助。从 multi_select_picker.rs 抽出。

use super::*;

impl MultiSelectPickerBuilder {
    /// 使用给定标题、可选副标题和事件发送器创建新的构建器。
    pub fn new(title: String, subtitle: Option<String>, app_event_tx: AppEventSender) -> Self {
        Self {
            title,
            subtitle,
            instructions: Vec::new(),
            items: Vec::new(),
            ordering_enabled: false,
            app_event_tx,
            keymap: RuntimeKeymap::defaults().list,
            preview_builder: None,
            on_change: None,
            on_confirm: None,
            on_cancel: None,
        }
    }

    /// 设置可选条目列表。
    pub fn items(mut self, items: Vec<MultiSelectItem>) -> Self {
        self.items = items;
        self
    }

    /// 启用左右方向键以重新排列条目。
    ///
    /// 仅当搜索查询为空时才能重新排序。
    pub fn enable_ordering(mut self) -> Self {
        self.ordering_enabled = true;
        self
    }

    /// 设置用于导航和完成操作的共享列表键位表。
    pub fn list_keymap(mut self, keymap: ListKeymap) -> Self {
        self.keymap = keymap;
        self
    }

    /// 设置根据当前条目状态生成预览行的回调。
    ///
    /// 中文说明：The callback receives all items and should return a [`Line`] to display,
    /// 中文说明：or `None` to hide the preview area.
    pub fn on_preview<F>(mut self, callback: F) -> Self
    where
        F: Fn(&[MultiSelectItem]) -> Option<Line<'static>> + Send + Sync + 'static,
    {
        self.preview_builder = Some(Box::new(callback));
        self
    }

    /// 设置条目状态每次变化时调用的回调。
    ///
    /// 这包括切换和重新排序操作。
    #[allow(dead_code)]
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(&[MultiSelectItem], &AppEventSender) + Send + Sync + 'static,
    {
        self.on_change = Some(Box::new(callback));
        self
    }

    /// 设置用户确认选择（Enter）时调用的回调。
    ///
    /// 回调接收所有已启用条目的 ID 列表。
    pub fn on_confirm<F>(mut self, callback: F) -> Self
    where
        F: Fn(&[String], &AppEventSender) + Send + Sync + 'static,
    {
        self.on_confirm = Some(Box::new(callback));
        self
    }

    /// 设置用户取消选择器（Escape）时调用的回调。
    pub fn on_cancel<F>(mut self, callback: F) -> Self
    where
        F: Fn(&AppEventSender) + Send + Sync + 'static,
    {
        self.on_cancel = Some(Box::new(callback));
        self
    }

    /// 使用所有已配置选项构建 [`MultiSelectPicker`]。
    ///
    /// 中文说明：Initializes the filter to show all items and generates the initial
    /// 中文说明：preview line if a preview callback was set.
    pub fn build(self) -> MultiSelectPicker {
        let mut header = ColumnRenderable::new();
        header.push(Line::from(self.title.bold()));

        if let Some(subtitle) = self.subtitle {
            header.push(Line::from(subtitle.dim()));
        }

        let instructions = if self.instructions.is_empty() {
            let mut spans = vec![
                "Press ".into(),
                key_hint::plain(KeyCode::Char(' ')).into(),
                " to toggle".into(),
            ];
            if self.ordering_enabled
                && let (Some(move_left), Some(move_right)) = (
                    primary_binding(&self.keymap.move_left),
                    primary_binding(&self.keymap.move_right),
                )
            {
                spans.push("; ".into());
                spans.push(move_left.into());
                spans.push("/".into());
                spans.push(move_right.into());
                spans.push(" to move".into());
            }
            if let Some(accept) = primary_binding(&self.keymap.accept) {
                spans.push("; ".into());
                spans.push(accept.into());
                spans.push(" to confirm and close".into());
            }
            if let Some(cancel) = primary_binding(&self.keymap.cancel) {
                spans.push("; ".into());
                spans.push(cancel.into());
                spans.push(" to close".into());
            }
            spans
        } else {
            self.instructions
        };

        let mut view = MultiSelectPicker {
            items: self.items,
            state: ScrollState::new(),
            complete: false,
            app_event_tx: self.app_event_tx,
            header: Box::new(header),
            footer_hint: Line::from(instructions),
            ordering_enabled: self.ordering_enabled,
            keymap: self.keymap,
            search_query: String::new(),
            filtered_indices: Vec::new(),
            preview_builder: self.preview_builder,
            preview_line: None,
            on_change: self.on_change,
            on_confirm: self.on_confirm,
            on_cancel: self.on_cancel,
        };
        view.apply_filter();
        view.update_preview_line();
        view
    }
}

/// 使用过滤字符串对条目执行模糊匹配。
///
/// 首先尝试匹配显示名称，若名称不同则回退匹配名称。返回匹配字符索引（若匹配显示名称）和用于排序的分数。
///
/// # 参数
///
/// * `filter` - 要匹配的搜索查询
/// * `display_name` - 要匹配的主要名称（向用户显示）
/// * `name` - 显示名称不匹配时尝试的次要/规范名称
///
/// # 返回值
///
/// * `Some((Some(indices), score))` - 匹配显示名称，并带有高亮索引
/// * `Some((None, score))` - 仅匹配 skill 名称（显示时不高亮）
/// * `None` - 无匹配
pub(crate) fn match_item(
    filter: &str,
    display_name: &str,
    name: &str,
) -> Option<(Option<Vec<usize>>, i32)> {
    if let Some((indices, score)) = fuzzy_match(display_name, filter) {
        return Some((Some(indices), score));
    }
    if display_name != name
        && let Some((_indices, score)) = fuzzy_match(name, filter)
    {
        return Some((None, score));
    }
    None
}
