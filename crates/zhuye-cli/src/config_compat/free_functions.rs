use super::*;

/// 为面向人类的诊断信息渲染 [`ConfigLayerSource`]，与上游签名 `(source, config_toml_file)` 一致。
pub fn format_config_layer_source(source: &ConfigLayerSource, config_toml_file: &str) -> String {
    match source {
        ConfigLayerSource::Mdm { domain, key } => format!("MDM ({domain}:{key})"),
        ConfigLayerSource::System { file } => format!("system ({})", file.display()),
        ConfigLayerSource::EnterpriseManaged { id, name } => {
            format!("enterprise-managed ({name}, {id})")
        }
        ConfigLayerSource::User { file, .. } => format!("user ({})", file.display()),
        ConfigLayerSource::Project { dot_reflect_folder } => {
            format!(
                "project ({}/{config_toml_file})",
                dot_reflect_folder.display()
            )
        }
        ConfigLayerSource::SessionFlags => "session-flags".to_string(),
        ConfigLayerSource::LegacyManagedConfigTomlFromFile { file } => {
            format!("legacy managed_config.toml ({})", file.display())
        }
        ConfigLayerSource::LegacyManagedConfigTomlFromMdm => {
            "legacy managed_config.toml (MDM)".to_string()
        }
    }
}

/// 针对 [`ConfigLoadError`] 的尽力而为的渲染器。存根错误不携带载荷，因此这里只生成一个稳定的占位字符串。
pub fn format_config_error(error: &ConfigLoadError) -> String {
    format!("{error}")
}

/// 将权限配置文件（在此存根中以不透明字符串表示）映射到它所隐含的沙箱模式。
/// 与上游签名一致，使内置的 debug-config 测试可以调用它；目前非测试调用方不依赖它。
pub fn sandbox_mode_requirement_for_permission_profile(
    _permission_profile: &str,
) -> SandboxModeRequirement {
    SandboxModeRequirement::WorkspaceWrite
}
