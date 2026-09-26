# 贡献指南

感谢关注 Reflect-TUI！本仓库只包含 **TUI 表现层**（`crates/reflect-tui`）；
核心引擎由 [Reflect-Agent](https://github.com/Asketisch/Reflect-Agent) 通过
git submodule 复用。请先读 [README](README.md) 的架构说明。

## 开发环境

```bash
git clone --recursive https://github.com/Asketisch/Reflect-TUI.git
cd Reflect-TUI
cargo build --release          # 产物在 target/release/reflect-tui
cargo test -p reflect-tui      # TUI 侧测试
```

配置：`~/.reflect/config.toml`（provider / MCP / LSP / hooks / permissions），
或 `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` 环境变量。离线体验可用
`REFLECT_MODEL=mock/mock-1`（内置 mock provider，零网络）。

## 提交前检查

CI（ubuntu + macOS）会跑以下三项，请本地先过：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

注意：不同 rustc 版本的 clippy lint 集有差异，若你的工具链比 CI 新，
以 `cargo check` 无错误 + 不新增警告为底线。

## 代码约定

- **分层**：本 crate 只负责表现层 —— 事件循环、布局、渲染、协议事件适配
  （`tui/conversion.rs` 是协议事件 → UI 事件的唯一转换边界）。引擎能力
  通过子模块库 crate 调用，不在 TUI 里重新实现引擎逻辑。
- **接线对齐**：bootstrap 的能力装配以 `reflect-exec/src/bootstrap.rs`
  （headless）为参照物；给 TUI 加引擎能力时先看那边怎么接。
- **注释**：解释「为什么」，不复述「是什么」。本项目注释密度偏高是有意
  为之（记录踩坑与设计取舍），但只写有信息量的。
- **协议演进**：wire 协议（Submission / Op / EventMsg）的变更属于
  Reflect-Agent 仓库；本仓库只做消费端适配。

## 提交 / PR

- commit message 用简洁的 `type: 摘要`（feat / fix / docs / test / chore / refactor）。
- PR 请描述动机、改动点和验证方式（跑了哪些测试 / 手工冒烟步骤）。
- 涉及 submodule 指针变更的 PR 请单独说明。

## 报告问题

- 开 issue 时附：操作系统、终端（iTerm/tmux/...）、复现步骤、
  `~/.reflect/logs/tui-<日期>.log` 的相关片段（注意脱敏）。
- 渲染类问题请附截图或屏幕文本 dump；崩溃类附 panic 全文。

## 行为准则

参与本项目即表示同意 [行为准则](CODE_OF_CONDUCT.md)。
