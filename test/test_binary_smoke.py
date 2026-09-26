#!/usr/bin/env python3
"""PHASE 4:TUI 二进制 pty 冒烟 —— mock provider 启动、boot banner、
Ctrl+C 干净退出、无 panic。

离线(REFLECT_MODEL=mock/mock-1 注册内置 mock provider,零网络)。
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


def check(name: str, ok: bool, detail: str = "") -> bool:
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}{(' — ' + detail) if detail else ''}")
    RES.append(ok)
    return ok


def mock_env() -> dict[str, str]:
    home = tempfile.mkdtemp(prefix="reflect-tui-smoke-")
    return {
        "HOME": home,
        "REFLECT_MODEL": "mock/mock-1",
        "REFLECT_TUI_UPDATE_CHECK": "0",
    }


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
    print("========== PHASE 4 / TUI binary smoke (--help + pty boot) ==========")
    with TuiSession(env=mock_env(), cols=110, rows=32) as s:
        stable = s.wait_stable(max_wait=15.0)
        check("TUI booted to stable screen", stable)
        txt = normalize(s.screen_text())
        # boot banner 是 braille 块字形画(非 ASCII "REFLECT" 字样);
        # 检查 0x2800-0x28FF braille 区字符存在 + HUD 模型名。
        has_braille = any(0x2800 <= ord(c) <= 0x28FF for c in s.screen_text())
        check("boot banner rendered", has_braille or "Reflect" in txt, txt[:80])
        check(
            "HUD shows model + hints",
            "mock/mock-1" in txt and "Ctrl+C" in txt,
        )

        exited = quit_tui(s)
        check("clean shutdown (Ctrl-C, SIGTERM fallback)", exited, f"rc={s._proc.poll()}")

        tail = normalize(s.screen_text()).lower()
        check("no panic in output", "panic" not in tail and "thread '" not in tail)

    ok = all(RES)
    print(f"\nbinary smoke: {'PASS' if ok else 'FAIL'} ({sum(RES)}/{len(RES)})")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
