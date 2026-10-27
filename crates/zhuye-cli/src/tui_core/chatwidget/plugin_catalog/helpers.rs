//! 插件目录纯函数辅助。从 plugin_catalog/mod.rs 抽出。

use std::collections::HashMap;
use std::path::Path;

use super::display::*;
use super::*;
use crate::app_server_protocol::PluginInstallPolicy;
use crate::app_server_protocol::PluginMarketplaceEntry;
use crate::app_server_protocol::PluginSource;
use crate::app_server_protocol::PluginSummary;
use crate::tui_core::render::renderable::{ColumnRenderable, Renderable};
use ratatui::text::Line;

// ── 提示行构建 ──

pub(super) fn plugins_popup_hint_line(
    can_remove_marketplace: bool,
    can_upgrade_marketplace: bool,
) -> Line<'static> {
    match (can_remove_marketplace, can_upgrade_marketplace) {
        (true, true) => Line::from(
            "ctrl + u upgrade · ctrl + r remove · space toggle · ←/→ tabs · enter details · esc close",
        ),
        (true, false) => {
            Line::from("ctrl + r remove · space toggle · ←/→ tabs · enter details · esc close")
        }
        (false, true) => {
            Line::from("ctrl + u upgrade · space toggle · ←/→ tabs · enter details · esc close")
        }
        (false, false) => Line::from(
            "space enable/disable · ←/→ select marketplace · enter view details · esc close",
        ),
    }
}

pub(crate) fn plugin_detail_hint_line() -> Line<'static> {
    Line::from("Press esc to close.")
}

pub(super) fn plugins_header(subtitle: String, count_line: String) -> Box<dyn Renderable> {
    let mut header = ColumnRenderable::new();
    header.push(Line::from("Plugins".bold()));
    header.push(Line::from(subtitle.dim()));
    header.push(Line::from(count_line.dim()));
    Box::new(header)
}

// ── 插件条目去重与排序 ──

pub(super) fn dedupe_plugin_entries<'a>(
    entries: Vec<(&'a PluginMarketplaceEntry, &'a PluginSummary, String)>,
) -> Vec<(&'a PluginMarketplaceEntry, &'a PluginSummary, String)> {
    let mut deduped: Vec<(&PluginMarketplaceEntry, &PluginSummary, String)> = Vec::new();
    let mut remote_entry_indexes = HashMap::new();
    for entry in entries {
        let Some(remote_plugin_id) = plugin_remote_identity(entry.1) else {
            deduped.push(entry);
            continue;
        };
        if let Some(existing_index) = remote_entry_indexes.get(&remote_plugin_id).copied() {
            if plugin_entry_preferred(&entry, &deduped[existing_index]) {
                deduped[existing_index] = entry;
            }
        } else {
            remote_entry_indexes.insert(remote_plugin_id, deduped.len());
            deduped.push(entry);
        }
    }
    deduped
}

pub(super) fn plugin_entry_preferred(
    candidate: &(&PluginMarketplaceEntry, &PluginSummary, String),
    existing: &(&PluginMarketplaceEntry, &PluginSummary, String),
) -> bool {
    if candidate.1.installed != existing.1.installed {
        return candidate.1.installed;
    }

    let candidate_is_admin_managed =
        candidate.1.install_policy == PluginInstallPolicy::InstalledByDefault;
    let existing_is_admin_managed =
        existing.1.install_policy == PluginInstallPolicy::InstalledByDefault;
    if candidate_is_admin_managed != existing_is_admin_managed {
        return candidate_is_admin_managed;
    }

    let candidate_is_local_share =
        candidate.1.share_context.is_some() && !matches!(&candidate.1.source, PluginSource::Remote);
    let existing_is_local_share =
        existing.1.share_context.is_some() && !matches!(&existing.1.source, PluginSource::Remote);
    if candidate_is_local_share != existing_is_local_share {
        return candidate_is_local_share;
    }

    !matches!(&candidate.1.source, PluginSource::Remote)
        && matches!(&existing.1.source, PluginSource::Remote)
}

pub(super) fn preferred_local_plugin_sources(
    marketplaces: &[PluginMarketplaceEntry],
) -> HashMap<String, PreferredLocalPluginSource> {
    let mut sources = HashMap::new();
    for marketplace in marketplaces {
        let Some(marketplace_path) = marketplace.path.as_ref() else {
            continue;
        };
        for plugin in &marketplace.plugins {
            if matches!(&plugin.source, PluginSource::Remote) {
                continue;
            }
            let Some(share_context) = plugin.share_context.as_ref() else {
                continue;
            };
            sources
                .entry(share_context.remote_plugin_id.clone())
                .or_insert_with(|| PreferredLocalPluginSource {
                    marketplace_path: AbsolutePathBuf::from(marketplace_path.to_path_buf()),
                    plugin_name: plugin.name.clone(),
                    installed: plugin.installed,
                    install_policy: plugin.install_policy,
                });
        }
    }
    sources
}

pub(super) fn plugin_entries_for_marketplaces<'a>(
    marketplaces: impl IntoIterator<Item = &'a PluginMarketplaceEntry>,
) -> Vec<(&'a PluginMarketplaceEntry, &'a PluginSummary, String)> {
    let entries = marketplaces
        .into_iter()
        .flat_map(|marketplace| {
            marketplace
                .plugins
                .iter()
                .map(move |plugin| (marketplace, plugin, plugin_display_name(plugin)))
        })
        .collect::<Vec<_>>();
    dedupe_plugin_entries(entries)
}

pub(super) fn sort_plugin_entries(
    entries: &mut [(&PluginMarketplaceEntry, &PluginSummary, String)],
) {
    entries.sort_by(|left, right| {
        right
            .1
            .installed
            .cmp(&left.1.installed)
            .then_with(|| {
                left.2
                    .to_ascii_lowercase()
                    .cmp(&right.2.to_ascii_lowercase())
            })
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.1.name.cmp(&right.1.name))
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
}

pub(crate) fn marketplace_tab_id(marketplace: &PluginMarketplaceEntry) -> String {
    match marketplace.path.as_ref() {
        Some(path) => marketplace_tab_id_from_path(path.as_path()),
        None => format!("marketplace:{}", marketplace.name),
    }
}

pub(crate) fn marketplace_tab_id_from_path(path: &Path) -> String {
    format!("{MARKETPLACE_TAB_ID_PREFIX}{}", path.display())
}

pub(crate) fn marketplace_tab_id_matching_saved_id(
    saved_tab_id: &str,
    marketplaces: &[PluginMarketplaceEntry],
) -> Option<String> {
    if let Some(tab_id) = remote_section_marketplace_tab_id(saved_tab_id, marketplaces) {
        return Some(tab_id);
    }

    if let Some(tab_id) = marketplaces.iter().find_map(|marketplace| {
        let tab_id = marketplace_tab_id(marketplace);
        (tab_id == saved_tab_id).then_some(tab_id)
    }) {
        return Some(tab_id);
    }

    let root = saved_tab_id.strip_prefix(MARKETPLACE_TAB_ID_PREFIX)?;
    if root.is_empty() {
        return None;
    }
    let root = Path::new(root);
    marketplaces.iter().find_map(|marketplace| {
        marketplace
            .path
            .as_ref()
            .is_some_and(|path| path.as_path().starts_with(root))
            .then(|| marketplace_tab_id(marketplace))
    })
}

pub(super) fn disambiguate_duplicate_tab_labels(labels: Vec<String>) -> Vec<String> {
    let mut counts = HashMap::new();
    for label in &labels {
        *counts.entry(label.clone()).or_insert(0) += 1;
    }

    let mut seen = HashMap::new();
    labels
        .into_iter()
        .map(|label| {
            let total = counts[&label];
            if total == 1 {
                return label;
            }

            let current = seen.entry(label.clone()).or_insert(0);
            *current += 1;
            format!("{label} ({current}/{total})")
        })
        .collect()
}

pub(crate) fn marketplace_display_name(marketplace: &PluginMarketplaceEntry) -> String {
    if let Some(label) = MarketplaceProduct::from_marketplace(marketplace).label() {
        return label.to_string();
    }
    marketplace
        .interface
        .as_ref()
        .and_then(|interface| interface.display_name.as_deref())
        .map(str::trim)
        .filter(|display_name| !display_name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| marketplace.name.clone())
}

pub(crate) fn marketplace_is_user_configured(config: &Config, marketplace_name: &str) -> bool {
    let Some(user_config) = config.config_layer_stack.effective_user_config() else {
        return false;
    };
    user_config
        .get("marketplaces")
        .and_then(toml::Value::as_table)
        .is_some_and(|marketplaces| marketplaces.contains_key(marketplace_name))
}

pub(crate) fn marketplace_is_user_configured_git(config: &Config, marketplace_name: &str) -> bool {
    config
        .config_layer_stack
        .get_active_user_layer()
        .and_then(|user_layer| user_layer.config.get("marketplaces"))
        .and_then(toml::Value::as_table)
        .and_then(|marketplaces| marketplaces.get(marketplace_name))
        .and_then(toml::Value::as_table)
        .and_then(|marketplace| marketplace.get("source_type"))
        .and_then(toml::Value::as_str)
        .is_some_and(|source_type| source_type == "git")
}
