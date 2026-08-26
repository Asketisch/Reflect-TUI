//! reflect_git_utils crate 的桩代码。
//!
//! 提供被内嵌的 tui_compat UI 代码使用的 git 相关工具。
//! 大多数函数是返回空/默认值的桩函数；Reflect
//! 适配器在需要时提供真正的实现。

use std::path::Path;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// FsmonitorProbeRunner trait（异步）
// ---------------------------------------------------------------------------

/// 文件系统监控探测运行器 trait。
///
/// 被 `get_git_diff.rs` 用于通过 `fsmonitor`/`watchman`
/// 检查受监视路径是否有变更。真正的实现会调用文件系统监控工具；
/// 此桩代码提供一个空的异步 trait。
#[async_trait::async_trait]
pub trait FsmonitorProbeRunner: Send + Sync {
    /// 执行探测命令并返回原始的 stdout 字节。
    async fn run_probe(&mut self, args: &[&str]) -> Option<Vec<u8>>;
}

/// fsmonitor 检测的覆盖配置。
#[derive(Debug, Clone, Default)]
pub struct FsmonitorOverride {
    pub program: Option<String>,
}

impl FsmonitorOverride {
    #[allow(non_upper_case_globals)]
    pub const Disabled: Self = Self { program: None };

    pub fn git_config_arg(&self) -> Option<String> {
        None
    }
}

/// 从环境/配置中检测 fsmonitor 覆盖设置。
pub async fn detect_fsmonitor_override<R: FsmonitorProbeRunner>(
    _probe_runner: &mut R,
) -> FsmonitorOverride {
    FsmonitorOverride::default()
}

// ---------------------------------------------------------------------------
// Git 工具函数
// ---------------------------------------------------------------------------

/// git 提交的信息。
#[derive(Debug, Clone, Default)]
pub struct CommitLogEntry {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
    pub sha: String,
    pub subject: String,
}

/// git diff 的摘要。
#[derive(Debug, Clone, Default)]
pub struct GitDiffSummary {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

/// 获取包含给定路径的 git 仓库根目录。
pub fn get_git_repo_root(cwd: &Path) -> Option<PathBuf> {
    // 从 cwd 向上遍历查找 .git 目录。
    let mut current = cwd.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// 列出本地 git 分支。
pub async fn local_git_branches(_cwd: &Path) -> Vec<String> {
    Vec::new()
}

/// 从 git 日志中获取最近的提交。
pub async fn recent_commits(_cwd: &Path, _limit: usize) -> Vec<CommitLogEntry> {
    Vec::new()
}

/// 获取给定仓库路径的当前 git 分支名称（异步）。
pub async fn current_branch_name(_cwd: &Path) -> Option<String> {
    None
}

/// 解析根 git 项目（用于信任目的）。
pub fn resolve_root_git_project_for_trust(
    path: &crate::utils_absolute_path::AbsolutePathBuf,
) -> Option<PathBuf> {
    get_git_repo_root(path.as_path())
}
