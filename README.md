# Reflect-TUI

[![CI](https://github.com/Asketisch/Reflect-TUI/actions/workflows/ci.yml/badge.svg)](https://github.com/Asketisch/Reflect-TUI/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%20%7C%20edition%202024-orange.svg)](rust-toolchain.toml)
[![English](https://img.shields.io/badge/README-English-blue.svg)](README.en.md)

> 📖 **English version**: [README.en.md](README.en.md) — translated README for international readers.
> 注意:本仓库所有代码注释、commit message、PR 描述仍**强制使用中文**(详见 [CONTRIBUTING.md](CONTRIBUTING.md))。

Reflect Agent 的终端交互式客户端,面向终端 AI Agent 工作流,基于 [ratatui](https://github.com/ratatui-org/ratatui)。

本仓库**只包含 TUI 表现层**(`reflect-tui` crate),agent 核心引擎(AgentThread / StateGraph / 工具 / 协议等)由 [Reflect-Agent](https://github.com/Asketisch/Reflect-Agent) 提供,通过 **git submodule** 复用。

## 仓库关系

```
Reflect-Agent (核心引擎,submodule 提供方)
  └─ crates/{protocol,abilities,resources,orchestration,integrations,runtime}
  └─ reflect-core (AgentThread + StateGraph)

Reflect-TUI (本仓库)  ──submodule──► Reflect-Agent
  └─ crates/reflect-tui (ratatui 交互界面 + 事件循环)
  └─ reflect-tui 二进制 (终端入口)
```

## 快速开始

```bash
# 1. 克隆(带 submodule)
git clone --recursive https://github.com/Asketisch/Reflect-TUI.git
cd Reflect-TUI

# 2. 构建
make build

# 3. 配置 LLM provider(二选一)
#    方式一:环境变量
export ANTHROPIC_API_KEY=sk-ant-...
#    方式二:~/.reflect/config.toml 设置 [active]/[anthropic]/[openai]

# 4. 运行
make run
# 或直接:
./target/release/reflect-tui
```

Rust 版本由 `rust-toolchain.toml` 锁定(stable,MSRV 1.85,edition 2024)。

## TUI 启动参数

无子命令默认启动 TUI。常用参数(完整列表见 `reflect-tui --help`):

```bash
reflect-tui [--cwd PATH] [--prompt TEXT] [-c] [-r N] [--resume ID]
            [--plan-mode] [--fullscreen] [--auto-root]
            [--ephemeral-tasks] [--ephemeral-teams]

# 示例
reflect-tui "帮我重构这个模块"        # 直接带 prompt 启动
reflect-tui --resume sess_abc123     # 恢复指定 session
reflect-tui -c                       # 继续上一次会话
reflect-tui -r 2                     # 恢复最近第 2 个 session
reflect-tui --plan-mode              # 以 Plan mode 启动
reflect-tui --fullscreen             # 全屏模式(备用屏;也可用 REFLECT_TUI_FULLSCREEN=1)
reflect-tui --cwd /path/to/project   # 指定工作目录
```

## TUI 内 slash 命令

输入 `/` 弹出命令列表。常用命令:

| 命令 | 说明 |
|------|------|
| `/model` | 切换模型与推理强度 |
| `/status` / `/usage` | 查看会话配置 / token 用量 |
| `/plan` | 进入 Plan mode(先出 plan 草稿,审批后执行) |
| `/goal` | 设置/查看长任务目标 |
| `/loop <secs> <cmd>` | 按间隔重复执行命令(`/loop stop` 取消) |
| `/fork` | 从历史 user prompt 处 fork 新子会话 |
| `/rename <name>` | 重命名当前 session |
| `/resume` / `/archive` / `/delete` | 恢复 / 归档 / 删除会话 |
| `/compact` | 压缩上下文 |
| `/review` | 审查当前变更 |
| `/mcp` / `/skills` / `/plugins` / `/hooks` | 管理扩展能力 |
| `/theme` / `/keymap` / `/vim` | 外观与按键 |
| `/quit` | 退出 |

## CLI 子命令

带子命令走 headless 路径,执行完即退出,不进 TUI。

### `session`(本地 session 管理)

```bash
# 列本地 session(支持 --limit/-n、--model 子串过滤)
reflect-tui session ls -n 20
reflect-tui session ls --model anthropic

# 看某 session 元数据 + 前 5 条消息预览 + 累计 tokens/cost
reflect-tui session show <id-or-prefix>

# 删除 session(--yes 跳过确认)
reflect-tui session rm <id> --yes

# fork 父会话截至当前的完整历史到新子会话
reflect-tui session fork <id> --branch my-branch
# fork 后打印 child id,提示:
#   Resume with: reflect-tui --resume <child_id> "..."

# 给会话设置人可读名(写到 ~/.reflect/sessions/_names/<id>.name)
reflect-tui session rename <id> "my session name"

# 导出为 markdown(无 --out 打印 stdout)
reflect-tui session export <id>
reflect-tui session export <id> --out session.md
```

`<id>` 支持完整 UUID 或前 8 字符前缀(唯一命中)。

### `traces`(本地 LLM 调用记录)

```bash
# 列 traces(每个 session 一行:session_id / title / calls / last / model)
reflect-tui traces ls -n 20

# 看某 session 的逐条 LLM 调用详情(model / latency / usage / request+response 摘要)
reflect-tui traces show <id-or-prefix>
```

数据来自 `~/.reflect/traces/model-io-sess_<session_id>.jsonl`(由 reflect-telemetry 写入)。

## 构建、测试与发布

```bash
make build          # release 构建(LTO)
make test           # cargo test --workspace
make test-fast      # nextest 加速(需 cargo-nextest)
make check-fast C=reflect-tui   # 单 crate 快速类型检查
make dist           # 构建 + 打包当前平台分发包(tar.gz/zip)到 dist/
make install        # 安装到 /usr/local/bin(或 ~/.local/bin,见 scripts/install.sh)
make help           # 全部 target
```

- `./scripts/build.sh --fast` 用 `release-fast` profile(无 LTO,增量编译快)。
- `test/` 目录集中存放全部测试脚本(`test/README.md` 有分层说明):`python3 test/run_all.py` 一键跑离线全量(unit / rust / cli / binary / mock E2E),`python3 test/test_workflow_live.py` 是本地 live 端到端脚本(真实 LLM + 真实工具),手动回归 Plan mode / 子代理 / `/goal` / `/loop` 等完整链路。
- CI(`.github/workflows/release.yml`)仅在 `v*` tag 时于 macOS/Linux/Windows 并行构建分发包,需递归 checkout submodule。

## 升级核心

Reflect-Agent 是 submodule。升级核心到最新:

```bash
git submodule update --remote reflect-agent
git add reflect-agent
git commit -m "chore: bump reflect-agent submodule"
```

本地联调核心改动:在 Reflect-Agent 仓库改代码 → `git push` → 本仓库 `git submodule update --remote`。完整流程(含分支切换、指针记录)见 [CONTRIBUTING.md](CONTRIBUTING.md) 的「submodule 工作流」小节。

## 相关文档

- [CONTRIBUTING.md](CONTRIBUTING.md) — 贡献指南(中文注释规范、submodule 工作流、PR 要求)
- [CLAUDE.md](CLAUDE.md) — AI 助手项目说明(强制中文注释规范的原文)
- [CHANGELOG.md](CHANGELOG.md) — 变更日志

## 社区与安全

- [CONTRIBUTING.md](CONTRIBUTING.md) — 如何贡献代码
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) — 行为准则
- [SECURITY.md](SECURITY.md) — 漏洞披露策略
- [GitHub Discussions](https://github.com/Asketisch/Reflect-TUI/discussions) — 提问与讨论
- [GitHub Issues](https://github.com/Asketisch/Reflect-TUI/issues) — Bug 报告与功能请求

## 致谢 / 第三方归属

本项目基于以下开源工作构建(完整归因见 [NOTICE](NOTICE)):

- [Reflect-Agent](https://github.com/Asketisch/Reflect-Agent) — 核心引擎(Apache-2.0),通过 git submodule 引入
- [nornagon/ratatui](https://github.com/nornagon/ratatui) 与 [nornagon/crossterm](https://github.com/nornagon/crossterm) — 暴露 unstable features 的 fork(MIT)
