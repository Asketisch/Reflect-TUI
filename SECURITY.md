# 安全策略

## 支持的版本

| 版本 | 支持 |
| ---- | ---- |
| 最新 release | ✅ |
| 旧版本 | ❌（请先升级） |

## 报告漏洞

**请勿通过公开 issue 报告安全漏洞。**

请使用 GitHub 的 [私密漏洞报告](https://github.com/Asketisch/Reflect-TUI/security/advisories/new)
（Security → Advisories → New draft security advisory）。

报告时请尽量包含：

- 受影响的版本 / commit
- 复现步骤或 PoC
- 影响评估（数据泄露 / 任意执行 / 提权等）
- 缓解建议（若有）

我们会在 **72 小时内**确认收到，并在修复前与您协商披露时间线。

## 安全设计要点（供审计参考）

- **沙箱**：bash 工具支持 macOS Seatbelt / Linux Landlock
  （`[sandbox].os_level = true` 或 `REFLECT_SANDBOX_OS_LEVEL=1`）。
- **权限链**：`~/.reflect/config.toml [permissions]`（只读展平）+
  `~/.reflect/permissions.toml`（运行期 always-allow 落盘）→ 链式解析；
  MCP / plugin / runtime 来源的工具强制 `Prompt` 安全下限，防静态放行。
- **脱敏**：rollout 落盘与工具输出默认经 `Sanitizer`（10 类密钥 pattern，
  `[sanitize]` 可扩展），16KiB 截断。
- **信任边界**：TUI 是进程内宿主，wire 协议（Submission / EventMsg）的
  外部暴露面在引擎仓库的 `reflect serve` stdio 模式；MCP server / LSP
  server 是按 config.toml 显式配置拉起的外部进程。
- **依赖**：每周 `cargo audit`（audit.yml）；git fork 的 ratatui/crossterm
  不在 crates.io 扫描范围，需人工跟 GSA。

## 已知取舍

- pty 测试 harness（`test/`）与 mock provider 仅用于本地测试，
  不应暴露给不可信输入环境。
