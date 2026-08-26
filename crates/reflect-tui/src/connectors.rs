//! reflect_connectors crate 的桩代码。

#[derive(Debug, Clone, Default)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub slug: String,
    pub description: Option<String>,
    pub title: String,
    pub config_name: String,
    pub is_enabled: bool,
    pub is_accessible: bool,
    pub category: String,
    pub version: String,
    pub publisher: String,
    pub install_url: Option<String>,
}

impl AppInfo {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            display_name: name.clone(),
            slug: name.to_lowercase().replace(' ', "-"),
            ..Default::default()
        }
    }
}

pub mod metadata {
    use super::AppInfo;

    pub fn connector_display_label(connector: &AppInfo) -> String {
        if connector.display_name.is_empty() {
            connector.name.clone()
        } else {
            connector.display_name.clone()
        }
    }

    pub fn connector_mention_slug(connector: &AppInfo) -> String {
        if connector.slug.is_empty() {
            connector.name.to_lowercase().replace(' ', "-")
        } else {
            connector.slug.clone()
        }
    }

    pub fn connector_mention_slug_from_name(name: &str) -> String {
        name.to_lowercase().replace(' ', "-")
    }
}

/// 顶层便捷函数。
pub fn connector_mention_slug_from_name(name: &str) -> String {
    metadata::connector_mention_slug_from_name(name)
}
