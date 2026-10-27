"""能力覆盖层的 pytest fixture(唯一职责:提供隔离环境与被测对象)。

共享设施在 `pty.py`(唯一命名模块)—— 不用 `from conftest import ...` 的写法:
若日后出现第二个 conftest.py,同名导入在一次性收集整个 tests/ 时会按
sys.path 顺序解析到错误的那一份。
"""

from __future__ import annotations

from pathlib import Path
from typing import Callable, Iterator

import pytest

from tui_pty import DRAIN_TIMEOUT, REFLECT_TUI_BIN, Sandbox, TuiSession, text_reply, tool_call


@pytest.fixture(scope="session", autouse=True)
def _require_binary() -> None:
    """被测二进制不存在时跳过整个模块(而不是失败)。

    缺依赖的批次自动 SKIP,不算失败 —— 与 submodule `tests/run_all.sh`
    的「缺依赖的批次自动 SKIP」语义一致。
    """
    if not REFLECT_TUI_BIN.is_file():
        pytest.skip(
            f"{REFLECT_TUI_BIN} 不存在;先执行 cargo build --release -p reflect-tui"
        )


@pytest.fixture()
def sandbox() -> Iterator[Sandbox]:
    """每个用例一份隔离 HOME + cwd(用例结束自动删除)。"""
    sb = Sandbox.create()
    yield sb
    sb.cleanup()


@pytest.fixture()
def mock_script(tmp_path: Path) -> Callable[[list[str]], Path]:
    """写 mock provider 脚本:JSONL,每行一次模型回复。

    行格式(见 reflect-llm/src/providers/mock.rs):
    `{"type":"text","text":"..."}` / `{"type":"tool_call","name":"t","args":{}}`
    """

    def _write(lines: list[str]) -> Path:
        path = tmp_path / "mock-script.jsonl"
        path.write_text("\n".join(lines) + "\n")
        return path

    return _write


@pytest.fixture()
def tui(sandbox: Sandbox) -> Iterator[Callable[..., TuiSession]]:
    """起 TUI 会话的工厂(自动收尾)。

    默认注入 mock provider 环境(`ZHUYE_MODEL=mock/mock-1`),零网络、
    免 API key;需要真实 provider 的用例显式传 `env=` 覆盖。
    """
    created: list[TuiSession] = []

    def _make(
        args: list[str] | None = None,
        env: dict[str, str] | None = None,
        cols: int = 110,
        rows: int = 34,
        script: Path | None = None,
        timeout: float = DRAIN_TIMEOUT,
    ) -> TuiSession:
        child_env = sandbox.mock_env(script)
        if env:
            child_env.update(env)
        s = TuiSession(sandbox, args=args, env=child_env, cols=cols, rows=rows)
        created.append(s)
        return s

    yield _make
    for s in created:
        try:
            s.close()
        except Exception:
            pass


__all__ = [
    "Sandbox",
    "TuiSession",
    "text_reply",
    "tool_call",
]
