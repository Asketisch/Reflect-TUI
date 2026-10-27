//! TUI 默认使用的统一提及弹窗。
//!
//! `mentions_v2` 特性开关暂时保留作为回滚路径：禁用它可恢复
//! 旧的拆分提及与文件搜索弹窗。

mod candidate;
mod filter;
mod footer;
mod popup;
mod render;
mod search_catalog;
mod search_mode;

pub(crate) use candidate::Selection as MentionV2Selection;
pub(crate) use popup::Popup as MentionV2Popup;
pub(crate) use search_catalog::build_search_catalog;
