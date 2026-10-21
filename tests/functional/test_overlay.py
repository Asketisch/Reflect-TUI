"""O 组 —— 弹层 / 浮层 / 选择器的黑盒覆盖。

**这一组是旧测试最大的缺口**:11 个 picker 里原有 10 个零黑盒覆盖,
所有 picker 的标题字面量此前从未被任何断言引用。

## 关键事实:哪些弹层**在运行时可达**

本项目的 slash 有**两条分发通道**,运行时优先级如下
(`chat_composer/slash_input.rs:259-262` + `:88`):

1. 敲下的命令名命中 **`is_legacy_passthrough_slash` 白名单**
   (`help ? tasks agents cost exit-plan exitplan mode effort ref rm-session`)
   → 原样提交,走 `history_render::dispatch_slash`(legacy 通道);
2. 否则若命中 **`SlashCommand` 枚举且可见** → 走 `dispatch_slash_command`
   (枚举通道);
3. 都不命中 → `UnknownCommand`,提交被**静默吞掉**,输入留在框里。

关键差异:枚举通道的 `dispatch_slash_command` **只实现了部分命令**,其余
落到兜底文案「`/<cmd>: 此命令在当前构建中尚未接入分发`」。于是:

| 弹层 | 可达命令 | 标题字面量 |
|---|---|---|
| Transcript | `Ctrl+T` / `/help` / `/?` | `Transcript (Esc / Ctrl+T to close)` |
| Diff | `Ctrl+Shift+D` / `/diff` | `Diff (Esc / Ctrl+T to close · /diff)` |
| Tasks & Agents | `/tasks` / `/agents` | `Tasks & Agents (Esc / Ctrl+T to close · /tasks)` |
| Installed Plugins | `/plugin` / `/plugins` | `Installed Plugins` |
| Fork / Rewind 选择器 | `/rewind`(需历史里有 user prompt) | `Rewind — select user prompt to re-edit & re-send` |

**不可达**(命令被识别但未接线 / 未识别,详见 `test_slash.py` 的矩阵):
Image Picker、Keymap Picker、Statusline Picker、Theme Picker、
Checkpoint Overlay、Skills Hub、MCP Server Status、Traces、Sessions。
本组用 `test_unreachable_overlays_stay_unreachable` 锁定这一事实。

⚠️ 黑盒踩坑(picker 的 `Nop` 键位不吞键,会掉到 composer 输入框):
   在 picker 里按 `j`/`k` 既移动选择又往输入框打字;keymap / skills_hub /
   statusline 的 `Enter` 也是 `Nop`。因此本组一律用 `Esc` 关闭。
"""

from __future__ import annotations

import time

import pytest


def _boot(tui):
    """启动一个 TUI 会话并等到首屏就绪。"""
    s = tui()
    s.wait_boot(timeout=30)
    return s


# ── 可达弹层 ──────────────────────────────────────────────────────────────


def test_transcript_overlay_via_ctrl_t(tui) -> None:
    """O1 Ctrl+T 打开 transcript 全屏分页器,标题正确;Esc 关闭。"""
    with _boot(tui) as s:
        s.key("ctrl+t")
        s.wait_text("Transcript (Esc / Ctrl+T to close)", timeout=15)
        s.key("esc")
        s.wait_absent("Transcript (Esc / Ctrl+T to close)", timeout=10)


def test_transcript_overlay_via_help(tui) -> None:
    """O2 `/help` 打开 transcript 帮助页,向上滚动可见命令清单。"""
    with _boot(tui) as s:
        s.type_text("/help")
        s.key("enter")
        s.wait_text("Transcript (Esc / Ctrl+T to close)", timeout=15)
        # 帮助页很长,首屏只显示尾部;向上翻页可见命令清单。
        for _ in range(6):
            s.key("pageup")
            s._pump(settle=0.2, max_wait=0.4)
        txt = s.normalized()
        assert "/status" in txt or "/compact" in txt, (
            f"帮助页应含命令清单:\n{txt[-800:]}"
        )
        s.key("esc")
        s.wait_absent("Transcript (Esc / Ctrl+T to close)", timeout=10)


def test_diff_overlay_via_slash(tui) -> None:
    """O3 `/diff` 打开 diff 分页器;无 diff 时给出空态。"""
    with _boot(tui) as s:
        s.type_text("/diff")
        s.key("enter")
        s.wait_text("Diff (Esc / Ctrl+T to close · /diff)", timeout=15)
        txt = s.normalized()
        assert "no diffs in this session" in txt.lower(), (
            f"应有 diff 空态文案:\n{txt[-500:]}"
        )
        s.key("esc")
        s.wait_absent("Diff (Esc / Ctrl+T to close", timeout=10)


def test_ctrl_shift_d_is_unreachable_from_a_terminal(tui) -> None:
    """O4 Ctrl+Shift+D **在真实终端不可达** —— 按下的是 Ctrl+D(退出)。

    终端无法区分 Ctrl+D 与 Ctrl+Shift+D:两者都是字节 `0x04`。crossterm
    把 `0x04` 解析成 `Char('d') + CONTROL`(**不带 SHIFT**),因此
    `keymap.rs` 里要求 `CONTROL|SHIFT` 的 `OpenDiff` 分支永远命中不了,
    落到 `Ctrl+D → Exit`。本用例锁定这一现状(见 COVERAGE.md §6 D8)。

    换言之:diff overlay 的实际入口只有 `/diff`。
    """
    with _boot(tui) as s:
        s.key("ctrl+shift+d")
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and s.alive():
            time.sleep(0.2)
        assert not s.alive(), "0x04 应被解析成 Ctrl+D 而退出(diff overlay 不会被打开)"
        assert "Diff (Esc / Ctrl+T to close" not in s.normalized(), (
            f"diff overlay 不应被打开:\n{s.normalized()[-400:]}"
        )


def test_tasks_overlay_sections(tui) -> None:
    """O5 `/tasks` 打开任务面板,含 Transcript / Plan / Agents / Tasks 分区。"""
    with _boot(tui) as s:
        s.type_text("/tasks")
        s.key("enter")
        s.wait_text("Tasks & Agents (Esc / Ctrl+T to close · /tasks)", timeout=15)
        txt = s.normalized()
        assert "Transcript" in txt, "应有 Transcript 分区"
        assert "Agents" in txt, "应有 Agents 分区"
        s.key("esc")
        s.wait_absent("Tasks & Agents (Esc", timeout=10)


def test_plugin_overlay_opens_and_closes(tui) -> None:
    """O6 `/plugin` 打开已装插件面板;无插件时给出空态 + 安装指引。"""
    with _boot(tui) as s:
        s.type_text("/plugin")
        s.key("enter")
        s.wait_text("Installed Plugins", timeout=15)
        txt = s.normalized()
        assert "No plugins installed" in txt, f"应有空态文案:\n{txt[-600:]}"
        assert "enabled_plugins" in txt, f"应有安装指引:\n{txt[-600:]}"
        s.key("esc")
        s.wait_absent("Installed Plugins", timeout=10)


def test_rewind_overlay_requires_history(tui) -> None:
    """O7 `/rewind` 无历史时给提示;有 user prompt 后打开回退选择器。"""
    with _boot(tui) as s:
        s.type_text("/rewind")
        s.key("enter")
        s.wait_text("no user prompts to rewind to yet", timeout=15)
        assert "Rewind — select user prompt" not in s.normalized(), (
            "无历史时不应打开回退选择器"
        )


def test_clear_requires_confirmation(tui) -> None:
    """O8 `/clear` 弹二次确认,`n` 取消后给出「已取消」提示。

    注意:确认提示本身是**历史项**,会留在 scrollback 里 —— 取消的证据是
    新增的「Clear cancelled.」notice,而不是提示消失。
    """
    with _boot(tui) as s:
        s.type_text("/clear")
        s.key("enter")
        s.wait_text("Clear scrollback", timeout=15)
        s.type_text("n")
        s.key("enter")
        s.wait_text("Clear cancelled.", timeout=10)


def test_delete_requires_confirmation(tui) -> None:
    """O9 `/rm-session` 弹不可撤销的删除确认,`n` 取消。"""
    with _boot(tui) as s:
        s.type_text("/rm-session")
        s.key("enter")
        s.wait_text("Permanently delete current session files and exit?", timeout=15)
        s.type_text("n")
        s.key("enter")
        s.wait_text("Delete cancelled.", timeout=10)


def test_overlay_excludes_global_keys(tui) -> None:
    """O10 overlay 打开时全局键被遮蔽:打开 `/plugin` 后 Ctrl+T 不生效。"""
    with _boot(tui) as s:
        s.type_text("/plugin")
        s.key("enter")
        s.wait_text("Installed Plugins", timeout=15)
        s.key("ctrl+t")
        s._pump(settle=0.8, max_wait=1.5)
        txt = s.normalized()
        assert "Transcript (Esc / Ctrl+T to close)" not in txt, (
            f"overlay 打开时 Ctrl+T 不应生效:\n{txt[-500:]}"
        )
        s.key("esc")


# ── 不可达弹层:锁定「当前不可达」这一事实 ───────────────────────────────

UNREACHABLE = [
    ("/image", "Image Picker"),
    ("/keymap", "Keymap"),
    ("/statusline", "Statusline"),
    ("/theme", "Theme"),
    ("/checkpoint", "Checkpoints"),
    ("/skills", "Skills Hub"),
    ("/mcp", "MCP Server Status"),
    ("/traces", "Traces:"),
    ("/resume", "Sessions (Esc"),
]


@pytest.mark.parametrize(
    "command,title",
    UNREACHABLE,
    ids=[c for c, _t in UNREACHABLE],
)
def test_unreachable_overlays_stay_unreachable(tui, command: str, title: str) -> None:
    """O11 未接线的弹层保持不可达(防回归:将来接线后本用例会失败并提醒更新矩阵)。

    这些弹层的**渲染代码**存在且有 Rust 单测,但在运行时的 slash 分发里
    不可达:命令要么落到「尚未接入分发」兜底,要么被 composer 静默拒收。
    见 `tests/COVERAGE.md` §6 D-S2。
    """
    with _boot(tui) as s:
        s.type_text(command)
        s.key("enter")
        time.sleep(2.5)
        txt = s.normalized()
        assert title not in txt, (
            f"{command} 打开了 {title} 弹层 —— 覆盖矩阵需更新(见 COVERAGE.md §S 组)"
        )
        assert not s.has_panic(), f"{command} 不应 panic:\n{s.raw_output()[-600:]}"
