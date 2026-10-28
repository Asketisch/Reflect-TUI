#!/usr/bin/env python3
"""覆盖自审:「声明了什么」× 「测了什么」的差集核对。

**为什么需要它**:本项目的对外功能面里有 60+ 条 slash 命令、11 个弹层。
手工维护覆盖矩阵(哪怕是 markdown 表)必然漂移 —— 新增一条命令却忘了加
用例,矩阵不会自己报警。本工具做**机械核对**:

1. 从 `crates/zhuye-cli/src/tui_core/slash_command.rs` 解析 `SlashCommand`
   枚举的**全部变体与其对外命令名**(含 `#[strum(serialize=...)]` 覆盖);
2. 从 `crates/zhuye-cli/src/history_render.rs` 解析 legacy 通道的
   **白名单**(`is_legacy_passthrough_slash`)与**实现臂**(`dispatch_slash`);
3. 从 `crates/zhuye-cli/src/picker/` 解析各 picker 的**标题字面量**;
4. 从 `tests/functional/test_slash.py` 的 `MATRIX` 解析**已被测的命令**;
5. 报告差集。

用法:
    python3 tests/tools/coverage_audit.py            # 人读报告
    python3 tests/tools/coverage_audit.py --json     # 机器读
    python3 tests/tools/coverage_audit.py --strict   # 有缺口即退出码 1

退出码:0 = 无缺口;1 = 有缺口(--strict);2 = 解析失败。
"""

from __future__ import annotations

import argparse
import json
import re
import sys

# Windows 控制台默认 cp1252,输出中文会 UnicodeEncodeError;强制 UTF-8。
try:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
except AttributeError:  # Py < 3.7 无 reconfigure
    pass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates" / "zhuye-cli" / "src"
SLASH_RS = SRC / "tui_core" / "slash_command.rs"
HISTORY_RS = SRC / "history_render.rs"
PICKER_DIR = SRC / "picker"
TEST_SLASH = ROOT / "tests" / "functional" / "test_slash.py"


# ── 解析:SlashCommand 枚举 ───────────────────────────────────────────────

def parse_enum_commands(path: Path) -> dict[str, str]:
    """返回 {对外命令名: 枚举变体名}。

    变体的对外名规则(与 `strum` 一致):
      - `#[strum(to_string = "X")]` → X;
      - `#[strum(serialize = "X")]` → X;
      - `#[strum(to_string = "X", serialize = "Y")]` → X(主名),Y(别名);
      - 否则变体名 kebab-case。
    """
    text = path.read_text(encoding="utf-8")
    m = re.search(r"pub enum SlashCommand\s*\{(.*?)\n\}", text, re.S)
    if not m:
        raise SystemExit(f"解析失败:在 {path} 找不到 `pub enum SlashCommand`")
    body = m.group(1)

    commands: dict[str, str] = {}
    pending_attrs: list[str] = []
    for raw in body.splitlines():
        line = raw.strip()
        if not line or line.startswith("//"):
            continue
        if line.startswith("#[strum"):
            pending_attrs.append(line)
            continue
        variant = re.match(r"^([A-Z][A-Za-z0-9]*)\s*,?$", line)
        if not variant:
            if not line.startswith("#["):
                pending_attrs = []
            continue
        name = variant.group(1)
        attrs = " ".join(pending_attrs)
        pending_attrs = []

        primary = None
        aliases: list[str] = []
        for attr in re.findall(r'to_string\s*=\s*"([^"]+)"', attrs):
            primary = attr
        for attr in re.findall(r'serialize\s*=\s*"([^"]+)"', attrs):
            if primary is None:
                primary = attr
            else:
                aliases.append(attr)
        if primary is None:
            primary = re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()
        commands[primary] = name
        for alias in aliases:
            commands.setdefault(alias, name)
    return commands


# ── 解析:legacy 通道 ────────────────────────────────────────────────────

def _extract_fn_body(text: str, sig: str) -> str:
    """按 **行缩进边界** 提取函数体。

    不用花括号配平:Rust 的注释 / 原始字符串 / 生命周期里都可能出现裸的
    `{` `}`,配平会漂移。本仓库 rustfmt 风格统一,函数体止于列 0 的 `}`。
    """
    lines = text.splitlines()
    start = None
    for i, line in enumerate(lines):
        if sig in line:
            start = i
            break
    if start is None:
        raise SystemExit(f"解析失败:找不到 `{sig}`")
    body: list[str] = []
    for line in lines[start + 1 :]:
        if line.startswith("}"):
            break
        body.append(line)
    return "\n".join(body)


def parse_legacy_whitelist(path: Path) -> set[str]:
    """`is_legacy_passthrough_slash` 的白名单命令名(不含前导 `/`)。"""
    text = path.read_text(encoding="utf-8")
    body = _extract_fn_body(text, "pub fn is_legacy_passthrough_slash(name: &str) -> bool")
    return {n.lstrip("/") for n in re.findall(r'"([^"]+)"', body)}


def parse_dispatch_arms(path: Path) -> set[str]:
    """`dispatch_slash` 里实现的 legacy 命令名(match 臂的字面量,不含前导 `/`)。"""
    text = path.read_text(encoding="utf-8")
    body = _extract_fn_body(text, "pub fn dispatch_slash(text: &str) -> SlashOutcome")
    arms: set[str] = set()
    # match 臂形态:`"/help" | "/?" => ...`;也可能是裸标识符臂(变量绑定)。
    # 只认**以 `/` 开头**的字面量 —— 否则会把 `/prompt` 臂内层的
    # `match rest { "list" => ..., "reload" => ... }` 当成命令。
    for arm in re.finditer(
        r'^[ \t]*(?:"(/[^"]+)"(?:[ \t]*\|[ \t]*"(/[^"]+)")?)[ \t]*=>', body, re.M
    ):
        for g in arm.groups():
            if g:
                arms.add(g.lstrip("/"))
    return arms


def parse_popup_legacy(path: Path) -> set[str]:
    """`legacy_popup_commands()` 里列出的命令(会在命令弹窗里展示)。"""
    text = path.read_text(encoding="utf-8")
    m = re.search(
        r"pub fn legacy_popup_commands\(\) -> &'static \[\(&'static str, &'static str\)\]\s*\{(.*?)\n\}",
        text,
        re.S,
    )
    if not m:
        return set()
    return {n.lstrip("/") for n in re.findall(r'\(\s*"([^"]+)"\s*,', m.group(1))}


# ── 解析:picker 标题 ─────────────────────────────────────────────────────

def parse_picker_titles(picker_dir: Path) -> dict[str, str]:
    """{picker 模块名: 标题字面量}。

    标题取形如 `" 🎨  Theme "` 的居中标题常量(以 `标题` / `TITLE` 命名或
    直接是字符串字面量的框标题)。取不到时用模块名占位。
    """
    titles: dict[str, str] = {}
    for rs in sorted(picker_dir.glob("*.rs")):
        if rs.name == "mod.rs":
            continue
        text = rs.read_text(encoding="utf-8")
        found: list[str] = []
        # 居中框标题:形如 `let title = " 🎨  Theme ";` 或 `format!(" ...")`.
        for m in re.finditer(r'"([^"\n]{3,60})"', text):
            s = m.group(1)
            if any(ch in s for ch in "🎨⌨📊🖼") or s.startswith(" ") and s.endswith(" "):
                found.append(s)
        if found:
            titles[rs.stem] = found[0]
        else:
            titles[rs.stem] = ""
    return titles


# ── 解析:测试矩阵 ────────────────────────────────────────────────────────

def parse_test_matrix(path: Path) -> set[str]:
    """`tests/functional/test_slash.py` 的 `MATRIX` 里已覆盖的命令名。"""
    text = path.read_text(encoding="utf-8")
    m = re.search(r"MATRIX[^=]*=\s*\[(.*?)\n\]", text, re.S)
    if not m:
        raise SystemExit(f"解析失败:在 {path} 找不到 MATRIX 表")
    return set(re.findall(r'\(\s*"([^"]+)"\s*,', m.group(1)))


# ── 审计 ─────────────────────────────────────────────────────────────────

def audit() -> dict:
    enum_commands = parse_enum_commands(SLASH_RS)
    whitelist = parse_legacy_whitelist(HISTORY_RS)
    arms = parse_dispatch_arms(HISTORY_RS)
    popup_legacy = parse_popup_legacy(HISTORY_RS)
    picker_titles = parse_picker_titles(PICKER_DIR)
    tested = parse_test_matrix(TEST_SLASH)

    declared = set(enum_commands) | whitelist | arms
    covered = declared & tested
    uncovered = sorted(declared - tested)

    # 枚举里声明但实现缺失的命令(走「尚未接入分发」兜底)。
    enum_not_implemented = sorted(set(enum_commands) - arms)

    return {
        "enum_commands": enum_commands,
        "legacy_whitelist": sorted(whitelist),
        "legacy_arms": sorted(arms),
        "popup_legacy": sorted(popup_legacy),
        "picker_titles": picker_titles,
        "tested": sorted(tested),
        "covered": sorted(covered),
        "uncovered": uncovered,
        "enum_not_implemented": enum_not_implemented,
        "counts": {
            "declared": len(declared),
            "tested": len(tested),
            "covered": len(covered),
            "uncovered": len(uncovered),
            "enum": len(enum_commands),
            "legacy_arms": len(arms),
            "pickers": len(picker_titles),
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="ZhuYe-CLI 覆盖自审")
    ap.add_argument("--json", action="store_true", help="输出 JSON")
    ap.add_argument("--strict", action="store_true", help="有未覆盖命令即退出码 1")
    args = ap.parse_args()

    report = audit()
    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
    else:
        c = report["counts"]
        print("=" * 60)
        print("  ZhuYe-CLI 覆盖自审")
        print("=" * 60)
        print(f"  声明的命令总数        : {c['declared']}")
        print(f"    └ SlashCommand 枚举 : {c['enum']}")
        print(f"    └ legacy 实现臂     : {c['legacy_arms']}")
        print(f"  测试矩阵已覆盖        : {c['covered']}")
        print(f"  未覆盖                : {c['uncovered']}")
        print(f"  弹层(picker)模块数   : {c['pickers']}")
        if report["uncovered"]:
            print()
            print("  ⚠ 未被测试矩阵覆盖的命令:")
            for name in report["uncovered"]:
                print(f"      /{name}")
        print()
        print("  枚举声明但未实现(运行时走「尚未接入分发」兜底):")
        for name in report["enum_not_implemented"]:
            print(f"      /{name}")

    if args.strict and report["uncovered"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
