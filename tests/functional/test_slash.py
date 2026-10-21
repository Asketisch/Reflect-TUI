"""S 组 —— slash 命令面(数据驱动的全量矩阵)。

**为什么是矩阵而不是逐条 test 函数**:本项目有 60+ 条 slash 命令,每条都有
明确的运行时结局(真行为 / 占位提示 / 未识别 / 退出)。逐条写 test 函数会让
「新增一条命令忘了加用例」这类漂移无法被发现 —— 矩阵表是**单一事实源**,
`tests/tools/coverage_audit.py` 会静态核对「Rust 里声明的命令」与「本表
覆盖的命令」,缺口直接判失败。

**覆盖口径**:每条命令至少一条**行为级断言**(屏幕可见文本 / 退出行为),
不是"退出码 0"。

## 三类结局(实测于 release 二进制,证据见 tests/COVERAGE.md §S 组)

| 类别 | 含义 | 断言 |
|---|---|---|
| `wired` | 已接线,有真实行为 | 屏幕出现指定文本 |
| `placeholder` | 命令被识别,但本构建未接入 | 屏幕出现「尚未接入分发」等占位文案 |
| `unrecognized` | 命令**未被识别**,提交被静默吞掉 | 原文留在输入框,且无任何 notice |
| `exit` | 退出 TUI | 进程退出码 0 |

> ⚠️ `unrecognized` 这一类是**当前实现的已知行为**(见 COVERAGE.md §6 D-S1):
> composer 的 `validate_submission` 只放行「枚举里可见的命令」与
> `is_legacy_passthrough_slash` 白名单,其余命令的 Info cell
> ("Unrecognized command '/x'")被 `ComposerInput::handle_app_events` 丢弃,
> 用户只看到「输入还在框里、没发出去」。本组**锁定当前行为**,防止静默漂移。
"""

from __future__ import annotations

import re
import time

import pytest

# ── 结局矩阵 ──────────────────────────────────────────────────────────────
# (命令, 参数, 类别, 期望文本)
# 期望文本取自真二进制的实测输出(见 tests/COVERAGE.md §S 组逐条证据)。
MATRIX: list[tuple[str, str, str, str]] = [
    # ── wired:有真实行为 ──────────────────────────────────────────────
    ("model", "mock/gpt-4o", "wired", "switched to 'mock/gpt-4o'"),
    ("vim", "", "wired", "Vim mode enabled"),
    ("status", "", "wired", "/status: model:"),
    ("usage", "", "wired", "/usage: input:"),
    ("cost", "", "wired", "/cost:"),
    ("subagents", "", "wired", "/subagents:"),
    ("compact", "", "wired", "Context compaction requested"),
    ("effort", "high", "wired", "reasoning effort set to high"),
    ("goal", "ship the release", "wired", "Goal mode entered"),
    ("loop", "30 /status", "wired", "every 30s"),
    ("side", "", "wired", "/side: 用法 /side <message>"),
    ("side", "hello", "wired", "侧边对话「hello」已收到"),
    ("btw", "hello", "wired", "侧边对话「hello」已收到"),
    ("mcp", "", "wired", "MCP server"),
    ("skills", "", "wired", "技能管理未接入当前构建"),
    ("hooks", "", "wired", "hooks 浏览未接入当前构建"),
    ("rewind", "", "wired", "no user prompts to rewind to yet"),
    ("clear", "", "wired", "Clear scrollback?"),
    ("rm-session", "", "wired", "Permanently delete current session files"),
    ("plan", "refactor the auth module", "wired", "Plan request:"),
    ("mode", "plan", "wired", "[plan]"),
    ("permissions", "", "wired", "[accept_edits]"),
    ("exit-plan", "", "wired", "[1] Auto / [2] Manual / [3]/[Esc] Revise"),
    ("exitplan", "", "wired", "[1] Auto / [2] Manual / [3]/[Esc] Revise"),
    ("ref", "", "wired", "switch search modes"),
    ("diff", "", "panel", "Diff (Esc / Ctrl+T to close"),
    ("plugin", "", "panel", "Installed Plugins"),
    ("plugins", "", "panel", "Installed Plugins"),
    ("help", "", "panel", "Transcript (Esc / Ctrl+T to close)"),
    ("?", "", "panel", "Transcript (Esc / Ctrl+T to close)"),
    ("tasks", "", "panel", "Tasks & Agents (Esc / Ctrl+T to close"),
    ("agents", "", "panel", "Tasks & Agents (Esc / Ctrl+T to close"),
    ("exit", "", "exit", ""),
    ("quit", "", "exit", ""),
    # ── placeholder:被识别但本构建未接入 ─────────────────────────────
    ("ide", "", "placeholder", "尚未接入分发"),
    ("keymap", "", "placeholder", "尚未接入分发"),
    ("experimental", "", "placeholder", "尚未接入分发"),
    ("approve", "", "placeholder", "尚未接入分发"),
    ("memories", "", "placeholder", "尚未接入分发"),
    ("import", "", "placeholder", "尚未接入分发"),
    ("review", "", "placeholder", "尚未接入分发"),
    ("new", "", "placeholder", "尚未接入分发"),
    ("archive", "", "placeholder", "尚未接入分发"),
    ("delete", "", "placeholder", "尚未接入分发"),
    ("resume", "", "placeholder", "尚未接入分发"),
    ("fork", "", "placeholder", "尚未接入分发"),
    ("app", "", "placeholder", "尚未接入分发"),
    ("init", "", "placeholder", "尚未接入分发"),
    ("agent", "", "placeholder", "尚未接入分发"),
    ("copy", "", "placeholder", "尚未接入分发"),
    ("raw", "", "placeholder", "尚未接入分发"),
    ("mention", "", "placeholder", "尚未接入分发"),
    ("debug-config", "", "placeholder", "尚未接入分发"),
    ("title", "", "placeholder", "尚未接入分发"),
    ("statusline", "", "placeholder", "尚未接入分发"),
    ("theme", "", "placeholder", "尚未接入分发"),
    ("pets", "", "placeholder", "尚未接入分发"),
    ("prompt", "", "placeholder", "尚未接入分发"),
    ("logout", "", "placeholder", "尚未接入分发"),
    ("feedback", "", "placeholder", "尚未接入分发"),
    ("ps", "", "placeholder", "尚未接入分发"),
    ("stop", "", "placeholder", "尚未接入分发"),
    ("clean", "", "placeholder", "尚未接入分发"),
    ("personality", "", "placeholder", "尚未接入分发"),
    ("debug-m-drop", "", "placeholder", "尚未接入分发"),
    ("debug-m-update", "", "placeholder", "尚未接入分发"),
    ("rename", "some-name", "placeholder", "尚未接入分发"),
    ("pet", "", "placeholder", "尚未接入分发"),
    # ── unrecognized:命令未被识别,输入被静默吞掉 ─────────────────────
    ("setup-default-sandbox", "", "unrecognized", ""),
    ("sandbox-add-read-dir", "/tmp", "unrecognized", ""),
    ("apps", "", "unrecognized", ""),
    ("rollout", "", "unrecognized", ""),
    ("test-approval", "", "unrecognized", ""),
    ("version", "", "unrecognized", ""),
    ("doctor", "", "unrecognized", ""),
    ("history", "", "unrecognized", ""),
    ("files", "", "unrecognized", ""),
    ("branch", "", "unrecognized", ""),
    ("image", "", "unrecognized", ""),
    ("img", "", "unrecognized", ""),
    ("checkpoint", "", "unrecognized", ""),
    ("traces", "", "unrecognized", ""),
    ("think", "", "unrecognized", ""),
    ("thinking", "", "unrecognized", ""),
    ("retry", "", "unrecognized", ""),
]


def _submit(s, command: str, args: str) -> None:
    """在 composer 里敲一条 slash 命令并回车。"""
    text = f"/{command} {args}".strip()
    s.type_text(text)
    s.key("enter")


@pytest.mark.parametrize(
    "command,args,kind,expect",
    MATRIX,
    ids=[f"{c} {a}".strip() for c, a, _k, _e in MATRIX],
)
def test_slash_command_outcome(tui, command: str, args: str, kind: str, expect: str) -> None:
    """S1 每条 slash 命令的运行时结局符合矩阵。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        _submit(s, command, args)

        if kind == "exit":
            deadline = time.monotonic() + 15
            rc = None
            while time.monotonic() < deadline:
                rc = s.returncode()
                if rc is not None:
                    break
                time.sleep(0.15)
            assert rc == 0, f"/{command} 应干净退出(0),实际 {rc}"
            return

        if kind == "unrecognized":
            # 未识别的命令:输入保留在 composer,且没有任何 notice。
            time.sleep(2.0)
            txt = s.normalized()
            assert "⚠" not in txt, f"/{command} 未被识别时不应出现 notice:\n{txt[-400:]}"
            assert f"/{command}" in txt, (
                f"/{command} 未被识别时应留在输入框:\n{txt[-400:]}"
            )
            assert not s.has_panic(), f"/{command} 不应 panic"
            return

        # wired / placeholder / panel:屏幕上应出现期望文本。
        if kind == "panel":
            # 面板类:先等面板标题,再 Esc 关闭。
            s.wait_text(expect, timeout=20)
        else:
            s.wait_text(expect, timeout=20)
        assert not s.has_panic(), f"/{command} 不应 panic:\n{s.raw_output()[-600:]}"


def test_unknown_command_is_silently_rejected(tui) -> None:
    """S2 完全不存在的命令:不报错、不提交,输入留在框里。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        _submit(s, "definitely-not-a-command", "")
        time.sleep(2.0)
        txt = s.normalized()
        assert "definitely-not-a-command" in txt, "未知命令应留在输入框"
        assert "Unrecognized" not in txt, (
            f"当前实现丢弃了 Unrecognized 提示(已知行为 D-S1),不应出现在屏幕上:\n{txt[-400:]}"
        )


def test_command_popup_lists_commands(tui) -> None:
    """S3 输入 `/` 弹出命令补全列表,含命令名与其描述。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("/")
        s._pump(settle=1.2, max_wait=2.0)
        txt = s.normalized()
        # 弹窗按枚举顺序展示**当前可见**的命令(列表会被面板高度截断,
        # 因此只断言首屏确实可见的几条)。
        assert "/model" in txt, f"命令弹窗应含 /model:\n{txt[-800:]}"
        assert "choose what model" in txt, f"命令弹窗应显示命令描述:\n{txt[-800:]}"
        assert "/permissions" in txt, f"命令弹窗应含 /permissions:\n{txt[-800:]}"


def test_command_popup_filters(tui) -> None:
    """S4 命令弹窗按前缀过滤。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("/stat")
        s._pump(settle=1.2, max_wait=2.0)
        txt = s.normalized()
        assert "status" in txt, f"过滤后应含 status:\n{txt[-800:]}"


def test_mode_switches_permission_mode(tui) -> None:
    """S5 `/mode plan` 切到 plan 权限模式:composer 占位符与 HUD 同步变化。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        s.wait_text("Send a message", timeout=10)
        _submit(s, "mode", "plan")
        s.wait_text("describe the task or /exit-plan to leave plan mode", timeout=15)
        txt = s.normalized()
        assert "[plan]" in txt, f"HUD 应显示 plan 模式标记:\n{txt[-500:]}"


def test_escaped_slash_is_plain_text(tui) -> None:
    """S6 名字里带 `/` 的输入不按命令解析(当普通文本)。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        _submit(s, "a/b", "")
        time.sleep(2.0)
        txt = s.normalized()
        assert "a/b" in txt, "带斜杠的文本应作为普通消息上屏"
        assert "尚未接入分发" not in txt, "不应被当作命令解析"


def test_leading_space_allows_unknown_command(tui) -> None:
    """S7 行首空格 → composer 不做「未知命令」拦截,按普通消息提交。

    `slash_input.rs:85` 的 `input_starts_with_space` 分支:行首有空格时,
    `validate_submission` 直接返回 Valid —— 未知命令不会被静默拒收,而是
    作为普通文本发给 agent。
    """
    with tui() as s:
        s.wait_boot(timeout=30)
        s.type_text(" /not-a-real-command")
        s.key("enter")
        time.sleep(2.5)
        txt = s.normalized()
        assert "/not-a-real-command" in txt, (
            f"行首空格的未知命令应作为普通消息提交:\n{txt[-500:]}"
        )


def test_leading_space_still_parses_known_command(tui) -> None:
    """S8 行首空格**不**阻止已知命令被解析(当前行为,见 COVERAGE.md §6 D-S3)。

    `validate_submission` 的 `input_starts_with_space` 只影响「未知命令」
    的拦截;legacy 通道的 `dispatch_slash` 入口做了 `text.trim()`,所以
    `" /clear"` 仍会被解析成 `/clear` 并弹二次确认。本用例锁定这一现状,
    避免静默漂移。
    """
    with tui() as s:
        s.wait_boot(timeout=30)
        s.type_text(" /clear")
        s.key("enter")
        s.wait_text("Clear scrollback", timeout=15)
        s.type_text("n")
        s.key("enter")
        # 确认提示是**历史项**,不会被移除;取消以新 notice 表明。
        s.wait_text("Clear cancelled.", timeout=10)
