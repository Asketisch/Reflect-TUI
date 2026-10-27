"""F 组 —— **全功能实战**(真实模型)。

把能力覆盖层的**全部功能域**搬到**真实模型**的会话里跑一遍:
slash 全量矩阵、弹层、对话轮次、持久化、headless CLI、键盘 / 渲染 / 退出。

## 为什么要有这一组

能力覆盖层(`tests/functional/`)是**离线**的:mock provider 给了确定性与
瞬时回复,所以 202 条断言能当 CI 门禁。但它测不到「真模型 + 真 provider
配置」下的行为:HUD 里的真实模型 spec、真实 token / cost 统计、真实工具
审批、真实 AskUser 面板、真实会话落盘内容、真实 compaction ……

F 组把**同一份断言口径**放到真实模型上跑 —— 矩阵数据**直接复用**能力覆盖层
的 `MATRIX`(单一事实源,`coverage_audit.py` 也解析它),区别只有「模型是真的」。

## 与 L 组(7 条)的分工

| 组 | 覆盖 | 特点 |
|---|---|---|
| **L 组** | 7 条端到端链路(loop / goal / 子代理 / plan / 工具 / 落盘 / 多轮) | 断言**链路**跑通 |
| **F 组** | 全功能面(84 条 slash + 弹层 + 轮次 + 持久化 + CLI + 键盘/渲染) | 断言**功能面**在真模型下无漂移 |

## 跑法

```bash
REFLECT_LIVE=1 ./tests/live/run.sh -k "full_feature"   # 或直接 pytest
```

⚠️ **耗时**:slash 矩阵逐条独立会话(与能力覆盖层同口径),约 10 分钟量级 ——
这是**手动跑的实战套件**,不是 CI 门禁。

⚠️ **断言口径**:同能力覆盖层 —— 屏幕可见文本 / 落盘内容 / 退出码,
**不断言模型回答的具体内容**(真实模型的输出不确定)。
"""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import time
from pathlib import Path

import pytest

from live_lane import (
    DRAIN_TIMEOUT,
    assistant_message_count,
    current_lane,
    is_busy,
    rate_limited,
    safe_text,
    session_stats,
    write_lane_config,
)
from tui_pty import ZHUYE_CLI_BIN, ROOT, Sandbox, TuiSession

FUNCTIONAL_DIR = Path(__file__).resolve().parents[1] / "functional"


def _load_slash_matrix() -> list[tuple[str, str, str, str]]:
    """从能力覆盖层加载 slash 矩阵(单一事实源)。

    用 `importlib` 按路径加载、模块名加前缀 —— 直接 `import test_slash`
    会在「一次性收集整个 tests/」时与能力覆盖层自己的同名模块冲突
    (pytest 的 import file mismatch)。
    """
    path = FUNCTIONAL_DIR / "test_slash.py"
    spec = importlib.util.spec_from_file_location("functional_slash_matrix", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return list(mod.MATRIX)


SLASH_MATRIX = _load_slash_matrix()


# ── 公共助手 ───────────────────────────────────────────────────────────────


def _submit(s: TuiSession, command: str, args: str = "") -> None:
    """在 composer 里敲一条 slash 命令并回车(整段写入)。

    ⚠️ **必须整段写**:分两段(`/` 先敲)会让**命令弹窗**进入屏幕 ——
    未匹配命令时弹窗自带 `⚠` 警告(破坏「未识别命令不应有 notice」的断言),
    参数里带 `/` 的命令(`/model openai/…`、`/loop 30 /status`)还会被
    弹窗的过滤逻辑干扰。整段写入走 paste-burst 分支(偶发丢字符 D12),
    但能力覆盖层的矩阵断言就是按整段口径校准的 —— 两边保持一致。
    """
    s.type_text(f"/{command} {args}".strip())
    s.key("enter")


def _skip_if_rate_limited(s: TuiSession) -> None:
    """限流 → 跳过(不是失败)。

    ⚠️ 要扫**全量原始字节流**:限流错误行(`stream error: rate limited`)会被
    后续输出顶出屏幕(实测 60s 能刷出 75KB 原始流),只看当前屏幕或流尾
    都会漏判 → 用例干等到超时再误报「失败」。
    """
    if rate_limited(safe_text(s)) or rate_limited(s.raw_output()):
        pytest.skip("免费池限流(429 / 凭据耗尽)—— 跳过,不是失败")


# ── F1:slash 全量矩阵(真实模型会话)────────────────────────────────────────


@pytest.mark.parametrize(
    "command,args,kind,expect",
    SLASH_MATRIX,
    ids=[f"{c} {a}".strip() for c, a, _k, _e in SLASH_MATRIX],
)
def test_live_slash_command_outcome(
    live_tui,
    live_model,
    command: str,
    args: str,
    kind: str,
    expect: str,
) -> None:
    """F1 每条 slash 命令在**真实模型**会话里的结局与矩阵一致。"""
    # `/model` 原矩阵切到 `mock/gpt-4o` —— 真实车道里必须切到当前模型
    # (否则整条会话掉回 mock 车道),期望文案同步替换。
    live_spec = f"openai/{live_model.model}"
    if command == "model" and args.startswith("mock/"):
        args = live_spec
        expect = f"switched to '{live_spec}'"

    with live_tui() as s:
        s.wait_boot(timeout=30)
        _submit(s, command, args)
        _skip_if_rate_limited(s)

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
            txt = safe_text(s)
            assert "⚠" not in txt, f"/{command} 未被识别时不应出现 notice:\n{txt[-400:]}"
            assert f"/{command}" in txt, f"/{command} 未被识别时应留在输入框:\n{txt[-400:]}"
            assert not s.has_panic(), f"/{command} 不应 panic"
            return

        # wired / placeholder / panel:屏幕上应出现期望文本。
        s.wait_text(expect, timeout=30)
        assert not s.has_panic(), f"/{command} 不应 panic:\n{s.raw_output()[-600:]}"


# ── F2:弹层面 ──────────────────────────────────────────────────────────────


def test_live_overlays_reachable(live_tui) -> None:
    """F2 可达弹层在真实会话里能打开(标题正确)且 Esc 能关。"""
    cases = [
        ("ctrl+t", "Transcript (Esc / Ctrl+T to close)"),
        ("/diff", "Diff (Esc / Ctrl+T to close"),
        ("/tasks", "Tasks & Agents (Esc / Ctrl+T to close"),
        ("/plugin", "Installed Plugins"),
    ]
    with live_tui(cols=130, rows=44) as s:
        s.wait_boot(timeout=30)
        for trigger, title in cases:
            if trigger.startswith("ctrl+"):
                s.key(trigger)
            else:
                _submit(s, trigger.lstrip("/"))
            s.wait_text(title, timeout=20)
            s.key("esc")
            s._pump(settle=0.6, max_wait=1.2)
            assert not s.has_panic(), f"{trigger} 不应 panic"


def test_live_overlays_unreachable_stay_unreachable(live_tui) -> None:
    """F3 反向门禁:未接线的弹层在真实会话里**仍然**不可达。"""
    for command in ("keymap", "statusline", "theme", "skills", "mcp"):
        with live_tui() as s:
            s.wait_boot(timeout=30)
            _submit(s, command)
            time.sleep(1.5)
            txt = safe_text(s)
            assert "尚未接入分发" in txt or f"/{command}" in txt, (
                f"/{command} 应是未接线占位或未识别:\n{txt[-400:]}"
            )
            assert not s.has_panic()


# ── F3:真实轮次(流式 / 工具 / 审批 / 统计 / AskUser)────────────────────────


def test_live_turn_streams_and_updates_stats(live_tui, live_sandbox: Sandbox) -> None:
    """F4 真实回合:流式上屏 + HUD 统计出现真实 token。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("用一句话回答:1+1 等于几?")
        s.key("enter")

        deadline = time.monotonic() + DRAIN_TIMEOUT
        saw_answer = False
        while time.monotonic() < deadline:
            _skip_if_rate_limited(s)
            txt = safe_text(s)
            if "•" in txt or "●" in txt:
                saw_answer = True
            if saw_answer and not is_busy(s.screen_text()):
                break
            s._pump(settle=0.6, max_wait=1.2)
        assert saw_answer, f"模型回答应上屏:\n{safe_text(s)[-600:]}"
        assert not s.has_panic()

        # 真实 token 统计:会话 JSONL 里有 token_count 记录。
        stats = session_stats(live_sandbox.session_files())
        assert stats["usage"]["total_tokens"] > 0, f"应记录真实 token:{stats}"
        assert stats["models"], f"应记录实际服务模型:{stats}"


def test_live_tool_call_approval_chain(live_tui) -> None:
    """F5 真实工具回合:模型调 bash → 审批条 → 批准 → 输出上屏。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text(
            "Use the bash tool to run exactly: echo FULL_LIVE_PROBE "
            "Then tell me what the output was."
        )
        s.key("enter")

        approved = False
        got_tool = False
        deadline = time.monotonic() + DRAIN_TIMEOUT
        while time.monotonic() < deadline:
            txt = safe_text(s)
            _skip_if_rate_limited(s)
            if "APPROVAL NEEDED" in txt:
                s.type_text("y")
                approved = True
                time.sleep(1.0)
                continue
            if "✓ bash" in txt.lower():
                got_tool = True
                break
            s._pump(settle=0.6, max_wait=1.2)
        assert approved, "工具调用应触发审批条"
        assert got_tool, "批准后应出现工具成功行"
        s.wait(lambda t: "FULL_LIVE_PROBE" in t, timeout=60, label="bash 输出上屏")
        assert not s.has_panic()


# AskUser 的提问提示:不同模型对「请你用 ask_user_question 工具问我」的理解
# 差别很大 —— 逐个换说法,谁先触发面板就用谁。
_ASKUSER_PROMPTS = [
    # 第一条是实测能稳定触发面板的说法(2026-10-23)。
    "Use the ask_user_question tool to ask me which branch to take. "
    "question 'Which branch?', options 'main' and 'dev'. "
    "You must call the tool, not answer directly.",
    "用 ask_user_question 工具问我一个问题:题目「选哪个分支?」,"
    "两个选项:「main」「dev」。必须调用该工具,不要直接回答。",
    "You must call the ask_user_question tool to ask the user to choose "
    "between two options: main and dev.",
]


def test_live_askuser_panel(live_tui, live_sandbox: Sandbox) -> None:
    """F6 真实 AskUser:模型提问 → 面板弹出 → 勾选 → 回合继续。

    模型不一定愿意「反问用户」——逐个换提示试;都��触发就是模型自由裁量,
    按**跳过**处理(能力覆盖层已用 mock 覆盖这条链路的接线)。
    """
    with live_tui(cols=130, rows=44) as s:
        s.wait_boot(timeout=30)
        for prompt in _ASKUSER_PROMPTS:
            # 回合收尾判据用「assistant 消息已落盘」,**不能**用「不忙」——
            # 提交后到回合真正启动之间有几秒空窗,那时也是「不忙」。
            baseline = assistant_message_count(live_sandbox.home)
            s.type_text(prompt)
            s.key("enter")
            deadline = time.monotonic() + 120
            while time.monotonic() < deadline:
                txt = safe_text(s)
                _skip_if_rate_limited(s)
                if "QUESTION" in txt:
                    break
                if assistant_message_count(live_sandbox.home) > baseline:
                    break  # 回合已收尾(模型没反问)→ 换下一种说法
                s._pump(settle=0.6, max_wait=1.2)
            if "QUESTION" in safe_text(s):
                break

        txt = safe_text(s)
        if "QUESTION" not in txt:
            pytest.skip("模型本轮没有调用 ask_user_question(自由裁量),跳过")

        assert "QUESTION" in txt, f"应有题号:\n{txt[-400:]}"
        s.type_text("1")  # 勾第一项
        s.key("enter")
        s.wait(
            lambda t: "✓ ask_user_question" in t,
            timeout=90,
            label="面板提交后工具成功行",
        )
        assert not s.has_panic()


def test_live_askuser_free_text(live_tui, live_sandbox: Sandbox) -> None:
    """F7 真实 `ask_user` 自由文本条:模型反问 → 输入答案 → 回答上屏。

    这是 AskUser 的**另一种形态**(自由文本而非选项面板)—— 两条路径都要
    在真模型下验。
    """
    prompts = [
        "Use the ask_user tool to ask me: 选哪个分支? (只调用工具,不要自己回答)",
        "用 ask_user 工具问我「选哪个分支?」,等我回答。",
        "You must call the ask_user tool to ask the user a question. "
        "Do not answer directly.",
    ]
    with live_tui(cols=130, rows=44) as s:
        s.wait_boot(timeout=30)
        for prompt in prompts:
            baseline = assistant_message_count(live_sandbox.home)
            s.type_text(prompt)
            s.key("enter")
            deadline = time.monotonic() + 120
            while time.monotonic() < deadline:
                txt = safe_text(s)
                _skip_if_rate_limited(s)
                if "回车=回答" in txt:
                    break
                if assistant_message_count(live_sandbox.home) > baseline:
                    break  # 回合已收尾(模型没反问)→ 换下一种说法
                s._pump(settle=0.6, max_wait=1.2)
            if "回车=回答" in safe_text(s):
                break

        txt = safe_text(s)
        if "回车=回答" not in txt:
            pytest.skip("模型本轮没有调用 ask_user(自由裁量),跳过")

        s.type_text("LIVE_MY_ANSWER")
        s.key("enter")
        txt = s.wait(
            lambda t: "(回答) LIVE_MY_ANSWER" in t,
            timeout=90,
            label="回答上屏",
        )
        assert "✓ ask_user" in txt, f"应记工具成功行:\n{txt[-400:]}"
        assert not s.has_panic()


# ── F4:持久化 + headless CLI(真实会话产物)─────────────────────────────────


def test_live_session_persists_and_cli_reads_it(
    live_tui,
    live_sandbox: Sandbox,
    live_model,
) -> None:
    """F7 真实会话落盘 → headless CLI 能读出模型 / token / 正文。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("用一句话说明什么是递归函数。")
        s.key("enter")
        deadline = time.monotonic() + DRAIN_TIMEOUT
        while time.monotonic() < deadline:
            _skip_if_rate_limited(s)
            if not is_busy(s.screen_text()) and assistant_message_count(live_sandbox.home) > 0:
                break
            s._pump(settle=0.6, max_wait=1.2)
        s._pump(settle=1.0, max_wait=2.0)

    files = live_sandbox.session_files()
    assert files, f"应有会话文件:{live_sandbox.home}"
    stats = session_stats(files)
    assert stats["assistant_messages"] >= 1, "会话应含 assistant 消息"
    assert stats["usage"]["total_tokens"] > 0, "会话应含真实 token 记录"

    # headless CLI:真实会话 → ls / show / export / stats。
    env = {"HOME": str(live_sandbox.home), "PATH": "/usr/bin:/bin:/usr/sbin:/sbin"}

    def _cli(*args: str) -> tuple[int, str]:
        proc = subprocess.run(
            [str(ZHUYE_CLI_BIN), *args],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        return proc.returncode, proc.stdout + proc.stderr

    rc, ls_out = _cli("session", "ls")
    assert rc == 0, f"session ls 失败:\n{ls_out}"
    assert "session_id" in ls_out and "tokens" in ls_out, f"session ls 表头:\n{ls_out}"
    # 模型列宽度有限,超长会被截断成 `…` —— 只断言可见前缀。
    assert live_model.model[:20] in ls_out, (
        f"session ls 应列出真实模型 {live_model.model}:\n{ls_out}"
    )

    rc, stats_out = _cli("session", "stats")
    assert rc == 0, f"session stats 失败:\n{stats_out}"
    assert "Per-model usage" in stats_out, f"session stats 应有 per-model 表:\n{stats_out}"
    assert str(stats["usage"]["total_tokens"]) in stats_out.replace(",", ""), (
        f"session stats 应含真实 token 总量 {stats['usage']['total_tokens']}:\n{stats_out}"
    )

    # 按真实模型过滤 —— 命中同一条会话。
    rc, filtered = _cli("session", "ls", "--model", live_model.model)
    assert rc == 0, f"session ls --model 失败:\n{filtered}"
    assert "session_id" in filtered, f"按真实模型过滤应命中:\n{filtered}"

    # 导出:真实会话正文能导出成 markdown。
    rc, export_out = _cli("session", "export", session_id_of(ls_out))
    assert rc == 0, f"session export 失败:\n{export_out}"
    assert export_out.strip(), "导出的 markdown 不应为空"


def session_id_of(cli_output: str) -> str:
    """从 `session ls` 输出里取第一条会话的 id。"""
    for line in cli_output.splitlines()[1:]:
        parts = line.split()
        if parts and "-" in parts[0]:
            return parts[0]
    return ""


def test_live_plan_markdown_is_written(live_tui, live_sandbox: Sandbox) -> None:
    """F8 Plan 模式:plan markdown 真落盘到 `.zhuye/plan/`。"""
    with live_tui(cols=130, rows=44) as s:
        s.wait_boot(timeout=30)
        _submit(s, "mode", "plan")
        s.wait_text("[plan]", timeout=20)

        s.type_text(
            "Use the PlanWrite tool to write a plan for creating a file named "
            "PLAN_LIVE_PROOF.md whose content is: hello-plan. "
            "Then call ExitPlanMode to request approval."
        )
        s.key("enter")

        deadline = time.monotonic() + DRAIN_TIMEOUT
        while time.monotonic() < deadline:
            _skip_if_rate_limited(s)
            if (live_sandbox.cwd / ".zhuye" / "plan").is_dir() and any(
                (live_sandbox.cwd / ".zhuye" / "plan").glob("*.md")
            ):
                break
            if "PLAN APPROVAL" in safe_text(s).upper():
                break
            s._pump(settle=0.6, max_wait=1.2)

    plan_dir = live_sandbox.cwd / ".zhuye" / "plan"
    if not plan_dir.is_dir():
        pytest.skip("模型本轮没走 plan 流程(自由裁量),跳过")
    plans = list(plan_dir.glob("*.md"))
    assert plans, "plan 目录应有 markdown"
    assert plans[0].read_text(errors="replace").strip(), "plan markdown 不应为空"


# ── F5:启动锚点 / 键盘 / 渲染 / 退出 ───────────────────────────────────────


def test_live_boot_shows_real_model(live_tui, live_model) -> None:
    """F9 HUD 身份行显示**真实模型 spec**(不是占位串)。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        txt = safe_text(s)
        assert "Ctrl+C to quit" in txt, f"HUD 应有退出提示:\n{txt[-400:]}"
        assert live_model.model in txt, (
            f"HUD 应显示真实模型 {live_model.model}:\n{txt[-400:]}"
        )
        assert "mock" not in txt.lower(), f"不应出现 mock:\n{txt[-400:]}"


def test_live_ctrl_c_exits_clean(live_tui) -> None:
    """F10 真实会话里 Ctrl+C 双击干净退出(退出码 0,无 panic)。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.key("ctrl+c")
        time.sleep(0.2)
        s.key("ctrl+c")
        deadline = time.monotonic() + 15
        rc = None
        while time.monotonic() < deadline:
            rc = s.returncode()
            if rc is not None:
                break
            time.sleep(0.15)
        assert rc == 0, f"Ctrl+C 退出码应为 0,实际 {rc}"
        assert not s.has_panic()


def test_live_resize_redraws(live_tui) -> None:
    """F11 真实会话里 resize 后重绘,进程存活。"""
    with live_tui(cols=120, rows=38) as s:
        s.wait_boot(timeout=30)
        s.resize(90, 28)
        s._pump(settle=1.0, max_wait=2.0)
        assert s.alive(), "resize 后进程应存活"
        assert not s.has_panic()


def test_live_transcript_scroller(live_tui) -> None:
    """F12 transcript 分页器在真实会话里可打开、可翻页、可关闭。"""
    with live_tui(cols=130, rows=44) as s:
        s.wait_boot(timeout=30)
        s.key("ctrl+t")
        s.wait_text("Transcript (Esc / Ctrl+T to close)", timeout=20)
        s.key("down")
        s.key("down")
        s._pump(settle=0.5, max_wait=1.0)
        s.key("esc")
        s._pump(settle=0.5, max_wait=1.0)
        assert not s.has_panic()


# ── F6:B 组 —— 启动 / 全屏 / model 解析(真实模型)──────────────────────────


def test_live_boot_anchors(live_tui, live_model) -> None:
    """F13 首屏锚点齐全:banner / 版本行 / HUD 真模型 / metrics / composer。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        txt = safe_text(s)
        raw = s.raw_output()
        assert "Reflect" in txt, f"首屏应含 Reflect:{txt[:300]}"
        assert "/help for commands" in txt, f"首屏应含 /help 提示:{txt[:300]}"
        assert "█" in raw and ("╗" in raw or "╔" in raw), "banner 应含块字形"
        assert "Ctrl+C to quit" in txt, "HUD 应有退出提示"
        assert "no tool calls yet" in txt, "HUD metrics 应有占位文案"
        assert "Send a message" in txt and "shift+enter" in txt, "composer 占位符"
        assert live_model.model in txt, f"HUD 应显示真实模型 {live_model.model}"
        # 终端标题(OSC 0)也带真实模型 spec。
        assert f"Reflect · openai/{live_model.model}" in raw, (
            f"终端标题应含真模型:{raw[:300]}"
        )


def test_live_ctrl_d_exits_clean(live_tui) -> None:
    """F14 Ctrl+D 干净退出(退出码 0)。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.key("ctrl+d")
        deadline = time.monotonic() + 15
        rc = None
        while time.monotonic() < deadline:
            rc = s.returncode()
            if rc is not None:
                break
            time.sleep(0.15)
        assert rc == 0, f"Ctrl+D 退出码应为 0,实际 {rc}"


def test_live_fullscreen_mode(live_tui) -> None:
    """F15 全屏模式(`--fullscreen`):进备用屏 + 块字 banner + composer。"""
    with live_tui(args=["--fullscreen"]) as s:
        s.wait_text("/help for commands", timeout=30)
        txt = safe_text(s)
        assert "███" in txt, f"全屏模式 banner 块字应渲染:{txt[:300]}"
        assert "Send a message" in txt, "全屏模式 composer 应渲染"
        assert "\x1b[?1049h" in s.raw_output(), "应发 ESC[?1049h 进备用屏"
        assert not s.has_panic()


def test_live_model_free_resolves_from_catalog(live_tui, live_sandbox: Sandbox) -> None:
    """F16 `model = "free"` 在**真实端点**下解析成真实免费模型 id。

    这是「免费模型发现」在真模型侧的闭环:配置只写 `free`,启动时从
    provider 清单接口挑一个免费模型,HUD 显示的是**真实 id**(不是占位串)。
    """
    cfg = live_sandbox.home / ".zhuye" / "config.toml"
    text = cfg.read_text()
    cfg.write_text(text.replace(f'model = "{_pinned_model(live_sandbox)}"', 'model = "free"'))

    with live_tui() as s:
        s.wait_boot(timeout=40)
        txt = safe_text(s)
        assert "openai/" in txt, f"HUD 应显示解析出的真实模型 spec:{txt[:400]}"
        assert "provider/free" not in txt, f"不应拿占位串当模型:{txt[:400]}"
        assert not s.has_panic()


def test_live_model_free_disabled_fails(live_tui, live_sandbox: Sandbox) -> None:
    """F17 `model = "free"` + `[model_discovery] enabled = false` → 启动即报错。"""
    cfg = live_sandbox.home / ".zhuye" / "config.toml"
    text = cfg.read_text()
    cfg.write_text(
        text.replace(f'model = "{_pinned_model(live_sandbox)}"', 'model = "free"').replace(
            "[model_discovery]\nenabled = true",
            "[model_discovery]\nenabled = false",
        )
    )

    with live_tui() as s:
        deadline = time.monotonic() + 30
        raw = ""
        while time.monotonic() < deadline:
            raw = s.raw_output()
            if "model_discovery" in raw or "free" in raw.lower():
                break
            s._pump(settle=0.5, max_wait=1.0)
        assert "model_discovery" in raw or "free" in raw.lower(), (
            f"应报出 model_discovery / free 相关错误:\n{raw[-600:]}"
        )


def _pinned_model(sandbox: Sandbox) -> str:
    """从车道配置里读出当前钉住的 model id。"""
    cfg = sandbox.home / ".zhuye" / "config.toml"
    for line in cfg.read_text().splitlines():
        if line.startswith("model = "):
            return line.split("=", 1)[1].strip().strip('"')
    return ""


# ── F7:V 组 —— 渲染与视觉(真实回复的内容)──────────────────────────────────


def test_live_cjk_and_emoji_render(live_tui) -> None:
    """F18 中文 + emoji 的真实回复完整上屏(宽字符渲染不丢字)。"""
    marker = "你好世界"
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text(f"请原样输出这句话,不要加任何解释:{marker} 🚀")
        s.key("enter")
        txt = _wait_reply(s, timeout=DRAIN_TIMEOUT)
        assert marker in txt, f"中文应完整上屏:\n{txt[-400:]}"
        assert "🚀" in txt, f"emoji 应上屏:\n{txt[-400:]}"


def test_live_markdown_code_block_renders(live_tui) -> None:
    """F19 真实回复里的 markdown 代码块被渲染(内容可见)。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text(
            "请只输出一个 markdown 代码块,内容是 python 代码 def add(a, b): return a + b"
        )
        s.key("enter")
        txt = _wait_reply(s, timeout=DRAIN_TIMEOUT)
        assert "add" in txt, f"代码块内容应渲染:\n{txt[-400:]}"


def test_live_long_reply_wraps(live_tui) -> None:
    """F20 长回复折行后首尾都不丢。"""
    with live_tui(cols=100, rows=34) as s:
        s.wait_boot(timeout=30)
        s.type_text(
            "请原样输出这 20 行,每行格式为 SEG000 到 SEG019,每行一个,不要编号:"
        )
        s.key("enter")
        txt = _wait_reply(s, timeout=DRAIN_TIMEOUT)
        assert "SEG000" in txt, f"折行后开头不应丢:\n{txt[-400:]}"
        assert "SEG019" in txt, f"折行后结尾不应丢:\n{txt[-400:]}"


def _wait_reply(s: TuiSession, timeout: float) -> str:
    """等模型回复上屏(agent 前缀出现);限流 → 跳过。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        txt = safe_text(s)
        _skip_if_rate_limited(s)
        if "•" in txt or "●" in txt:
            return txt
        s._pump(settle=0.6, max_wait=1.2)
    raise AssertionError(f"{timeout:.0f}s 内模型未回复:\n{safe_text(s)[-600:]}")


# ── F8:K 组 —— 键盘交互(真实会话)──────────────────────────────────────────


def test_live_shift_tab_permission_cycle(live_tui) -> None:
    """F21 Shift+Tab 权限模式循环:Auto → accept_edits → plan → Auto。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.key("shift+tab")
        s._pump(settle=0.6, max_wait=1.2)
        assert "[accept_edits]" in safe_text(s), "第 1 次应为 accept_edits"
        s.key("shift+tab")
        s._pump(settle=0.6, max_wait=1.2)
        assert "[plan]" in safe_text(s), "第 2 次应为 plan"


def test_live_vim_normal_mode(live_tui) -> None:
    """F22 vim normal 模式:提示语 + HUD [NORMAL] 段。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("/vim")
        s.key("enter")
        txt = s.wait_text("Press i to enter INSERT", timeout=20)
        assert "Vim mode enabled" in txt, f"应提示 vim 模式:{txt[-300:]}"
        assert "[NORMAL]" in safe_text(s), "HUD 应显示 [NORMAL] 段"


def test_live_ctrl_r_history_search(live_tui, live_sandbox: Sandbox) -> None:
    """F23 Ctrl+R 反向搜索 prompt 历史并回填命中条目。"""
    hist = live_sandbox.home / ".zhuye" / "prompt_history.jsonl"
    hist.parent.mkdir(parents=True, exist_ok=True)
    hist.write_text(
        "\n".join(json.dumps(e) for e in ["alpha live one", "beta live two"]) + "\n"
    )
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.key("ctrl+r")
        s._pump(settle=0.6, max_wait=1.2)
        s.type_text("beta")
        txt = s.wait(lambda t: "beta live two" in t, timeout=15, label="beta live two")
        assert "beta live two" in txt, "反搜应命中并回填"


def test_live_up_recalls_history(live_tui, live_sandbox: Sandbox) -> None:
    """F24 ↑ 召回最近一条 prompt 历史。"""
    hist = live_sandbox.home / ".zhuye" / "prompt_history.jsonl"
    hist.parent.mkdir(parents=True, exist_ok=True)
    hist.write_text(json.dumps("LIVE_HIST_LAST_ENTRY") + "\n")
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.key("up")
        txt = s.wait(lambda t: "LIVE_HIST_LAST_ENTRY" in t, timeout=15, label="history entry")
        assert "LIVE_HIST_LAST_ENTRY" in txt


def test_live_ctrl_g_external_editor(live_tui, live_sandbox: Sandbox) -> None:
    """F25 Ctrl+G 拉起 $EDITOR,编辑器内容回填 composer。"""
    editor = live_sandbox.home / "fake-editor.sh"
    editor.write_text(
        '#!/bin/sh\n# 测试替身编辑器:把固定文本写进给定文件\nprintf "TEXT_FROM_EDITOR" > "$1"\n'
    )
    editor.chmod(0o755)
    with live_tui(env={"EDITOR": str(editor), "VISUAL": str(editor)}) as s:
        s.wait_boot(timeout=30)
        s.type_text("draft before editor")
        s._pump(settle=0.5, max_wait=1.0)
        s.key("ctrl+g")
        txt = s.wait_text("External editor closed", timeout=20)
        assert "content loaded into composer" in txt, f"应提示回填:\n{txt[-300:]}"


def test_live_ctrl_o_copies_last_reply(live_tui, live_sandbox: Sandbox) -> None:
    """F26 Ctrl+O 复制最后一条真实回复。

    ⚠️ 必须等**回合收尾**再按:流式标记(`•`)出现 ≠ 回复已提交 ——
    回合还在滚的时候按 Ctrl+O,复制的是空的「最后回复」。
    """
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("用一句话回答:什么是递归?")
        s.key("enter")
        deadline = time.monotonic() + DRAIN_TIMEOUT
        while time.monotonic() < deadline:
            _skip_if_rate_limited(s)
            if not is_busy(s.screen_text()) and assistant_message_count(live_sandbox.home) > 0:
                break
            s._pump(settle=0.6, max_wait=1.2)
        s.key("ctrl+o")
        txt = s.wait(lambda t: "clipboard" in t.lower(), timeout=20, label="copy notice")
        assert "Copied last reply" in txt, f"复制通知:\n{txt[-300:]}"


def test_live_multiline_submit(live_tui, live_sandbox: Sandbox) -> None:
    """F27 Alt+Enter 多行输入:真实提交后落盘消息体是多行。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("K7_LINE_ONE")
        s.key("alt+enter")
        s.type_text("K7_LINE_TWO")
        s._pump(settle=0.5, max_wait=1.0)
        s.key("enter")
        deadline = time.monotonic() + DRAIN_TIMEOUT
        while time.monotonic() < deadline:
            _skip_if_rate_limited(s)
            if not is_busy(s.screen_text()) and assistant_message_count(live_sandbox.home) > 0:
                break
            s._pump(settle=0.6, max_wait=1.2)

    body = "\n".join(
        p.read_text(errors="replace") for p in live_sandbox.session_files()
    )
    assert "K7_LINE_ONE" in body, "第一行应落盘"
    assert "K7_LINE_TWO" in body, "第二行应落盘"


# ── F9:C 组 —— headless CLI 全语义(真实会话产物)──────────────────────────


def test_live_cli_fork_rename_rm(live_tui, live_sandbox: Sandbox) -> None:
    """F28 真实会话 → CLI 的 fork / rename / rm 全链路。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("用一句话回答:什么是二分查找?")
        s.key("enter")
        deadline = time.monotonic() + DRAIN_TIMEOUT
        while time.monotonic() < deadline:
            _skip_if_rate_limited(s)
            if not is_busy(s.screen_text()) and assistant_message_count(live_sandbox.home) > 0:
                break
            s._pump(settle=0.6, max_wait=1.2)

    env = {"HOME": str(live_sandbox.home), "PATH": "/usr/bin:/bin:/usr/sbin:/sbin"}

    def _cli(*args: str) -> tuple[int, str]:
        proc = subprocess.run(
            [str(ZHUYE_CLI_BIN), *args],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        return proc.returncode, proc.stdout + proc.stderr

    rc, out = _cli("session", "ls")
    assert rc == 0, f"session ls 失败:\n{out}"
    sid = session_id_of(out)
    assert sid, f"应有会话 id:\n{out}"

    # rename:写 .name,再 show 能看到。
    rc, out = _cli("session", "rename", sid, "live-renamed")
    assert rc == 0, f"session rename 失败:\n{out}"
    assert "Renamed session" in out, f"rename 应打印成功行:\n{out}"
    rc, out = _cli("session", "show", sid)
    assert "live-renamed" in out, f"show 应显示新名字:\n{out}"

    # fork:生成子会话并打印续作提示。
    rc, out = _cli("session", "fork", sid, "--branch", "live-branch")
    assert rc == 0, f"session fork 失败:\n{out}"
    assert "Forked session" in out and "Resume with" in out, f"fork 输出:\n{out}"

    # rm:真实删除。
    rc, out = _cli("session", "rm", sid, "--yes")
    assert rc == 0, f"session rm 失败:\n{out}"
    assert "removed" in out, f"rm 应打印 removed:\n{out}"


# ── F10:G 组 —— 插件面(需要 submodule 构建出的 reflect CLI)────────────────

# submodule 构建出的 reflect-cli(bin 名是 `reflect`)。
_CLI_CANDIDATES = [
    ROOT / "reflect-agent" / "target" / "release" / "reflect",
    ROOT / "reflect-agent" / "target" / "debug" / "reflect",
]
_EXAMPLE_PLUGIN = ROOT / "reflect-agent" / "examples" / "plugin-demo"


def _find_submodule_cli() -> Path | None:
    for candidate in _CLI_CANDIDATES:
        if candidate.exists() and os.access(candidate, os.X_OK):
            return candidate
    return None


@pytest.fixture(scope="module")
def plugin_home(tmp_path_factory) -> str:
    """临时 HOME + 用真实 CLI 安装 / 启用示例插件(写真实表与 config)。

    与离线 G 组同款前置:submodule 的 reflect CLI 没构建就跳过(增强项,
    不阻塞基础回归)。
    """
    cli = _find_submodule_cli()
    if cli is None:
        pytest.skip(
            "submodule 的 reflect CLI 未构建(需在 reflect-agent 下 cargo build -p reflect-cli);插件面跳过"
        )
    if not _EXAMPLE_PLUGIN.is_dir():
        pytest.skip(f"示例插件不存在:{_EXAMPLE_PLUGIN}")

    home = tmp_path_factory.mktemp("live-plugin-home")
    env = {**os.environ, "HOME": str(home)}
    for args in (
        ["plugin", "install", str(_EXAMPLE_PLUGIN)],
        ["plugin", "enable", "demo@inline"],
    ):
        r = subprocess.run(
            [str(cli), *args],
            env=env,
            capture_output=True,
            text=True,
            timeout=180,
        )
        if r.returncode != 0:
            pytest.skip(f"示例插件安装失败({args}):{r.stdout}{r.stderr}")
    return str(home)


def _extract_plugins_section(toml_text: str) -> str:
    """从 TOML 文本里摘出 `[plugins]` 段(含 `[plugins.*]` 子表)。"""
    kept: list[str] = []
    inside = False
    for line in toml_text.splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            inside = stripped == "[plugins]" or stripped.startswith("[plugins.")
        if inside:
            kept.append(line)
    return "\n".join(kept)


def _write_lane_config_keeping_plugins(home: Path, model: str) -> None:
    """写车道配置,但**保留**已有的 `[plugins]` 段。

    `reflect plugin enable` 是把 `enabled_plugins` 写进 `~/.zhuye/config.toml`
    的 —— 直接覆盖配置会把插件启用记录抹掉。
    """
    cfg = home / ".zhuye" / "config.toml"
    original = cfg.read_text() if cfg.exists() else ""
    write_lane_config(home, model=model, lane=current_lane())
    plugins = _extract_plugins_section(original)
    if plugins:
        with cfg.open("a") as fh:
            fh.write("\n" + plugins + "\n")


def test_live_plugin_surface(live_tui, plugin_home: str, live_model) -> None:
    """F29 插件面在真实会话里:启动通知 + 命令注入 + `/plugin` overlay。

    插件由 submodule 的 `reflect-plugin` 提供,TUI 侧只消费三处 ——
    与离线 G 组同一口径,区别是这次 provider 是真的。
    """
    lane = current_lane()
    cfg_home = Path(plugin_home)
    _write_lane_config_keeping_plugins(cfg_home, model=live_model.model)

    env = {
        "HOME": str(cfg_home),
        "OPENAI_API_KEY": "",
        "ANTHROPIC_API_KEY": "",
        "ZHUYE_MODEL": "",
        "ZHUYE_PROVIDER": "",
        "REFLECT_TUI_UPDATE_CHECK": "0",
    }
    with live_tui(env=env) as s:
        s.wait_boot(timeout=40)
        txt = safe_text(s)
        assert "Plugin 'demo@inline' loaded" in txt, f"应有插件加载通知:\n{txt[-600:]}"

        # 命令弹窗应列出插件命令(分两段输入,避开 paste-burst 丢字符)。
        s.type_text("/")
        s._pump(settle=0.8, max_wait=1.2)
        s.type_text("demo")
        s._pump(settle=1.0, max_wait=1.5)
        txt = safe_text(s)
        assert "demo:hello" in txt, f"命令弹窗应列出插件命令:\n{txt[-800:]}"
        s.key("esc")
        s._pump(settle=0.8, max_wait=1.2)

        assert not s.has_panic()


def test_live_plugin_overlay(live_tui, plugin_home: str, live_model) -> None:
    """F30 `/plugin` overlay 在装了插件的真实会话里展示已装插件。

    单独开会话 —— 上一条用例的 composer 里还留着 `/demo` 草稿,串场会让
    后续命令拼成 `/demo/plugin`。
    """
    cfg_home = Path(plugin_home)
    _write_lane_config_keeping_plugins(cfg_home, model=live_model.model)
    env = {
        "HOME": str(cfg_home),
        "OPENAI_API_KEY": "",
        "ANTHROPIC_API_KEY": "",
        "ZHUYE_MODEL": "",
        "ZHUYE_PROVIDER": "",
        "REFLECT_TUI_UPDATE_CHECK": "0",
    }
    with live_tui(env=env) as s:
        s.wait_boot(timeout=40)
        _submit(s, "plugin")
        s.wait_text("Installed Plugins", timeout=20)
        s._pump(settle=0.8, max_wait=1.5)
        txt = safe_text(s)
        assert "demo@inline" in txt, f"overlay 应展示插件 id:\n{txt[-600:]}"
        assert not s.has_panic()


# ── F11:S 组独立用例(弹窗 / 未知命令 / 转义 / 行首空格)────────────────────


def test_live_unknown_command_stays_in_composer(live_tui) -> None:
    """F31 完全不存在的命令:不提交、输入留在框里、无 Unrecognized 提示。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("/definitely-not-a-command")
        s.key("enter")
        time.sleep(2.0)
        txt = safe_text(s)
        assert "/definitely-not-a-command" in txt, "未知命令应留在输入框"
        assert "Unrecognized" not in txt, f"当前实现丢弃 Unrecognized 提示:\n{txt[-400:]}"


def test_live_command_popup_lists_commands(live_tui) -> None:
    """F32 命令弹窗:列出命令名与描述。"""
    with live_tui(cols=130, rows=44) as s:
        s.wait_boot(timeout=30)
        s.type_text("/")
        s._pump(settle=1.2, max_wait=2.0)
        txt = safe_text(s)
        assert "/model" in txt, f"命令弹窗应含 /model:\n{txt[-800:]}"
        assert "choose what model" in txt, f"弹窗应显示命令描述:\n{txt[-800:]}"
        assert "/permissions" in txt, f"命令弹窗应含 /permissions:\n{txt[-800:]}"


def test_live_command_popup_filters(live_tui) -> None:
    """F33 命令弹窗按前缀过滤。

    ⚠️ **单独开会话**:弹窗开着时续输会拼成 `//stat`(composer 里已有 `/`),
    过滤不到 —— 能力覆盖层 S4 也是独立会话,口径保持一致。
    """
    with live_tui(cols=130, rows=44) as s:
        s.wait_boot(timeout=30)
        s.type_text("/stat")
        s._pump(settle=1.2, max_wait=2.0)
        txt = safe_text(s)
        assert "status" in txt, f"过滤后应含 status:\n{txt[-800:]}"


def test_live_escaped_slash_is_plain_text(live_tui) -> None:
    """F34 名字里带 `/` 的输入不按命令解析(当普通文本)。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("a/b")
        s.key("enter")
        time.sleep(2.0)
        txt = safe_text(s)
        assert "a/b" in txt, "带斜杠的文本应作为普通消息上屏"
        assert "尚未接入分发" not in txt, "不应被当作命令解析"


def test_live_leading_space_allows_unknown_command(live_tui, live_sandbox: Sandbox) -> None:
    """F35 行首空格 → composer 不做「未知命令」拦截,按普通消息发给 agent。

    真模型这一条更有意义:消息**真的**进了模型回合(离线层只能断言上屏)。
    """
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text(" /not-a-real-command")
        s.key("enter")
        _wait_turn_end(s, live_sandbox)
    body = _first_user_text(live_sandbox.home, "/not-a-real-command")
    assert body is not None and "/not-a-real-command" in body, (
        f"行首空格的未知命令应作为普通消息提交:{body!r}"
    )


def test_live_leading_space_still_parses_known_command(live_tui) -> None:
    """F36 行首空格**不**阻止已知命令被解析(` /clear` 仍弹二次确认)。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text(" /clear")
        s.key("enter")
        s.wait_text("Clear scrollback", timeout=15)
        s.type_text("n")
        s.key("enter")


# ── F12:K 组剩余(大文本 / 分段粘贴 / Ctrl+A 行首编辑)───────────────────────


def test_live_large_paste_survives_submission(live_tui, live_sandbox: Sandbox) -> None:
    """F37 一次粘贴 300+ 字符:提交后落盘内容完整不丢。"""
    payload = "PASTE_BLOB_" + "x" * 300
    with live_tui() as s:
        s.wait_boot(timeout=30)
        # 探针串放**句首**、指令放句尾:模型看到无意义串会跑去「探索工作区」,
        # 回合能拖好几分钟;句首让断言仍能按前缀定位。
        s.type_text(f"{payload} 请只回复上面这段文字,不要做任何别的事")
        s.key("enter")
        _wait_turn_end(s, live_sandbox)

    body = _first_user_text(live_sandbox.home, "PASTE_BLOB_")
    assert body is not None and body.startswith(payload), (
        f"落盘内容应完整({len(payload)} 字符),实际 {len(body or '')} 字符"
    )


def test_live_burst_paste_chunks_stay_coherent(live_tui, live_sandbox: Sandbox) -> None:
    """F38 连续多段快速输入(模拟粘贴突发):composer 不丢段。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("BURST_PART_ONE ")
        s.type_text("BURST_PART_TWO ")
        s.type_text("BURST_PART_THREE 请只回复上面这段文字,不要做任何别的事")
        s.key("enter")
        _wait_turn_end(s, live_sandbox)

    body = _first_user_text(live_sandbox.home, "BURST_PART_ONE")
    assert body is not None and body.startswith(
        "BURST_PART_ONE BURST_PART_TWO BURST_PART_THREE"
    ), f"三段应按序拼合:{body!r}"


def test_live_inline_editing_ctrl_a_home(live_tui, live_sandbox: Sandbox) -> None:
    """F39 Ctrl+A 跳行首后补前缀:提交的最终文本 = 前缀 + 原文。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("TAIL_PART 请只回复上面这段文字,不要做任何别的事")
        s._pump(settle=0.4, max_wait=0.8)
        s.key("ctrl+a")
        s.type_text("HEAD_PART ")
        s._pump(settle=0.4, max_wait=0.8)
        s.key("enter")
        _wait_turn_end(s, live_sandbox)

    body = _first_user_text(live_sandbox.home, "HEAD_PART")
    assert body is not None and body.startswith("HEAD_PART TAIL_PART"), (
        f"Ctrl+A 后插入应落在行首:{body!r}"
    )


# ── F13:V 组剩余(无色 / 浅色主题 / 表格列表 / 极小窗口)──────────────────────


def test_live_nocolor_env_boots(live_tui) -> None:
    """F40 NO_COLOR=1 主题分支:首屏锚点齐全、不 panic。"""
    with live_tui(env={"NO_COLOR": "1"}) as s:
        s.wait_boot(timeout=30)
        txt = safe_text(s)
        assert "Send a message" in txt, f"composer 占位符应存在:\n{txt[-400:]}"
        assert not s.has_panic()


def test_live_light_theme_env_boots(live_tui) -> None:
    """F41 REFLECT_TUI_THEME=light 主题分支:首屏锚点齐全。"""
    with live_tui(env={"REFLECT_TUI_THEME": "light"}) as s:
        s.wait_boot(timeout=30)
        txt = safe_text(s)
        assert "Send a message" in txt, f"composer 占位符应存在:\n{txt[-400:]}"
        assert not s.has_panic()


def test_live_tiny_window_does_not_crash(live_tui) -> None:
    """F42 极小窗口 40x10:不 panic、进程存活。"""
    with live_tui(cols=40, rows=10) as s:
        s._pump(settle=2.0, max_wait=3.0)
        assert s.alive(), "极小窗口下进程应存活"
        assert s.screen_text().strip(), "极小窗口下应有渲染输出"
        assert not s.has_panic()


def test_live_markdown_list_renders(live_tui) -> None:
    """F43 真实回复里的 markdown 列表项渲染上屏。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("请只输出一个 markdown 无序列表,两项分别是 LISTITEM_ALPHA 和 LISTITEM_BETA")
        s.key("enter")
        txt = _wait_reply(s, timeout=DRAIN_TIMEOUT)
        assert "LISTITEM_ALPHA" in txt and "LISTITEM_BETA" in txt, (
            f"列表项应渲染:\n{txt[-400:]}"
        )


# ── F14:P 组剩余(resume 恢复 / prompt 历史 / 无效 id)──────────────────────


def test_live_resume_continues_session(live_tui, live_sandbox: Sandbox) -> None:
    """F44 `--resume <id>` 续作真实会话:上轮内容被回放进上下文。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("记住口令 RAINBOW-77,只回复:好的")
        s.key("enter")
        _wait_turn_end(s, live_sandbox)

    files = live_sandbox.session_files()
    assert files, "应有会话文件"
    session_id = files[0].stem

    # 续作:新进程 + --resume <id> → 上轮内容应回填进 scrollback。
    with live_tui(args=["--resume", session_id]) as s:
        s.wait_boot(timeout=40)
        s._pump(settle=1.5, max_wait=3.0)
        txt = safe_text(s)
        assert "RAINBOW-77" in txt, f"resume 后上轮内容应回填:\n{txt[-600:]}"
        assert not s.has_panic()


def test_live_resume_with_invalid_id_fails_fast(live_tui) -> None:
    """F45 **格式非法**的 resume id:快速失败(非 0 退出),不是挂死。

    注意:合法但**不存在**的 UUID 不会报错(当作新会话起)—— 失败快的只有
    格式非法的 id,与能力覆盖层 P5 同口径。
    """
    with live_tui(args=["--resume", "not-a-uuid"]) as s:
        deadline = time.monotonic() + 20
        rc = None
        while time.monotonic() < deadline:
            rc = s.returncode()
            if rc is not None:
                break
            s._pump(settle=0.4, max_wait=0.8)
        assert rc is not None, "格式非法的 id 应快速退出而不是挂死"
        assert rc != 0, f"应非 0 退出,实际 {rc}"
        raw = s.raw_output().lower()
        assert "thread id" in raw or "invalid" in raw, f"应有 id 诊断文案:{raw[-400:]}"


def test_live_prompt_history_persisted(live_tui, live_sandbox: Sandbox) -> None:
    """F46 真实回合后 prompt 历史落盘(`~/.zhuye/prompt_history.jsonl`)。"""
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("HISTORY_PROBE_LINE 请只回复上面这段文字,不要做任何别的事")
        s.key("enter")
        _wait_turn_end(s, live_sandbox)

    hist = live_sandbox.home / ".zhuye" / "prompt_history.jsonl"
    assert hist.exists(), "应有 prompt 历史文件"
    assert "HISTORY_PROBE_LINE" in hist.read_text(errors="replace"), "历史应含本轮输入"


# ── F15:插件命令提交(不被「未知命令」拦截)────────────────────────────────


def test_live_plugin_slash_command_submits(live_tui, plugin_home: str, live_model) -> None:
    """F47 插件 slash 命令能被 composer 放行(不被当未知命令静默吞掉)。"""
    cfg_home = Path(plugin_home)
    _write_lane_config_keeping_plugins(cfg_home, model=live_model.model)
    env = {
        "HOME": str(cfg_home),
        "OPENAI_API_KEY": "",
        "ANTHROPIC_API_KEY": "",
        "ZHUYE_MODEL": "",
        "ZHUYE_PROVIDER": "",
        "REFLECT_TUI_UPDATE_CHECK": "0",
    }
    with live_tui(env=env) as s:
        s.wait_boot(timeout=40)
        s.type_text("/demo:hello")
        s.key("enter")
        time.sleep(2.5)
        txt = safe_text(s)
        assert "Unrecognized" not in txt, f"插件命令不应被当未知命令:\n{txt[-500:]}"
        assert "/demo:hello" in txt, f"插件命令应上屏:\n{txt[-500:]}"


# ── 落盘查询助手 ───────────────────────────────────────────────────────────


def _first_user_text(home: Path, prefix: str) -> str | None:
    """从会话 JSONL 里找第一条以 `prefix` 开头的 user 消息正文。"""
    for path in sorted((home / ".zhuye" / "sessions").glob("*/*/*/*.jsonl")):
        try:
            lines = path.read_text(errors="replace").splitlines()
        except OSError:
            continue
        for line in lines:
            try:
                rec = json.loads(line)
            except Exception:  # noqa: BLE001 - 坏行跳过
                continue
            if rec.get("type") != "message" or rec.get("role") != "user":
                continue
            content = rec.get("content")
            blocks = content if isinstance(content, list) else []
            for block in blocks:
                if not isinstance(block, dict):
                    continue
                text = block.get("text")
                if isinstance(text, str) and text.startswith(prefix):
                    return text
    return None


# 回合等待预算(秒)—— 真模型一个回合可能几十秒;配额抖动时会退化成重试,
# 预算收紧一点,好让「上游限流」尽快以「跳过」而不是「失败」呈现。
TURN_BUDGET = 150.0

# 上游异常痕迹(raw 流里)—— 命中即按「上游问题」跳过,而不是算产品失败。
UPSTREAM_ERROR_MARKERS = (
    "rate limited",
    "exhausted",
    "promotion_model_limit",
    "stream error",
    "too many requests",
    "invalid request",
)


def _wait_turn_end(s: TuiSession, sb: Sandbox, timeout: float = TURN_BUDGET) -> str:
    """等回合收尾(不忙 + assistant 消息已落盘)。

    - 限流 → 跳过(测的是代码,不是配额);
    - 超时且原始流里有**上游异常痕迹** → 跳过;
    - 超时且没有上游痕迹 → 失败(真的收不了尾)。
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        _skip_if_rate_limited(s)
        txt = safe_text(s)
        # ⚠️ 真实模型常在回合里申请工具(`ls -la` 之类)—— 没人批准,回合就一直
        # 挂在审批条上(既不收尾也不报错)。这里顺手批掉。
        if "APPROVAL NEEDED" in txt:
            s.type_text("y")
            time.sleep(1.0)
            continue
        if not is_busy(s.screen_text()) and assistant_message_count(sb.home) > 0:
            return safe_text(s)
        s._pump(settle=0.6, max_wait=1.2)
    raw = s.raw_output().lower()
    if any(marker in raw for marker in UPSTREAM_ERROR_MARKERS):
        pytest.skip("回合未收尾且上游报错(限流/端点故障)—— 跳过,不是失败")
    raise AssertionError(f"{timeout:.0f}s 内回合未收尾:\n{safe_text(s)[-500:]}")


# ── F16:回退选择器 + 中断(真模型回合)──────────────────────────────────────


def test_live_rewind_picker_after_real_turn(live_tui, live_sandbox: Sandbox) -> None:
    """F48 `/rewind` 在有真实历史时打开回退选择器。

    无历史时是占位提示「no user prompts to rewind to yet」;真模型跑一轮之后
    历史里有 user prompt → 打开 `Rewind — select user prompt to re-edit &
    re-send` 选择器。这是第 5 个**可达**弹层(前 4 个已在 F2 覆盖)。
    """
    with live_tui(cols=130, rows=44) as s:
        s.wait_boot(timeout=30)
        s.type_text("用一句话回答:什么是递归?")
        s.key("enter")
        _wait_turn_end(s, live_sandbox)

        _submit(s, "rewind")
        txt = s.wait_text("Rewind — select user prompt", timeout=20)
        assert "re-edit" in txt, f"选择器标题应完整:\n{txt[-300:]}"
        s.key("esc")
        s._pump(settle=0.5, max_wait=1.0)
        assert not s.has_panic()


def test_live_esc_interrupts_real_turn(live_tui, live_sandbox: Sandbox) -> None:
    """F49 Esc 中断真实回合:屏幕给出「已发送中断请求」,进程不崩。

    真模型的首字通常要 5~30s —— 提交后立刻按 Esc,正好落在回合进行中。
    """
    with live_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("请详细写一篇 800 字的中文文章,主题是分布式系统的一致性。")
        s.key("enter")
        # ⚠️ 必须等 HUD **转 busy** 再按 Esc:idle 态的 Esc 是「回到上一条消息
        # 编辑」,不是中断 —— 回合还没开始就按 Esc,什么都不会发生。
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline and not is_busy(s.screen_text()):
            s._pump(settle=0.4, max_wait=0.8)
        assert is_busy(s.screen_text()), "回合未进入 busy 态(模型还没开始回话)"
        s.key("esc")
        txt = s.wait_text("已发送中断请求", timeout=30)
        assert "已发送中断请求" in txt, f"应有中断提示:\n{txt[-300:]}"
        assert not s.has_panic()
