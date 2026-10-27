# 贡献指南

感谢关注 ZhuYe-CLI！本仓库只包含 **TUI 表现层**（`crates/zhuye-cli`）；
核心引擎由 [Reflect-Agent](https://github.com/Asketisch/Reflect-Agent) 通过
git submodule 复用。请先读 [README](README.md) 的架构说明。

## 开发环境

```bash
git clone --recursive https://github.com/Asketisch/Reflect-TUI.git
cd ZhuYe-CLI
cargo build --release          # 产物在 target/release/zhuye-cli
cargo test -p zhuye-cli      # TUI 侧测试
```

配置：`~/.zhuye/config.toml`（provider / MCP / LSP / hooks / permissions），
或 `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` 环境变量。离线体验可用
`ZHUYE_MODEL=mock/mock-1`（内置 mock provider，零网络）。

## 提交前检查

CI（ubuntu + macOS）会跑以下几项，请本地先过：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./tests/run_all.sh functional     # 能力覆盖层（真二进制 + pty 行为断言）
python3 tests/tools/coverage_audit.py --strict   # 覆盖自审
```

> **新功能或行为修改必须与测试脚本一起提交。** 改了 TUI 对外可见的行为
> （渲染文本 / 按键路由 / 弹层 / slash 命令 / 对话轮次 / 会话落盘 /
> CLI 输出），必须在 `tests/functional/` 有对应用例，且断言到**行为级**
> （屏幕可见文本 / 落盘文件内容 / 退出码），不接受「退出码 0」「不崩」
> 这类伪断言。新增 slash 命令还要在 `tests/functional/test_slash.py` 的
> `MATRIX` 补一行，否则覆盖自审直接失败。完整检查表（改动面 → 用例落点）
> 见 [AGENTS.md](AGENTS.md) 的「开发流程检查表」。

后两项是**功能面**的门禁，与前两项的**编译面**互补：

- `./tests/run_all.sh functional` 起真实的 `zhuye-cli` 进程，用 pty +
  屏幕快照断言「用户真的看到了什么、真的落了什么盘」，并覆盖全部
  84 条 slash 命令的运行时结局。详见 [tests/README.md](tests/README.md)。
- `coverage_audit.py` 从 Rust 源码解析出声明的 slash 命令，与测试矩阵做
  差集 —— **新增命令忘了加用例，这里直接失败**。改 slash 命令时，
  `tests/functional/test_slash.py` 的 `MATRIX` 要同步补一行。

> 前置：`cargo build --release -p zhuye-cli`（入口脚本会自动补建）、
> Python 3 + `pytest` + `pyte`（`pip install pytest pyte`）。
> 缺依赖的批次自动 SKIP，不算失败。

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
- **slash 命令**：本项目有**两条分发通道**（legacy 文本通道与枚举通道），
  行为不一致 —— 改命令时两条都要看，并把运行时结局登记进
  `tests/functional/test_slash.py` 的 `MATRIX` 与
  [tests/COVERAGE.md](tests/COVERAGE.md) §S 组。

## Submodule 工作流

核心引擎（Reflect-Agent）以 git submodule 形式引入 `reflect-agent/`，
本仓库只做消费端适配。升级 submodule 的标准流程：

```bash
git submodule update --remote reflect-agent   # 拉取最新指针
git -C reflect-agent log --oneline <旧指针>..HEAD   # 逐条读变更
cargo check -p zhuye-cli --all-targets       # 确认编译面
cargo test -p zhuye-cli && cargo clippy -p zhuye-cli --all-targets -- -D warnings
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
- 本仓库的 `AGENTS.md` 记录本项目的架构与约束,submodule 侧的说明见
  `reflect-agent/AGENTS.md`。

### 接线记录

| submodule 指针 | 接线内容 |
| --- | --- |
| `34238dd` | CI 修复批次:reflect-sandbox 的 seatbelt 相关代码(`seatbelt_argv` / `seatbelt_profile` / 原子序号常量)补 `#[cfg(target_os = "macos")]` 门控,`seatbelt_argv` 另补非 macOS 占位实现(调用方按 status 运行时分派,占位实现运行时不可达),修复 Windows 编译失败(`std::os::unix` 不存在);macOS/Linux 行为不变,本仓库无消费点适配。该修复已以 `e7bbb4b` 并入 reflect-agent main(烛夜更名后路径为 `crates/resources/zhuye-sandbox/`);本仓库指针暂留 `34238dd` —— 更名后结构本仓库尚不能消费,指针随外层更名适配一并前移,临时分支已删除 |
| `e4560bc` | 修复批次(D1–D7 收口):会话恢复回放换 submodule 共享 API —— `reader::replay` 升级后支持任意日期桶 + 轮转副本合并 + 文件名/内部 id 双入口,删除本仓库此前维护的全盘扫描 workaround(`resume.rs::collect_session_files`);`JsonlRolloutWriter` 恢复后续写原日期目录的原文件(会话不再被拆成两份);「会话不存在」仍显式报错(共享 API 返回空 Vec,包装层兜底) |
| `7912323` | **未接线 submodule 能力全量接线**(v1.5 / v1.7):`EventMsg::ToolCallArgsDelta` 流式工具参数(live 区渐进预览 + 参数摘要进调用行)、`EventMsg::Routing` 路由事件(failover / 冷却 / `SpecFallback` 跨模型降级)、`reflect_rollout::search_sessions` 跨会话全文搜索(新增 `session search`)、`reflect_config::resolve_model_spec` + `[model_discovery]`(`model = "free"` 运行时解析,配 `routing_policy_with` 传真实 spec)、`reflect_llm::model_catalog` provider 模型清单 + 免费判定(`/model` 无参列出)、`reflect_goal::GoalState` 权威读口(`/goal` 无参显示状态 + `--verify` / `--budget` 选项)、`reflect_config::init_exporter` OTLP 分析遥测、`reflect_config::ConfigWatcher` 配置热重载、`reflect_mcp::curated_catalog` MCP 精选目录(`/mcp` overlay)、`reflect_lsp::DiagnosticRegistry` 诊断(`/lsp`)、`AgentConfig::todos_slot` 待办清单(`/todo`)、`reflect_sandbox::OsSandbox::status_line` 沙箱状态(`/status`)、`reflect_stream::workspace_sync` 工作区克隆(新增 `workspace clone`)、`reflect_discussion::cli::run` 多 agent 讨论(新增 `discussion run`)、`reflect_integration::security_scan` 安全扫描(新增 `security audit`)、`reflect_config::TuiNotificationsSection` 终端通知开关与通道(`[notifications.tui]` 真正生效);同时修 `[sandbox].os_level` 改 `Option<bool>` 带来的编译断点 |
| `d771c9e` | 动态 model 解析接线(`model = "free"` → 启动期经 `reflect_config::resolve_model_spec` 从 provider 清单挑出真实 id,不再拿占位串 `provider/free` 去打请求)+ routing policy 改用 `routing_policy_with(已解析 spec)`(同步版 `routing_policy()` 会让占位串盖掉真实 id)。`SpecSlot.fallbacks` 跨模型降级链由引擎侧接通,TUI 沿用 `EventMsg::Routing` 兜底分支不新增 UI |
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

## 发布前审查

发版前跑一次全量审查，覆盖**编译面 + 行为面 + 覆盖自审**：

```bash
make release-check
```

它依次执行：

1. `./tests/run_all.sh` —— Rust 编译面（`cargo test` / `clippy -D warnings` /
   `fmt` / `doc`）+ 能力覆盖层（真二进制 + pty 的 215 条行为断言 + CLI 参数矩阵）；
2. `tests/tools/coverage_audit.py --strict` —— 确认「声明的 slash 命令」
   全部有行为级用例。

三者任一失败即**不应发版**。功能覆盖矩阵（含已知产品限制、已知缺陷、
未覆盖项的诚实清单）见 [tests/COVERAGE.md](tests/COVERAGE.md)。

> 发布用的是 `./scripts/build.sh --dist`，它只负责构建与打包，**不含**测试。
> 审查与打包是两步：先 `make release-check`，再 `make dist`。

## 报告问题

- 开 issue 时附：操作系统、终端（iTerm/tmux/...）、复现步骤、
  `~/.zhuye/logs/tui-<日期>.log` 的相关片段（注意脱敏）。
- 渲染类问题请附截图或屏幕文本 dump；崩溃类附 panic 全文。

## 行为准则

参与本项目即表示同意 [行为准则](CODE_OF_CONDUCT.md)。
