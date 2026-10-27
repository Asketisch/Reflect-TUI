//! `ChatWidget` 的代码评审流程状态。

use crate::tui_core::auto_review_denials::RecentAutoReviewDenials;
use crate::tui_core::token_usage::TokenUsageInfo;

#[derive(Debug, Default)]
pub(super) struct ReviewState {
    pub(super) recent_auto_review_denials: RecentAutoReviewDenials,
    /// 简单的评审模式标志；用于调整布局和横幅。
    pub(super) is_review_mode: bool,
    /// 用于在退出评审模式后恢复的 token 用量快照。
    pub(super) pre_review_token_info: Option<Option<TokenUsageInfo>>,
}
