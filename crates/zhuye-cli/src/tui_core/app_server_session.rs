//! app_server_session 模块的桩。

use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct AppServerSession;

#[derive(Debug, Clone, Default)]
pub struct AppServerStartedThread;

impl AppServerSession {
    pub async fn fs_read_file_path<P: AsRef<Path>>(
        &mut self,
        _path: P,
    ) -> std::io::Result<Vec<u8>> {
        Ok(Vec::new())
    }
    pub async fn fs_write_file_path<P: AsRef<Path>>(
        &mut self,
        _path: P,
        _data: &[u8],
    ) -> std::io::Result<()> {
        Ok(())
    }
    pub async fn fs_create_directory_all_path<P: AsRef<Path>>(
        &mut self,
        _path: P,
    ) -> std::io::Result<()> {
        Ok(())
    }
    pub async fn fs_remove_path<P: AsRef<Path>>(&mut self, _path: P) -> std::io::Result<()> {
        Ok(())
    }
    pub async fn next_event(&mut self) -> Option<crate::app_server_client::AppServerEvent> {
        None
    }
}
