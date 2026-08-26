use super::*;

/// 权限授予持续生效的作用域。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGrantScope {
    #[default]
    Turn,
    Session,
}

/// 代理为当前回合/会话请求的权限。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPermissionProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<crate::protocol_compat::models::NetworkPermissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_system: Option<crate::protocol_compat::models::FileSystemPermissions>,
}

impl RequestPermissionProfile {
    pub fn is_empty(&self) -> bool {
        self.network.is_none() && self.file_system.is_none()
    }
}

impl From<crate::app_server_protocol::RequestPermissionProfile> for RequestPermissionProfile {
    fn from(_value: crate::app_server_protocol::RequestPermissionProfile) -> Self {
        Self {
            network: None,
            file_system: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPermissionsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub permissions: RequestPermissionProfile,
}

/// 对 `request_permissions` 工具调用的响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPermissionsResponse {
    pub permissions: RequestPermissionProfile,
    #[serde(default)]
    pub scope: PermissionGrantScope,
    #[serde(default)]
    pub strict_auto_review: bool,
}

/// 代理调用 `request_permissions` 时抛出的事件。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPermissionsEvent {
    pub call_id: String,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub permissions: RequestPermissionProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}
