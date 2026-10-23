"""真实 HTTP 链路的能力覆盖层:Anthropic Messages API 的 SSE 协议 + 确定性 busy 窗口。

既有能力覆盖层用 `REFLECT_MODEL=mock/mock-1` —— 那是 reflect-llm
`ModelClient` 层的内置 provider,**完全绕过** HTTP 传输、SSE 分帧与
Anthropic 协议形态;内置 mock 还「瞬时完成」,离线无法确定性制造 busy
窗口(见 `tests/COVERAGE.md` §6 的相关记录)。

本模块起一个本地 `mock_anthropic_server.py`,让 TUI 真的走一遍:
- 请求构造(URL / header / body);
- SSE 事件分帧:`content_block_start` / `content_block_delta` /
  `content_block_stop` / `message_stop`;
- `tool_use` 块与 `content_block_delta` 聚合。

另外用 `MOCK_ANTHROPIC_DELAY` 人为拉长模型响应,制造**确定性 busy 窗口**,
让「回合中断」这类需要时序的用例可复现。
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

import pytest

from tui_pty import Sandbox, TuiSession

HERE = Path(__file__).resolve().parent
MOCK_SERVER = HERE / "mock_anthropic_server.py"

# busy 用例需要的模型响应延迟(秒)。留出足够窗口让用例按下 Esc,
# 同时不至于让整条测试链路拖太久。
BUSY_DELAY_SECS = 6.0


def _free_port() -> int:
    """取一个空闲端口(避免与并行跑的其他实例撞端口)。"""
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _start_server(delay_secs: float) -> tuple[subprocess.Popen, str]:
    """起一个 mock Anthropic server,返回 (进程, base_url)。"""
    port = _free_port()
    env = dict(os.environ)
    env["MOCK_ANTHROPIC_PORT"] = str(port)
    env["MOCK_ANTHROPIC_DELAY"] = str(delay_secs)
    proc = subprocess.Popen(
        [sys.executable, str(MOCK_SERVER)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    base_url = f"http://127.0.0.1:{port}"
    deadline = time.time() + 15.0
    while time.time() < deadline:
        if proc.poll() is not None:
            raise RuntimeError("mock Anthropic server 启动即退出")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return proc, base_url
        except OSError:
            time.sleep(0.1)
    proc.terminate()
    raise RuntimeError("mock Anthropic server 启动超时")


def _stop_server(proc: subprocess.Popen) -> None:
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()


@pytest.fixture(scope="session")
def http_mock() -> str:
    """无延迟的 mock Anthropic server(会话级,全程复用)。"""
    proc, base_url = _start_server(0.0)
    try:
        yield base_url
    finally:
        _stop_server(proc)


@pytest.fixture(scope="session")
def http_mock_slow() -> str:
    """带延迟的 mock Anthropic server:制造确定性 busy 窗口。"""
    proc, base_url = _start_server(BUSY_DELAY_SECS)
    try:
        yield base_url
    finally:
        _stop_server(proc)


def _write_http_config(sandbox: Sandbox, base_url: str) -> None:
    """把 config.toml 指向本地 mock server(走真实 Anthropic provider)。"""
    (sandbox.home / ".reflect" / "config.toml").write_text(
        "[active]\n"
        'provider = "anthropic"\n'
        "\n"
        "[anthropic]\n"
        'api_key = "sk-ant-mock"\n'
        f'base_url = "{base_url}"\n'
        'model = "mock-claude"\n'
    )


@pytest.fixture()
def http_tui(sandbox: Sandbox, http_mock: str, monkeypatch: pytest.MonkeyPatch):
    """起一个走真实 HTTP(mock server)的 TUI 会话工厂。"""
    # 宿主机若设了 REFLECT_MODEL=mock/... 会被子进程继承,把真实 provider
    # 顶掉 —— 去掉它,让 config.toml 的 anthropic 段生效。
    monkeypatch.delenv("REFLECT_MODEL", raising=False)
    _write_http_config(sandbox, http_mock)
    created: list[TuiSession] = []

    def _make(
        args: list[str] | None = None,
        env: dict[str, str] | None = None,
        cols: int = 110,
        rows: int = 34,
    ) -> TuiSession:
        child_env = dict(env or {})
        s = TuiSession(sandbox, args=args, env=child_env, cols=cols, rows=rows)
        created.append(s)
        return s

    yield _make
    for s in created:
        try:
            s.close()
        except Exception:
            pass


def _session_id_of(path: Path) -> str:
    """从会话 JSONL 首行读内部 session_id(双 ID 设计下它可能 ≠ 文件名)。"""
    first = path.read_text(encoding="utf-8").splitlines()[0]
    record = json.loads(first)
    if record.get("type") == "session_meta":
        return str(record["session_id"])
    return path.stem


def test_http_anthropic_sse_roundtrip(http_tui) -> None:
    """H1 真实 HTTP + SSE:一次对话往返,回复经 `content_block_delta` 聚合上屏。

    内置 mock provider 在 `ModelClient` 层就返回了内容,这一层完全不碰
    HTTP 与 Anthropic 协议;本用例让请求真的打到本地 mock server 上,
    走完 URL / header / body / SSE 分帧 / delta 聚合整条链路。
    """
    with http_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("HTTP_PROBE")
        s.key("enter")
        txt = s.wait_text("MOCK REPLY", timeout=40)
        assert "MOCK REPLY" in txt, f"应收到 mock server 的回复:\n{txt[-400:]}"
        assert "HTTP_PROBE" in txt, f"用户消息应上屏:\n{txt[-400:]}"
        assert not s.has_panic()


def test_resume_across_date_buckets_over_http(http_tui, sandbox) -> None:
    """H2 真实 HTTP 产生的会话:搬到旧日期桶后 `--resume` 仍能回放历史。

    会话文件是 TUI 自己经真实 HTTP 回合写出来的(不是手搓的 fixture),
    把它移到 `2026/01/01` 桶模拟「3 天以上前创建的会话」——这正是旧实现
    `reader::replay` 只扫近 3 天日期桶时会静默丢历史的场景。
    """
    with http_tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("OLD_SESSION_PROBE")
        s.key("enter")
        s.wait_text("MOCK REPLY", timeout=40)

    # 会话文件应已落盘。
    files = sandbox.session_files()
    assert files, "回合结束后应有会话 JSONL 落盘"
    src = files[0]
    sid = _session_id_of(src)

    # 搬到旧日期桶。
    old_dir = sandbox.home / ".reflect" / "sessions" / "2026" / "01" / "01"
    old_dir.mkdir(parents=True, exist_ok=True)
    dst = old_dir / src.name
    src.rename(dst)

    with http_tui(args=["--resume", sid]) as s:
        s.wait_boot(timeout=30)
        txt = s.wait_text("已恢复历史会话", timeout=20)
        assert "回放" in txt, f"跨天会话应回放既往消息:\n{txt[-400:]}"
        assert "回放 0 条消息" not in txt, f"不应回放 0 条:\n{txt[-400:]}"
        assert not s.has_panic()


def test_esc_interrupt_during_deterministic_busy_window(http_tui, sandbox) -> None:
    """H3 确定性 busy 窗口下按 Esc 中断回合,且连按只发一次中断请求。

    内置 mock provider 瞬时完成,离线造不出 busy 窗口(见 COVERAGE §6),
    所以这里用带延迟的 mock server 把模型响应拉长到 `BUSY_DELAY_SECS`。

    断言:
    - 按 Esc 后出现「已发送中断请求」提示;
    - 连按 3 次 Esc,提示**只出现一次**(去抖生效,见 tui/mod.rs 的
      `interrupt_requested` 标记)。
    """
    proc, base_url = _start_server(BUSY_DELAY_SECS)
    try:
        _write_http_config(sandbox, base_url)
        with http_tui() as s:
            s.wait_boot(timeout=30)
            s.type_text("BUSY_PROBE")
            s.key("enter")
            # 等模型响应的延迟窗口打开(请求已在途,回合处于 busy)。
            time.sleep(1.5)
            s.key("escape")
            txt = s.wait_text("已发送中断请求", timeout=15)
            assert "已发送中断请求" in txt, f"应提示已发送中断:\n{txt[-400:]}"

            # 连按两次:去抖生效则不再产生新提示。
            s.key("escape")
            s.key("escape")
            time.sleep(0.8)
            after = s.normalized()
            assert after.count("已发送中断请求") == 1, (
                f"连按 Esc 只应发一次中断请求,实际 {after.count('已发送中断请求')} 次:\n{after[-400:]}"
            )
            assert not s.has_panic()
    finally:
        _stop_server(proc)
