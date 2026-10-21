"""B 组 —— 启动、渲染锚点与退出生命周期。

**这一组锁的是「用户第一眼看到什么」**:banner、HUD identity 行、HUD metrics
行、composer 占位符、终端标题,以及各种退出路径。

锚点字面量来源(证据见 `tests/COVERAGE.md` §B 组):
  - `  Reflect v<version> — /help for commands`  cells.rs:160
  - `Send a message… (shift+enter for newline)`   events.rs:593 / tui/mod.rs:990
  - ` (no tool calls yet)`                        statusline.rs:273
  - `Ctrl+C to quit`                              statusline.rs:218
  - 终端标题 `Reflect · <model> · [<mode>] · …`   tui/mod.rs:958-971
"""

from __future__ import annotations

import re

from tui_pty import Sandbox, TuiSession

# 终端标题(OSC 0):`Reflect · <model> · [<mode>] · git:(<branch>)`
_TITLE_RE = re.compile(r"\x1b\]0;([^\x07\x1b]*)")


def test_boot_renders_banner_and_version_line(tui) -> None:
    """B1 启动到稳定首屏:banner + 版本提示行。"""
    with tui() as s:
        txt = s.wait_boot(timeout=30)
        assert "Reflect" in txt, f"首屏应含 Reflect 字样: {txt[:300]}"
        assert "/help for commands" in txt, f"首屏应含 /help 提示: {txt[:300]}"


def test_boot_shows_reflect_block_letter_art(tui) -> None:
    """B2 启动 banner 含 "REFLECT" 块字形画(非 braille 动画帧)。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        raw = s.screen_text()
        # 块字形画用 █ / ╗ / ╝ 等制表符号绘制 "REFLECT"。
        assert "█" in raw, "banner 应含块字形字符"
        assert "╗" in raw or "╔" in raw, "banner 应含块字形边框"


def test_hud_identity_shows_model_and_quit_hint(tui) -> None:
    """B3 HUD identity 行:模型名 + 退出提示。"""
    with tui() as s:
        txt = s.wait_boot(timeout=30)
        assert "mock/mock-1" in txt, f"HUD 应显示当前模型: {txt[:400]}"
        assert "Ctrl+C to quit" in txt, f"HUD 应显示退出提示: {txt[:400]}"


def test_hud_metrics_line_shows_no_tool_calls(tui) -> None:
    """B4 HUD metrics 行:尚无工具调用时的占位文案。"""
    with tui() as s:
        txt = s.wait_boot(timeout=30)
        assert "no tool calls yet" in txt, f"HUD metrics 应有占位文案: {txt[:400]}"


def test_composer_placeholder_visible(tui) -> None:
    """B5 composer 占位符文案正确(shift+enter 换行提示)。"""
    with tui() as s:
        txt = s.wait_boot(timeout=30)
        assert "Send a message" in txt, f"应有 composer 占位符: {txt[:400]}"
        assert "shift+enter" in txt, f"占位符应含换行提示: {txt[:400]}"


def test_terminal_title_contains_model(tui) -> None:
    """B6 终端标题(OSC 0)含模型名。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        raw = s.raw_output()
        titles = _TITLE_RE.findall(raw)
        assert titles, "应发出终端标题序列(OSC 0)"
        assert any("mock/mock-1" in t for t in titles), f"标题应含模型名: {titles}"


def test_ctrl_c_exits_cleanly(tui) -> None:
    """B7 Ctrl+C 空闲态直接退出,进程干净结束且无 panic。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        rc = s.quit(timeout=10)
        assert rc is not None, "Ctrl+C 后进程应退出"
        assert rc == 0, f"退出码应为 0,实际 {rc}"
        assert not s.has_panic(), f"退出时不应 panic:\n{s.raw_output()[-800:]}"


def test_ctrl_d_exits_cleanly(tui) -> None:
    """B8 Ctrl+D(EOF 语义)退出,进程干净结束。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        s.key("ctrl+d")
        deadline = 10.0
        rc = None
        import time

        start = time.monotonic()
        while time.monotonic() - start < deadline:
            rc = s.returncode()
            if rc is not None:
                break
            time.sleep(0.15)
        assert rc is not None, "Ctrl+D 后进程应退出"
        assert rc == 0, f"退出码应为 0,实际 {rc}"


def test_resize_rerenders_without_panic(tui) -> None:
    """B9 改窗口尺寸后重绘,不 panic、不崩。"""
    with tui(cols=110, rows=34) as s:
        s.wait_boot(timeout=30)
        s.resize(80, 24)
        s._pump(settle=0.6, max_wait=1.5)
        assert not s.has_panic(), f"resize 后不应 panic:\n{s.raw_output()[-800:]}"
        assert s.alive(), "resize 后进程应存活"


def test_missing_provider_exits_with_error(sandbox: Sandbox) -> None:
    """B10 未配置 provider 且无 REFLECT_MODEL 时,报清晰错误并退出(非 0)。"""
    from tui_pty import TuiSession

    # 显式清掉 provider 变量:boot 需要 provider,缺配置应快速失败而不是挂死。
    env = sandbox.env()
    env.pop("REFLECT_MODEL", None)
    env.pop("OPENAI_API_KEY", None)
    env.pop("ANTHROPIC_API_KEY", None)
    with TuiSession(sandbox, env=env, cols=100, rows=30) as s:
        import time

        deadline = time.monotonic() + 30
        while time.monotonic() < deadline and s.returncode() is None:
            time.sleep(0.2)
        rc = s.returncode()
        assert rc is not None, "无 provider 时应快速退出而不是挂死"
        assert rc != 0, f"无 provider 应以非 0 退出,实际 {rc}"
        raw = s.raw_output()
        assert "provider" in raw.lower(), f"错误信息应提到 provider:\n{raw[-600:]}"


def test_fullscreen_enters_alt_screen(tui) -> None:
    """B11 `--fullscreen` 启动:进备用屏(ESC[?1049h),首屏锚点齐全。

    全屏走 `draw_fullscreen`(tui/mod.rs,与 inline 视口不同的渲染路径),
    此前只有 clap 参数互斥测试,从未真正跑过这条路径。
    """
    with tui(args=["--fullscreen"]) as s:
        s.wait_text("/help for commands", timeout=30)
        txt = s.normalized()
        assert "███" in txt, "全屏模式 banner 块字应渲染"
        assert "Send a message" in txt, "全屏模式 composer 占位符应渲染"
        # alt-screen 进入序列必须出现在原始字节流里。
        assert "\x1b[?1049h" in s.raw_output(), "应发送 ESC[?1049h 进入备用屏"
        assert not s.has_panic()


def test_fullscreen_exit_restores_main_screen(tui) -> None:
    """B12 全屏模式退出:还原主屏(ESC[?1049l),退出码 0。"""
    import time as _time

    with tui(args=["--fullscreen"]) as s:
        s.wait_text("/help for commands", timeout=30)
        rc = s.quit(timeout=10)
        assert rc == 0, f"全屏模式退出码应为 0,实际 {rc}"
        # 退出序列在进程退出前写出;给读线程一点排空时间再检查。
        _time.sleep(0.5)
        assert "\x1b[?1049l" in s.raw_output(), "退出时应发送 ESC[?1049l 还原主屏"
        assert not s.has_panic()
