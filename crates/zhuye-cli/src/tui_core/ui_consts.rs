//! TUI 内布局与对齐的共享 UI 常量。

/// 为实时单元格与对齐 widget 保留的左侧沟槽/前缀宽度（以终端列计）。
///
/// 语义：
/// - 聊天输入框为左边框 + 内边距预留这么多列。
/// - 状态指示器行以这么多空格开头以对齐。
/// - 用户历史行在换行时计入这么多列（例如 "▌ "）。
pub(crate) const LIVE_PREFIX_COLS: u16 = 2;
pub(crate) const FOOTER_INDENT_COLS: usize = LIVE_PREFIX_COLS as usize;
pub(crate) const TRANSCRIPT_HINT: &str = "ctrl + t to view transcript";
