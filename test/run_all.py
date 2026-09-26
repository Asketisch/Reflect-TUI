#!/usr/bin/env python3
"""Reflect-TUI 测试入口:Python unit → Rust 单测 → CLI → pty 冒烟 → mock E2E(→ live)。

用法(仓库根目录)::

    python3 test/run_all.py                 # 离线全套(不含 live)
    python3 test/run_all.py --all           # 追加 live LLM E2E(需 API key)
    python3 test/run_all.py --unit-only     # 只跑 PHASE 1
    python3 test/run_all.py --no-rust --no-binary --no-mock

原脚本丢失后依据 `__pycache__` 符号表与 `test_workflow_live.py` 调用面重建。
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent

TIMEOUT = 600.0


def _run(cmd: list[str], timeout: float = TIMEOUT) -> tuple[int, str]:
    """跑命令,返回 (returncode, stdout+stderr)。超时/缺失按失败处理。"""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return r.returncode, r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        return 124, f"TIMEOUT after {timeout:.0f}s: {' '.join(cmd)}"
    except FileNotFoundError:
        return 127, f"NOT FOUND: {' '.join(cmd)}"


def _section(title: str) -> None:
    print()
    print("=" * 64)
    print(title)
    print("=" * 64)


def phase_unit() -> bool:
    _section("PHASE 1 / Python unit (test_unit_utils.py)")
    rc, out = _run([sys.executable, str(HERE / "test_unit_utils.py")])
    print(out)
    return rc == 0


def phase_rust() -> bool:
    _section("PHASE 2 / Rust unit (cargo test -p reflect-tui)")
    rc, out = _run(["cargo", "test", "-p", "reflect-tui", "--lib", "--quiet"])
    # 只打尾巴:cargo test 输出很长。
    print("\n".join(out.splitlines()[-15:]))
    return rc == 0


def phase_cli() -> bool:
    _section("PHASE 3 / Headless CLI (session ls / traces ls)")
    rc, out = _run([sys.executable, str(HERE / "test_cli_help.py")])
    print(out)
    return rc == 0


def phase_binary() -> bool:
    _section("PHASE 4 / TUI binary smoke (--help + pty)")
    rc, out = _run([sys.executable, str(HERE / "test_binary_smoke.py")])
    print(out)
    return rc == 0


def phase_mock_e2e() -> bool:
    _section("PHASE 5 / Mock LLM E2E")
    rc, out = _run([sys.executable, str(HERE / "test_mock_e2e.py")])
    print(out)
    return rc == 0


def phase_resume() -> bool:
    _section("PHASE 6 / Resume E2E (-c)")
    rc, out = _run([sys.executable, str(HERE / "test_resume.py")])
    print(out)
    return rc == 0


def phase_plugin_ui() -> bool:
    _section("PHASE 7 / Plugin UI E2E (popup / overlay / PluginLoaded notice)")
    rc, out = _run([sys.executable, str(HERE / "test_plugin_ui.py")])
    print(out)
    return rc == 0


def phase_live_e2e() -> bool:
    _section("PHASE 8 / Live LLM E2E (needs API key)")
    target = HERE / "test_workflow_live.py"
    if not target.exists():
        print(f"[skip] {target.name} not present")
        return True
    rc, out = _run([sys.executable, str(target)], timeout=900.0)
    print(out)
    return rc == 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Reflect-TUI test runner")
    ap.add_argument("--all", action="store_true", help="也跑 live LLM E2E(需 API key)")
    ap.add_argument("--unit-only", action="store_true", help="只跑 PHASE 1")
    ap.add_argument("--no-rust", action="store_true", help="跳过 cargo test")
    ap.add_argument("--no-binary", action="store_true", help="跳过 pty 冒烟")
    ap.add_argument("--no-mock", action="store_true", help="跳过 mock E2E")
    ap.add_argument("--no-plugin", action="store_true", help="跳过插件 UI E2E")
    args = ap.parse_args()

    started = time.time()
    results: list[tuple[str, bool]] = []

    results.append(("unit", phase_unit()))
    if args.unit_only:
        results = results[-1:]
    else:
        if not args.no_rust:
            results.append(("rust", phase_rust()))
        results.append(("cli", phase_cli()))
        if not args.no_binary:
            results.append(("binary-smoke", phase_binary()))
        if not args.no_mock:
            results.append(("mock-e2e", phase_mock_e2e()))
        if not args.no_mock:
            results.append(("resume-e2e", phase_resume()))
        if not args.no_plugin:
            results.append(("plugin-ui-e2e", phase_plugin_ui()))
        if args.all:
            results.append(("live-e2e", phase_live_e2e()))

    _section("SUMMARY")
    for name, ok in results:
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}")
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} phases passed "
          f"in {time.time() - started:.0f}s")
    if failed:
        print(f"failed: {', '.join(failed)}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
