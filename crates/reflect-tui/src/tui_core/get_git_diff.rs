//! 计算工作目录当前 Git diff 的工具。
//!
//! 返回已跟踪变更以及任何未跟踪文件的 diff。当
//! 当前目录不在 Git 仓库内时，函数返回
//! `Ok((false, String::new()))`。

use std::path::Path;
use std::time::Duration;

use crate::git_utils::FsmonitorOverride;
use crate::git_utils::FsmonitorProbeRunner;
use crate::git_utils::detect_fsmonitor_override;
use crate::tui_core::workspace_command::WorkspaceCommand;
use crate::tui_core::workspace_command::WorkspaceCommandExecutor;
use crate::tui_core::workspace_command::WorkspaceCommandOutput;

const DIFF_COMMAND_TIMEOUT: Duration = Duration::from_secs(/*secs*/ 30);
const DISABLE_HOOKS_CONFIG: &str = if cfg!(windows) {
    "core.hooksPath=NUL"
} else {
    "core.hooksPath=/dev/null"
};
const EXECUTABLE_FILTER_CONFIG_PATTERN: &str = r"^filter\..*\.(clean|process)$";

// `/diff` 可能通过远程工作区执行 Git，因此 git-utils 负责探测策略，
// 而此适配器把命令执行保留在 TUI 层。每次调用都以 WorkspaceCommand 为界；
// `/diff` 没有聚合的命令截止时间。
struct WorkspaceFsmonitorProbeRunner<'a> {
    runner: &'a dyn WorkspaceCommandExecutor,
    cwd: &'a Path,
}

#[async_trait::async_trait]
impl FsmonitorProbeRunner for WorkspaceFsmonitorProbeRunner<'_> {
    async fn run_probe(&mut self, args: &[&str]) -> Option<Vec<u8>> {
        let argv = ["git"].into_iter().chain(args.iter().copied());
        let command = WorkspaceCommand::new(argv).cwd(self.cwd.to_path_buf());
        match self.runner.run(command).await {
            Ok(output) if output.success() => Some(output.stdout.into_bytes()),
            _ => None,
        }
    }
}

/// [`get_git_diff`] 的返回值。
///
/// * `bool` – 当前工作目录是否位于 Git 仓库内。
/// * `String` – 拼接后的 diff（可能为空）。
pub(crate) async fn get_git_diff(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Result<(bool, String), String> {
    // 先检查是否位于 Git 仓库内。
    if !inside_git_repo(runner, cwd).await? {
        return Ok((false, String::new()));
    }

    // 每次 `/diff` 探测一次，后续所有 Git 命令复用其结果。
    let mut probe_runner = WorkspaceFsmonitorProbeRunner { runner, cwd };
    let fsmonitor = detect_fsmonitor_override(&mut probe_runner).await;

    // 保持 `/diff` 的信息性：仓库配置不得选中可执行的 diff 辅助程序。
    let diff_config_overrides =
        diff_filter_config_overrides(runner, cwd, fsmonitor.clone()).await?;
    let (tracked_diff_res, untracked_output_res) = tokio::join!(
        run_git_capture_diff(
            runner,
            cwd,
            fsmonitor.clone(),
            &diff_config_overrides,
            &[
                "diff",
                "--no-textconv",
                "--no-ext-diff",
                "--submodule=short",
                "--ignore-submodules=dirty",
                "--color",
            ],
        ),
        run_git_capture_stdout(
            runner,
            cwd,
            fsmonitor.clone(),
            &["ls-files", "--others", "--exclude-standard"],
        ),
    );
    let tracked_diff = tracked_diff_res?;
    let untracked_output = untracked_output_res?;

    let mut untracked_diff = String::new();
    let null_device: &Path = if cfg!(windows) {
        Path::new("NUL")
    } else {
        Path::new("/dev/null")
    };

    let null_path = null_device.to_str().unwrap_or("/dev/null");
    for file in untracked_output
        .split('\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let args = [
            "diff",
            "--no-textconv",
            "--no-ext-diff",
            "--submodule=short",
            "--ignore-submodules=dirty",
            "--color",
            "--no-index",
            "--",
            null_path,
            file,
        ];
        let diff = run_git_capture_diff(
            runner,
            cwd,
            fsmonitor.clone(),
            &diff_config_overrides,
            &args,
        )
        .await?;
        untracked_diff.push_str(&diff);
    }

    Ok((true, format!("{tracked_diff}{untracked_diff}")))
}

/// 用给定的 `args` 执行 `git` 并以 UTF-8 字符串返回 `stdout` 的辅助函数。
/// 任何非零退出状态都被视为 *错误*。
async fn run_git_capture_stdout(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    fsmonitor: FsmonitorOverride,
    args: &[&str],
) -> Result<String, String> {
    let output = run_git_command(runner, cwd, fsmonitor, &[], args).await?;
    if output.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git {:?} failed with status {}",
            args, output.exit_code
        ))
    }
}

/// 与 [`run_git_capture_stdout`] 类似，但把退出状态 1 视为成功并返回 stdout。
/// Git 在存在差异时对 diff 返回 1。
async fn run_git_capture_diff(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    fsmonitor: FsmonitorOverride,
    config_overrides: &[(String, String)],
    args: &[&str],
) -> Result<String, String> {
    let output = run_git_command(runner, cwd, fsmonitor, config_overrides, args).await?;
    if output.success() || output.exit_code == 1 {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git {:?} failed with status {}",
            args, output.exit_code
        ))
    }
}

/// 返回可防止配置的过滤驱动在生成 diff 时执行的
/// Git 配置覆盖项。
async fn diff_filter_config_overrides(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    fsmonitor: FsmonitorOverride,
) -> Result<Vec<(String, String)>, String> {
    let args = [
        "config",
        "--null",
        "--name-only",
        "--get-regexp",
        EXECUTABLE_FILTER_CONFIG_PATTERN,
    ];
    let output = run_git_command(runner, cwd, fsmonitor, &[], &args).await?;
    if output.exit_code != 0 && output.exit_code != 1 {
        return Err(format!(
            "git {:?} failed with status {}",
            args, output.exit_code
        ));
    }

    let mut drivers = output
        .stdout
        .split('\0')
        .filter_map(|key| {
            key.strip_suffix(".clean")
                .or_else(|| key.strip_suffix(".process"))
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    drivers.sort();
    drivers.dedup();

    Ok(drivers
        .into_iter()
        .flat_map(|driver| {
            [
                (format!("{driver}.clean"), String::new()),
                (format!("{driver}.process"), String::new()),
                (format!("{driver}.required"), "false".to_string()),
            ]
        })
        .collect())
}

/// 确定当前目录是否位于 Git 仓库内。
async fn inside_git_repo(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
) -> Result<bool, String> {
    // `rev-parse` 不检查工作树，且在此之前探测
    // 还会在仓库之外运行额外的 Git 命令。
    let output = run_git_command(
        runner,
        cwd,
        FsmonitorOverride::Disabled,
        &[],
        &["rev-parse", "--is-inside-work-tree"],
    )
    .await?;
    Ok(output.success())
}

async fn run_git_command(
    runner: &dyn WorkspaceCommandExecutor,
    cwd: &Path,
    fsmonitor: FsmonitorOverride,
    config_overrides: &[(String, String)],
    args: &[&str],
) -> Result<WorkspaceCommandOutput, String> {
    let fsmonitor_arg = fsmonitor.git_config_arg().unwrap_or_default();
    let argv = [
        "git",
        "-c",
        fsmonitor_arg.as_str(),
        "-c",
        DISABLE_HOOKS_CONFIG,
    ]
    .into_iter()
    .chain(args.iter().copied());
    let mut command = WorkspaceCommand::new(argv)
        .cwd(cwd.to_path_buf())
        .timeout(DIFF_COMMAND_TIMEOUT)
        .disable_output_cap();
    if !config_overrides.is_empty() {
        command = command.env("GIT_CONFIG_COUNT", config_overrides.len().to_string());
        for (index, (key, value)) in config_overrides.iter().enumerate() {
            command = command
                .env(format!("GIT_CONFIG_KEY_{index}"), key)
                .env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
    }
    runner.run(command).await.map_err(|err| err.to_string())
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
