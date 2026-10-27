//! 状态卡片标签/权限/审批格式化与 model provider/base_url 辅助函数簇。从 status/card.rs 抽出。

use super::*;

pub(super) fn status_permission_summary(
    permission_profile: &PermissionProfile,
    cwd: &AbsolutePathBuf,
    workspace_roots: &[AbsolutePathBuf],
) -> String {
    let summary = summarize_permission_profile(permission_profile, cwd, workspace_roots);
    if let Some(details) = summary.strip_prefix("read-only") {
        if details.contains("(network access enabled)") {
            return "read-only with network access".to_string();
        }
        return "read-only".to_string();
    }
    if let Some(details) = summary.strip_prefix("workspace-write") {
        if details.contains("(network access enabled)") {
            return "workspace with network access".to_string();
        }
        return "workspace".to_string();
    }
    if summary == "custom permissions (network access enabled)" {
        return "custom permissions with network access".to_string();
    }
    summary
}

pub(super) fn workspace_root_suffix(
    workspace_roots: &[AbsolutePathBuf],
    cwd: &AbsolutePathBuf,
) -> Option<String> {
    let extra_roots = workspace_roots
        .iter()
        .filter(|root| *root != cwd)
        .map(|root| root.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if extra_roots.is_empty() {
        None
    } else {
        Some(format!(" [{}]", extra_roots.join(", ")))
    }
}

pub(super) fn status_permissions_label(
    active_permission_profile: Option<&ActivePermissionProfile>,
    permission_profile: &PermissionProfile,
    approval_policy: AskForApproval,
    sandbox: &str,
    approval: &str,
    workspace_root_suffix: Option<&str>,
) -> String {
    let active_id = active_permission_profile.map(|active| active.id.as_str());
    match active_id {
        Some(BUILT_IN_PERMISSION_PROFILE_READ_ONLY) => {
            let label = if sandbox == "read-only with network access" {
                "Read Only with network access"
            } else {
                "Read Only"
            };
            return format!("{label} ({approval})");
        }
        Some(BUILT_IN_PERMISSION_PROFILE_WORKSPACE) => match sandbox {
            "workspace" => {
                return format!(
                    "Workspace{} ({approval})",
                    workspace_root_suffix.unwrap_or("")
                );
            }
            "workspace with network access" => {
                return format!(
                    "Workspace with network access{} ({approval})",
                    workspace_root_suffix.unwrap_or("")
                );
            }
            _ => {}
        },
        Some(BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS)
            if permission_profile == &PermissionProfile::Disabled =>
        {
            return if approval_policy == AskForApproval::Never {
                "Full Access".to_string()
            } else {
                format!("No Sandbox ({approval})")
            };
        }
        Some(id) => {
            let sandbox = decorate_workspace_sandbox_label(sandbox, workspace_root_suffix);
            return format!("Profile {id} ({sandbox}, {approval})");
        }
        None => {}
    }

    if sandbox == "read-only" {
        return format!("Read Only ({approval})");
    }
    if approval_policy == AskForApproval::OnRequest && sandbox == "workspace" {
        return format!(
            "Workspace{} ({approval})",
            workspace_root_suffix.unwrap_or("")
        );
    }
    if approval_policy == AskForApproval::Never
        && permission_profile == &PermissionProfile::Disabled
    {
        return "Full Access".to_string();
    }
    let sandbox = decorate_workspace_sandbox_label(sandbox, workspace_root_suffix);
    format!("Custom ({sandbox}, {approval})")
}

pub(super) fn decorate_workspace_sandbox_label(
    sandbox: &str,
    workspace_root_suffix: Option<&str>,
) -> String {
    match workspace_root_suffix {
        Some(suffix) if sandbox.starts_with("workspace") => format!("{sandbox}{suffix}"),
        _ => sandbox.to_string(),
    }
}

pub(super) fn status_approval_label(
    approval_policy: AskForApproval,
    approvals_reviewer: ApprovalsReviewer,
    approval: &str,
) -> String {
    if approval_policy == AskForApproval::OnRequest {
        return match approvals_reviewer {
            ApprovalsReviewer::AutoReview => "Approve for me".to_string(),
            ApprovalsReviewer::User => "Ask for approval".to_string(),
        };
    }

    approval.to_string()
}

pub(super) fn format_model_provider(
    config: &Config,
    runtime_base_url: Option<&str>,
) -> Option<String> {
    let provider = &config.model_provider;
    let name = provider.name.trim();
    let provider_name = if name.is_empty() {
        config.model_provider_id.as_str()
    } else {
        name
    };
    let base_url = runtime_base_url.and_then(sanitize_base_url);
    let is_default_openai = provider.is_openai() && base_url.is_none();
    if is_default_openai {
        return None;
    }

    Some(match base_url {
        Some(base_url) => format!("{provider_name} - {base_url}"),
        None => provider_name.to_string(),
    })
}

pub(super) fn sanitize_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let Ok(mut url) = Url::parse(trimmed) else {
        return None;
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string().trim_end_matches('/').to_string()).filter(|value| !value.is_empty())
}
