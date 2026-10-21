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

## Submodule 工作流

核心引擎（Reflect-Agent）以 git submodule 形式引入 `reflect-agent/`，
本仓库只做消费端适配。升级 submodule 的标准流程：

```bash
git submodule update --remote reflect-agent   # 拉取最新指针
git -C reflect-agent log --oneline <旧指针>..HEAD   # 逐条读变更
cargo check -p reflect-tui --all-targets       # 确认编译面
cargo test -p reflect-tui && cargo clippy -p reflect-tui --all-targets -- -D warnings
git add reflect-agent && git commit            # 指针与代码适配一起提交
```

要点：

- **指针变更必须与代码适配一起提交** —— 只改指针不改代码会留下编译不过的仓库。
- 升级后逐条核对新增 / 变更的公开 API：本仓库的直接依赖是 `reflect-core`、
  `reflect-protocol`、`reflect-rollout`、`reflect-telemetry`、`reflect-config`、
  `reflect-compact` 等，这些 crate 的 API 变更需要在本仓库有对应消费点。
- **消费点优先走共享 API**：submodule 提供的读取 / 聚合能力（如
  `reflect_rollout::session_stats`），不要在本仓库复刻一份聚合逻辑，
  否则两边会漂移。
- 本仓库的 `CLAUDE.md` 记录本项目的架构与约束，submodule 侧的说明见
  `reflect-agent/AGENTS.md`。

### 接线记录

| submodule 指针 | 接线内容 |
| --- | --- |
| `e4bc182` | 模型级 token 统计（`TokenCountEvent.model` → HUD / `/status` 按模型用量）、`reflect_rollout::session_stats` / `all_model_usage` 接入 `session show` / 新增 `session stats`、`[telemetry].redact` / `max_bytes` / `record_tool_schemas` 真正生效、span 日志读取侧（新增 `traces spans`）、会话文件定位改走共享 API（文件名 thread id 与内部 `session_id` 可能不同） |
| `46dc156` | 提示词分层库接入 + `/prompt` 运行时管理 |
| `3cb791f` | `fetch_update` 弃用告警收口 + CI 编译修复 |
| `8564f66` | 安全边界与正确性收口（沙箱逃逸 / 路径穿越 / Plan mode） |
| `af0483b` | 持久 shell 状态 + `exec --image` 多模态 + `web_fetch` DNS rebinding 防护 |
| `bb2eaeb` | Linux CI 全绿（landlock 降级重试 + 环境自适应测试） |
| `d9b88de` | `/permissions` 弹窗修复 + MCP Prompts 工具接入 |
| `b625b5e` | `/rewind` 弹窗补齐 + clippy 门槛修复 |

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
