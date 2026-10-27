//! `zhuye-cli discussion run ...` —— 多 agent 讨论(离线/在线)。
//!
//! v1.x 接线:submodule 的 `zhuye_discussion::cli::run` 是**可复用的库
//! 入口**(CLI 逻辑在库 crate 里,不在 `reflect-cli` 那个二进制里)——
//! 读 discussion.toml、起 orchestrator、跑多 agent 讨论、落 transcript。
//! 此前 TUI 完全够不着这条链路(只有 `reflect-cli` 的 `reflect discussion`
//! 能触达),TUI 的 `/subagents` 面板只反映**单个会话内**的子 agent。
//!
//! 本包装层只做参数转发,逻辑全走共享 API(不在本仓库复刻 orchestrator)。

use std::path::PathBuf;

/// `zhuye-cli discussion run --config <path> [--out <path>]`。
///
/// 未配置 provider 时共享 API 降级跑 `run_noop`(状态机跑通、transcript
/// 为空)—— 离线 / CI 友好,不报错。
pub async fn run(config: PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
    zhuye_discussion::cli::run(&config, output.as_deref()).await?;
    Ok(())
}
