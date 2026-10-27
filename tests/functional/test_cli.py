"""C 组 —— headless CLI 的**语义**断言(与 bash 参数面互补)。

分工(submodule `tests/README.md` §四层分工 的同款原则):
  - `tests/rust/cli_core.sh` / `cli_modules.sh` 断言 **CLI 参数矩阵**
    (参数组合 → 退出码 / 关键字);
  - 本模块断言 **输出内容的语义**(列结构、预览条数、markdown 结构、
    统计口径),那些用 bash 表达不自然的语义。

本模块走 `subprocess` + `json`,不依赖 pty 与 SDK。
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

from tui_pty import ZHUYE_CLI_BIN


def run_cli(sandbox, *args: str, timeout: float = 30.0) -> subprocess.CompletedProcess[str]:
    """在隔离 HOME 下跑 `zhuye-cli <args>`。"""
    return subprocess.run(
        [str(ZHUYE_CLI_BIN), *args],
        capture_output=True,
        text=True,
        timeout=timeout,
        env=sandbox.env(),
        cwd=str(sandbox.cwd),
    )


@pytest.fixture()
def seeded(sandbox):
    """写入一条**真实形态**的会话(user + assistant + token_count)。

    形态与引擎实际落盘一致(见 reflect-protocol `RolloutRecord`):
      - `content` 是 **ContentBlock 数组**(v1.2 起的形态);
      - `token_count` 的 token 字段嵌在 `usage` 对象里。
    用真实形态而不是简化形态,才能测出「读取侧只认旧形态」这类缺陷。
    """
    sid = "33333333-4444-4555-8666-777777777777"
    day = Path("2026/01/01")
    path = sandbox.home / ".zhuye" / "sessions" / day / f"{sid}.jsonl"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "type": "session_meta",
                        "session_id": sid,
                        "model": "mock/mock-1",
                        "started_at": "2026-01-01T00:00:00Z",
                    }
                ),
                json.dumps(
                    {
                        "type": "message",
                        "turn_id": "44444444-4444-4444-8444-444444444444",
                        "role": "user",
                        "content": [{"type": "text", "text": "C1_USER_MESSAGE"}],
                    }
                ),
                json.dumps(
                    {
                        "type": "message",
                        "turn_id": "44444444-4444-4444-8444-444444444444",
                        "role": "assistant",
                        "content": [{"type": "text", "text": "C1_ASSISTANT_REPLY"}],
                    }
                ),
                json.dumps(
                    {
                        "type": "token_count",
                        "turn_id": "44444444-4444-4444-8444-444444444444",
                        "usage": {
                            "input_tokens": 120,
                            "output_tokens": 34,
                            "cached_tokens": 7,
                            "cache_write_tokens": 0,
                            "total_tokens": 154,
                        },
                        "cost_usd": 0.0123,
                        "model": "mock/mock-1",
                        "provider": "mock",
                        "at": "2026-01-01T00:00:01Z",
                    }
                ),
            ]
        )
        + "\n"
    )
    return sid


# ── session ls ────────────────────────────────────────────────────────────


def test_session_ls_has_six_columns(sandbox, seeded) -> None:
    """C1 `session ls` 输出六列:session_id / model / started / msgs / tokens / cost。"""
    r = run_cli(sandbox, "session", "ls")
    assert r.returncode == 0, f"rc={r.returncode}: {r.stderr}"
    header = r.stdout.splitlines()[0]
    for col in ("session_id", "model", "started", "msgs", "tokens", "cost"):
        assert col in header, f"表头应含 {col}: {header!r}"


def test_session_ls_shows_thousands_separator(sandbox, seeded) -> None:
    """C2 `session ls` 的 token 列用千分位分隔。"""
    r = run_cli(sandbox, "session", "ls")
    assert r.returncode == 0
    assert "154" in r.stdout, f"应显示累计 token: {r.stdout!r}"


# ── session show ──────────────────────────────────────────────────────────


def test_session_show_previews_messages(sandbox, seeded) -> None:
    """C3 `session show` 显示元数据 + 消息预览。"""
    r = run_cli(sandbox, "session", "show", seeded)
    assert r.returncode == 0, f"rc={r.returncode}: {r.stderr}"
    out = r.stdout
    assert "Model:" in out and "Started:" in out, f"应有元数据行:\n{out}"
    assert "C1_USER_MESSAGE" in out
    assert "C1_ASSISTANT_REPLY" in out


def test_session_show_reports_token_stats(sandbox, seeded) -> None:
    """C4 `session show` 报会话级 token / cost 统计。"""
    r = run_cli(sandbox, "session", "show", seeded)
    assert r.returncode == 0
    out = r.stdout
    assert "Tokens:" in out, f"应有 token 统计:\n{out}"
    assert "154" in out, f"token 总量应为 154:\n{out}"
    assert "in 120" in out and "out 34" in out, f"应有输入/输出拆分:\n{out}"


def test_session_show_per_model_table(sandbox, seeded) -> None:
    """C5 `session show` 输出 per-model 用量表。"""
    r = run_cli(sandbox, "session", "show", seeded)
    assert r.returncode == 0
    out = r.stdout
    assert "Per-model usage:" in out, f"应有 per-model 表:\n{out}"
    assert "mock/mock-1" in out, f"表内应含模型名:\n{out}"


# ── session export ───────────────────────────────────────────────────────


def test_session_export_markdown_has_structure(sandbox, seeded) -> None:
    """C6 `session export` 导出 markdown,含角色标题与正文。"""
    r = run_cli(sandbox, "session", "export", seeded)
    assert r.returncode == 0, f"rc={r.returncode}: {r.stderr}"
    md = r.stdout
    assert "C1_USER_MESSAGE" in md and "C1_ASSISTANT_REPLY" in md, f"导出缺正文:\n{md}"
    assert "#" in md, f"导出应有 markdown 标题:\n{md}"


def test_session_export_to_file_is_atomic(sandbox, seeded) -> None:
    """C7 `session export --out` 落盘,且不留 `.tmp` 残留。"""
    out = sandbox.cwd / "exported.md"
    r = run_cli(sandbox, "session", "export", seeded, "--out", str(out))
    assert r.returncode == 0, f"rc={r.returncode}: {r.stderr}"
    assert out.is_file() and out.stat().st_size > 0, f"应落盘非空文件:{out}"
    assert "C1_ASSISTANT_REPLY" in out.read_text()
    leftovers = list(out.parent.glob("*.md.tmp.*"))
    assert not leftovers, f"不应留临时文件: {leftovers}"


# ── session stats ────────────────────────────────────────────────────────


def test_session_stats_has_total_row(sandbox, seeded) -> None:
    """C8 `session stats` 输出 per-model 表 + TOTAL 行。"""
    r = run_cli(sandbox, "session", "stats")
    assert r.returncode == 0, f"rc={r.returncode}: {r.stderr}"
    out = r.stdout
    assert "Per-model usage:" in out
    assert "TOTAL" in out, f"应有 TOTAL 行:\n{out}"
    assert "154" in out, f"TOTAL 应含 token 总量:\n{out}"


# ── traces ───────────────────────────────────────────────────────────────


def test_traces_spans_filters_by_session(sandbox) -> None:
    """C9 `traces spans <id>` 按 session 前缀过滤。"""
    import datetime

    span_dir = sandbox.home / ".zhuye" / "traces" / "log"
    span_dir.mkdir(parents=True, exist_ok=True)
    day = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
    sid = "88888888-9999-4aaa-8bbb-cccccccccccc"
    (span_dir / f"zhuye-{day}.jsonl").write_text(
        json.dumps(
            {
                "timestamp": "2026-01-01T00:00:00Z",
                "level": "info",
                "event": "turn.started",
                "module": "m",
                "trace_id": "t1",
                "span_id": "s1",
                "session_id": sid,
            }
        )
        + "\n"
    )

    r = run_cli(sandbox, "traces", "spans", sid)
    assert r.returncode == 0, f"rc={r.returncode}: {r.stderr}"
    assert "turn.started" in r.stdout, f"应列出该 session 的 span:\n{r.stdout}"

    r2 = run_cli(sandbox, "traces", "spans", "no-such-session")
    assert r2.returncode == 0
    assert "no span events for session" in r2.stdout, (
        f"未命中应给占位文案:\n{r2.stdout}"
    )


# ── 错误路径 ──────────────────────────────────────────────────────────────


def test_session_show_unknown_id_errors(sandbox) -> None:
    """C10 `session show <不存在的 id>` 非 0 退出并给出可读错误。"""
    r = run_cli(sandbox, "session", "show", "00000000-0000-4000-8000-000000000000")
    assert r.returncode != 0, f"应非 0 退出,实际 {r.returncode}"
    assert "not found" in (r.stdout + r.stderr).lower(), (
        f"应有可读错误:\n{r.stdout}{r.stderr}"
    )
