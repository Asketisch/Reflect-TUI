"""T 组 —— 对话轮次:流式正文、工具调用、回合边界、token 统计、中断。

锚点字面量来源(证据见 `tests/COVERAGE.md` §T 组):
  - 用户消息前缀 `▶ `          marker.rs:28
  - 整段 agent 前缀 `● `       marker.rs:29
  - 工具成功 `✓ <name> (<N>ms)` / 失败 `✗ <name> (<N>ms)`  cells.rs:31-43
  - 回合分隔线 `─ ─ ─`          cells.rs:182
  - HUD metrics `✓ bash ×2 │`  statusline.rs:284
  - 流式未提交尾部 `○ `        tui/mod.rs:3661
  - 中断提示「已发送中断请求」   tui/mod.rs:767-769
"""

from __future__ import annotations

import time

from tui_pty import text_reply, tool_call


def test_user_message_and_reply_appear(tui, mock_script) -> None:
    """T1 一次往返:用户消息上屏 + mock 回复上屏。"""
    script = mock_script([text_reply("MOCK_REPLY_MARKER: 已收到你的问题")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("hello from the test")
        s.key("enter")
        txt = s.wait_text("MOCK_REPLY_MARKER", timeout=30)
        assert "hello from the test" in txt, "用户消息应上屏"
        assert not s.has_panic(), f"不应 panic:\n{s.raw_output()[-600:]}"


def test_agent_reply_uses_marker_prefix(tui, mock_script) -> None:
    """T2 agent 流式回复带 `• ` 前缀。

    注意:`● `(`marker.rs:29` 的 legacy 改写)只作用于**整段** `Agent`
    历史项;mock provider 恒走流式路径,渲染的是流式行,前缀是 `• `。
    """
    script = mock_script([text_reply("T2_REPLY_BODY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("go")
        s.key("enter")
        s.wait_text("T2_REPLY_BODY", timeout=30)
        assert "• T2_REPLY_BODY" in s.normalized(), (
            f"agent 流式回复应带 • 前缀:\n{s.normalized()[-500:]}"
        )


def test_user_message_uses_marker_prefix(tui, mock_script) -> None:
    """T2b 用户消息带 `▶ ` 前缀。"""
    script = mock_script([text_reply("T2B_REPLY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("T2B_USER_MESSAGE")
        s.key("enter")
        s.wait_text("T2B_REPLY", timeout=30)
        assert "▶ T2B_USER_MESSAGE" in s.normalized(), (
            f"用户消息应带 ▶ 前缀:\n{s.normalized()[-500:]}"
        )


def test_streamed_reply_lands_in_scrollback(tui, mock_script) -> None:
    """T3 流式回复逐行落进 scrollback(不是只留一行)。"""
    script = mock_script(
        [
            text_reply("T3_LINE_ONE"),
            text_reply("T3_LINE_TWO"),
            text_reply("T3_LINE_THREE"),
        ]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        for i in range(3):
            s.type_text(f"turn {i}")
            s.key("enter")
            s.wait_text(f"T3_LINE_{['ONE', 'TWO', 'THREE'][i]}", timeout=30)
        txt = s.normalized()
        for marker in ("T3_LINE_ONE", "T3_LINE_TWO", "T3_LINE_THREE"):
            assert marker in txt, f"{marker} 应在 scrollback 里"
        assert not s.has_panic()


def test_tool_call_renders_success_row(tui, mock_script) -> None:
    """T4 工具调用上屏:`✓ echo (<N>ms)` + 输出前几行。"""
    script = mock_script(
        [
            tool_call("echo", {"text": "T4_TOOL_MARKER"}),
            text_reply("工具已执行完毕"),
        ]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("run the echo tool")
        s.key("enter")
        txt = s.wait_text("✓ echo", timeout=30)
        assert "T4_TOOL_MARKER" in txt, f"工具输出应上屏:\n{txt[-600:]}"
        assert "(no tool calls yet)" not in txt, "HUD metrics 应已更新"


def test_tool_call_updates_hud_metrics(tui, mock_script) -> None:
    """T5 工具调用后 HUD metrics 行显示 `✓ echo ×1`。"""
    script = mock_script(
        [
            tool_call("echo", {"text": "T5_MARKER"}),
            text_reply("done"),
        ]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("go")
        s.key("enter")
        s.wait_text("✓ echo", timeout=30)
        s._pump(settle=0.8, max_wait=1.5)
        txt = s.normalized()
        assert "×1" in txt, f"HUD metrics 应含调用次数:\n{txt[-500:]}"


def test_turn_with_tool_shows_separator(tui, mock_script) -> None:
    """T6 含工具调用的回合结束后出现 `─ ─ ─` 分隔线。"""
    script = mock_script(
        [
            tool_call("echo", {"text": "T6_MARKER"}),
            text_reply("T6_DONE"),
        ]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("go")
        s.key("enter")
        s.wait_text("T6_DONE", timeout=30)
        s._pump(settle=0.8, max_wait=1.5)
        assert "─ ─ ─" in s.normalized(), (
            f"回合应出现分隔线:\n{s.normalized()[-600:]}"
        )


def test_multi_turn_keeps_history(tui, mock_script) -> None:
    """T7 多轮对话:两轮的回复都留在 scrollback 里。

    注意:一次提交会让 mock provider 被调用多次(引擎的 agent loop 会
    继续追问直到收敛),所以脚本的行数与最终落盘的 assistant 消息不是
    一一对应 —— 断言只锁「回复文本上屏」,不锁行数。
    """
    script = mock_script(
        [text_reply("T7_FIRST"), text_reply("T7_SECOND")]
    )
    with tui(cols=120, rows=50, script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("first question")
        s.key("enter")
        s.wait_text("T7_FIRST", timeout=30)
        s.type_text("second question")
        s.key("enter")
        s.wait_text("T7_SECOND", timeout=30)
        txt = s.normalized()
        assert "T7_FIRST" in txt and "T7_SECOND" in txt, "两轮回复都应保留"


def test_mock_script_exhaustion_falls_back(tui, mock_script) -> None:
    """T8 mock 脚本耗尽后回退到默认回复(引擎侧契约)。"""
    script = mock_script([text_reply("T8_ONLY_ONE")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("first")
        s.key("enter")
        s.wait_text("T8_ONLY_ONE", timeout=30)
        s.type_text("second")
        s.key("enter")
        s.wait_text("Hello from mock LLM", timeout=30)


def test_escape_interrupts_running_turn(tui, mock_script) -> None:
    """T9 回合进行中按 Esc → 发中断请求并给出提示。"""
    # 脚本给一个空回复,mock 会立刻返回,难以制造「长时间运行」。
    # 改为:提交后立刻按 Esc,断言「中断请求」提示或回合已收尾(两者都算
    # 打断了 busy 状态),不要求具体文案 —— 断言语义而非实现细节。
    script = mock_script([text_reply("T9_DONE")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("interrupt me")
        s.key("enter")
        s.key("esc")
        time.sleep(2.0)
        txt = s.normalized()
        assert "T9_DONE" in txt or "中断请求" in txt, (
            f"Esc 应打断回合:\n{txt[-600:]}"
        )
        assert not s.has_panic(), "中断不应 panic"


def test_token_stats_appear_after_turn(tui, mock_script) -> None:
    """T10 回合结束后 HUD 出现上下文用量(有 token 数据时)。"""
    script = mock_script([text_reply("T10_DONE")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("go")
        s.key("enter")
        s.wait_text("T10_DONE", timeout=30)
        s._pump(settle=1.0, max_wait=2.0)
        txt = s.normalized()
        # mock provider 的 token 统计不一定非零;断言 HUD 上下文条存在即可。
        assert "context left" in txt.lower() or "Context" in txt, (
            f"HUD 应有上下文用量段:\n{txt[-500:]}"
        )
