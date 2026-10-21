# tests/ —— Reflect-TUI 全功能测试套件

本目录是仓库的**统一测试入口**,覆盖 TUI 表现层的全部对外功能:
启动与渲染(含全屏模式)、84 条 slash 命令、11 个弹层、对话轮次、
键盘交互、渲染与视觉、审批 / AskUser 工作流、容错与信号、会话持久化、
headless CLI,以及 Rust 编译面。

全程**离线**(`REFLECT_MODEL=mock/mock-1`,零网络、零 API key)+
**隔离 HOME**(mktemp,不污染真实 `~/.reflect`)。

## 快速开始

```bash
./tests/run_all.sh              # 全部(能力覆盖层 + Rust 侧 + 覆盖自审)
./tests/run_all.sh functional   # 能力覆盖层(CI 用):pty 行为断言 + CLI 参数矩阵
./tests/run_all.sh rust         # Rust 侧(cargo test/clippy/fmt/doc + CLI 参数矩阵)
./tests/run_all.sh audit        # 覆盖自审(slash 命令 × 测试矩阵的差集核对)
./tests/run_all.sh live         # live 端到端(需真实 LLM + API key)
```

前置:`cargo build --release -p reflect-tui`(入口脚本会自动补建)、
Python 3 + pytest + pyte(`pip install pytest pyte`)、Node 不需要。
缺依赖的批次自动 **SKIP**,不算失败。

Makefile 快捷方式:

```bash
make test-suite        # = ./tests/run_all.sh
make test-suite-fast   # = ./tests/run_all.sh functional(跳过 cargo 编译面)
make coverage-audit    # = 覆盖自审(有未覆盖命令即失败)
```

## 三层分工

> 分层的理由:本仓是**表现层**,对外只有一个二进制(`reflect-tui`),
> 但它有两类完全不同的测试对象 —— **屏幕上的像素** 与 **命令行的输出**。
> 所以分三层,各测各的,不互相顶替。

| 层 | 位置 | 测什么 | 规模 |
|------|------|--------|--------|
| **能力覆盖层**(主) | `functional/` | **真二进制 + pty 上的行为**:渲染出的文字、按键路由、落盘的文件 | 199 条 |
| **覆盖自审层** | `tools/coverage_audit.py` | 「声明了什么」×「测了什么」的**机械差集** | 4 条门禁 |
| **CLI 参数面** | `rust/*.sh` | 子命令的**参数矩阵**与输出契约(bash) | 100+ 断言 |
| Rust 编译面 | `rust/workspace_tests.sh` | `cargo test` / `clippy -D warnings` / `fmt` / `doc` | 既有基线 |

**判据**:
- 断言「用户真的看到了什么 / 真的落了什么盘」→ 能力覆盖层;
- 断言「某个 CLI 参数组合的输出契约」→ bash 参数面;
- **新增能力断言只落能力覆盖层一处**。

## 目录结构

| 路径 | 内容 |
|------|------|
| `lib.sh` | 公共助手:隔离环境、mock provider、断言、用例统计 |
| `run_all.sh` | 统一入口(套件编排) |
| `COVERAGE.md` | **功能覆盖矩阵**:能力 × 覆盖层映射、已知缺陷、未覆盖清单 |
| `functional/` | 能力覆盖层(主) |
| `functional/tui_pty.py` | pty 会话设施(pyte 屏幕快照 + 原始字节流 + 等待/超时) |
| `functional/conftest.py` | pytest fixture:隔离 HOME、mock 脚本、TUI 会话工厂 |
| `functional/test_boot.py` | B 组:启动 / 渲染锚点 / 退出生命周期 / 全屏模式 |
| `functional/test_slash.py` | S 组:**slash 命令全量矩阵**(84 条,数据驱动) |
| `functional/test_overlay.py` | O 组:弹层 / 浮层 / 选择器 + 未接线弹层与鼠标的反向门禁 |
| `functional/test_turn.py` | T 组:对话轮次 / 流式 / 工具调用 / 统计 |
| `functional/test_session.py` | P 组:会话持久化 / 恢复 / fork / plan 落盘 |
| `functional/test_cli.py` | C 组:headless CLI 的**语义**断言 |
| `functional/test_keys.py` | K 组:键盘交互(权限循环 / 历史召回 / 外部编辑器 / 复制 / 多行 / vim) |
| `functional/test_render.py` | V 组:渲染与视觉(宽字符 / 折行 / markdown / 主题 / resize / 小窗) |
| `functional/test_flow.py` | W 组:工作流与容错(审批 / AskUser / loop / 恢复边界 / 坏输入 / 信号) |
| `functional/test_coverage_audit.py` | 覆盖自审门禁(新增命令忘了加用例 → 失败) |
| `rust/run.sh` | Rust 侧总入口 |
| `rust/workspace_tests.sh` | `cargo test` + `clippy` + `fmt` + `doc` |
| `rust/cli_core.sh` | 顶层 CLI 参数矩阵(--help / --version / --cwd / 分派) |
| `rust/cli_modules.sh` | `session` / `traces` 参数矩阵与输出契约 |
| `tools/coverage_audit.py` | 覆盖自审工具(可独立运行,出报告) |
| `tools/probe_slash_commands.py` | slash 命令运行时行为**探针**(产出真值表,不判定) |
| `live/` | live 端到端(真实 LLM + 真实工具,需 API key) |

## 设计要点

- **真二进制 + pty**:能力覆盖层起真实 `reflect-tui` 进程,用 `pyte`
  维护屏幕快照,断言**屏幕上真实出现的文字**。不 mock 渲染层 ——
  接线错误(事件没接到 UI、命令没注册、文件没落)在单元测试里看不见。
- **屏幕断言带超时**:所有「等待某段文字出现」的断言都有墙钟上限
  (默认 30s),回归时**断言失败**并回显屏幕尾部与原始输出,而不是
  拖死整个套件。
- **mock provider 的语义**:`REFLECT_MOCK_SCRIPT` 是 JSONL,**每次模型
  调用消费一行**;脚本耗尽回退 `Hello from mock LLM!`。一次用户提交
  会被引擎调用模型多次(实测 4 次),所以落盘的 assistant 消息是
  **最后一轮**的回复 —— 断言不要写死脚本行与消息的对应关系。
- **隔离 HOME**:每个用例一份 `mktemp` HOME + 独立 cwd,用例结束自动
  删除。`~/.reflect/config.toml` 预写为空配置(显式成立,不依赖宿主机
  残留配置)。
- **覆盖自审是门禁**:`tools/coverage_audit.py` 从 Rust 源码解析出
  `SlashCommand` 枚举 + legacy 通道白名单 + 实现臂,与
  `test_slash.py` 的矩阵做差集。**新增一条 slash 命令却忘了加用例,
  CI 直接红。** 探针工具 `probe_slash_commands.py` 可随时重跑以刷新
  真值表。

## 与 submodule 的关系

本套件的结构与判据对齐 submodule `reflect-agent/tests/`
(统一入口 / 分层 / 覆盖矩阵 / 已知缺陷清单),但覆盖对象不同:

| | submodule `reflect-agent/tests/` | 本仓 `tests/` |
|---|---|---|
| 测什么 | agent 基座的 headless `reflect serve` | 终端 TUI 的表现层 |
| 怎么驱动 | 原始 JSONL wire(`subprocess` + `json`) | pty + pyte 屏幕快照 |
| 客户端 SDK | Python / TS 两套绑定层 | 无(本仓不提供 SDK) |
| 覆盖面 | 17 个子命令 + 19 Op + 工具 + 编排 | 84 条 slash + 11 弹层 + 轮次 + 按键 + 渲染 + 审批/AskUser + 容错 + 持久化 + CLI |

## 相关文档

- [`COVERAGE.md`](COVERAGE.md) —— 功能覆盖矩阵、已知产品限制、已知缺陷、未覆盖清单
- [`../CLAUDE.md`](../CLAUDE.md) —— 项目架构与关键约束
- [`../CONTRIBUTING.md`](../CONTRIBUTING.md) —— 提交前检查与 submodule 工作流
