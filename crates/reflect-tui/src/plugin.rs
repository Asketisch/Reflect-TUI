//! reflect_plugin crate 的桩实现。

#[derive(Debug, Clone, Default)]
pub struct AppConnectorId(pub String);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginCapabilitySummary {
    pub config_name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub app_connector_ids: Vec<String>,
    pub mcp_server_names: Vec<String>,
    pub has_skills: bool,
    pub install_url: Option<String>,
}

impl PluginCapabilitySummary {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            config_name: name.clone(),
            display_name: name,
            ..Default::default()
        }
    }
}
