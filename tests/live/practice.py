#!/usr/bin/env python3
"""免费模型横向对比演练(终端 TUI 层)。

跑一组**真实场景** × **从 provider 接口实时取到的免费模型**,输出 Markdown
对比表(终态 / 耗时 / token / 工具调用数 / 实际服务模型)。

**不是门禁**:退出码只反映脚本自身错误;模型表现差异只体现在报告里。

## 与 submodule 的区别

submodule `reflect-agent/scripts/free_model_practice.sh` 驱动的是 headless
`reflect exec`(事件流 JSONL);本脚本驱动的是**终端 TUI**(pty + pyte),
因此测的是「真实模型 + 真实 TUI 渲染 + 真实工具审批」这一整条链路。

## 用法

```bash
python3 tests/live/practice.py                     # 默认车道 + 前 3 个模型
python3 tests/live/practice.py --count 2           # 从清单取 2 个
python3 tests/live/practice.py --models a,b,c      # 指定模型
python3 tests/live/practice.py --out report.md     # 报告落盘
python3 tests/live/practice.py --dry-run           # 只列计划,不真跑
```

⚠️ 上游可能记录 prompt:脚本只用**合成数据**,不要把真实仓库内容 /
凭据喂给免费池。
"""

from __future__ import annotations

import argparse
import os
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
FUNCTIONAL_DIR = HERE.parent / "functional"
for _p in (str(HERE), str(FUNCTIONAL_DIR)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from live_lane import (  # noqa: E402
    DRAIN_TIMEOUT,
    assistant_message_count,
    current_lane,
    fetch_free_models,
    is_busy,
    probe_tool_support,
    rate_limited,
    session_stats,
    write_lane_config,
)
from tui_pty import Sandbox, TuiSession  # noqa: E402

# 场景集(合成数据,不含任何真实仓库内容)。
# 每项:名称 + 提示词序列 + 是否需要批准工具。
SCENARIOS: list[tuple[str, list[str], bool]] = [
    (
        "单轮问答",
        ["请用一句话回答:1+1 等于几? 不要调用任何工具。"],
        False,
    ),
    (
        "工具调用",
        ["请用 bash 工具执行 echo practice-probe,然后把输出原样告诉我。"],
        True,
    ),
    (
        "多轮上下文",
        [
            "记住这个数字 4242,只回复「记住了」四个字。",
            "我刚才让你记的数字是多少?只回复数字本身。",
        ],
        False,
    ),
]


@dataclass
class ScenarioResult:
    """一个 (模型, 场景) 的跑批结果。"""

    model: str
    scenario: str
    status: str = "-"
    elapsed_ms: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    total_tokens: int = 0
    tool_calls: int = 0
    served_by: str = "-"
    note: str = ""


@dataclass
class Report:
    """整场演练的结果集合。"""

    rows: list[ScenarioResult] = field(default_factory=list)


def _count_tool_calls(screen: str) -> int:
    """数屏幕上的工具成功行(`✓ <name> (<N>ms)`)。"""
    count = 0
    for line in screen.splitlines():
        stripped = line.strip()
        if stripped.startswith("✓ ") and "(" in stripped and "ms)" in stripped:
            count += 1
    return count


def _run_scenario(
    model: str,
    scenario: str,
    prompts: list[str],
    approve: bool,
    fallbacks: list[str],
) -> ScenarioResult:
    """跑一个场景:起 TUI → 逐轮提交 → 收集结果。"""
    result = ScenarioResult(model=model, scenario=scenario)
    sb = Sandbox.create(prefix="reflect-tui-practice-")
    lane = current_lane()
    write_lane_config(sb.home, model=model, lane=lane, fallbacks=fallbacks)
    child_env = {var: "" for var in ("OPENAI_API_KEY", "ANTHROPIC_API_KEY", "ZHUYE_MODEL", "ZHUYE_PROVIDER")}
    child_env["REFLECT_TUI_UPDATE_CHECK"] = "0"
    # 隔离 HOME 显式写死(`Sandbox.env()` 先设 HOME 再叠加 extra,
    # extra 带宿主 HOME 会把隔离目录覆盖掉)。
    child_env["HOME"] = str(sb.home)

    started = time.monotonic()
    try:
        with TuiSession(sb, env=child_env, cols=120, rows=38) as s:
            s.wait_stable(max_wait=15.0)
            for prompt in prompts:
                baseline = assistant_message_count(sb.home)
                s.type_text(prompt)
                s.key("enter")
                # 等回合收尾;期间顺手批掉工具审批。
                deadline = time.monotonic() + DRAIN_TIMEOUT
                done = False
                limited = False
                while time.monotonic() < deadline:
                    txt = s.normalized()
                    if rate_limited(txt):
                        limited = True
                        break
                    if approve and "APPROVAL NEEDED" in txt:
                        s.type_text("y")
                        time.sleep(1.0)
                        continue
                    if not is_busy(s.screen_text()) and assistant_message_count(sb.home) > baseline:
                        done = True
                        break
                    s._pump(settle=0.6, max_wait=1.2)
                if limited:
                    result.status = "rate_limited"
                    result.note = "上游 429 / 凭据耗尽"
                    break
                if not done:
                    result.status = "timeout"
                    break
            else:
                result.status = "ok"
            result.tool_calls = _count_tool_calls(s.screen_text())
        # 会话统计必须在清理隔离 HOME **之前**读 —— 落盘数据在
        # `~/.zhuye/sessions/**.jsonl`,cleanup 会把整个目录删掉。
        stats = session_stats(sb.session_files())
        result.input_tokens = stats["usage"]["input_tokens"]
        result.output_tokens = stats["usage"]["output_tokens"]
        result.total_tokens = stats["usage"]["total_tokens"]
        result.served_by = stats["models"][0] if stats["models"] else "-"
    except Exception as exc:  # noqa: BLE001 - 单场景失败不影响整场演练
        result.status = "error"
        result.note = str(exc)[:80]
    finally:
        sb.cleanup()

    result.elapsed_ms = int((time.monotonic() - started) * 1000)
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="免费模型横向对比演练(TUI 层)")
    parser.add_argument(
        "--models",
        default=os.environ.get("REFLECT_LIVE_MODELS", ""),
        help="逗号分隔的 model id(默认从清单取)",
    )
    parser.add_argument(
        "--count",
        type=int,
        default=int(os.environ.get("REFLECT_LIVE_MODEL_COUNT", "3")),
        help="从清单取几个模型",
    )
    parser.add_argument(
        "--out",
        default=os.environ.get("PRACTICE_OUT", ""),
        help="报告落盘路径",
    )
    parser.add_argument("--dry-run", action="store_true", help="只列计划,不真跑")
    args = parser.parse_args(argv)

    lane = current_lane()
    print("═══ 免费模型横向对比演练(TUI)═══")
    print(f"车道:     {lane.base_url}")
    print("模型来源: provider 清单接口(实时拉取)")
    print()

    if args.models:
        candidates = [m.strip() for m in args.models.split(",") if m.strip()]
    else:
        candidates = fetch_free_models(lane)
        if not candidates:
            print("FAIL: 拿不到免费模型清单(检查端点 / 出口地区 / 配额)", file=sys.stderr)
            return 3
        candidates = sorted(candidates)[: args.count]

    print(f"模型:     {', '.join(candidates)}")
    print()

    if args.dry_run:
        for model in candidates:
            for name, prompts, _ in SCENARIOS:
                print(f"  · {model} × {name}({len(prompts)} 轮)")
        print("(dry-run:不发起推理请求)")
        return 0

    report = Report()
    for model in candidates:
        ok, reason = probe_tool_support(lane, model)
        if not ok:
            print(f"── 模型: {model} —— 跳过(不支持 tools:{reason})")
            for name, _prompts, _approve in SCENARIOS:
                report.rows.append(
                    ScenarioResult(model=model, scenario=name, status="skipped", note=reason[:60])
                )
            continue
        print(f"── 模型: {model}")
        fallbacks = [m for m in candidates if m != model][:2]
        for name, prompts, approve in SCENARIOS:
            res = _run_scenario(model, name, prompts, approve, fallbacks)
            report.rows.append(res)
            print(f"   · {name}: {res.status} ({res.elapsed_ms}ms, {res.total_tokens} tok)")

    # ── 报告 ────────────────────────────────────────────────────────────
    lines = [
        "| 模型 | 场景 | 终态 | 耗时(ms) | 输入 tok | 输出 tok | 工具调用 | 实际模型 | 备注 |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for r in report.rows:
        lines.append(
            f"| {r.model} | {r.scenario} | {r.status} | {r.elapsed_ms} | "
            f"{r.input_tokens} | {r.output_tokens} | {r.tool_calls} | "
            f"{r.served_by} | {r.note or '-'} |"
        )
    table = "\n".join(lines)

    print()
    print(table)

    if args.out:
        Path(args.out).write_text(
            f"# 免费模型横向对比演练(TUI 层)\n\n"
            f"- 车道:`{lane.base_url}`\n"
            f"- 生成时间:{time.strftime('%Y-%m-%d %H:%M:%S')}\n\n"
            f"{table}\n"
        )
        print()
        print(f"报告已写入: {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
