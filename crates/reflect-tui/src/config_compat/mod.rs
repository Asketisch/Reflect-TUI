//! 配置存根模块，镜像上游 `config_compat` crate 的公共接口。
//!
//! 这里只建模 `crate::tui_core::*` 实际消费的符号。
//! 字段类型在可行时尽量保持为普通基本类型（`String`、`Option`、`Vec`、`bool`），
//! 以免存根引入额外的依赖。当内置代码依赖丰富的结构体字段（例如 `TuiKeymap`
//! 和 MCP 传输配置）时，则按原样保留上游 crate 的真实形态，使调用点继续通过类型检查。

use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;

/// `$REFLECT_HOME` 中每个用户 Reflect 配置文件的相对位置。
pub const CONFIG_TOML_FILE: &str = ".reflect/config.toml";

// ── 子模块声明 ──

mod config_layer;
pub use config_layer::*;

mod constraint;
pub use constraint::*;

mod requirements;
pub use requirements::*;

mod free_functions;
pub use free_functions::*;

pub mod types;
pub use types::*;

// =============================================================================
// 跨模块 impl（需在 crate 根模块）
// =============================================================================

impl crate::config_compat::types::ApprovalsReviewer {
    pub fn to_core(&self) -> crate::protocol_compat::config_types::ApprovalsReviewer {
        match self {
            crate::config_compat::types::ApprovalsReviewer::User => {
                crate::protocol_compat::config_types::ApprovalsReviewer::User
            }
            crate::config_compat::types::ApprovalsReviewer::AutoReview => {
                crate::protocol_compat::config_types::ApprovalsReviewer::AutoReview
            }
        }
    }
}

impl From<crate::protocol_compat::config_types::ApprovalsReviewer>
    for crate::config_compat::types::ApprovalsReviewer
{
    fn from(v: crate::protocol_compat::config_types::ApprovalsReviewer) -> Self {
        match v {
            crate::protocol_compat::config_types::ApprovalsReviewer::User => {
                crate::config_compat::types::ApprovalsReviewer::User
            }
            crate::protocol_compat::config_types::ApprovalsReviewer::AutoReview => {
                crate::config_compat::types::ApprovalsReviewer::AutoReview
            }
        }
    }
}

impl crate::config_compat::RequirementSource {
    pub fn as_ref(&self) -> String {
        String::new()
    }
}

impl crate::config_compat::ConfigRequirements {
    pub fn exec_policy_source(&self) -> () {}
}

impl crate::config_compat::ManagedHooksRequirementsToml {
    pub fn handler_count(&self) -> () {}
}
