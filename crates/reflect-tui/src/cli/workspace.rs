//! `reflect-tui workspace ...` —— Git 克隆 / 工作区同步。
//!
//! v1.x 接线:submodule 的 `reflect_stream::workspace_sync` 提供
//! `clone_repo` / `sync_status_line`(`git clone` 包装 + 状态行)。此前 TUI
//! 既没有这个子命令,`reflect-stream` 依赖也是**声明了但零引用** —— 用户
//! 想把远程工作区拉到本地只能绕开 TUI。
//!
//! 与 reflect-cli 的 `reflect workspace clone` 对称,底层调用走同一套共享
//! API(不在本仓库复刻一份 clone 逻辑)。

use std::path::PathBuf;

use reflect_stream::workspace_sync::{WorkspaceSyncOptions, clone_repo, sync_status_line};

/// `reflect-tui workspace clone <url> [--dest DIR] [--branch B] [--depth N]`
/// —— 克隆远程仓库到本地目录。
pub fn clone(
    url: String,
    dest: Option<PathBuf>,
    branch: Option<String>,
    depth: u32,
) -> anyhow::Result<()> {
    println!("Reflect workspace clone");
    println!("======================");
    // `--dest` 缺省 → 用仓库名(URL 最后一段)作目录名。
    let dest = dest.unwrap_or_else(|| default_dest(&url));
    let opts = WorkspaceSyncOptions {
        url,
        dest: dest.clone(),
        branch,
        depth,
    };
    let path = clone_repo(&opts)?;
    println!("✓ cloned to {}", path.display());
    Ok(())
}

/// `reflect-tui workspace` —— 打印用法提示。
pub fn hint() -> anyhow::Result<()> {
    println!("{}", sync_status_line());
    Ok(())
}

/// 从 URL 猜一个默认目标目录名(取最后一段,去掉 `.git` 后缀)。
fn default_dest(url: &str) -> PathBuf {
    let trimmed = url.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() {
        PathBuf::from("workspace")
    } else {
        PathBuf::from(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dest_uses_repo_name() {
        assert_eq!(
            default_dest("https://github.com/foo/bar.git"),
            PathBuf::from("bar")
        );
        assert_eq!(
            default_dest("https://github.com/foo/bar"),
            PathBuf::from("bar")
        );
    }

    #[test]
    fn default_dest_handles_trailing_slash() {
        assert_eq!(
            default_dest("https://github.com/foo/bar/"),
            PathBuf::from("bar")
        );
    }

    #[test]
    fn default_dest_falls_back_when_empty() {
        assert_eq!(default_dest(""), PathBuf::from("workspace"));
    }
}
