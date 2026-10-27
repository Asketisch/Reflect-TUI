//! 状态面 rate-limit 窗口查找与配置展示辅助函数簇。从 status_surfaces.rs 抽出。

use super::*;

pub(super) fn five_hour_status_window(
    snapshot: &RateLimitSnapshotDisplay,
) -> Option<(&RateLimitWindowDisplay, bool)> {
    find_primary_reflect_window(snapshot, "5h")
        .or_else(|| secondary_window_with_label_when_weekly_is_available(snapshot, "5h"))
        .or_else(|| non_weekly_primary_window(snapshot))
        .or_else(|| non_weekly_secondary_window_when_primary_is_weekly(snapshot))
}

pub(super) fn weekly_status_window(
    snapshot: &RateLimitSnapshotDisplay,
) -> Option<(&RateLimitWindowDisplay, bool)> {
    find_reflect_window(snapshot, "weekly")
        .or_else(|| snapshot.secondary.as_ref().map(|window| (window, true)))
}

pub(super) fn find_reflect_window<'a>(
    snapshot: &'a RateLimitSnapshotDisplay,
    label: &str,
) -> Option<(&'a RateLimitWindowDisplay, bool)> {
    if let Some(primary) = snapshot.primary.as_ref()
        && matches_window_label(primary, label)
    {
        return Some((primary, false));
    }

    if let Some(secondary) = snapshot.secondary.as_ref()
        && matches_window_label(secondary, label)
    {
        return Some((secondary, true));
    }

    None
}

pub(super) fn find_primary_reflect_window<'a>(
    snapshot: &'a RateLimitSnapshotDisplay,
    label: &str,
) -> Option<(&'a RateLimitWindowDisplay, bool)> {
    let primary = snapshot.primary.as_ref()?;
    if matches_window_label(primary, label) {
        Some((primary, false))
    } else {
        None
    }
}

pub(super) fn secondary_window_with_label_when_weekly_is_available<'a>(
    snapshot: &'a RateLimitSnapshotDisplay,
    label: &str,
) -> Option<(&'a RateLimitWindowDisplay, bool)> {
    find_reflect_window(snapshot, "weekly")?;

    let secondary = snapshot.secondary.as_ref()?;
    if matches_window_label(secondary, label) {
        Some((secondary, true))
    } else {
        None
    }
}

pub(super) fn non_weekly_primary_window(
    snapshot: &RateLimitSnapshotDisplay,
) -> Option<(&RateLimitWindowDisplay, bool)> {
    let primary = snapshot.primary.as_ref()?;
    if matches_window_label(primary, "weekly") {
        None
    } else {
        Some((primary, false))
    }
}

pub(super) fn non_weekly_secondary_window_when_primary_is_weekly(
    snapshot: &RateLimitSnapshotDisplay,
) -> Option<(&RateLimitWindowDisplay, bool)> {
    let primary = snapshot.primary.as_ref()?;
    if !matches_window_label(primary, "weekly") {
        return None;
    }

    let secondary = snapshot.secondary.as_ref()?;
    if matches_window_label(secondary, "weekly") {
        None
    } else {
        Some((secondary, true))
    }
}

pub(super) fn matches_window_label(window: &RateLimitWindowDisplay, label: &str) -> bool {
    window
        .window_minutes
        .and_then(get_limits_duration)
        .as_deref()
        == Some(label)
}

pub(super) fn permissions_display(config: &Config) -> String {
    let active_permission_profile = config.permissions.active_permission_profile();
    if let Some(active_permission_profile) = active_permission_profile.as_ref()
        && !active_permission_profile.id.starts_with(':')
    {
        return active_permission_profile.id.clone();
    }

    let permission_profile = config.permissions.effective_permission_profile();
    let workspace_roots = config.effective_workspace_roots();
    let summary =
        summarize_permission_profile(&permission_profile, &config.cwd, workspace_roots.as_slice());
    if let Some(details) = summary.strip_prefix("read-only")
        && !details.contains("(network access enabled)")
    {
        return "Read Only".to_string();
    }
    if let Some(details) = summary.strip_prefix("workspace-write")
        && !details.contains("(network access enabled)")
    {
        return "Workspace".to_string();
    }
    if permission_profile == PermissionProfile::Disabled {
        return "Full Access".to_string();
    }

    "Custom permissions".to_string()
}

pub(super) fn approval_mode_display(config: &Config) -> String {
    let approval_policy = AskForApproval::from(config.permissions.approval_policy.value());
    if approval_policy == AskForApproval::OnRequest {
        return match config.approvals_reviewer {
            crate::config_compat::types::ApprovalsReviewer::AutoReview => {
                "Approve for me".to_string()
            }
            crate::config_compat::types::ApprovalsReviewer::User => "Ask for approval".to_string(),
        };
    }

    config.permissions.approval_policy.value().to_string()
}

pub(super) fn parse_items_with_invalids<T>(
    ids: impl IntoIterator<Item = String>,
) -> (Vec<T>, Vec<String>)
where
    T: std::str::FromStr,
{
    let mut invalid = Vec::new();
    let mut invalid_seen = HashSet::new();
    let mut items = Vec::new();
    for id in ids {
        match id.parse::<T>() {
            Ok(item) => items.push(item),
            Err(_) => {
                if invalid_seen.insert(id.clone()) {
                    invalid.push(format!(r#""{id}""#));
                }
            }
        }
    }
    (items, invalid)
}
