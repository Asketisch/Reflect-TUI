"""能力覆盖层设施自身的单元测试(不依赖真二进制)。

这些用例守的是**测试基础设施的正确性**:隔离环境真的隔离、按键映射
真的覆盖、mock 脚本真的按行消费。基础设施坏了,上面所有能力断言都会
变成「假绿」。

替代旧 `test/test_unit_utils.py`(那一份测的是已被本套件取代的
`test/harness.py`)。
"""

from __future__ import annotations

import json
import os
from pathlib import Path

from tui_pty import KEY_SEQS, Sandbox, normalize, text_reply, tool_call


def test_normalize_collapses_whitespace() -> None:
    """U1 `normalize` 把折行/多空白折叠成单空格。"""
    assert normalize("a\n  b\tc") == "a b c"
    assert normalize("  多个   空格  ") == "多个 空格"
    assert normalize("") == ""


def test_sandbox_isolates_home_and_cwd(tmp_path: Path) -> None:
    """U2 Sandbox 的 HOME / cwd 互不干扰,且预写空 config。"""
    a = Sandbox.create(prefix="u2-a-")
    b = Sandbox.create(prefix="u2-b-")
    try:
        assert a.home != b.home, "两个沙箱的 HOME 必须不同"
        assert a.cwd != b.cwd, "两个沙箱的 cwd 必须不同"
        cfg = a.home / ".reflect" / "config.toml"
        assert cfg.is_file(), "应预写 config.toml"
        assert cfg.read_text().strip().startswith("#"), "config 应是空配置(注释)"
    finally:
        a.cleanup()
        b.cleanup()


def test_sandbox_env_points_home_at_isolation(tmp_path: Path) -> None:
    """U3 Sandbox.env() 把 HOME 指向隔离目录,其余继承。"""
    sb = Sandbox.create(prefix="u3-")
    try:
        env = sb.env({"REFLECT_MODEL": "mock/mock-1"})
        assert env["HOME"] == str(sb.home), "HOME 应指向隔离目录"
        assert env["REFLECT_MODEL"] == "mock/mock-1"
        assert env["TERM"] not in (None, "", "dumb"), "TERM 应是可渲染的"
        # 宿主 HOME 不应泄漏进子进程 env。
        assert env["HOME"] != os.environ.get("HOME")
    finally:
        sb.cleanup()


def test_sandbox_cleanup_removes_everything(tmp_path: Path) -> None:
    """U4 cleanup 后隔离目录整体消失(不留残留)。"""
    sb = Sandbox.create(prefix="u4-")
    root = sb.home.parent
    assert root.exists()
    sb.cleanup()
    assert not root.exists(), f"隔离目录应被删除:{root}"


def test_sandbox_session_file_globbing(tmp_path: Path) -> None:
    """U5 session_files() 按 `YYYY/MM/DD/<id>.jsonl` 层级找会话文件。"""
    sb = Sandbox.create(prefix="u5-")
    try:
        assert sb.session_files() == []
        target = sb.home / ".reflect" / "sessions" / "2026" / "01" / "01" / "abc.jsonl"
        target.parent.mkdir(parents=True)
        target.write_text("{}\n")
        found = sb.session_files()
        assert len(found) == 1, f"应找到 1 个会话文件,实际 {found}"
        assert found[0].name == "abc.jsonl"
    finally:
        sb.cleanup()


def test_key_sequences_cover_tui_shortcuts() -> None:
    """U6 按键表覆盖 TUI 用到的全部快捷键。"""
    for name in (
        "enter", "esc", "tab", "backspace", "up", "down", "left", "right",
        "home", "end", "pageup", "pagedown", "delete",
        "ctrl+c", "ctrl+d", "ctrl+t", "ctrl+o", "ctrl+g", "ctrl+l",
        "ctrl+shift+d", "alt+enter",
    ):
        assert name in KEY_SEQS, f"按键表缺 {name}"
        assert isinstance(KEY_SEQS[name], bytes) and KEY_SEQS[name], (
            f"按键 {name} 应映射到非空字节序列"
        )


def test_mock_script_helpers_produce_valid_lines() -> None:
    """U7 mock 脚本助手产出引擎能解析的 JSONL 行。"""
    line = text_reply("你好")
    obj = json.loads(line)
    assert obj == {"type": "text", "text": "你好"}

    line = tool_call("echo", {"text": "hi"})
    obj = json.loads(line)
    assert obj == {"type": "tool_call", "name": "echo", "args": {"text": "hi"}}


def test_tool_call_without_args_is_valid() -> None:
    """U8 无参 tool_call 也产出合法 JSON。"""
    obj = json.loads(tool_call("read"))
    assert obj["type"] == "tool_call"
    assert obj["args"] == {}
