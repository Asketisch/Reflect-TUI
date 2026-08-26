//! reflect-tui — 终端交互界面独立二进制入口。
//!
//! 原 Reflect-Agent monorepo 中,TUI 由顶层 `reflect` 二进制通过
//! `reflect_tui::run` 进程内转发。拆仓后,TUI 自带 thin binary。
//!
//! v1.x:在纯 TUI 模式之外,新增 headless 子命令(`session` / `traces`),
//! 复用 reflect-rollout / reflect-telemetry 的公开 API,与 reflect-cli 的
//! 同名子命令对称。无子命令(默认)仍启动 TUI,行为完全不变。
//!
//! headless 子命令执行完即退出,不进 TUI。fork 子命令创建子会话后提示用
//! `reflect-tui --resume <child_id>` 续作。

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use reflect_tui::TuiArgs;

/// reflect-tui 顶层 CLI。无子命令(默认)启动 TUI;带子命令走 headless 路径。
#[derive(Debug, Parser)]
#[command(
    name = "reflect-tui",
    version,
    about = "Reflect-TUI — 终端交互界面(默认启动 TUI)"
)]
struct Cli {
    /// 工作目录(默认:当前目录)。在 dispatch 前通过 `set_current_dir`
    /// 应用,使 TUI 内部的 `current_dir()` 调用一致。
    #[arg(long, short = 'C', value_name = "PATH", global = true)]
    cwd: Option<PathBuf>,

    /// 可选子命令。`None`(默认)→ 启动 TUI;`Some(...)` → headless 执行。
    #[command(subcommand)]
    command: Option<Command>,

    /// TUI 参数(flatten 进顶层)。仅在没有子命令(TUI 模式)时生效;
    /// headless 模式下被忽略。
    #[command(flatten)]
    tui_args: TuiArgs,
}

/// 顶层子命令。
#[derive(Debug, Subcommand)]
enum Command {
    /// 列 / 看 / 删 / fork / rename / export 本地 session。
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// 列 / 看本地 LLM 调用记录(`~/.reflect/traces/`)。
    Traces {
        #[command(subcommand)]
        action: TracesAction,
    },
}

/// `session` 二级子命令。
#[derive(Debug, Subcommand)]
enum SessionAction {
    /// 列出本地 session(默认 20 条)。
    Ls {
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
        #[arg(long, value_name = "SUBSTR")]
        model: Option<String>,
    },
    /// 显示某 session 元数据 + 前 5 条消息预览。
    Show { id: String },
    /// 删除某 session 文件。
    Rm {
        id: String,
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// fork 父会话截止当前完整历史到新子会话。
    Fork {
        id: String,
        #[arg(long, value_name = "NAME")]
        branch: Option<String>,
    },
    /// 给会话设置人可读名称。
    Rename { id: String, name: String },
    /// 把会话导出为人类可读 markdown。
    Export {
        id: String,
        #[arg(long, short = 'o', value_name = "FILE")]
        out: Option<PathBuf>,
    },
}

/// `traces` 二级子命令。
#[derive(Debug, Subcommand)]
enum TracesAction {
    /// 列出本地 LLM 调用 session(默认 20 条)。
    Ls {
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
    },
    /// 显示某 session 的逐条 LLM 调用详情。
    Show { id: String },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // 全局 `--cwd` / `-C`:最早应用,后续 `current_dir()` / 配置路径都基于此。
    if let Some(p) = &cli.cwd {
        std::env::set_current_dir(p)
            .map_err(|e| anyhow::anyhow!("--cwd {}: {e}", p.display()))?;
    }
    match cli.command {
        None => reflect_tui::run(cli.tui_args),
        Some(Command::Session { action }) => match action {
            SessionAction::Ls { limit, model } => {
                reflect_tui::cli::session::ls(limit, model.as_deref())
            }
            SessionAction::Show { id } => reflect_tui::cli::session::show(&id),
            SessionAction::Rm { id, yes } => reflect_tui::cli::session::rm(&id, yes),
            SessionAction::Fork { id, branch } => {
                reflect_tui::cli::session::fork(&id, branch.as_deref())
            }
            SessionAction::Rename { id, name } => {
                reflect_tui::cli::session::rename(&id, &name)
            }
            SessionAction::Export { id, out } => {
                reflect_tui::cli::session::export(&id, out.as_deref())
            }
        },
        Some(Command::Traces { action }) => match action {
            TracesAction::Ls { limit } => reflect_tui::cli::traces::ls(limit),
            TracesAction::Show { id } => reflect_tui::cli::traces::show(&id),
        },
    }
}
