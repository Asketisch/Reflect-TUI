//! 为 TUI 的 `/ide` 支持获取 IDE 上下文的私有传输层。

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

#[cfg(any(unix, windows))]
use serde_json::Value;
#[cfg(any(unix, windows, test))]
use serde_json::json;
use thiserror::Error;

use super::IdeContext;

// 桌面 IPC 客户端给请求 5 秒完成时间，此处沿用该提示期预算：
// 获取 IDE 上下文包含路由发现与扩展事件循环等工作，若 TUI 截止时间更短，
// 即使 IDE 正常应答，也可能错误地跳过上下文。
const IDE_CONTEXT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(any(unix, windows))]
const MAX_IPC_FRAME_BYTES: usize = 256 * 1024 * 1024;
#[cfg(any(unix, windows))]
const TUI_SOURCE_CLIENT_ID: &str = "reflect-tui";
#[cfg(any(unix, windows))]
const OPEN_IDE_HINT: &str = "Open this project in your IDE with the Reflect extension active.";
#[cfg(any(unix, windows))]
const IDE_DID_NOT_PROVIDE_CONTEXT_HINT: &str = "The IDE extension did not provide context.";
#[cfg(any(unix, windows))]
const KEEP_TRYING_HINT: &str = "Reflect will keep trying on future messages.";

#[derive(Debug, Error)]
pub(crate) enum IdeContextError {
    #[cfg(any(unix, windows))]
    #[error("failed to connect to IDE context provider: {0}")]
    Connect(std::io::Error),
    #[cfg(any(unix, windows))]
    #[error("failed to request IDE context: {0}")]
    Send(std::io::Error),
    #[cfg(any(unix, windows))]
    #[error("failed to read IDE context: {0}")]
    Read(std::io::Error),
    #[cfg(any(unix, windows))]
    #[error("invalid IDE context response: {0}")]
    InvalidResponse(String),
    #[cfg(any(unix, windows))]
    #[error("IDE context response exceeded maximum size")]
    ResponseTooLarge,
    #[cfg(any(unix, windows))]
    #[error("IDE context request failed")]
    RequestFailed(String),
    #[cfg(not(any(unix, windows)))]
    #[error("IDE context is not supported on this platform")]
    UnsupportedPlatform,
}

impl IdeContextError {
    #[cfg(any(unix, windows))]
    pub(crate) fn user_facing_hint(&self) -> String {
        match self {
            IdeContextError::Connect(_) => OPEN_IDE_HINT.to_string(),
            IdeContextError::RequestFailed(error) if error == "no-client-found" => {
                OPEN_IDE_HINT.to_string()
            }
            IdeContextError::RequestFailed(_) => {
                format!("{IDE_DID_NOT_PROVIDE_CONTEXT_HINT} Try /ide again.")
            }
            IdeContextError::ResponseTooLarge => {
                "The selected IDE context is too large. Clear any large selection in your IDE and try /ide again.".to_string()
            }
            IdeContextError::Send(_) => {
                "Reflect could not request IDE context. Try /ide again.".to_string()
            }
            IdeContextError::Read(_) | IdeContextError::InvalidResponse(_) => {
                "Reflect could not read IDE context. Try /ide again.".to_string()
            }
        }
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn prompt_skip_hint(&self) -> String {
        match self {
            IdeContextError::ResponseTooLarge => {
                "The selected IDE context is too large. Clear any large selection in your IDE."
                    .to_string()
            }
            IdeContextError::Connect(_) => OPEN_IDE_HINT.to_string(),
            IdeContextError::RequestFailed(error) if error == "no-client-found" => {
                OPEN_IDE_HINT.to_string()
            }
            IdeContextError::Read(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                "Reflect timed out waiting for IDE context. It will keep trying on future messages."
                    .to_string()
            }
            IdeContextError::RequestFailed(error) if error == "client-disconnected" => {
                hint_with_retry("The IDE connection changed while Reflect was requesting context.")
            }
            IdeContextError::RequestFailed(error) if error == "request-timeout" => {
                hint_with_retry("The IDE extension did not answer in time.")
            }
            IdeContextError::RequestFailed(error) if error == "request-version-mismatch" => {
                "The connected IDE extension is not compatible with this IDE context request."
                    .to_string()
            }
            IdeContextError::RequestFailed(error) if error == "no-handler-for-request" => {
                "The connected IDE client does not support IDE context requests.".to_string()
            }
            IdeContextError::Send(_) => {
                hint_with_retry("Reflect lost the IDE connection while requesting context.")
            }
            IdeContextError::InvalidResponse(_) => {
                hint_with_retry("Reflect received an unexpected IDE context response.")
            }
            IdeContextError::RequestFailed(_) => hint_with_retry(IDE_DID_NOT_PROVIDE_CONTEXT_HINT),
            IdeContextError::Read(_) => hint_with_retry("Reflect could not read IDE context."),
        }
    }

    #[cfg(not(any(unix, windows)))]
    pub(crate) fn user_facing_hint(&self) -> String {
        self.to_string()
    }

    #[cfg(not(any(unix, windows)))]
    pub(crate) fn prompt_skip_hint(&self) -> String {
        self.to_string()
    }
}

#[cfg(any(unix, windows))]
fn hint_with_retry(message: &str) -> String {
    format!("{message} {KEEP_TRYING_HINT}")
}

// ── Unix socket 底层连接/校验（外移子模块） ──
#[cfg(unix)]
mod unix_socket;
#[cfg(unix)]
use unix_socket::*;

#[cfg(unix)]
type IdeContextStream = UnixDeadlineStream;

#[cfg(windows)]
type IdeContextStream = super::windows_pipe::WindowsPipeStream;

#[cfg(unix)]
pub(crate) fn fetch_ide_context(
    workspace_root: &Path,
    reflect_home: &Path,
) -> Result<IdeContext, IdeContextError> {
    let deadline = Instant::now() + IDE_CONTEXT_REQUEST_TIMEOUT;
    let primary_socket_path = primary_ipc_socket_path(reflect_home);
    let uid = unsafe { libc::getuid() };
    let legacy_socket_paths = legacy_ipc_socket_paths(&std::env::temp_dir(), uid);
    fetch_ide_context_from_unix_socket_paths(
        primary_socket_path,
        legacy_socket_paths,
        workspace_root,
        deadline,
    )
}

#[cfg(windows)]
pub(crate) fn fetch_ide_context(
    workspace_root: &Path,
    _reflect_home: &Path,
) -> Result<IdeContext, IdeContextError> {
    fetch_ide_context_from_socket(
        default_ipc_socket_path(),
        workspace_root,
        IDE_CONTEXT_REQUEST_TIMEOUT,
    )
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn fetch_ide_context(
    _workspace_root: &Path,
    _reflect_home: &Path,
) -> Result<IdeContext, IdeContextError> {
    Err(IdeContextError::UnsupportedPlatform)
}

#[cfg(unix)]
fn primary_ipc_socket_path(reflect_home: &Path) -> PathBuf {
    reflect_home.join("ipc").join("ipc.sock")
}

#[cfg(unix)]
fn legacy_ipc_socket_paths(temp_dir: &Path, uid: libc::uid_t) -> Vec<PathBuf> {
    let ipc_dir = temp_dir.join("reflect-ipc");
    if uid == 0 {
        vec![ipc_dir.join("ipc.sock"), ipc_dir.join("ipc-0.sock")]
    } else {
        vec![ipc_dir.join(format!("ipc-{uid}.sock"))]
    }
}

#[cfg(windows)]
fn default_ipc_socket_path() -> PathBuf {
    PathBuf::from(r"\\.\pipe\reflect-ipc")
}

#[cfg(not(any(unix, windows)))]
fn default_ipc_socket_path() -> PathBuf {
    PathBuf::new()
}

#[cfg(windows)]
fn fetch_ide_context_from_socket(
    socket_path: PathBuf,
    workspace_root: &Path,
    timeout: Duration,
) -> Result<IdeContext, IdeContextError> {
    let deadline = Instant::now() + timeout;
    let mut stream = connect_stream(socket_path, deadline)?;
    fetch_ide_context_from_stream(&mut stream, workspace_root, deadline)
}

#[cfg(unix)]
fn fetch_ide_context_from_unix_socket_paths(
    primary_socket_path: PathBuf,
    legacy_socket_paths: Vec<PathBuf>,
    workspace_root: &Path,
    deadline: Instant,
) -> Result<IdeContext, IdeContextError> {
    let mut last_error = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no IDE IPC socket paths were available",
    );
    let mut stream = None;
    for socket_path in std::iter::once(primary_socket_path).chain(legacy_socket_paths) {
        match UnixDeadlineStream::connect(socket_path, deadline) {
            Ok(connected) => {
                stream = Some(connected);
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
                return Err(IdeContextError::Connect(err));
            }
            Err(err) if Instant::now() >= deadline => {
                return Err(IdeContextError::Connect(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("IDE IPC connection exhausted the request deadline: {err}"),
                )));
            }
            Err(err) => last_error = err,
        }
    }
    let mut stream = stream.ok_or(IdeContextError::Connect(last_error))?;
    fetch_ide_context_from_stream(&mut stream, workspace_root, deadline)
}

#[cfg(unix)]
#[cfg(windows)]

fn connect_stream(
    socket_path: PathBuf,
    deadline: Instant,
) -> Result<IdeContextStream, IdeContextError> {
    super::windows_pipe::WindowsPipeStream::connect(socket_path, deadline)
        .map_err(IdeContextError::Connect)
}

#[cfg(any(unix, windows))]
fn answer_unsupported_request<T: std::io::Write + ?Sized>(
    stream: &mut T,
    message: &Value,
) -> Result<(), IdeContextError> {
    if let Some(inbound_request_id) = message.get("requestId").and_then(Value::as_str) {
        let response = json!({
            "type": "response",
            "requestId": inbound_request_id,
            "resultType": "error",
            "error": "no-handler-for-request",
        });
        write_frame(stream, &response).map_err(IdeContextError::Send)?;
    }
    Ok(())
}

#[cfg(any(unix, windows))]
fn fetch_ide_context_from_stream(
    stream: &mut IdeContextStream,
    workspace_root: &Path,
    deadline: Instant,
) -> Result<IdeContext, IdeContextError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    write_ide_context_request(stream, &request_id, workspace_root)
        .map_err(IdeContextError::Send)?;
    let response = read_response_frame(stream, &request_id, deadline)?;
    extract_ide_context(response)
}

#[cfg(any(unix, windows))]
fn write_ide_context_request<T: std::io::Write + ?Sized>(
    stream: &mut T,
    request_id: &str,
    workspace_root: &Path,
) -> std::io::Result<()> {
    let ide_context_request = json!({
        "type": "request",
        "requestId": request_id,
        "sourceClientId": TUI_SOURCE_CLIENT_ID,
        "version": 0,
        "method": "ide-context",
        "params": {
            "workspaceRoot": workspace_root.to_string_lossy(),
        },
    });
    write_frame(stream, &ide_context_request)
}

#[cfg(any(unix, windows))]
fn write_frame<T: std::io::Write + ?Sized>(stream: &mut T, message: &Value) -> std::io::Result<()> {
    let payload = serde_json::to_vec(message).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid IDE context JSON message: {err}"),
        )
    })?;
    let payload_len = u32::try_from(payload.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "IDE context payload exceeds u32 length",
        )
    })?;
    stream.write_all(&payload_len.to_le_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()
}

#[cfg(any(unix, windows))]
fn read_frame<T: std::io::Read + ?Sized>(
    stream: &mut T,
    deadline: Instant,
) -> Result<Value, IdeContextError> {
    let mut len_bytes = [0_u8; 4];
    read_exact_before_deadline(stream, &mut len_bytes, deadline)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len > MAX_IPC_FRAME_BYTES {
        return Err(IdeContextError::ResponseTooLarge);
    }

    let mut payload = vec![0_u8; len];
    read_exact_before_deadline(stream, &mut payload, deadline)?;
    serde_json::from_slice(&payload)
        .map_err(|err| IdeContextError::InvalidResponse(format!("invalid JSON payload: {err}")))
}

#[cfg(any(unix, windows))]
fn read_exact_before_deadline<T: std::io::Read + ?Sized>(
    stream: &mut T,
    buf: &mut [u8],
    deadline: Instant,
) -> Result<(), IdeContextError> {
    // std::io::Read::read_exact 无法在多次部分读取之间感知我们的请求截止时间。
    // 让帧头与载荷与外围的响应等待共用同一预算。
    let mut read_so_far = 0;
    while read_so_far < buf.len() {
        ensure_deadline_not_expired(deadline)?;
        match stream.read(&mut buf[read_so_far..]) {
            Ok(0) => {
                return Err(IdeContextError::Read(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "failed to fill whole IDE context frame",
                )));
            }
            Ok(bytes_read) => {
                read_so_far += bytes_read;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(IdeContextError::Read(error)),
        }
    }

    ensure_deadline_not_expired(deadline)
}

#[cfg(any(unix, windows))]
fn read_response_frame(
    stream: &mut IdeContextStream,
    request_id: &str,
    deadline: Instant,
) -> Result<Value, IdeContextError> {
    loop {
        ensure_deadline_not_expired(deadline)?;
        stream.set_deadline(deadline);
        let message = read_frame(stream, deadline)?;
        match message.get("type").and_then(Value::as_str) {
            Some("response") => {
                if message.get("requestId").and_then(Value::as_str) == Some(request_id) {
                    return Ok(message);
                }
            }
            Some("broadcast") => {}
            Some("client-discovery-request") => {
                if let Some(discovery_request_id) = message.get("requestId").and_then(Value::as_str)
                {
                    let response = json!({
                        "type": "client-discovery-response",
                        "requestId": discovery_request_id,
                        "response": {
                            "canHandle": false,
                        },
                    });
                    write_frame(stream, &response).map_err(IdeContextError::Send)?;
                }
            }
            Some("client-discovery-response") => {}
            Some("request") => {
                answer_unsupported_request(stream, &message)?;
            }
            Some(other) => {
                return Err(IdeContextError::InvalidResponse(format!(
                    "unexpected IDE context message type: {other}"
                )));
            }
            None => {
                return Err(IdeContextError::InvalidResponse(
                    "IDE context message did not include a type".to_string(),
                ));
            }
        }
    }
}

#[cfg(any(unix, windows))]
fn ensure_deadline_not_expired(deadline: Instant) -> Result<(), IdeContextError> {
    if Instant::now() >= deadline {
        return Err(timeout_error());
    }

    Ok(())
}

#[cfg(any(unix, windows))]
fn timeout_error() -> IdeContextError {
    IdeContextError::Read(deadline_timeout_io_error())
}

#[cfg(any(unix, windows))]
fn deadline_timeout_io_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "timed out waiting for IDE context",
    )
}

#[cfg(unix)]
fn permission_denied_io_error(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, message)
}

#[cfg(any(unix, windows))]
fn extract_ide_context(response: Value) -> Result<IdeContext, IdeContextError> {
    ensure_success_response(&response)?;
    let ide_context = response
        .get("result")
        .and_then(|result| result.get("ideContext"))
        .cloned()
        .ok_or_else(|| {
            IdeContextError::InvalidResponse(
                "ide-context response did not include result.ideContext".to_string(),
            )
        })?;
    serde_json::from_value(ide_context)
        .map_err(|err| IdeContextError::InvalidResponse(err.to_string()))
}

#[cfg(any(unix, windows))]
fn ensure_success_response(response: &Value) -> Result<(), IdeContextError> {
    match response.get("resultType").and_then(Value::as_str) {
        Some("success") => Ok(()),
        Some("error") => Err(IdeContextError::RequestFailed(
            response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
        )),
        _ => Err(IdeContextError::InvalidResponse(
            "response did not include a success or error resultType".to_string(),
        )),
    }
}

#[cfg(all(test, unix))]
#[cfg(all(test, unix))]
mod tests;
