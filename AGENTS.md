# AGENTS.md

> 本文件是本仓库的 AI 助手说明(原 `CLAUDE.md`,2026-10 起改名)。
> submodule 侧的同名文件见 `reflect-agent/AGENTS.md`,两者结构对标。

## 项目概览

Reflect-TUI — Reflect Agent 的终端交互式客户端(ratatui + crossterm),面向终端 AI Agent 工作流。

- 单 crate workspace:仅 `crates/reflect-tui`(lib `reflect_tui` + thin binary `reflect-tui`)
- 核心引擎(AgentThread / StateGraph / 工具 / 协议)由 `reflect-agent/` **git submodule** 提供,
  本 crate 只负责表现层:事件循环、布局、渲染、协议事件适配
- Rust stable,edition 2024,MSRV 1.85(`rust-toolchain.toml` 锁定)
- 用户数据目录:`~/.zhuye/`(config.toml / sessions/ / traces/);Plan 草稿写 `<workspace>/.zhuye/plan/`

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
make test-suite                 # ./tests/run_all.sh:全功能套件(能力覆盖层+编译面+覆盖自审)
make test-suite-fast            # ./tests/run_all.sh functional:只跑能力覆盖层(跳过 cargo 编译面)
make test-suite-live            # ./tests/run_all.sh live:live 端到端(需 API key,手动跑)
make coverage-audit             # 覆盖自审:slash 命令 × 测试矩阵的差集核对(有缺口即失败)
make release-check              # 发版前全量审查(编译面 + 能力覆盖层 + 覆盖自审)
make dist                       # ./scripts/build.sh --dist:构建+打包当前平台分发包到 dist/
make dist-dry                   # 预览 dist 打包命令
make install                    # 装到 /usr/local/bin(或 --local → ~/.local/bin)
make gc                         # 清 target/debug 中 cargo 不自动 GC 的中间产物

# 全功能测试套件(tests/ 目录,分层见 tests/README.md,覆盖矩阵见 tests/COVERAGE.md)
./tests/run_all.sh              # 离线全量:能力覆盖层 + Rust 编译面 + 覆盖自审
./tests/run_all.sh functional   # 只跑能力覆盖层(真二进制 + pty 行为断言 + CLI 参数矩阵)
./tests/run_all.sh audit        # 只跑覆盖自审
./tests/run_all.sh live         # live 端到端(真实 LLM + 真实工具,需 API key,手动跑)
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

`reflect-agent/crates/{protocol,abilities,resources,orchestration,integrations,runtime}(烛夜更名后 crate 名为 `zhuye-*`)` 六层。
本 crate 直接依赖的关键 crate:reflect-core(AgentThread)、reflect-llm(ModelRegistry)、
reflect-tools(内置工具)、reflect-config、reflect-rollout、reflect-telemetry、
reflect-task/subagent、reflect-protocol。

## 关键约束(改代码前必读)

1. **Cargo 不支持嵌套 workspace**:核心 crate **不**列入本仓库 `workspace.members`,
   只用 `path` 依赖(`zhuye-core = { path = "reflect-agent/crates/runtime/zhuye-core" }`)。
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
7. Plan mode 写盘:PlanWrite 工具把 plan markdown 写到 `<workspace>/.zhuye/plan/<name>.md`,
   ExitPlanMode 读取;`PlanWriteTool` 必须与 Enter/ExitPlanMode 一起注册(见 `bootstrap.rs`)。
8. **slash 命令有两条分发通道**,行为不一致,改命令时两条都要看:
   - `history_render::dispatch_slash`(legacy 通道)—— 命令名命中
     `is_legacy_passthrough_slash` 白名单时走这条;
   - `tui/mod.rs::dispatch_slash_command`(枚举通道)—— 其余可见的
     `SlashCommand` 走这条,**只实现了部分命令**,其余落到
     「此命令在当前构建中尚未接入分发」兜底。
   全量矩阵见 [tests/COVERAGE.md](tests/COVERAGE.md) §S 组;
   **新增 slash 命令必须同时补 `tests/functional/test_slash.py` 的 MATRIX**,
   否则 `make coverage-audit` 会失败。
9. **对话轮次落盘的 assistant 消息是最后一轮的回复**:引擎一次用户提交会
   多次调用模型(mock provider 每次调用消费脚本一行),流式中间态不入盘。

## 开发流程检查表(改任何代码前必读)

> 本节结构与 submodule `reflect-agent/AGENTS.md` 的同名章节对标。
> **新功能或修改必须与测试脚本一起提交** —— 不接受「先合功能、后补测试」,
> 也不接受只有「退出码 0 / 不崩」这类伪断言。

**新功能**或 **bug 修复**提交前,必须逐条走完:

1. **能力覆盖脚本同步** —— 判断本次改动是否需要新增 / 更新能力覆盖用例:
   - 改了 TUI 对外可见的行为(渲染文本、按键路由、弹层、slash 命令、
     对话轮次、会话落盘、CLI 输出)
     → 必须在 `tests/functional/` 有对应用例,断言到**行为级**
     (屏幕可见文本 / 落盘文件内容 / 进程退出码),
     **不接受**「退出码 0」或「不抛错」作为能力断言;
   - 已有相关用例 → 检查是否需要**更新**(断言口径变了 / 新增字段 / 语义变化);
   - 能力覆盖用例**只写在 `tests/functional/`** —— 不要在 `tests/rust/*.sh`
     里重复写行为断言,那里只管 CLI **参数面**;
   - 只改内部纯逻辑(渲染辅助函数、CJK 换行、状态机转换)
     → 补 Rust 单测(`crates/reflect-tui/src/**`),不必硬凑 pty 用例。
2. **按改动面选测试落点** —— 能力覆盖层按面分组,新用例进对应组:

   | 改动面 | 用例落点 |
   |---|---|
   | 启动 / 渲染锚点 / 退出生命周期 / 全屏模式 | `tests/functional/test_boot.py`(B 组) |
   | slash 命令面 | `tests/functional/test_slash.py` 的 `MATRIX`(S 组) |
   | 弹层 / 浮层 / 选择器 | `tests/functional/test_overlay.py`(O 组) |
   | 对话轮次 / 流式 / 工具调用 / 统计 | `tests/functional/test_turn.py`(T 组) |
   | 会话持久化 / 恢复 / fork / plan 落盘 | `tests/functional/test_session.py`(P 组) |
   | headless CLI 语义 | `tests/functional/test_cli.py`(C 组) |
   | 键盘交互 | `tests/functional/test_keys.py`(K 组) |
   | 渲染与视觉 | `tests/functional/test_render.py`(V 组) |
   | 审批 / AskUser / 容错 / 信号 | `tests/functional/test_flow.py`(W 组) |
   | 插件面 | `tests/functional/test_plugin.py`(G 组) |
   | CLI 参数矩阵 | `tests/rust/cli_core.sh`、`tests/rust/cli_modules.sh` |
   | 内部纯逻辑 | `crates/reflect-tui/src/**` 的 Rust 单测 |

   分层判据与用例规模见 [tests/README.md](tests/README.md);
   「能力 × 覆盖层」映射见 [tests/COVERAGE.md](tests/COVERAGE.md)。
3. **slash 命令双通道 + 覆盖门禁** —— 新增 / 改名 / 改结局的 slash 命令:
   - 两条分发通道都要看:`history_render::dispatch_slash`(legacy 白名单)
     与 `tui/mod.rs::dispatch_slash_command`(枚举通道);
   - `tests/functional/test_slash.py` 的 `MATRIX` 必须补一行,写明结局类别
     (`wired` / `placeholder` / `unrecognized` / `exit`)与期望屏幕文本;
   - `make coverage-audit`(`tests/tools/coverage_audit.py --strict`)会做
     「Rust 里声明的命令 × 矩阵覆盖」的**机械差集**,漏一条直接失败。
4. **覆盖矩阵同步** —— `tests/COVERAGE.md` 随改动更新:
   新增能力补「能力 × 覆盖层」表的一行;移除能力移到「未覆盖项」并写明原因;
   发现的缺陷记入「已知缺陷」。
5. **验证** —— 按下方「验证矩阵」跑对应命令,全绿再提交。
6. **注释与提交信息** —— 注释一律中文;commit 用 `type(scope): 描述`
   conventional 风格(参考 git log)。

## 验证矩阵(按改动范围选跑)

| 改动范围 | 必跑命令 |
|---|---|
| 任何改动 | `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust 代码 / crate | `cargo test --workspace` |
| TUI 行为(渲染 / 按键 / 弹层 / 轮次 / 落盘 / CLI 语义) | `./tests/run_all.sh functional` |
| slash 命令 | `./tests/run_all.sh functional` + `make coverage-audit` |
| CLI 子命令参数面 | `./tests/run_all.sh rust` |
| 仅文档 / 注释 | 无需跑测试,自查链接与命令是否仍有效 |
| 发版前 | `make release-check` |

> 全程离线:`ZHUYE_MODEL=mock/mock-1` + 隔离 HOME(mktemp),
> 不污染真实 `~/.zhuye`;缺 pytest / pyte 的批次自动 SKIP,不算失败。
> 首次运行需 `pip install pytest pyte`(Node 不需要)。

## Submodule 维护

- 升级:`git submodule update --remote reflect-agent && git add reflect-agent && git commit`
- 指针变更必须与代码适配一起提交;接线记录追加到 CONTRIBUTING.md「submodule 工作流」段。
- 完整流程(分支切换、联调)见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 第三方来源

ratatui/crossterm fork 等第三方来源与许可证统一见 [NOTICE](NOTICE)。
