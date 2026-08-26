//! reflect_app_server_client crate 的桩实现。

pub const DEFAULT_IN_PROCESS_CHANNEL_CAPACITY: usize = 64;

use crate::utils_absolute_path::AbsolutePathBuf;

#[derive(Debug, Clone, Default)]
pub struct InProcessRequestHandle;

#[derive(Debug, Clone)]
pub enum AppServerRequestHandle {
    InProcess(InProcessRequestHandle),
}

impl Default for AppServerRequestHandle {
    fn default() -> Self {
        Self::InProcess(InProcessRequestHandle)
    }
}

#[derive(Debug, Clone, Default)]
pub enum AppServerEvent {
    #[default]
    Empty,
    ServerNotification(crate::app_server_protocol::ServerNotification),
    Disconnected {
        message: String,
    },
    Lagged,
    ServerRequest(crate::app_server_protocol::ServerRequest),
}

#[derive(Debug, Clone, Default)]
pub struct AppServerClient;

#[derive(Debug, Clone, Default)]
pub struct InProcessAppServerClient;

#[derive(Debug, Clone, Default)]
pub struct RemoteAppServerClient;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppServerPath(pub String);
impl AppServerPath {
    pub fn components(&self) -> std::path::Components<'_> {
        std::path::Path::new(&self.0).components()
    }
}

#[derive(Debug, Clone, Default)]
pub enum AppServerTarget {
    #[default]
    InProcess,
    Embedded,
    LocalDaemon {
        endpoint: RemoteAppServerEndpoint,
    },
    Remote {
        endpoint: RemoteAppServerEndpoint,
    },
}

#[derive(Debug, Clone)]
pub enum RemoteAppServerEndpoint {
    WebSocket {
        websocket_url: String,
        auth_token: Option<String>,
    },
    UnixSocket {
        socket_path: AbsolutePathBuf,
    },
}

impl RemoteAppServerEndpoint {
    pub fn websocket_url(&self) -> Option<&str> {
        match self {
            Self::WebSocket { websocket_url, .. } => Some(websocket_url.as_str()),
            _ => None,
        }
    }
}

impl std::fmt::Display for AppServerPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AppServerPath {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    pub fn from_absolute_str(path: &str) -> Self {
        Self(path.to_string())
    }

    pub fn join(&self, other: impl Into<String>) -> Self {
        let other_str = other.into();
        let mut s = self.0.clone();
        if !s.ends_with('/') && !other_str.starts_with('/') {
            s.push('/');
        }
        s.push_str(&other_str);
        Self(s)
    }

    pub fn join_string(&self, other: impl Into<String>) -> Self {
        let other_str = other.into();
        self.join(&other_str)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<std::path::Path> for AppServerPath {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.0)
    }
}

impl std::ops::Deref for AppServerPath {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<std::path::PathBuf> for AppServerPath {
    fn from(p: std::path::PathBuf) -> Self {
        Self(p.to_string_lossy().into_owned())
    }
}

#[derive(Debug, Clone, Default)]
pub struct InProcessClientStartArgs {
    pub arg0_paths: Arg0DispatchPaths,
    pub config: std::sync::Arc<crate::tui_core::legacy_core::config::Config>,
    pub strict_config: bool,
    pub session_source: String,
    pub client_name: String,
    pub client_version: String,
    pub experimental_api: bool,
    pub mcp_server_openai_form_elicitation: bool,
    pub channel_capacity: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Arg0DispatchPaths;

#[derive(Debug, Clone, Default)]
pub struct EnvironmentManager;

impl EnvironmentManager {
    pub fn default_for_tests() -> Self {
        Self::default()
    }
}

impl InProcessAppServerClient {
    pub async fn start(_args: InProcessClientStartArgs) -> color_eyre::Result<Self> {
        Ok(Self::default())
    }

    pub fn request_handle(&self) -> AppServerRequestHandle {
        AppServerRequestHandle::default()
    }
}

impl AppServerRequestHandle {
    pub async fn request_typed<T>(
        &self,
        _request: crate::app_server_protocol::ClientRequest,
    ) -> color_eyre::eyre::Result<T> {
        Err(color_eyre::eyre::eyre!("not implemented"))
    }
}
