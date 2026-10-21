"""W 组 —— 工作流与容错:审批链路、AskUser 面板、/loop 定时、恢复边界、坏输入、信号。

COVERAGE §8 曾把审批链路与 AskUser 面板登记为「离线难稳定触发」——
实证推翻:mock 脚本注入 `tool_call` 即可稳定触发(工具名全小写,如
`bash` / `ask_user` / `ask_user_question`),本组由此把这两块纳入离线覆盖。

锚点字面量来源(2026-10 实证,证据见 `tests/COVERAGE.md` §W 组):
  - 审批条 ` ⚠ APPROVAL NEEDED ` + `Tool: bash({…})` + `[Y]es / [N]o / [A]ll / [S]cope`
    (adapter.rs:889 approval_block;批准 → `✓ bash (Nms)`,拒绝 →
    `✗ bash (0ms)` + `Approval denied: denied by user`)
  - AskUserQuestion 面板 `❓ QUESTION 1/1 · <header> · 单选` + `[ ] <label> — <desc>`
    + `空格/数字勾选 · Enter 确认`;提交后 `✓ ask_user_question` + `selected":[0]`
  - ask_user 文本条 `❓ AGENT 等待你的回答` + `(回车=回答 · Esc=取消)`;
    回答后 `▶ (回答) <text>` + `✓ ask_user`
  - `/loop` 触发 `▶ /status` 重复上屏;`/loop stop` → `⚠ /loop: stopped (was: /status every Ns)`
  - `-c` 空目录 → `Error: no sessions found … nothing to resume`(非 0 退出)
  - `--resume` 短前缀 → `Error: invalid thread id '…': invalid length`(只认完整 UUID)
  - 非法 `REFLECT_MODEL` → `Error: 未配置任何 provider …`(非 0 退出)
  - 坏 config.toml → **静默回落默认配置**,TUI 照常启动(锁定现状,见 §8 说明)
  - SIGTERM → 进程被信号终止(rc=-15),无 panic,会话文件已落盘
  - stdin 非 tty → `Error: Device not configured (os error 6)`,快速失败不挂死
"""

from __future__ import annotations

import json
import os
import signal
import subprocess
import time
from pathlib import Path

from tui_pty import REFLECT_TUI_BIN, Sandbox, TuiSession, text_reply, tool_call


def _wait_exit(s: TuiSession, timeout: float = 25.0) -> int | None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline and s.returncode() is None:
        time.sleep(0.2)
    return s.returncode()


# HUD busy 时 identity 行以 spinner 帧字符开头(statusline.rs:143-146)。
_SPINNER_CHARS = ("⠸", "⠹", "⠺", "⠼", "⠻", "⠶", "⠦", "⠧", "⠇", "⠏")


def _seed_session(sandbox: Sandbox, sid: str, msg: str) -> Path:
    """预置一条最小会话 JSONL(形态与引擎落盘一致)。"""
    base = sandbox.home / ".reflect" / "sessions" / "2026" / "01" / "01"
    base.mkdir(parents=True, exist_ok=True)
    p = base / f"{sid}.jsonl"
    p.write_text(
        json.dumps({"type": "session_meta", "session_id": sid, "model": "mock/mock-1", "started_at": "2026-01-01T00:00:00Z"}) + "\n"
        + json.dumps({"type": "message", "turn_id": "t1", "role": "user", "content": [{"type": "text", "text": msg}]}) + "\n"
    )
    return p


# ── 审批链路 ──────────────────────────────────────────────────────────────


def test_approval_grant_executes_tool(tui, mock_script) -> None:
    """W1 审批批准:tool_call(bash) 弹审批条,y 后工具执行、输出上屏。"""
    script = mock_script(
        [tool_call("bash", {"command": "echo W1_PROBE_OK"}), text_reply("W1_AFTER")]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("run bash")
        s.key("enter")
        txt = s.wait_text("APPROVAL NEEDED", timeout=20)
        assert "Tool: bash" in txt, f"审批条应展示工具与参数:\n{txt[-300:]}"
        assert "[Y]es" in txt and "[N]o" in txt, "应给出 y/n 按键提示"

        s.type_text("y")
        # 审批条本身展示命令文本(`echo W1_PROBE_OK`),不能等输出子串 ——
        # 专等只有执行后才会出现的 `✓ bash` 工具成功行。
        txt = s.wait(lambda t: "✓ bash" in t, timeout=20, label="✓ bash")
        assert "W1_PROBE_OK" in txt, f"批准后工具输出应上屏:\n{txt[-300:]}"
        assert not s.has_panic()


def test_approval_deny_rejects_tool(tui, mock_script) -> None:
    """W2 审批拒绝:n 后工具不执行,给出拒绝原因。"""
    script = mock_script(
        [tool_call("bash", {"command": "echo W2_SHOULD_NOT_RUN"}), text_reply("W2_AFTER")]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("run bash")
        s.key("enter")
        s.wait_text("APPROVAL NEEDED", timeout=20)
        s.type_text("n")
        txt = s.wait_text("Approval denied", timeout=20)
        assert "denied by user" in txt, f"应给出用户拒绝原因:\n{txt[-300:]}"
        assert "✗ bash" in txt, "应有工具失败行"
        assert "W2_SHOULD_NOT_RUN" not in txt, "被拒命令不应执行"


# ── AskUser 面板 ──────────────────────────────────────────────────────────


def _question_args() -> dict:
    return {
        "questions": [
            {
                "header": "W3分支",
                "question": "W3选哪个分支?",
                "options": [
                    {"label": "main", "description": "W3主分支"},
                    {"label": "dev", "description": "W3开发分支"},
                ],
                "multi_select": False,
            }
        ]
    }


def test_ask_user_question_panel_selects_option(tui, mock_script) -> None:
    """W3 AskUserQuestion 面板:面板弹出 → 数字勾选 → Enter 确认 → 回合继续。"""
    script = mock_script(
        [tool_call("ask_user_question", _question_args()), text_reply("W3_AFTER_PANEL")]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("ask me")
        s.key("enter")
        txt = s.wait(lambda t: "W3选哪个分支" in t, timeout=20, label="问题面板")
        assert "QUESTION 1/1" in txt, f"面板应有题号:\n{txt[-300:]}"
        assert "[ ] main" in txt and "[ ] dev" in txt, "两个选项都应列出"

        s.type_text("1")  # 数字勾选第一项
        s.key("enter")  # 确认
        txt = s.wait_text("W3_AFTER_PANEL", timeout=30)
        assert "✓ ask_user_question" in txt, f"面板提交应记工具成功行:\n{txt[-300:]}"
        assert not s.has_panic()


def test_ask_user_free_text_answers_and_continues(tui, mock_script) -> None:
    """W4 ask_user 自由文本条:输入回答回车,回答被记录,回合继续。"""
    script = mock_script(
        [tool_call("ask_user", {"prompt": "W4_PROBE_PROMPT"}), text_reply("W4_AFTER_ASK")]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("ask")
        s.key("enter")
        txt = s.wait(lambda t: "W4_PROBE_PROMPT" in t and "AGENT" in t.upper(), timeout=20, label="ask_user 输入条")
        assert "回车=回答" in txt, f"应提示回车回答:\n{txt[-300:]}"

        s.type_text("W4_MY_ANSWER")
        s.key("enter")
        txt = s.wait_text("W4_AFTER_ASK", timeout=30)
        assert "(回答) W4_MY_ANSWER" in txt, f"回答应以 (回答) 前缀上屏:\n{txt[-300:]}"
        assert "✓ ask_user" in txt
        assert not s.has_panic()


# ── /loop 定时器(离线) ──────────────────────────────────────────────────


def test_loop_fires_repeatedly_and_stops(tui, mock_script) -> None:
    """W5 `/loop 3 /status` 每 3s 触发一次,`/loop stop` 停止。

    触发次数用 raw 流里的**单词**标记计数:`▶ /status` 行会随旧内容滚出
    pyte 视口(视口重绘带整屏擦除,视口计数会漏),而流式渲染以词为单位
    写出,单词 `LOOPFIRE` 一次写就、不会被光标定位序列打断 ——
    一次触发 = 4 次模型调用 = 4 个标记,≥8 即 ≥2 轮。
    间隔取 3s(而非 live 用例的 2s):拉宽迭代间的空闲窗口,让 stop
    有稳定机会落在空闲期(busy 窗口内的提交会被 composer 吞掉)。
    """
    script = mock_script([text_reply("LOOPFIRE")] * 20)
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("/loop 3 /status")
        s.key("enter")
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if s.raw_output().count("LOOPFIRE") >= 8:
                break
            s._pump(settle=0.5, max_wait=1.0)
        assert s.raw_output().count("LOOPFIRE") >= 8, (
            f"/loop 应至少触发 2 次(≥8 个 LOOPFIRE):\n{s.raw_output()[-600:]}"
        )

        # `/loop stop` 是本地即时分发,但提交若落在某次迭代的 busy 窗口会被吞,
        # 且草稿留在 composer 里(下次 type 会拼成非法命令)。每次提交前:
        #   1. 等空闲:identity 行连续两次采样都无 spinner 前缀;
        #   2. ctrl+u 清空可能残留的草稿。
        stopped = False
        for _attempt in range(4):
            calm_since = None
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                line = next(
                    (l for l in reversed(s.screen_text().splitlines()) if "mock/mock-1" in l),
                    "",
                )
                if line and not line.strip().startswith(_SPINNER_CHARS):
                    if calm_since is None:
                        calm_since = time.monotonic()
                    elif time.monotonic() - calm_since >= 0.5:
                        break  # 连续 0.5s 无 spinner,视为空闲
                else:
                    calm_since = None
                s._pump(settle=0.2, max_wait=0.4)
            s.key("ctrl+u")
            s.type_text("/loop stop")
            s.key("enter")
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline:
                if "/loop: stopped" in s.raw_output():
                    stopped = True
                    break
                s._pump(settle=0.4, max_wait=0.8)
            if stopped:
                break
        assert stopped, f"/loop stop 重试 4 次仍未生效:\n{s.raw_output()[-600:]}"
        assert "was: /status every 3s" in s.raw_output(), "停止提示应带原配置"
        assert not s.has_panic()


# ── 会话恢复边界 ──────────────────────────────────────────────────────────


def test_continue_flag_with_no_sessions_fails_fast(tui) -> None:
    """W6 `-c` 无历史会话:给出可诊断错误并退出,不挂死。"""
    with tui(args=["-c"]) as s:
        rc = _wait_exit(s, timeout=25)
        assert rc is not None, "无会话可恢复时应快速退出而不是挂死"
        assert rc != 0, "无会话可恢复应以非 0 退出"
        raw = s.raw_output()
        assert "no sessions found" in raw, f"错误应说明没有会话:\n{raw[-400:]}"
        assert "nothing to resume" in raw, f"错误应说明无可恢复:\n{raw[-400:]}"


def test_resume_short_prefix_is_rejected(tui, sandbox) -> None:
    """W7 `--resume` 只认完整 UUID:短前缀(即使无歧义)被快速拒绝。"""
    _seed_session(sandbox, "aaa11111-1111-4111-8111-111111111111", "W7_SEED_MSG")
    with tui(args=["--resume", "aaa11111"]) as s:
        rc = _wait_exit(s, timeout=25)
        assert rc is not None, "非法 resume id 应快速退出"
        assert rc != 0, "非法 resume id 应非 0 退出"
        raw = s.raw_output()
        assert "invalid thread id" in raw, f"错误应说明 id 非法:\n{raw[-400:]}"


def test_large_transcript_resume_stays_interactive(tui, mock_script, sandbox) -> None:
    """W8 200 条消息的大会话恢复:不挂死、可继续对话。

    旧消息经 insert_history 写入终端原生 scrollback,pyte 视口看不到 ——
    断言「恢复后仍可交互」而非「旧消息在视口内」。
    """
    sid = "bbb33333-2222-4333-8444-555555555555"
    base = sandbox.home / ".reflect" / "sessions" / "2026" / "01" / "01"
    base.mkdir(parents=True, exist_ok=True)
    lines = [json.dumps({"type": "session_meta", "session_id": sid, "model": "mock/mock-1", "started_at": "2026-01-01T00:00:00Z"})]
    for i in range(100):
        lines.append(json.dumps({"type": "message", "turn_id": f"t{i}", "role": "user", "content": [{"type": "text", "text": f"big user message {i} " + "x" * 40}]}))
        lines.append(json.dumps({"type": "message", "turn_id": f"t{i}", "role": "assistant", "content": [{"type": "text", "text": f"big assistant reply {i} " + "y" * 40}]}))
    (base / f"{sid}.jsonl").write_text("\n".join(lines) + "\n")

    script = mock_script([text_reply("W8_PONG")] * 6)
    with tui(args=["--resume", sid], script=script) as s:
        s.wait_boot(timeout=40)
        assert not s.has_panic(), "大会话恢复不应 panic"
        s.type_text("ping")
        s.key("enter")
        txt = s.wait_text("W8_PONG", timeout=40)
        assert not s.has_panic(), "大会话恢复后继续对话不应 panic"


# ── 容错:坏输入与异常环境 ────────────────────────────────────────────────


def test_malformed_config_falls_back_to_defaults(tui, sandbox) -> None:
    """W9 坏 config.toml 不挡启动:静默回落默认配置,TUI 正常可用。

    锁定现状(见 COVERAGE §8):解析失败无任何用户可见告警,属产品
    决策 —— 本用例防的是「坏配置导致启动崩溃/挂死」这一退化。
    """
    (sandbox.home / ".reflect" / "config.toml").write_text("this is [ not valid toml {{{\n")
    with tui() as s:
        s.wait_boot(timeout=30)
        txt = s.wait_text("/help for commands", timeout=15)
        assert "Send a message" in txt
        assert not s.has_panic(), "坏配置不应导致 panic"
        assert s.alive(), "坏配置下进程应存活"


def test_invalid_model_env_fails_fast(tui) -> None:
    """W10 `REFLECT_MODEL=nonexistent/x`:给出可行动的错误并退出。"""
    with tui(env={"REFLECT_MODEL": "nonexistent/x"}) as s:
        rc = _wait_exit(s, timeout=25)
        assert rc is not None, "未知 provider 应快速退出而不是挂死"
        assert rc != 0, "未知 provider 应非 0 退出"
        raw = s.raw_output()
        assert "provider" in raw.lower(), f"错误应提到 provider:\n{raw[-400:]}"


def test_sigterm_terminates_cleanly(tui, sandbox, mock_script) -> None:
    """W11 SIGTERM:进程被信号终止,无 panic,会话文件已落盘。"""
    script = mock_script([text_reply("W11_REPLY")] * 6)
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("hello before signal")
        s.key("enter")
        s._pump(settle=1.5, max_wait=2.0)
        os.killpg(os.getpgid(s._proc.pid), signal.SIGTERM)
        rc = _wait_exit(s, timeout=15)
        assert rc == -signal.SIGTERM, f"应死于 SIGTERM(rc=-15),实际 rc={rc}"
        assert not s.has_panic(), "SIGTERM 不应触发 panic"
        assert sandbox.session_files(), "终止前会话文件应已落盘"


def test_stdin_not_a_tty_fails_fast(sandbox) -> None:
    """W12 stdin 非 tty(管道):快速失败不挂死,错误可诊断。"""
    env = sandbox.env(sandbox.mock_env())
    proc = subprocess.run(
        [str(REFLECT_TUI_BIN)],
        input="",
        capture_output=True,
        text=True,
        timeout=20,
        cwd=str(sandbox.cwd),
        env=env,
    )
    assert proc.returncode != 0, f"无 tty 应非 0 退出,实际 {proc.returncode}"
    out = proc.stdout + proc.stderr
    assert "Device not configured" in out or "Error" in out, (
        f"应给出可诊断错误:\n{out[-400:]}"
    )
