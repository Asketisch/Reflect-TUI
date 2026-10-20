# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 修复（对标 codex / claude code 审计后的 TUI 层缺陷）

- **Ctrl-C 不再在回合进行中直接退出 TUI**：此前 Ctrl-C 无条件映射 `Exit`，
  长回合中误触即丢失整个会话。改为对齐 codex / claude code —— busy 时首次
  Ctrl-C 发 `Op::Interrupt` 打断当前回合，空闲时才一次退出；中断请求已发出而
  回合仍未收尾时，1.5s 内第二次 Ctrl-C 放行退出，保证引擎卡死时用户有出路。
  判定逻辑收敛到 `keymap::exit_key_action` 纯函数。
- **`/copy` 与 Ctrl+O 修复**：`adapter::agent_replies_in_order` 早已写好却零调用方，
  而 `copy_last_reply` / `copy_reply_n` / copy-history overlay 都只匹配整段
  `UiHistoryItem::Agent` —— 正常流式路径下 agent 正文全部以 `StreamedAgent`
  片段入 history，导致每次真实对话后 Ctrl+O 与 `/copy` 都报「no agent reply
  yet」。三处统一改走该辅助函数（合并连续流式片段为一条回复）。
- **`/mcp` overlay 不再显示假数据**：此前硬编码一份
  `[("node_repl", "Node.js REPL environment", "connected", 3)]` 虚构清单。改为
  由 `McpServerStarted` / `McpServerFailed` 生命周期事件 upsert 出
  `state.mcp_servers` 连接态快照，overlay 只读快照；没连上就如实显示
  「No MCP servers connected」，连接成功时展开协议事件本就携带的 `tool_names`。
- **全屏模式补绘 ask_user / question 面板**：`draw_fullscreen` 的 layout 漏了
  `question_h` / `ask_user_h` 两段，`--fullscreen` 下引擎阻塞在
  `AskUserQuestion` / `AskUser` 时用户看到的是空白，而键位仍被面板接管 ——
  表现为「界面卡死但按键有反应」。与 inline `draw` 对齐补齐。
- **自动压缩与配额耗尽不再静默**：`ContextCompacted` / `QuotaExhausted` 此前
  整条 `return None` 丢弃，用户看到的是「上下文突然变短」「模型突然不记得
  前面的话」「回合莫名停住」，界面上没有任何解释。改为 Notice，携带压缩策略 /
  移除消息数 / token 前后对比，以及配额 provider / credential / 用量 / 窗口
  重置时间。
- **`/diff` 窄终端下溢 panic**：`diff_render::render_change` 的 `wrap_cols - 4`
  在终端宽度 ≤ 4 时下溢（dev/test profile 开了 overflow-checks）。钳到
  `saturating_sub(4).max(1)`，调用侧 `open_diff_overlay` 的 width 也钳到 ≥ 8。
- `cargo fmt` 收口：`question_panel_key` 内 `apply_select` 签名单行化。

### 能力接线（对齐 headless reflect-exec，引擎能力在 TUI 内真正可用）

- M4 完整装配：agent 定义（`.reflect/agents/`）、技能系统（扫描 + 内置技能包 +
  `load_skill` / `read_skill_resource` 工具）、分层记忆（会话内存 + 项目/用户文件）、
  session notes（JSONL 落盘 + `add_session_note` 工具）、post-compact 文件恢复、
  子代理调用登记表、LLM 摘要压缩（`LlmSummarizer`，Role::Compact 路由，可选
  记忆固化）、prompt 构建器与 coordinator 模式注入。
- `[sanitize]` 脱敏器与 `[hooks]` hook engine（含自定义 shell hooks）注入
  AgentThread —— 此前两段配置在 TUI 内是死信。
- 子代理能力补齐：`[subagent_providers]` 独立 child registry、父技能共享、
  `call_<role>` 工具补入模型 always-on 可见集、`SubagentRuntimeRegistry` +
  会话取消令牌级联（退出 TUI 不留下飞子代理）。
- MCP：`[mcp_servers]` 连接管理器接线，外部工具注册进工具表，生命周期事件
  （started/failed）以 Notice 行进对话流；注册 MCP 资源读取工具。
- LSP：`[lsp_servers]` 连接管理器 + `lsp` 工具接线。
- telemetry：`[telemetry]` 本地 trace sink + `[langfuse]` 云端并行投递。
- 真实分词器：启用 reflect-compact `tokenizer` feature（tiktoken），压缩触发
  阈值与上下文估算不再走 len/3.5 启发式。

### 会话恢复闭环

- `--resume <id>` / `-c` / `-r N` 启动参数真实消费：rollout 索引定位 → replay →
  历史预载注入引擎 + scrollback 回填渲染。
- `/resume` overlay 列出真实历史会话（rollout 索引，名字优先 `/rename` 落盘值）。
- `/archive` 落地：`[archived] ` 前缀重命名（可逆，rollout 暂无独立归档存储），
  y/n 确认后退出时执行；`/delete` 落地：删除当前会话全部落盘文件（JSONL +
  轮转副本 + 名字文件，不可逆），y/n 确认后退出时执行。

### 交互补全

- `/model` 真正切换模型（`AgentConfig::set_model`，registry 校验，下一回合生效）。
- 审批弹窗选 always-allow（a）时，Allow 规则持久化到 `~/.reflect/permissions.toml`，
  重启后同类工具调用继续免审批。
- v1.4 子代理状态快照（`SubagentStatus`）接入 `/tasks` 面板实时刷新。

### 体验

- v1.4 `ToolCallOutputDelta`：长时工具（bash / web_fetch）输出逐段实时上屏
  （live 区 tail，最多 10 行）；`REFLECT_TUI_TOOL_STREAM=0` 可关闭。

### 测试

- 重建 `test/` 离线测试套件（原脚本丢失）：`harness.py`（pty + pyte + TERM/CPR
  自动应答）、`run_all.py`（7 阶段入口）、unit / cli help / binary smoke /
  mock E2E / resume E2E（`-c` 回填断言）。

### 工程

- CI 增加 Windows 测试 job（发布产物已含 Windows 包）。

## [0.1.0] — 初始公开版本

- TUI 表现层（ratatui / crossterm fork）：流式 markdown 渲染、60+ slash 命令、
  审批弹窗、Plan mode 闭环、fork/rewind/checkpoint、会话/任务/技能/主题面板、
  通知、IDE 上下文 IPC、双模式（inline / alt-screen）。
- 核心引擎由 [reflect-agent](https://github.com/Asketisch/Reflect-Agent) 子模块
  提供（协议 v1.5：子代理可观测双通道、request_human_input 持久化、可插拔
  tokenizer 等）。
- 开源基建：LICENSE / NOTICE / CI / release / audit workflows。
