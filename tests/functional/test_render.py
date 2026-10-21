"""V 组 —— 渲染与视觉:宽字符 / emoji / 长行折行 / markdown / 主题分支 / resize / 小窗口。

此前套件只断言 ASCII 锚点的存在性;本组补齐 TUI 渲染面上最容易退化的
几类场景(多字节字符列宽、长内容不丢失、终端能力分支、窗口尺寸边界)。

锚点字面量来源(2026-10 实证,证据见 `tests/COVERAGE.md` §V 组):
  - 首屏锚点 `/help for commands` / `Send a message` / 块字 banner `███`
  - 主题分支:`theme.rs resolve_startup` 读 NO_COLOR > REFLECT_TUI_THEME > 持久化
  - resize:`TuiSession.resize` 发 TIOCSWINSZ + `ESC[8;R;Ct` 触发重排
"""

from __future__ import annotations

from tui_pty import text_reply


def test_cjk_roundtrip_renders_correctly(tui, mock_script, sandbox) -> None:
    """V1 中文消息与中文回复完整上屏、落盘,无 panic。"""
    # 一次提交会触发多次模型调用(D7,实测 4 次),每行都会被消费;
    # 给足同款行,保证最终**落盘**的 assistant 消息也是中文回复而非回退文本。
    script = mock_script([text_reply("中文回复标记:你好,世界。")] * 6)
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("你好,世界这是一段中文输入")
        s.key("enter")
        txt = s.wait_text("中文回复标记", timeout=30)
        assert "你好,世界" in txt, f"中文回复应完整上屏:\n{txt[-300:]}"
        assert not s.has_panic()

    body = "\n".join(f.read_text() for f in sandbox.session_files())
    assert "这是一段中文输入" in body, "中文输入应原样落盘"
    assert "中文回复标记" in body, "中文回复应原样落盘"


def test_emoji_renders_without_crash(tui, mock_script) -> None:
    """V2 emoji(含 ZWJ 组合)输入与回复不崩、上屏。"""
    script = mock_script([text_reply("emoji-ok 🚀 done 👍🏽")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("thumbs 👍 and rocket 🚀 and family 👨‍👩‍👧")
        s.key("enter")
        txt = s.wait_text("emoji-ok", timeout=30)
        assert "🚀" in txt, "emoji 应在屏幕上出现"
        assert not s.has_panic(), f"emoji 渲染不应 panic:\n{s.raw_output()[-600:]}"


def test_long_reply_wraps_without_loss(tui, mock_script) -> None:
    """V3 超长回复折行渲染,首尾内容都在屏幕上(无整段丢失)。"""
    long_body = " ".join(f"SEG{i:03d}" for i in range(60))  # ≈ 480 字符,必然折行
    script = mock_script([text_reply(f"V3_HEAD {long_body} V3_TAIL")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("go")
        s.key("enter")
        txt = s.wait(lambda t: "V3_TAIL" in t, timeout=30, label="V3_TAIL")
        assert "V3_HEAD" in txt, "折行后开头不应丢失"
        assert "SEG000" in txt and "SEG059" in txt, "中段内容不应整块丢失"
        assert not s.has_panic()


def test_markdown_code_block_renders(tui, mock_script) -> None:
    """V4 markdown 代码块围栏内容上屏(内容可见即算,不锁渲染样式)。"""
    code = "```rust\nfn probe_render_fn() -> u32 {\n    42\n}\n```"
    script = mock_script([text_reply(f"看这段代码:\n{code}\n完毕")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("show code")
        s.key("enter")
        txt = s.wait(lambda t: "probe_render_fn" in t, timeout=30, label="probe_render_fn")
        assert "42" in txt, "代码块内容应渲染"
        assert not s.has_panic()


def test_markdown_table_and_list_render(tui, mock_script) -> None:
    """V5 markdown 表格与列表内容上屏。"""
    md = (
        "核对表:\n"
        "| 名称 | 值 |\n|---|---|\n| CELLKEY42 | 1 |\n\n"
        "要点:\n- LISTITEM_ALPHA\n- LISTITEM_BETA\n"
    )
    script = mock_script([text_reply(md)])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("show table")
        s.key("enter")
        txt = s.wait(lambda t: "CELLKEY42" in t, timeout=30, label="CELLKEY42")
        assert "LISTITEM_ALPHA" in txt and "LISTITEM_BETA" in txt, "列表项应渲染"
        assert not s.has_panic()


def test_nocolor_env_boots_with_anchors(tui) -> None:
    """V6 NO_COLOR=1 主题分支:首屏锚点齐全,正常可用。"""
    with tui(env={"NO_COLOR": "1"}) as s:
        s.wait_boot(timeout=30)
        txt = s.wait_text("/help for commands", timeout=15)
        assert "Send a message" in txt, "composer 占位符应存在"
        assert not s.has_panic(), f"NO_COLOR 不应 panic:\n{s.raw_output()[-500:]}"


def test_light_theme_env_boots_with_anchors(tui) -> None:
    """V7 REFLECT_TUI_THEME=light 主题分支:首屏锚点齐全。"""
    with tui(env={"REFLECT_TUI_THEME": "light"}) as s:
        s.wait_boot(timeout=30)
        txt = s.wait_text("/help for commands", timeout=15)
        assert "Send a message" in txt
        assert not s.has_panic()


def test_resize_keeps_layout_anchors(tui) -> None:
    """V8 resize 后 HUD 与 composer 锚点仍在(升级 B9 的「只验不崩」)。"""
    with tui(cols=80, rows=24) as s:
        s.wait_boot(timeout=30)
        s.resize(120, 40)
        s._pump(settle=1.0, max_wait=2.0)
        txt = s.normalized()
        assert "mock/mock-1" in txt, f"resize 后 HUD 模型名应可见:\n{txt[-300:]}"
        assert "Send a message" in txt, "resize 后 composer 占位符应可见"
        assert not s.has_panic()
        assert s.alive(), "resize 后进程应存活"

        s.resize(80, 24)
        s._pump(settle=0.8, max_wait=1.5)
        txt = s.normalized()
        assert "mock/mock-1" in txt and "Send a message" in txt, "缩回原尺寸后锚点应仍在"


def test_tiny_window_does_not_crash(tui) -> None:
    """V9 极小窗口 40x10:不 panic、不越界崩溃。"""
    with tui(cols=40, rows=10) as s:
        # 40 列下 HUD 文案被截断,wait_boot 的锚点可能永远不完整 ——
        # 改为静置观察:有渲染输出、进程活着、无 panic 即通过。
        s._pump(settle=2.0, max_wait=3.0)
        assert s.alive(), "极小窗口下进程应存活"
        assert s.screen_text().strip(), "极小窗口下应有渲染输出"
        assert not s.has_panic(), f"极小窗口不应 panic:\n{s.raw_output()[-500:]}"
        s.type_text("hi")
        s.key("enter")
        s._pump(settle=1.5, max_wait=2.5)
        assert s.alive(), "极小窗口下交互不应崩死进程"
        assert not s.has_panic()
