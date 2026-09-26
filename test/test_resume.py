#!/usr/bin/env python3
"""PHASE 6:会话恢复 E2E —— `-c` 续最近会话,历史回填进 scrollback。

离线(mock provider)。流程:启动 → 发一条消息 → 退出 → `-c` 重启 →
断言「已恢复历史会话」回填标记与既往 user 消息出现在恢复的会话里。
"""
from __future__ import annotations

import os
import signal
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from harness import TuiSession, normalize  # noqa: E402

RES: list[bool] = []

PRIOR_MSG = "hello for resume test"
RESUME_MARKER = "已恢复历史会话"


def check(name: str, ok: bool, detail: str = "") -> bool:
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}{(' — ' + detail) if detail else ''}")
    RES.append(ok)
    return ok


def wait_for(s: TuiSession, pred, timeout: float, label: str) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if pred(normalize(s.screen_text())):
            return True
        s._pump(settle=0.4, max_wait=0.6)
    return False


def quit_tui(s: TuiSession, timeout: float = 6.0) -> bool:
    """双击 Ctrl-C 收尾;哑 pty 下进程退出不可靠,SIGTERM 兜底。"""
    s._send(b"\x03")
    time.sleep(0.1)
    s._send(b"\x03")
    deadline = time.time() + timeout
    while time.time() < deadline:
        if s._proc.poll() is not None:
            return True
        time.sleep(0.2)
    try:
        os.killpg(os.getpgid(s._proc.pid), signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        pass
    deadline = time.time() + 2.0
    while time.time() < deadline:
        if s._proc.poll() is not None:
            return True
        time.sleep(0.2)
    return s._proc.poll() is not None


def main() -> int:
    print("========== PHASE 6 / Resume E2E (-c) ==========")
    home = tempfile.mkdtemp(prefix="reflect-tui-resume-")
    env = {
        "HOME": home,
        "REFLECT_MODEL": "mock/mock-1",
        "REFLECT_TUI_UPDATE_CHECK": "0",
    }

    # 会话 1:发一条消息(mock 回复),退出。
    with TuiSession(env=env, cols=110, rows=32) as s:
        s.wait_stable(max_wait=15.0)
        s.type_fast(PRIOR_MSG)
        s.enter(settle=1.0)
        replied = wait_for(
            s, lambda t: "Hello from mock LLM" in t, timeout=30.0, label="mock reply"
        )
        check("session 1: mock reply rendered", replied)
        check("session 1: clean shutdown", quit_tui(s))

    # 会话 2:-c 恢复,断言回填。
    with TuiSession(args=["-c"], env=env, cols=110, rows=40) as s:
        ok = False
        deadline = time.time() + 15.0
        while time.time() < deadline:
            t = normalize(s.screen_text())
            if RESUME_MARKER in t and PRIOR_MSG in t:
                ok = True
                break
            time.sleep(0.3)
        check("resume: backfill marker + prior message restored", ok)
        check("resume: clean shutdown", quit_tui(s))

    ok = all(RES)
    print(f"\nresume e2e: {'PASS' if ok else 'FAIL'} ({sum(RES)}/{len(RES)})")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
