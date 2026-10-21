"""覆盖自审的 pytest 门禁。

`tests/tools/coverage_audit.py` 是**机械核对**工具:从 Rust 源码解析出
「声明了哪些 slash 命令」,与 `tests/functional/test_slash.py` 的 `MATRIX`
逐条比对。本模块把它的结论变成 CI 门禁 —— 新增一条 slash 命令却忘了加
用例,这里就会失败。

失败时的提示指向修复方式:
  - 补 `tests/functional/test_slash.py` 的 `MATRIX` 一行;
  - 或在 `tests/COVERAGE.md` 的 §7「未覆盖清单」里显式登记为已知缺口。
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests" / "tools"))

from coverage_audit import audit  # noqa: E402


@pytest.fixture(scope="module")
def report() -> dict:
    return audit()


def test_no_uncovered_slash_commands(report: dict) -> None:
    """A1 每条声明出来的 slash 命令都必须有行为级用例。"""
    uncovered = report["uncovered"]
    assert not uncovered, (
        "以下 slash 命令已声明但无用例覆盖:\n  "
        + "\n  ".join(f"/{c}" for c in uncovered)
        + "\n\n修复:在 tests/functional/test_slash.py 的 MATRIX 补一行;"
        " 或在 tests/COVERAGE.md §7 登记为已知缺口并在此处豁免。"
    )


def test_matrix_covers_every_declared_command(report: dict) -> None:
    """A2 矩阵覆盖率 100%(与 A1 同源的数值化断言,便于 CI 日志阅读)。"""
    counts = report["counts"]
    assert counts["declared"] > 0, "应至少解析出一条命令"
    assert counts["covered"] == counts["declared"], (
        f"覆盖率 {counts['covered']}/{counts['declared']} < 100%"
    )


def test_audit_parses_all_pickers(report: dict) -> None:
    """A3 覆盖自审能解析出全部 11 个 picker 模块(解析器没退化)。"""
    assert len(report["picker_titles"]) == 11, (
        f"应解析出 11 个 picker,实际 {len(report['picker_titles'])}: "
        f"{sorted(report['picker_titles'])}"
    )


def test_enum_command_count_is_stable(report: dict) -> None:
    """A4 `SlashCommand` 枚举变体数量不低于基线(漏解析 = 工具坏了)。"""
    # 枚举当前 58 个变体(含别名共 61 个命令名);留出余量只做下限断言,
    # 具体新增命令由 A1 负责报缺口。
    assert report["counts"]["enum"] >= 61, (
        f"枚举命令数 {report['counts']['enum']} < 基线 61,解析器可能失效"
    )
