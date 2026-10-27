//! 插件目录 marketplace tab-id 与 remote-section 构建辅助函数簇。从 plugin_catalog.rs 抽出。

use super::*;

pub(super) fn remote_section_marketplace_tab_id(
    saved_tab_id: &str,
    marketplaces: &[PluginMarketplaceEntry],
) -> Option<String> {
    let section = REMOTE_MARKETPLACE_SECTIONS
        .into_iter()
        .find(|section| section.is_fallback_tab_id(saved_tab_id))?;

    section
        .marketplace_names
        .iter()
        .find_map(|marketplace_name| {
            marketplaces
                .iter()
                .find(|marketplace| marketplace.name.as_str() == *marketplace_name)
                .map(marketplace_tab_id)
        })
}

pub(super) fn plugin_tab_id_matching_saved_id(
    saved_tab_id: &str,
    tabs: &[SelectionTab],
) -> Option<String> {
    if let Some(tab_id) = tabs
        .iter()
        .find(|tab| tab.id.as_str() == saved_tab_id)
        .map(|tab| tab.id.clone())
    {
        return Some(tab_id);
    }

    let section = REMOTE_MARKETPLACE_SECTIONS
        .into_iter()
        .find(|section| section.contains_tab_id(saved_tab_id))?;

    tabs.iter()
        .find(|tab| section.contains_tab_id(&tab.id))
        .map(|tab| tab.id.clone())
}

pub(crate) fn merge_remote_marketplaces(
    response: &mut PluginListResponse,
    remote_marketplaces: Vec<PluginMarketplaceEntry>,
) {
    let remote_names = remote_marketplaces
        .iter()
        .map(|marketplace| marketplace.name.clone())
        .collect::<std::collections::HashSet<_>>();
    let remote_curated_present = remote_names.contains(REMOTE_GLOBAL_MARKETPLACE_NAME);
    response.marketplaces.retain(|marketplace| {
        if remote_curated_present
            && marketplace.path.is_some()
            && is_reflect_curated_marketplace_name(&marketplace.name)
        {
            return false;
        }

        marketplace.path.is_some()
            || !REMOTE_MARKETPLACE_SECTIONS
                .into_iter()
                .any(|section| section.contains_marketplace(&marketplace.name))
                && !remote_names.contains(marketplace.name.as_str())
    });
    response.marketplaces.extend(remote_marketplaces);
}

pub(super) fn is_personal_marketplace_path(marketplace_path: &std::path::Path) -> bool {
    dirs::home_dir()
        .and_then(|home| {
            AbsolutePathBuf::try_from(home.join(PERSONAL_MARKETPLACE_RELATIVE_PATH)).ok()
        })
        .is_some_and(|personal_path| {
            personal_path
                == crate::utils_absolute_path::AbsolutePathBuf::from(marketplace_path.to_path_buf())
        })
}

pub(super) fn remote_section_loading_item(label: &str, description: &str) -> SelectionItem {
    SelectionItem {
        name: format!("Loading {label} plugins..."),
        description: Some(description.to_string()),
        is_disabled: true,
        ..Default::default()
    }
}

pub(super) fn remote_section_error_item(label: &str, message: &str) -> SelectionItem {
    SelectionItem {
        name: format!("{label} unavailable"),
        description: Some(message.to_string()),
        is_disabled: true,
        ..Default::default()
    }
}

pub(super) fn plugin_remote_section_error<'a>(
    section_errors: &'a [PluginRemoteSectionError],
    section_id: &str,
) -> Option<&'a PluginRemoteSectionError> {
    section_errors
        .iter()
        .find(|section_error| section_error.section_id == section_id)
}

pub(super) fn remote_section_loading_tab(
    id: &str,
    label: &str,
    item_description: &str,
) -> SelectionTab {
    SelectionTab {
        id: format!("{REMOTE_LOADING_TAB_ID_PREFIX}{id}"),
        label: label.to_string(),
        header: plugins_header(
            format!("Loading {label} plugins."),
            "Local plugin functionality is already available.".to_string(),
        ),
        items: vec![remote_section_loading_item(label, item_description)],
    }
}

pub(super) fn remote_section_empty_tab(
    id: &str,
    label: &str,
    item_name: &str,
    item_description: &str,
) -> SelectionTab {
    SelectionTab {
        id: format!("{REMOTE_EMPTY_TAB_ID_PREFIX}{id}"),
        label: label.to_string(),
        header: plugins_header(
            format!("{label}."),
            "This section loaded successfully.".to_string(),
        ),
        items: vec![SelectionItem {
            name: item_name.to_string(),
            description: Some(item_description.to_string()),
            is_disabled: true,
            ..Default::default()
        }],
    }
}

pub(super) fn remote_section_error_tab(section_error: &PluginRemoteSectionError) -> SelectionTab {
    SelectionTab {
        id: format!("{REMOTE_ERROR_TAB_ID_PREFIX}{}", section_error.section_id),
        label: section_error.label.clone(),
        header: plugins_header(
            format!("{} unavailable.", section_error.label),
            "Local plugin functionality is still available.".to_string(),
        ),
        items: vec![remote_section_error_item(
            &section_error.label,
            &section_error.message,
        )],
    }
}
