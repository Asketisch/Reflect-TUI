//! 由 app-server 支撑的工作区命令执行，供 TUI 自有的后台查询使用。
//!
//! 本模块是面向非交互式命令的 TUI 边界，这些命令需要在活动工作区所在的任何位置运行。
//! 调用方以 argv、cwd、环境变量覆盖、超时和输出上限来描述命令；运行器会将该请求
//! 转换为 app-server 的 `command/exec`。将这一抽象保留在 TUI 本地，可以让状态界面
//! 无需感知当前 app-server 是内嵌的还是远程的。
//!
//! 通过此路径发送的命令不应等待 stdin 输入。大多数调用方应保持输出有界，
//! 以免元数据刷新演变成无界的后台进程；拥有完整用户可见负载的调用方
//! （例如 `/diff`）可以显式选择不受输出上限约束。

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::app_server_client::AppServerRequestHandle;
use crate::app_server_protocol::ClientRequest;
use crate::app_server_protocol::CommandExecParams;
use crate::app_server_protocol::CommandExecResponse;
use crate::app_server_protocol::RequestId;
use uuid::Uuid;

/// 供 TUI 组件运行工作区命令的共享句柄。
pub(crate) type WorkspaceCommandRunner = Arc<dyn WorkspaceCommandExecutor>;

/// 描述一条在活动工作区中执行的、有界的非交互式命令。
///
/// 该命令刻意采用 argv 形式而非 shell 形式，使调用方无需对用户或仓库数据做引号转义。
/// `cwd` 由 app-server 相对于活动会话的工作区规则来解释，
/// 这正是让同一请求结构可以同时适用于内嵌和远程 app-server 实例的原因。
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceCommand {
    /// 要执行的程序和参数，不经过 shell 插值。
    pub(crate) argv: Vec<String>,
    /// 命令的工作目录，当其与 app-server 会话的 cwd 不同时使用。
    pub(crate) cwd: Option<PathBuf>,
    /// 环境变量覆盖，值为 `None` 表示移除该变量。
    pub(crate) env: HashMap<String, Option<String>>,
    /// app-server 取消该命令前允许的最大实际运行时长（wall-clock）。
    pub(crate) timeout: Duration,
    /// app-server 返回的已捕获 stdout/stderr 的最大字节数。
    pub(crate) output_bytes_cap: usize,
    /// app-server 是否应返回不受上限约束的 stdout/stderr。
    pub(crate) disable_output_cap: bool,
}

impl WorkspaceCommand {
    /// 以适合元数据探测的保守默认值创建一条工作区命令。
    pub(crate) fn new(argv: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            argv: argv.into_iter().map(Into::into).collect(),
            cwd: None,
            env: HashMap::new(),
            timeout: Duration::from_secs(/*secs*/ 5),
            output_bytes_cap: 64 * 1024,
            disable_output_cap: false,
        }
    }

    /// 设置命令的工作目录。
    pub(crate) fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// 新增或替换一个环境变量覆盖。
    pub(crate) fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), Some(value.into()));
        self
    }

    /// 设置 app-server 取消该命令前允许的最大实际运行时长。
    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 请求 app-server 捕获不受上限约束的 stdout/stderr。
    pub(crate) fn disable_output_cap(mut self) -> Self {
        self.disable_output_cap = true;
        self
    }
}

/// 已完成的工作区命令的捕获结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceCommandOutput {
    /// app-server 报告的进程退出状态码。
    pub(crate) exit_code: i32,
    /// 经 app-server 输出上限处理后捕获的 stdout。
    pub(crate) stdout: String,
    /// 经 app-server 输出上限处理后捕获的 stderr。
    pub(crate) stderr: String,
}

impl WorkspaceCommandOutput {
    /// 返回进程是否成功退出。
    pub(crate) fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// 在命令结果可用之前发生的传输或协议失败。
///
/// 非零的进程退出会以 `WorkspaceCommandOutput` 的形式表示，使调用方能够区分
/// 普通的探测未命中与 app-server 请求失败。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceCommandError {
    message: String,
}

impl WorkspaceCommandError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for WorkspaceCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for WorkspaceCommandError {}

/// 通过当前活动的 TUI app-server 会话执行非交互式工作区命令。
///
/// 由具体实现决定工作区所在的位置。调用方提供 argv/cwd/env，
/// 且不应针对本地执行还是远程执行进行分支处理。
pub(crate) trait WorkspaceCommandExecutor: Send + Sync {
    /// 运行一条工作区命令，并返回捕获的输出或 app-server 请求错误。
    ///
    /// 调用方应将错误视为基础设施故障，并将带有非零退出码的成功输出视为
    /// 普通的命令失败。返回装箱的 future 可使该 trait 保持对象安全（object-safe）。
    fn run(
        &self,
        command: WorkspaceCommand,
    ) -> Pin<
        Box<dyn Future<Output = Result<WorkspaceCommandOutput, WorkspaceCommandError>> + Send + '_>,
    >;
}

/// 将每个请求都转发给活动 app-server 的工作区命令运行器。
#[derive(Clone)]
pub(crate) struct AppServerWorkspaceCommandRunner {
    request_handle: AppServerRequestHandle,
}

impl AppServerWorkspaceCommandRunner {
    /// 使用当前 TUI 会话持有的 app-server 请求句柄创建一个运行器。
    pub(crate) fn new(request_handle: AppServerRequestHandle) -> Self {
        Self { request_handle }
    }
}

impl WorkspaceCommandExecutor for AppServerWorkspaceCommandRunner {
    /// 将命令作为一次性的 app-server `command/exec` 请求发送。
    ///
    /// 该请求不使用 tty、不流式传输 stdin/stdout/stderr，并使用调用方指定的超时
    /// 和输出上限。它将沙箱与权限配置的选择留给 app-server，从而使同一运行器
    /// 遵循活动会话的内嵌或远程执行策略。
    fn run(
        &self,
        command: WorkspaceCommand,
    ) -> Pin<
        Box<dyn Future<Output = Result<WorkspaceCommandOutput, WorkspaceCommandError>> + Send + '_>,
    > {
        Box::pin(async move {
            let timeout_ms = i64::try_from(command.timeout.as_millis()).unwrap_or(i64::MAX);
            let env = if command.env.is_empty() {
                None
            } else {
                Some(
                    command
                        .env
                        .into_iter()
                        .filter_map(|(k, v)| v.map(|val| (k, val)))
                        .collect(),
                )
            };
            let response: CommandExecResponse = self
                .request_handle
                .request_typed(ClientRequest::OneOffCommandExec {
                    request_id: RequestId::String(format!("workspace-command-{}", Uuid::new_v4())),
                    params: CommandExecParams {
                        command: command.argv,
                        process_id: None,
                        tty: false,
                        stream_stdin: false,
                        stream_stdout_stderr: false,
                        output_bytes_cap: (!command.disable_output_cap)
                            .then_some(command.output_bytes_cap as u64),
                        disable_output_cap: command.disable_output_cap,
                        disable_timeout: false,
                        timeout_ms: Some(timeout_ms as u64),
                        cwd: command.cwd,
                        env,
                        size: None,
                        sandbox_policy: None,
                        permission_profile: None,
                    },
                })
                .await
                .map_err(|err| WorkspaceCommandError::new(err.to_string()))?;

            Ok(WorkspaceCommandOutput {
                exit_code: response.exit_code,
                stdout: response.stdout,
                stderr: response.stderr,
            })
        })
    }
}
