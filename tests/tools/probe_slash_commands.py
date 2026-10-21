#!/usr/bin/env python3
"""探测:逐条 slash 命令在真二进制上的**运行时实际行为**。

用途:为 `tests/COVERAGE.md` 提供**实证**的覆盖矩阵,而不是静态推测。
本脚本不是断言测试(那是 `tests/functional/test_slash.py` 的职责),
它只产出「命令 → 屏幕可见效果」的事实清单。

用法:
    python3 tests/tools/probe_slash_commands.py [--out FILE]
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "functional"))

from tui_pty import Sandbox, TuiSession  # noqa: E402

# 全部候选命令:枚举 58 条 + legacy 通道名。
ENUM_COMMANDS = [
    "model", "ide", "permissions", "keymap", "vim", "setup-default-sandbox",
    "sandbox-add-read-dir", "experimental", "approve", "memories", "skills",
    "import", "hooks", "review", "rename", "new", "archive", "delete", "resume",
    "rewind", "fork", "app", "init", "compact", "plan", "goal", "loop", "agent",
    "side", "btw", "copy", "raw", "diff", "mention", "status", "usage",
    "debug-config", "title", "statusline", "theme", "pets", "mcp", "apps",
    "plugins", "plugin", "prompt", "logout", "quit", "exit", "feedback",
    "rollout", "ps", "stop", "clean", "clear", "personality", "test-approval",
    "subagents", "debug-m-drop", "debug-m-update",
]

LEGACY_ONLY = [
    "help", "?", "tasks", "agents", "cost", "exit-plan", "exitplan", "mode",
    "effort", "ref", "rm-session", "version", "doctor", "history", "files",
    "branch", "image", "img", "checkpoint", "traces", "think", "thinking",
    "retry", "prompt", "usage",
]

# 需要参数的命令(无参会走 usage 分支,单列一组)。
NEEDS_ARGS = {
    "plan": "refactor the auth module",
    "goal": "ship the release",
    "model": "gpt-4o",
    "effort": "high",
    "mode": "plan",
    "loop": "30 /status",
    "sandbox-add-read-dir": "/tmp",
    "rename": "probe-name",
}


def snapshot(s: TuiSession) -> str:
    return s.normalized()


def probe(sandbox: Sandbox, command: str, args: str = "") -> dict:
    """跑一条 slash 命令,返回可见效果。"""
    sb = sandbox
    result: dict = {"command": command, "args": args}
    with TuiSession(sb, env=sb.mock_env(), cols=120, rows=40) as s:
        before = ""
        try:
            s.wait_boot(timeout=30)
            before = snapshot(s)
        except AssertionError as e:
            result["error"] = f"boot failed: {e}"
            return result
        text = f"/{command} {args}".strip() if args else f"/{command}"
        s.type_text(text)
        s.key("enter")
        # 给足时间让 notice / overlay / 提交落定。
        deadline = time.monotonic() + 6.0
        after = before
        while time.monotonic() < deadline:
            after = snapshot(s)
            if after != before:
                break
            time.sleep(0.25)
        # 再多等一点,捕捉异步落定的内容。
        time.sleep(1.0)
        after = snapshot(s)
        result["alive"] = s.alive()
        result["exited"] = not s.alive()
        result["output"] = after
        result["changed"] = after != before
        # 抽出新增行(相对启动态)作为「可见效果」。
        new_lines = [
            ln for ln in after.split("  ") if ln and ln not in before
        ]
        result["diff_hint"] = " | ".join(new_lines[:8])[:600]
    return result


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=None)
    ap.add_argument("--only", default=None, help="只探测这些命令(逗号分隔)")
    args = ap.parse_args()

    commands = ENUM_COMMANDS + LEGACY_ONLY
    if args.only:
        wanted = {c.strip() for c in args.only.split(",")}
        commands = [c for c in commands if c in wanted]

    sandbox = Sandbox.create("reflect-tui-probe-")
    out: list[dict] = []
    try:
        for cmd in commands:
            extra = NEEDS_ARGS.get(cmd, "")
            if cmd in NEEDS_ARGS:
                r = probe(sandbox, cmd, extra)
            else:
                r = probe(sandbox, cmd)
            out.append(r)
            flag = "EXIT" if r.get("exited") else ("CHG " if r.get("changed") else "same")
            print(f"[{flag}] /{cmd:24s} {r.get('diff_hint', '')[:110]}", flush=True)
    finally:
        sandbox.cleanup()

    payload = json.dumps(out, ensure_ascii=False, indent=2)
    if args.out:
        Path(args.out).write_text(payload)
        print(f"\n→ {args.out}")
    else:
        print(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
