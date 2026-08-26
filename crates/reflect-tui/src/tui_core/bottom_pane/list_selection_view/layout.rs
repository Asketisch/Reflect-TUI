//! 列表选择视图布局宽度计算辅助。从 list_selection_view.rs 抽出。

use super::*;

impl Default for SideContentWidth {
    fn default() -> Self {
        Self::Fixed(0)
    }
}

/// 返回减去共享菜单表面水平内边距（每侧 2 列）后的弹窗内容宽度。
pub(crate) fn popup_content_width(total_width: u16) -> u16 {
    total_width.saturating_sub(MENU_SURFACE_HORIZONTAL_INSET)
}

/// 布局可容纳时，以 `(list_width, side_width)` 返回并排布局宽度。侧面板已禁用、过窄，或剩余列表宽度小到无法使用时返回 `None`。
pub(crate) fn side_by_side_layout_widths(
    content_width: u16,
    side_content_width: SideContentWidth,
    side_content_min_width: u16,
) -> Option<(u16, u16)> {
    let side_width = match side_content_width {
        SideContentWidth::Fixed(0) => return None,
        SideContentWidth::Fixed(width) => width,
        SideContentWidth::Half => content_width.saturating_sub(SIDE_CONTENT_GAP) / 2,
    };
    if side_width < side_content_min_width {
        return None;
    }
    let list_width = content_width.saturating_sub(SIDE_CONTENT_GAP + side_width);
    (list_width >= MIN_LIST_WIDTH_FOR_SIDE).then_some((list_width, side_width))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SelectionRowDisplay {
    #[default]
    Wrapped,
    SingleLine,
}

/// 通用选择列表中的一个可选条目。
pub(crate) type SelectionAction = Box<dyn Fn(&AppEventSender) + Send + Sync>;
pub(crate) type SelectionToggleAction = dyn Fn(bool, &AppEventSender) + Send + Sync;

pub(crate) struct SelectionToggle {
    pub is_on: bool,
    pub action: Box<SelectionToggleAction>,
}

/// 每当高亮条目变化（方向键、搜索过滤、数字键跳转）时调用的回调。接收未过滤 `items` 列表中的*实际*索引和事件发送器。供主题选择器用于实时预览。
pub(crate) type OnSelectionChangedCallback =
    Option<Box<dyn Fn(usize, &AppEventSender) + Send + Sync>>;

/// 选择器未接受便被关闭（Esc 或 Ctrl+C）时调用的回调。供主题选择器恢复打开前的主题。
pub(crate) type OnCancelCallback = Option<Box<dyn Fn(&AppEventSender) + Send + Sync>>;

/// [`ListSelectionView`] 选择列表中的一行。
///
/// 这是过滤并格式化为渲染行之前，行状态的权威模型。当 `is_disabled` 为 true 或存在 `disabled_reason` 时，该行视为已禁用；禁用行无法接受，并会被键盘导航跳过。
#[derive(Default)]
pub(crate) struct SelectionItem {
    pub name: String,
    pub name_prefix_spans: Vec<Span<'static>>,
    pub toggle: Option<SelectionToggle>,
    pub toggle_placeholder: Option<&'static str>,
    pub display_shortcut: Option<KeyBinding>,
    pub description: Option<String>,
    pub selected_description: Option<String>,
    pub is_current: bool,
    pub is_default: bool,
    pub is_disabled: bool,
    pub actions: Vec<SelectionAction>,
    pub dismiss_on_select: bool,
    pub dismiss_parent_on_child_accept: bool,
    pub search_value: Option<String>,
    pub disabled_reason: Option<String>,
}

/// [`ListSelectionView`] 的构造时配置。
///
/// 此配置由 [`ListSelectionView::new`] 一次性消费。构造后，可变交互状态（过滤、滚动及选中行）保存在视图本身。
///
/// `col_width_mode` 控制选择列表的列宽模式：
/// `AutoVisible`（默认）仅测量视口中可见的行
/// `AutoAllRows` 测量所有行，以确保用户滚动时列宽保持稳定
/// `Fixed` 使用固定的 30/70 列分割
/// `row_display` 控制行是可以换行，还是保持单行并用省略号截断
pub(crate) struct SelectionViewParams {
    pub view_id: Option<&'static str>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub footer_note: Option<Line<'static>>,
    pub footer_hint: Option<Line<'static>>,
    pub tab_footer_hints: Vec<(String, Line<'static>)>,
    pub items: Vec<SelectionItem>,
    pub tabs: Vec<SelectionTab>,
    pub initial_tab_id: Option<String>,
    pub is_searchable: bool,
    pub search_placeholder: Option<String>,
    pub col_width_mode: ColumnWidthMode,
    pub row_display: SelectionRowDisplay,
    /// 自动调整行大小时使用的左列渲染宽度。
    pub name_column_width: Option<usize>,
    pub header: Box<dyn Renderable>,
    pub initial_selected_idx: Option<usize>,

    /// 中文说明：Rich content rendered beside (wide terminals) or below (narrow terminals)
    /// 中文说明：the list items, inside the bordered menu surface. Used by the theme picker
    /// 中文说明：to show a syntax-highlighted preview.
    pub side_content: Box<dyn Renderable>,

    /// 并排布局启用时侧边内容的宽度模式。
    pub side_content_width: SideContentWidth,

    /// 启用并排布局所需的最小侧面板宽度。
    pub side_content_min_width: u16,

    /// 并排布局无法容纳时渲染的可选后备内容。
    /// 缺省时复用 `side_content`。
    pub stacked_side_content: Option<Box<dyn Renderable>>,

    /// 在并排模式下渲染后保留侧边内容背景色。
    /// 中文说明：Disabled by default so existing popups preserve their reset-background look.
    pub preserve_side_content_bg: bool,

    /// 高亮条目变化（导航、过滤、数字键）时调用。
    /// 接收条目的*实际*索引，而非过滤后/可见索引。
    pub on_selection_changed: OnSelectionChangedCallback,

    /// 取消键是否可以关闭选择器。
    pub allow_cancel: bool,

    /// 选择器未做选择便通过 Esc/Ctrl+C 关闭时调用。
    pub on_cancel: OnCancelCallback,
}
