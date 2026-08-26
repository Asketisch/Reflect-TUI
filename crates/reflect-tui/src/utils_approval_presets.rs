//! reflect_utils_approval_presets crate 的桩实现。

use crate::protocol_compat::models::PermissionProfile;

#[derive(Debug, Clone, Default)]
pub struct ApprovalPreset {
    pub id: String,
    pub label: String,
    pub description: String,
    pub active_permission_profile: Option<String>,
    pub permission_profile: PermissionProfile,
    pub approval: crate::app_server_protocol::AskForApproval,
}

impl ApprovalPreset {
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            id: id.clone(),
            label: id,
            ..Default::default()
        }
    }
}

pub fn builtin_approval_presets() -> Vec<ApprovalPreset> {
    vec![ApprovalPreset::new("on-request")]
}

pub fn builtin_permission_profile_for_active_permission_profile(_profile: &str) -> String {
    "default".to_string()
}
