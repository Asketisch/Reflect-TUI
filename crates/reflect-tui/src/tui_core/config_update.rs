//! config_update 模块的桩。

#[derive(Debug, Clone, Default)]
pub struct ConfigUpdate;

/// write_trusted_project 的桩。
pub async fn write_trusted_project(
    _handle: crate::app_server_client::AppServerRequestHandle,
    _path: &crate::utils_absolute_path::AbsolutePathBuf,
) -> color_eyre::eyre::Result<()> {
    Ok(())
}

/// format_config_error 的桩。
pub fn format_config_error(_error: &(impl std::fmt::Debug + ?Sized)) -> String {
    "config error".to_string()
}
