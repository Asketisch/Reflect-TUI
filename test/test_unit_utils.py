#!/usr/bin/env python3
"""PHASE 1:Python 侧单元测试(harness 纯逻辑,离线、无 pty)。"""
from __future__ import annotations

import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import harness  # noqa: E402

RES: list[bool] = []


def check(name: str, ok: bool, detail: str = "") -> bool:
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}{(' — ' + detail) if detail else ''}")
    RES.append(ok)
    return ok


def test_normalize() -> None:
    print("\n---------- normalize ----------")
    check("collapses whitespace", harness.normalize("a\n  b\t c") == "a b c")
    check("keeps single spaces", harness.normalize("mock reply ok") == "mock reply ok")
    check("marker match survives wrap", "MOCK_E2E_OK: fine" in harness.normalize("x\nMOCK_E2E_OK:\n  fine"))


def test_find_binary_env() -> None:
    print("\n---------- find_binary (REFLECT_TUI_BIN) ----------")
    os.environ["REFLECT_TUI_BIN"] = "/bin/sh" if sys.platform != "win32" else "cmd"
    try:
        p = harness.find_binary()
        check("env override found", Path(p).exists(), p)
    except FileNotFoundError:
        # /bin/sh 不存在的裸环境(如 Windows)只提示,不判 FAIL。
        print("  [SKIP] /bin/sh unavailable on this platform")
    finally:
        os.environ.pop("REFLECT_TUI_BIN", None)


def test_find_binary_missing_raises() -> None:
    print("\n---------- find_binary (missing) ----------")
    # 指向不存在的 env 且清掉候选路径的探测基点:临时把 REPO_ROOT 换走,
    # 避免 CI 上 target/release 恰好存在。
    os.environ["REFLECT_TUI_BIN"] = str(HERE / "definitely-missing-bin")
    orig_root = harness.REPO_ROOT
    try:
        harness.REPO_ROOT = HERE / "empty-root"
        try:
            harness.find_binary()
            check("missing binary raises", False, "expected FileNotFoundError")
        except FileNotFoundError:
            check("missing binary raises", True)
    finally:
        harness.REPO_ROOT = orig_root
        os.environ.pop("REFLECT_TUI_BIN", None)


def main() -> int:
    print("========== PHASE 1 / Python unit (test_unit_utils) ==========")
    test_normalize()
    test_find_binary_env()
    test_find_binary_missing_raises()
    ok = all(RES)
    print(f"\nunit: {'PASS' if ok else 'FAIL'} ({sum(RES)}/{len(RES)})")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
