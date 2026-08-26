# CLAUDE.md

## 项目概览

Reflect-TUI — Reflect Agent 的终端交互式客户端(ratatui + crossterm),面向终端 AI Agent 工作流。

- 单 crate workspace:仅 `crates/reflect-tui`(lib `reflect_tui` + thin binary `reflect-tui`)
- 核心引擎(AgentThread / StateGraph / 工具 / 协议)由 `reflect-agent/` **git submodule** 提供,
  本 crate 只负责表现层:事件循环、布局、渲染、协议事件适配
- Rust stable,edition 2024,MSRV 1.85(`rust-toolchain.toml` 锁定)
- 用户数据目录:`~/.reflect/`(config.toml / sessions/ / traces/);Plan 草稿写 `<workspace>/.reflect/plan/`

## ⚠️ 代码规范:注释必须为中文

**本仓库所有代码注释必须使用中文**,包括:

- 行注释(`//`)与块注释(`/* */`)
- 文档注释(`///`、`//!`)——crate 模块文档、结构体/函数/字段文档
- 提交信息与 PR 描述同样使用中文(参考 git log)

新增或修改代码时一律遵守。第三方/许可证声明保留原文。
**用户可见文案(UI 文本、错误提示、日志)不在此限制内**,按现有代码风格。

## 常用命令

```bash
make build                      # cargo build --release -p reflect-tui
make run                        # 构建并启动 TUI
make check / make check-fast C=reflect-tui   # 类型检查(全 workspace / 单 crate)
make test                       # cargo test --workspace
make test-fast                  # cargo nextest(需已安装 cargo-nextest)
make dist                       # ./scripts/build.sh --dist:构建+打包当前平台分发包到 dist/
make dist-dry                   # 预览 dist 打包命令
make install                    # 装到 /usr/local/bin(或 --local → ~/.local/bin)
make gc                         # 清 target/debug 中 cargo 不自动 GC 的中间产物

# 本地测试(test/ 目录,分层说明见 test/README.md)
python3 test/run_all.py            # 离线全量:unit / rust / cli / binary / mock E2E
python3 test/test_workflow_live.py # live 端到端(真实 LLM + 真实工具,手动跑)
```

## 架构

### 本 crate 内部分层(依赖方向自上而下)

| 模块 | 职责 |
|------|------|
| `src/bin/main.rs` | thin binary:clap 路由。无子命令 → TUI;`session`/`traces` → headless 执行完退出 |
| `bootstrap.rs` | 加载 config、构建 ModelRegistry / provider / ToolRegistry / AgentThread,进入 TUI |
| `tui/` + `events.rs` + `adapter.rs` | **本地**事件循环、布局、`UiEvent`/`UiState` 状态机、协议事件适配边界 |
| `tui_core/` | 自包含呈现原语(自定义终端后端、流式历史插入、CJK 换行、composer 等) |
| `history_render/`、`transcript_pager.rs`、`tasks_pager.rs` | 历史行渲染、滚动回放、任务面板 |
| `cli/{session,traces}.rs` | headless 子命令实现(调 reflect-rollout / reflect-telemetry) |

### 核心引擎(submodule,只读消费,勿改)

`reflect-agent/crates/{protocol,abilities,resources,orchestration,integrations,runtime}` 六层。
本 crate 直接依赖的关键 crate:reflect-core(AgentThread)、reflect-llm(ModelRegistry)、
reflect-tools(内置工具)、reflect-config、reflect-rollout、reflect-telemetry、
reflect-task/subagent、reflect-protocol。

## 关键约束(改代码前必读)

1. **Cargo 不支持嵌套 workspace**:核心 crate **不**列入本仓库 `workspace.members`,
   只用 `path` 依赖(`reflect-core = { path = "reflect-agent/crates/runtime/reflect-core" }`)。
   核心 crate 大量 `workspace = true` 继承依赖会解析到**本仓库**根 `[workspace.dependencies]`,
   因此该段必须**完整声明**核心引用的全部 reflect-* 与外部依赖 —— 升级 submodule 或新增
   核心 crate 依赖时,同步补声明,否则跨 workspace 解析失败。详见 [CONTRIBUTING.md](CONTRIBUTING.md) 的「submodule 工作流」小节。
2. **ratatui/crossterm 是 nornagon fork**(根 `Cargo.toml` `[patch.crates-io]` 固定 rev),
   依赖官方版没有的 unstable feature(`scrolling-regions`、`unstable-backend-writer`、
   `rendered-line-info`、`WidgetRef`、`event-stream`、`bracketed-paste`)。不要改回 crates.io 官方版。
3. **bootstrap 必须用 `current_thread` runtime**:fork 版 crossterm 的 `event::poll/read`
   绑定调用线程内部锁,多线程 runtime 下 poll 会因锁竞争恒 false。见 `bootstrap.rs` 注释。
4. `.cargo/config.toml` 设 `INSTA_WORKSPACE_ROOT=""` 是 insta 在嵌套 workspace 下的路径修复,勿删。
5. `tui_core/` 为本仓库自包含的呈现原语,行为以本仓库代码为准;第三方来源与
   许可证见 [NOTICE](NOTICE)。
6. vendored 上游测试 fixtures 由 feature `tui-upstream-tests` gate,默认关闭
   (`cargo test -p reflect-tui --features tui-upstream-tests` 才跑)。
7. Plan mode 写盘:PlanWrite 工具把 plan markdown 写到 `<workspace>/.reflect/plan/<name>.md`,
   ExitPlanMode 读取;`PlanWriteTool` 必须与 Enter/ExitPlanMode 一起注册(见 `bootstrap.rs`)。

## Submodule 维护

- 升级:`git submodule update --remote reflect-agent && git add reflect-agent && git commit`
- 指针变更必须与代码适配一起提交;接线记录追加到 CONTRIBUTING.md「submodule 工作流」段。
- 完整流程(分支切换、联调)见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 第三方来源

ratatui/crossterm fork 等第三方来源与许可证统一见 [NOTICE](NOTICE)。
