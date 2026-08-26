//! TUI 的状态输出格式化与显示适配器。
//!
//! 本模块将协议级快照转换为稳定的显示结构，供 `/status` 输出以及页脚/状态行
//! 辅助函数使用，同时让渲染相关逻辑不进入面向传输的代码。
//!
//! `rate_limits` 是状态行用量限制项的主要集成点：它将原始窗口快照转换为
//! 本地时间标签，并把数据分类为可用、过期或缺失。
mod account;
mod card;
mod format;
mod helpers;
mod rate_limits;
pub(crate) mod remote_connection;

pub(crate) use account::StatusAccountDisplay;
pub(crate) use card::StatusHistoryHandle;
#[cfg(test)]
pub(crate) use card::new_status_output;
#[cfg(test)]
pub(crate) use card::new_status_output_with_rate_limits;
pub(crate) use card::new_status_output_with_rate_limits_handle;
pub(crate) use helpers::compose_agents_summary;
pub(crate) use helpers::format_directory_display;
pub(crate) use helpers::format_tokens_compact;
pub(crate) use rate_limits::RateLimitSnapshotDisplay;
pub(crate) use rate_limits::RateLimitWindowDisplay;
#[cfg(test)]
pub(crate) use rate_limits::rate_limit_snapshot_display;
pub(crate) use rate_limits::rate_limit_snapshot_display_for_limit;

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
