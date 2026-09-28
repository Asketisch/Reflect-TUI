//! 内置审批 preset —— 权限弹窗(/permissions)的三档预设。
//!
//! v1.6 修复:此前是桩实现,只返回 `on-request` 一条,而
//! `open_permission_profiles_popup` 要求 `read-only` / `auto` /
//! `full-access` 三条 —— 打开弹窗必然报 "missing the 'read-only'
//! approval preset" 内部错误。语义对齐 codex 的
//! `utils/approval-presets`,文案适配 Reflect 的权限模型。

use crate::app_server_protocol::AskForApproval;
use crate::protocol_compat::models::{
    BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS, BUILT_IN_PERMISSION_PROFILE_READ_ONLY,
    BUILT_IN_PERMISSION_PROFILE_WORKSPACE, PermissionProfile,
};

#[derive(Debug, Clone, Default)]
pub struct ApprovalPreset {
    pub id: String,
    pub label: String,
    pub description: String,
    pub active_permission_profile: Option<String>,
    pub permission_profile: PermissionProfile,
    pub approval: AskForApproval,
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

/// 内置 preset 清单:审批策略 × 权限档位的成对预设。
///
/// 保持 UI 无关,权限弹窗与未来的 MCP server 都可复用。
pub fn builtin_approval_presets() -> Vec<ApprovalPreset> {
    vec![
        ApprovalPreset {
            id: "read-only".into(),
            label: "Read Only".into(),
            description: "只读模式:agent 可读取工作区文件与执行只读搜索, \
                          写文件 / 联网 / 危险命令都需要审批。"
                .into(),
            approval: AskForApproval::OnRequest,
            active_permission_profile: Some(BUILT_IN_PERMISSION_PROFILE_READ_ONLY.into()),
            permission_profile: PermissionProfile::read_only(),
        },
        ApprovalPreset {
            id: "auto".into(),
            label: "Agent".into(),
            description: "默认模式:agent 可读写工作区文件并执行命令, \
                          联网与工作区外写入需要审批。"
                .into(),
            approval: AskForApproval::OnRequest,
            active_permission_profile: Some(BUILT_IN_PERMISSION_PROFILE_WORKSPACE.into()),
            permission_profile: PermissionProfile::workspace_write(),
        },
        ApprovalPreset {
            id: "full-access".into(),
            label: "Full Access".into(),
            description: "完全信任:工作区外写入与联网不再询问,请谨慎使用。".into(),
            approval: AskForApproval::Never,
            active_permission_profile: Some(BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS.into()),
            permission_profile: PermissionProfile::Disabled,
        },
        // 兼容保留:on-request 单独成条(仅审批策略,不动权限档位)。
        ApprovalPreset {
            id: "on-request".into(),
            label: "Ask (on request)".into(),
            description: "按需审批:agent 自主判断何时请求人工确认。".into(),
            approval: AskForApproval::OnRequest,
            active_permission_profile: None,
            permission_profile: PermissionProfile::workspace_write(),
        },
    ]
}

/// 按内置 active profile id 返回对应的权限档位(非内置 id → None)。
pub fn builtin_permission_profile_for_active_permission_profile(
    profile: &str,
) -> Option<PermissionProfile> {
    match profile {
        BUILT_IN_PERMISSION_PROFILE_READ_ONLY => Some(PermissionProfile::read_only()),
        BUILT_IN_PERMISSION_PROFILE_WORKSPACE => Some(PermissionProfile::workspace_write()),
        BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS => Some(PermissionProfile::Disabled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 权限弹窗依赖的三个 id 必须齐备(此前的桩导致弹窗必然报错)。
    #[test]
    fn presets_cover_ids_required_by_permissions_menu() {
        let presets = builtin_approval_presets();
        for id in ["read-only", "auto", "full-access"] {
            assert!(
                presets.iter().any(|p| p.id == id),
                "missing preset '{id}' required by permissions_menu"
            );
        }
    }

    /// full-access 必须是 Never 审批 + Disabled 沙箱(语义安全:危险档
    /// 不该再弹审批);read-only / auto 是 OnRequest。
    #[test]
    fn preset_semantics_pair_approval_with_profile() {
        let presets = builtin_approval_presets();
        let full = presets.iter().find(|p| p.id == "full-access").unwrap();
        assert_eq!(full.approval, AskForApproval::Never);
        assert_eq!(full.permission_profile, PermissionProfile::Disabled);

        for id in ["read-only", "auto"] {
            let p = presets.iter().find(|p| p.id == id).unwrap();
            assert_eq!(p.approval, AskForApproval::OnRequest, "preset '{id}'");
        }
    }

    /// 内置 id → 档位映射:三个内置 id 命中,其余 None。
    #[test]
    fn builtin_profile_lookup() {
        assert_eq!(
            builtin_permission_profile_for_active_permission_profile(":read-only"),
            Some(PermissionProfile::read_only())
        );
        assert_eq!(
            builtin_permission_profile_for_active_permission_profile(":workspace"),
            Some(PermissionProfile::workspace_write())
        );
        assert_eq!(
            builtin_permission_profile_for_active_permission_profile(":danger-full-access"),
            Some(PermissionProfile::Disabled)
        );
        assert_eq!(
            builtin_permission_profile_for_active_permission_profile("custom-profile"),
            None
        );
    }
}
