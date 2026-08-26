//! JSON-RPC 基础类型。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

/// JSON-RPC 请求标识符。存根镜像了上游的两种变体枚举，以便 UI
/// 能为外发请求构造 `RequestId::String(...)`。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RequestId {
    String(String),
    Integer(i64),
}

impl Default for RequestId {
    fn default() -> Self {
        RequestId::Integer(0)
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestId::String(value) => f.write_str(value),
            RequestId::Integer(value) => write!(f, "{value}"),
        }
    }
}
