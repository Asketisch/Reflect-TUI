//! 插件目录展示/标签辅助函数簇。从 plugin_catalog.rs 抽出。

use super::*;

pub(super) fn plugin_detail_status_label(plugin: &PluginSummary) -> &'static str {
    if plugin.availability == PluginAvailability::DisabledByAdmin {
        return "Disabled by admin";
    }
    if plugin.install_policy == PluginInstallPolicy::InstalledByDefault {
        return if plugin.installed {
            "Installed by admin"
        } else {
            "Enabled by Admin"
        };
    }
    if plugin.installed {
        if plugin.enabled {
            "Installed"
        } else {
            "Disabled"
        }
    } else {
        match plugin.install_policy {
            PluginInstallPolicy::NotAvailable => "Not installable",
            PluginInstallPolicy::Available => "Can be installed",
            PluginInstallPolicy::InstalledByDefault => "Installed by admin",
        }
    }
}

pub(super) fn plugin_metadata_items(plugin: &PluginDetail) -> Vec<SelectionItem> {
    let mut items = Vec::new();
    items.push(SelectionItem {
        name: "Source".to_string(),
        description: Some(plugin_source_summary(plugin)),
        is_disabled: true,
        ..Default::default()
    });
    items.push(SelectionItem {
        name: "Auth".to_string(),
        description: Some(plugin_auth_policy_summary(plugin.summary.auth_policy)),
        is_disabled: true,
        ..Default::default()
    });
    if let Some(version) = plugin_version_summary(&plugin.summary) {
        items.push(SelectionItem {
            name: "Version".to_string(),
            description: Some(version),
            is_disabled: true,
            ..Default::default()
        });
    }
    if let Some(share_context) = &plugin.summary.share_context {
        items.push(SelectionItem {
            name: "Sharing".to_string(),
            description: Some(plugin_share_context_summary(share_context)),
            is_disabled: true,
            ..Default::default()
        });
    }
    items
}

pub(super) fn plugin_source_summary(plugin: &PluginDetail) -> String {
    match &plugin.summary.source {
        PluginSource::Local { .. } => "Local".to_string(),
        PluginSource::Git { url, ref_name, .. } => match ref_name {
            Some(ref_name) => format!("Git · {url}@{ref_name}"),
            None => format!("Git · {url}"),
        },
        PluginSource::Npm {
            package, version, ..
        } => match version {
            Some(version) => format!("npm · {package}@{version}"),
            None => format!("npm · {package}"),
        },
        PluginSource::Remote => {
            let marketplace_label =
                MarketplaceProduct::from_marketplace_name(&plugin.marketplace_name)
                    .label()
                    .unwrap_or(plugin.marketplace_name.as_str());
            format!("Remote · {marketplace_label}")
        }
    }
}

pub(super) fn plugin_auth_policy_summary(auth_policy: PluginAuthPolicy) -> String {
    match auth_policy {
        PluginAuthPolicy::OnInstall => "Auth on install".to_string(),
        PluginAuthPolicy::OnUse => "Auth on use".to_string(),
    }
}

pub(super) fn plugin_version_summary(plugin: &PluginSummary) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(local_version) = plugin.local_version.as_deref() {
        parts.push(format!("local {local_version}"));
    }
    if let Some(remote_version) = plugin
        .share_context
        .as_ref()
        .and_then(|context| context.remote_version.as_deref())
    {
        parts.push(format!("remote {remote_version}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(super) fn plugin_share_context_summary(context: &PluginShareContext) -> String {
    let mut parts = Vec::new();
    if let Some(discoverability) = context.discoverability {
        parts.push(plugin_share_discoverability_label(discoverability).to_string());
    }
    if let Some(creator_summary) = plugin_share_creator_summary(context) {
        parts.push(creator_summary);
    }
    if let Some(principals) = context.share_principals.as_ref() {
        parts.push(plugin_share_principals_summary(principals));
    }
    if let Some(share_url) = context
        .share_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
    {
        parts.push(share_url.to_string());
    }
    if parts.is_empty() {
        format!("Remote ID {}", context.remote_plugin_id)
    } else {
        parts.join(" · ")
    }
}

pub(super) fn plugin_share_discoverability_label(
    discoverability: PluginShareDiscoverability,
) -> &'static str {
    match discoverability {
        PluginShareDiscoverability::Listed => "Listed",
        PluginShareDiscoverability::Unlisted => "Workspace link",
        PluginShareDiscoverability::Private => "Private",
    }
}

pub(super) fn plugin_share_creator_summary(context: &PluginShareContext) -> Option<String> {
    match (
        context.creator_name.as_deref(),
        context.creator_account_user_id.as_deref(),
    ) {
        (Some(name), Some(account_id)) => Some(format!("creator {name} ({account_id})")),
        (Some(name), None) => Some(format!("creator {name}")),
        (None, Some(account_id)) => Some(format!("creator account {account_id}")),
        (None, None) => None,
    }
}

pub(super) fn plugin_share_principals_summary(principals: &[PluginSharePrincipal]) -> String {
    match principals.len() {
        0 => "No explicit principals".to_string(),
        1 => format!("1 principal: {}", principals[0].name),
        count => format!("{count} principals"),
    }
}

pub(super) fn plugin_display_name(plugin: &PluginSummary) -> String {
    plugin
        .interface
        .as_ref()
        .and_then(|interface| interface.display_name.as_deref())
        .map(str::trim)
        .filter(|display_name| !display_name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| plugin.name.clone())
}

pub(super) fn plugin_brief_description(
    plugin: &PluginSummary,
    marketplace_label: &str,
    status_label_width: usize,
) -> String {
    let status_label = plugin_status_label(plugin);
    let status_label = format!("{status_label:<status_label_width$}");
    match plugin_description(plugin) {
        Some(description) => format!("{status_label} · {marketplace_label} · {description}"),
        None => format!("{status_label} · {marketplace_label}"),
    }
}

pub(super) fn plugin_brief_description_without_marketplace(
    plugin: &PluginSummary,
    status_label_width: usize,
) -> String {
    let status_label = plugin_status_label(plugin);
    let status_label = format!("{status_label:<status_label_width$}");
    match plugin_description(plugin) {
        Some(description) => format!("{status_label} · {description}"),
        None => status_label,
    }
}

pub(super) fn plugin_status_label(plugin: &PluginSummary) -> &'static str {
    if plugin.availability == PluginAvailability::DisabledByAdmin {
        return "Disabled";
    }
    if !plugin.installed && plugin.install_policy == PluginInstallPolicy::InstalledByDefault {
        return "Admin assigned";
    }
    if plugin.installed {
        if plugin.enabled {
            "Installed"
        } else {
            "Disabled"
        }
    } else {
        match plugin.install_policy {
            PluginInstallPolicy::NotAvailable => "Not installable",
            PluginInstallPolicy::Available => "Available",
            PluginInstallPolicy::InstalledByDefault => "Installed",
        }
    }
}

pub(super) fn plugin_location_for_marketplace(
    marketplace: &PluginMarketplaceEntry,
    plugin: &PluginSummary,
) -> Option<PluginLocation> {
    if let Some(marketplace_path) = marketplace.path.clone() {
        return Some(PluginLocation::Local {
            marketplace_path: marketplace_path.into(),
        });
    }
    plugin_remote_identity(plugin).map(|_| PluginLocation::Remote {
        marketplace_name: marketplace.name.clone(),
    })
}

pub(super) fn plugin_detail_location(plugin: &PluginDetail) -> Option<PluginLocation> {
    if let Some(marketplace_path) = plugin.marketplace_path.clone() {
        return Some(PluginLocation::Local {
            marketplace_path: marketplace_path.into(),
        });
    }
    plugin_remote_identity(&plugin.summary).map(|_| PluginLocation::Remote {
        marketplace_name: plugin.marketplace_name.clone(),
    })
}

pub(super) fn plugin_detail_request_for_entry(
    marketplace: &PluginMarketplaceEntry,
    plugin: &PluginSummary,
    preferred_local_sources: &HashMap<String, PreferredLocalPluginSource>,
) -> Option<(PluginLocation, String)> {
    if matches!(&plugin.source, PluginSource::Remote)
        && let Some(remote_plugin_id) = plugin_remote_identity(plugin)
        && let Some(preferred_source) = preferred_local_sources.get(remote_plugin_id)
        && preferred_source.installed == plugin.installed
        && preferred_source.install_policy == plugin.install_policy
    {
        return Some((
            PluginLocation::Local {
                marketplace_path: preferred_source.marketplace_path.clone(),
            },
            preferred_source.plugin_name.clone(),
        ));
    }

    plugin_location_for_marketplace(marketplace, plugin)
        .map(|location| (location, plugin_request_name(plugin)))
}

pub(super) fn plugin_request_name(plugin: &PluginSummary) -> String {
    if matches!(&plugin.source, PluginSource::Remote)
        && let Some(remote_plugin_id) = plugin_remote_identity(plugin)
    {
        return remote_plugin_id.to_string();
    }
    plugin.name.clone()
}

pub(super) fn plugin_remote_identity(plugin: &PluginSummary) -> Option<&str> {
    plugin
        .share_context
        .as_ref()
        .map(|context| context.remote_plugin_id.as_str())
        .or(plugin.remote_plugin_id.as_deref())
}

pub(super) fn plugin_uninstall_id(plugin: &PluginSummary) -> Option<String> {
    if matches!(&plugin.source, PluginSource::Remote) {
        return plugin_remote_identity(plugin).map(str::to_string);
    }
    Some(plugin.id.clone())
}

pub(super) fn plugin_description(plugin: &PluginSummary) -> Option<String> {
    plugin
        .interface
        .as_ref()
        .and_then(|interface| {
            interface
                .short_description
                .as_deref()
                .or(interface.long_description.as_deref())
        })
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string)
}

pub(super) fn plugin_detail_description(plugin: &PluginDetail) -> Option<String> {
    plugin
        .description
        .as_deref()
        .or_else(|| {
            plugin
                .summary
                .interface
                .as_ref()
                .and_then(|interface| interface.long_description.as_deref())
        })
        .or_else(|| {
            plugin
                .summary
                .interface
                .as_ref()
                .and_then(|interface| interface.short_description.as_deref())
        })
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string)
}

pub(super) fn plugin_skill_summary(plugin: &PluginDetail) -> String {
    if plugin.skills.is_empty() {
        "No plugin skills.".to_string()
    } else {
        plugin
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(super) fn plugin_app_summary(plugin: &PluginDetail) -> String {
    if plugin.apps.is_empty() {
        "No plugin apps.".to_string()
    } else {
        plugin
            .apps
            .iter()
            .map(|app| app.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(super) fn plugin_hook_summary(plugin: &PluginDetail) -> String {
    if plugin.hooks.is_empty() {
        "No plugin hooks.".to_string()
    } else {
        let mut event_counts = Vec::<(crate::app_server_protocol::HookEventName, usize)>::new();
        for hook in &plugin.hooks {
            if let Some((_, handler_count)) = event_counts
                .iter_mut()
                .find(|(event_name, _)| *event_name == hook.event_name)
            {
                *handler_count += 1;
            } else {
                event_counts.push((hook.event_name, 1));
            }
        }
        event_counts
            .into_iter()
            .map(|(event_name, handler_count)| format!("{event_name:?} ({handler_count})"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(super) fn plugin_mcp_summary(plugin: &PluginDetail) -> String {
    if plugin.mcp_servers.is_empty() {
        "No plugin MCP servers.".to_string()
    } else {
        plugin.mcp_servers.join(", ")
    }
}
