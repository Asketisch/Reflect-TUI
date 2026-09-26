#!/usr/bin/env python3
"""PHASE 3:Headless CLI 冒烟 —— `--help` 与 session / traces 子命令。

离线、无 pty、无 LLM。二进制由 `harness.find_binary()` 定位。
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from harness import find_binary  # noqa: E402

RES: list[bool] = []


def check(name: str, ok: bool, detail: str = "") -> bool:
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}{(' — ' + detail) if detail else ''}")
    RES.append(ok)
    return ok


def run_cli(args: list[str], timeout: float = 30.0) -> tuple[int, str]:
    env = dict(os.environ)
    env.setdefault("REFLECT_TUI_UPDATE_CHECK", "0")
    try:
        r = subprocess.run(
            [find_binary(), *args],
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )
        return r.returncode, r.stdout + r.stderr
    except FileNotFoundError as e:
        return 127, str(e)
    except subprocess.TimeoutExpired:
        return 124, "TIMEOUT"


def expect(name: str, args: list[str], *keywords: str, code: int = 0) -> None:
    rc, out = run_cli(args)
    ok = rc == code and all(k.lower() in out.lower() for k in keywords)
    check(name, ok, f"rc={rc}" + ("" if ok else f" out={out[:200]!r}"))


def main() -> int:
    print("========== PHASE 3 / Headless CLI (session / traces) ==========")
    expect("--help shows usage", ["--help"], "usage", "session", "traces")
    expect("session --help", ["session", "--help"], "usage")
    expect("session ls runs", ["session", "ls", "--limit", "3"], "session")
    expect("traces --help", ["traces", "--help"], "usage")
    ok = all(RES)
    print(f"\ncli help: {'PASS' if ok else 'FAIL'} ({sum(RES)}/{len(RES)})")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
