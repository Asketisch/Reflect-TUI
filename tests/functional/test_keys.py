"""K 组 —— 键盘交互:权限模式循环、历史召回、外部编辑器、复制、多行与 vim。

此前套件对「按键」只覆盖了提交 / Esc / Ctrl+C / Ctrl+D / overlay 滚动;
本组把 `tui_pty.KEY_SEQS` 里定义了但无用例引用的按键逐一变成行为断言。

锚点字面量来源(2026-10 实证,证据见 `tests/COVERAGE.md` §K 组):
  - 权限模式 HUD 段 `[accept_edits]` / `[plan]`   statusline.rs identity 行
    (实测循环为 Auto(无标记) → AcceptEdits → Plan → Auto,三态)
  - 复制通知 `⚠ Copied last reply (N chars) to clipboard.`
  - 编辑器通知 `⚠ External editor closed; content loaded into composer.`
  - 历史文件 `~/.reflect/prompt_history.jsonl` 每行一个**裸 JSON 字符串**
    (`prompt_history.rs:149 parse_entries`,不是对象)
  - vim 提示 `⚠ Vim mode enabled — composer now in NORMAL. Press i to enter INSERT.`
    + HUD identity 行 `[NORMAL]` 段
  - Ctrl+C busy 首按中断:mock 回复瞬时完成,离线无法确定性制造 busy 窗口,
    不设用例(见 COVERAGE §8);Esc 中断已由 T9 覆盖,空闲退出由 B7 覆盖。
"""

from __future__ import annotations

import json
import time
from pathlib import Path

from tui_pty import Sandbox, text_reply


def _identity_line(s) -> str:
    """取 HUD identity 行(含模型名 `mock/mock-1` 的最后一行)。"""
    return next(
        (l for l in reversed(s.screen_text().splitlines()) if "mock/mock-1" in l),
        "",
    )


def _seed_history(sandbox: Sandbox, entries: list[str]) -> Path:
    """预置 prompt 历史:每行一个裸 JSON 字符串(与引擎落盘形态一致)。"""
    hist = sandbox.home / ".reflect" / "prompt_history.jsonl"
    hist.parent.mkdir(parents=True, exist_ok=True)
    hist.write_text("\n".join(json.dumps(e) for e in entries) + "\n")
    return hist


def test_shift_tab_cycles_permission_modes(tui) -> None:
    """K1 Shift+Tab 循环权限模式:Auto(无标记)→ accept_edits → plan → Auto。

    keymap.rs 注释宣称五模式,实测 HUD 可见的循环是三态 —— 锁定真实行为。
    """
    with tui() as s:
        s.wait_boot(timeout=30)
        s.key("shift+tab")
        s._pump(settle=0.6, max_wait=1.2)
        line = _identity_line(s)
        assert "[accept_edits]" in line, f"第 1 次应为 accept_edits:{line!r}"

        s.key("shift+tab")
        s._pump(settle=0.6, max_wait=1.2)
        line = _identity_line(s)
        assert "[plan]" in line, f"第 2 次应为 plan:{line!r}"

        s.key("shift+tab")
        s._pump(settle=0.6, max_wait=1.2)
        line = _identity_line(s)
        assert "accept_edits" not in line and "[plan]" not in line, (
            f"第 3 次应回到 Auto(无模式标记):{line!r}"
        )
        assert not s.has_panic()


def test_ctrl_o_copies_last_reply(tui, mock_script) -> None:
    """K2 Ctrl+O 复制最后一条回复,给出字符数通知。"""
    script = mock_script([text_reply("K2_REPLY_TO_COPY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("hi")
        s.key("enter")
        # Ctrl+O 只认**已提交**的回复(流中按会提示 no agent reply yet):
        # 先等文本上屏,再等回合收尾,避开慢机器上的流中竞态。
        s.wait_text("K2_REPLY_TO_COPY", timeout=30)
        s.wait_idle(timeout=30)
        s.key("ctrl+o")
        txt = s.wait_text("Copied last reply", timeout=10)
        assert "clipboard" in txt, f"通知应提到 clipboard:\n{txt[-300:]}"
        assert not s.has_panic()


def test_ctrl_g_external_editor_roundtrip(tui, sandbox) -> None:
    """K3 Ctrl+G 拉起 $EDITOR,编辑器写入的文本回填 composer 并可提交。"""
    editor = sandbox.home / "fake-editor.sh"
    editor.write_text(
        '#!/bin/sh\n# 测试替身编辑器:把固定文本写进给定文件\nprintf "TEXT_FROM_EDITOR" > "$1"\n'
    )
    editor.chmod(0o755)

    with tui(env={"EDITOR": str(editor), "VISUAL": str(editor)}) as s:
        s.wait_boot(timeout=30)
        s.type_text("draft before editor")
        s._pump(settle=0.5, max_wait=1.0)
        s.key("ctrl+g")
        txt = s.wait_text("External editor closed", timeout=15)
        assert "content loaded into composer" in txt, f"应提示回填:\n{txt[-300:]}"
        s.key("enter")
        # composer 回显就是 › TEXT_FROM_EDITOR,必须专等提交后的 ▶ 前缀形态。
        txt = s.wait(lambda t: "▶ TEXT_FROM_EDITOR" in t, timeout=30, label="▶ TEXT_FROM_EDITOR")
        assert not s.has_panic()


def test_ctrl_g_external_editor_in_fullscreen_leaves_and_reenters_alt_screen(
    tui, sandbox
) -> None:
    """K3b 全屏模式下 Ctrl+G:必须真正离开备用屏跑编辑器,退出后再重进。

    全屏模式下 overlay 与主界面共享同一屏,`TerminalGuard::leave_alt_screen`
    在 fullscreen 下会短路(不真正 LeaveAlternateScreen)——若外部编辑器沿用
    它,编辑器就会跑在 alt-screen 上被 TUI 重绘覆盖。修复后走
    `suspend_alt_screen` 变体(绕开短路),编辑器退出后再重进备用屏。

    断言:原始字节流里出现「离开备用屏 ESC[?1049l」,且其后又有一次
    「进入备用屏 ESC[?1049h」(启动时那一次之前)。
    """
    editor = sandbox.home / "fake-editor.sh"
    editor.write_text(
        '#!/bin/sh\n# 测试替身编辑器:把固定文本写进给定文件\nprintf "FS_TEXT_FROM_EDITOR" > "$1"\n'
    )
    editor.chmod(0o755)

    with tui(args=["--fullscreen"], env={"EDITOR": str(editor), "VISUAL": str(editor)}) as s:
        s.wait_text("/help for commands", timeout=30)
        s.key("ctrl+g")
        txt = s.wait_text("External editor closed", timeout=20)
        assert "content loaded into composer" in txt, f"应提示回填:\n{txt[-300:]}"

        raw = s.raw_output()
        leave = raw.find("\x1b[?1049l")
        assert leave != -1, "全屏下跑外部编辑器前应真正离开备用屏(ESC[?1049l)"
        reenter = raw.find("\x1b[?1049h", leave)
        assert reenter != -1, "编辑器退出后应重进备用屏(ESC[?1049h)"
        assert not s.has_panic()


def test_ctrl_r_searches_prompt_history(tui, sandbox) -> None:
    """K4 Ctrl+R 反向搜索历史:输入关键字回填命中条目。"""
    _seed_history(sandbox, ["alpha probe one", "beta probe two", "gamma probe three"])
    with tui() as s:
        s.wait_boot(timeout=30)
        s.key("ctrl+r")
        s._pump(settle=0.6, max_wait=1.2)
        s.type_text("beta")
        txt = s.wait(lambda t: "beta probe two" in t, timeout=10, label="beta probe two")
        assert "beta probe two" in txt, "反搜应命中并回填 beta 条目"
        assert not s.has_panic()


def test_up_arrow_recalls_last_history_entry(tui, sandbox) -> None:
    """K5 Up 召回最近一条 prompt 历史。"""
    _seed_history(sandbox, ["HIST_LAST_ENTRY"])
    with tui() as s:
        s.wait_boot(timeout=30)
        s.key("up")
        txt = s.wait(lambda t: "HIST_LAST_ENTRY" in t, timeout=10, label="HIST_LAST_ENTRY")
        assert "› HIST_LAST_ENTRY" in txt.replace("  ", " › ") or "HIST_LAST_ENTRY" in txt
        assert not s.has_panic()


def test_alt_enter_composes_multiline_message(tui, mock_script, sandbox) -> None:
    """K7 alt+enter 在 composer 内换行,提交后落盘的消息体含换行符。

    真终端的 shift+enter 依赖 kitty 键盘协议,哑 pty 发不出等价序列;
    alt+enter(`ESC\r`)是 composer 支持的另一条换行路径,以此锁定多行语义。
    """
    script = mock_script([text_reply("K7_ACK")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("K7_LINE_ONE")
        s.key("alt+enter")
        s.type_text("K7_LINE_TWO")
        s._pump(settle=0.5, max_wait=1.0)
        screen = s.screen_text()
        assert "K7_LINE_TWO" in screen, "第二行应已进入 composer"
        s.key("enter")
        s.wait_text("K7_ACK", timeout=30)

    body = _first_user_text(sandbox, "K7_LINE_ONE")
    assert body is not None and "\n" in body, f"落盘消息体应为多行:{body!r}"


def test_large_paste_survives_submission(tui, mock_script, sandbox) -> None:
    """K8 一次粘贴 300+ 字符,提交后落盘内容完整不丢失。"""
    payload = "PASTE_BLOB_" + "x" * 300
    script = mock_script([text_reply("K8_ACK")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text(payload)
        s.key("enter")
        s.wait_text("K8_ACK", timeout=30)

    body = _first_user_text(sandbox, "PASTE_BLOB_")
    assert body == payload, f"落盘内容应完整({len(payload)} 字符),实际 {len(body or '')} 字符"


def test_burst_paste_chunks_stay_coherent(tui, mock_script, sandbox) -> None:
    """K11 连续多段快速输入(模拟粘贴突发),composer 不丢段。"""
    script = mock_script([text_reply("K11_ACK")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("BURST_PART_ONE ")
        s.type_text("BURST_PART_TWO ")
        s.type_text("BURST_PART_THREE")
        s.key("enter")
        s.wait_text("K11_ACK", timeout=30)

    body = _first_user_text(sandbox, "BURST_PART_ONE")
    assert body == "BURST_PART_ONE BURST_PART_TWO BURST_PART_THREE", (
        f"三段应按序拼合:{body!r}"
    )


def test_vim_mode_edits_in_normal_state(tui) -> None:
    """K9 `/vim` 启用后:NORMAL 态 `x` 删除末字符,i 进入 INSERT 后可输入。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("/vim")
        s.key("enter")
        txt = s.wait_text("Vim mode enabled", timeout=10)
        assert "NORMAL" in txt and "Press i to enter INSERT" in txt
        assert "[NORMAL]" in _identity_line(s), "HUD 应显示 [NORMAL] 段"

        s.type_text("i")  # 进入 INSERT
        s.type_text("hello vim world")
        s._pump(settle=0.5, max_wait=1.0)
        s.key("esc")  # 回 NORMAL
        s._pump(settle=0.3, max_wait=0.6)
        s.type_text("x")  # NORMAL 态删除末字符
        txt = s.wait(lambda t: "hello vim worl" in t, timeout=10, label="hello vim worl")
        assert "hello vim world" not in txt.replace("hello vim worl", "", 1), (
            "末字符 d 应被 x 删除"
        )
        assert not s.has_panic()


def test_inline_editing_ctrl_a_home(tui, mock_script, sandbox) -> None:
    """K10 Ctrl+A 跳行首后补前缀,提交的最终文本 = 前缀 + 原文。"""
    script = mock_script([text_reply("K10_ACK")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("TAIL_PART")
        s._pump(settle=0.4, max_wait=0.8)
        s.key("ctrl+a")
        s.type_text("HEAD_PART ")
        s._pump(settle=0.4, max_wait=0.8)
        s.key("enter")
        s.wait_text("K10_ACK", timeout=30)

    body = _first_user_text(sandbox, "HEAD_PART")
    assert body == "HEAD_PART TAIL_PART", f"Ctrl+A 后插入应落在行首:{body!r}"


# ── 落盘查询助手 ──────────────────────────────────────────────────────────


def _first_user_text(sandbox: Sandbox, marker: str) -> str | None:
    """找落盘会话里第一条含 `marker` 的 user 消息文本(无则 None)。"""
    import time as _t

    for _ in range(10):  # 落盘有延迟,轮询
        for f in sandbox.session_files():
            for line in f.read_text().splitlines():
                try:
                    rec = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if rec.get("type") != "message" or rec.get("role") != "user":
                    continue
                blocks = rec.get("content")
                if isinstance(blocks, list):
                    for b in blocks:
                        if isinstance(b, dict) and marker in b.get("text", ""):
                            return b["text"]
                elif isinstance(blocks, str) and marker in blocks:
                    return blocks
        _t.sleep(0.3)
    return None
