//! 上游协议 crate 的替代实现。
//!
//! 真正的上游协议 crate 会拉入 `schemars`、`ts-rs`、`strum`、
//! `rmcp`、绝对路径辅助函数以及大量相互依赖的类型。
//! Reflect TUI 只需要本模块很小的一部分表面来编译
//! 其内置的 Reflect 视图，因此我们用朴素的
//! `String` / `Option` / `Vec` 字段和最小方法桩镜像类型形态。
//!
//! 覆盖范围由 `src/tui_core/` 内的 `crate::protocol_compat::...` 引用驱动。
//! 这里为每个被引用的子模块、结构体、枚举、函数和
//! 常量提供桩；无关的上游类型被有意
//! 省略，以保持文件自包含。

use std::fmt;

use serde::{Deserialize, Serialize};

// ── 线程标识类型（上游 UUIDv7 新类型） ──

/// Reflect 线程的标识符。
///
/// 上游这是由 `uuid::Uuid` 支撑的 UUIDv7 新类型；我们保持
/// 相同的形态，使 `ThreadId` 保持 `Copy`（内置 Reflect 视图依赖
/// 这一点——例如 `#[derive(Copy)]` 结构体内的 `Option<ThreadId>` 字段类型）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(uuid::Uuid);

impl ThreadId {
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }

    pub fn from_string(s: &str) -> Result<Self, uuid::Error> {
        Ok(Self(uuid::Uuid::parse_str(s)?))
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<&str> for ThreadId {
    type Error = uuid::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_string(value)
    }
}

impl TryFrom<String> for ThreadId {
    type Error = uuid::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_string(&value)
    }
}

impl From<ThreadId> for String {
    fn from(value: ThreadId) -> Self {
        value.0.to_string()
    }
}

// ── 子模块声明 ──

pub mod config_types;
pub use config_types::*;

pub mod openai_models;
pub use openai_models::*;

pub mod approvals;
pub use approvals::*;

pub mod models;
pub use models::*;

pub mod permissions;
pub use permissions::*;

pub mod request_permissions;
pub use request_permissions::*;

pub mod mcp;
pub use mcp::*;

pub mod mcp_approval_meta;
pub use mcp_approval_meta::*;

pub mod user_input;
pub use user_input::*;

pub mod parse_command;
pub use parse_command::*;

pub mod plan_tool;
pub use plan_tool::*;

pub mod account;
pub use account::*;

pub mod auth;
pub use auth::*;

pub mod num_format;
pub use num_format::*;

pub mod memory_citation;
pub use memory_citation::*;

pub mod items;
pub use items::*;

pub mod error;
pub use error::*;

pub mod protocol;
pub use protocol::*;

// =============================================================================
// 跨模块 impl（PermissionProfile 扩展方法，需在 crate 根模块）
// =============================================================================

impl crate::protocol_compat::models::PermissionProfile {
    pub fn file_system_sandbox_policy(
        &self,
    ) -> crate::protocol_compat::permissions::FileSystemSandboxPolicy {
        use crate::protocol_compat::models::PermissionProfile;
        use crate::protocol_compat::permissions::FileSystemSandboxKind;
        let kind = match self {
            PermissionProfile::WorkspaceWrite => FileSystemSandboxKind::WorkspaceWrite,
            PermissionProfile::Unrestricted | PermissionProfile::Disabled => {
                FileSystemSandboxKind::Unrestricted
            }
            PermissionProfile::External { .. } => FileSystemSandboxKind::ExternalSandbox,
            PermissionProfile::ReadOnly => FileSystemSandboxKind::Restricted,
            PermissionProfile::Managed { .. } => FileSystemSandboxKind::Restricted,
        };
        crate::protocol_compat::permissions::FileSystemSandboxPolicy {
            kind,
            ..crate::protocol_compat::permissions::FileSystemSandboxPolicy::default()
        }
    }
    pub fn from_legacy_sandbox_policy_for_cwd(_policy: &str, _cwd: &std::path::Path) -> Self {
        crate::protocol_compat::models::PermissionProfile::default()
    }
    pub fn network_sandbox_policy(
        &self,
    ) -> crate::protocol_compat::permissions::FileSystemSandboxPolicy {
        crate::protocol_compat::permissions::FileSystemSandboxPolicy::default()
    }
}
