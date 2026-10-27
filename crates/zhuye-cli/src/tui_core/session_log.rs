//! session_log 模块的桩代码。

#[derive(Debug, Clone, Default)]
pub struct SessionLog;

/// log_inbound_app_event 的桩实现。
pub fn log_inbound_app_event(_event: &(impl std::fmt::Debug + ?Sized)) {}

/// log_outbound_op 的桩实现。
pub fn log_outbound_op(_op: &(impl std::fmt::Debug + ?Sized)) {}
