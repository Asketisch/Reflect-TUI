#!/usr/bin/env python3
"""PHASE 5:Mock LLM E2E —— REFLECT_MOCK_SCRIPT 脚本化回复端到端上屏。

引擎侧协议(reflect-llm/src/providers/mock.rs):
  - `REFLECT_MODEL=mock/mock-1`(或 REFLECT_PROVIDER=mock)注册 mock client;
  - `REFLECT_MOCK_SCRIPT` 指向 JSONL,每行一次模型调用:
      {"type":"text","text":"..."} 或 {"type":"tool_call","name":"...","args":{...}}
  - 脚本耗尽回退 "Hello from mock LLM!"。

离线、零 API key。
"""
from __future__ import annotations

import json
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

REPLY_1 = "MOCK_E2E_OK: the scripted reply path renders end to end."
REPLY_2 = "Hello from mock LLM!"


def check(name: str, ok: bool, detail: str = "") -> bool:
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}{(' — ' + detail) if detail else ''}")
    RES.append(ok)
    return ok


def wait_for(s: TuiSession, pred, timeout: float, label: str):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        txt = s.screen_text()
        last = txt
        if pred(normalize(txt)):
            return True, txt
        s._pump(settle=0.4, max_wait=0.6)
    print(f"      [timeout {label} ({timeout:.0f}s)] screen tail:")
    for ln in last.splitlines()[-9:]:
        if ln.strip():
            print(f"        |{ln}")
    return False, last


def quit_tui(s: TuiSession, timeout: float = 6.0) -> bool:
    """收尾:双击 Ctrl-C(应用的多 picker 双击退出语义);哑 pty 下进程退出
    不可靠(真实终端行为正常),超时则 SIGTERM 兜底。返回是否已终止。"""
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
    print("========== PHASE 5 / Mock LLM E2E ==========")
    # 两行脚本:turn1 固定回复(验证脚本消费),turn2 回退默认(验证耗尽路径)。
    script_lines = [json.dumps({"type": "text", "text": REPLY_1})]
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".jsonl", encoding="utf-8", delete=False
    ) as f:
        f.write("\n".join(script_lines) + "\n")
        script_path = f.name

    env = {
        "HOME": tempfile.mkdtemp(prefix="reflect-tui-mock-"),
        "REFLECT_MODEL": "mock/mock-1",
        "REFLECT_TUI_UPDATE_CHECK": "0",
        "REFLECT_MOCK_SCRIPT": script_path,
    }
    try:
        with TuiSession(env=env, cols=110, rows=34) as s:
            s.wait_stable(max_wait=15.0)
            s.type_fast("give me the scripted reply")
            s.enter(settle=1.0)
            ok, _ = wait_for(
                s, lambda t: REPLY_1 in t, timeout=30.0, label="scripted reply"
            )
            check("scripted mock reply on screen", ok)

            # 第二问消耗脚本后的回退路径。
            s.type_fast("and one more")
            s.enter(settle=1.0)
            ok2, _ = wait_for(
                s, lambda t: REPLY_2 in t, timeout=30.0, label="fallback reply"
            )
            check("script-exhausted fallback reply", ok2)

            exited = quit_tui(s)
            check("clean shutdown (Ctrl-C, SIGTERM fallback)", exited)
            tail = normalize(s.screen_text()).lower()
            check("no panic in output", "panic" not in tail)
    finally:
        Path(script_path).unlink(missing_ok=True)

    ok = all(RES)
    print(f"\nmock e2e: {'PASS' if ok else 'FAIL'} ({sum(RES)}/{len(RES)})")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
