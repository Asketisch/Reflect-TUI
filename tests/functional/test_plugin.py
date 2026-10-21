"""G 组 —— 插件面:启动通知、slash 命令注入、`/plugin` overlay。

插件能力由 submodule 的 `reflect-plugin` 提供,本仓 TUI 侧只消费三处:
1. **启动通知**:bootstrap 挂载成功后 `PluginLoaded` 事件经 lifecycle 通道
   进通知流(scrollback 出现 `Plugin 'demo@inline' loaded …`);
2. **slash 命令注入**:插件命令快照注入 composer,输入 `/demo` 弹窗列出
   `demo:hello`;提交 `/demo:hello` 不被「未知命令」校验拦截;
3. **`/plugin` overlay**:展示真实已装插件(id / version / enabled 标志)。

**前置**:submodule 构建出的 `reflect` CLI(用于安装示例插件)。
缺失则整组 SKIP(不算失败)—— 插件 e2e 是增强项,不阻塞基础回归。

离线(`REFLECT_MODEL=mock/mock-1`,零网络)。
"""

from __future__ import annotations

import os
import subprocess
import time
from pathlib import Path

import pytest

from tui_pty import ROOT

# submodule 构建出的 reflect-cli(bin 名是 `reflect`)。
_CLI_CANDIDATES = [
    ROOT / "reflect-agent" / "target" / "release" / "reflect",
    ROOT / "reflect-agent" / "target" / "debug" / "reflect",
]
_EXAMPLE_PLUGIN = ROOT / "reflect-agent" / "examples" / "plugin-demo"


def _find_cli() -> Path | None:
    for candidate in _CLI_CANDIDATES:
        if candidate.exists() and os.access(candidate, os.X_OK):
            return candidate
    return None


@pytest.fixture(scope="module")
def plugin_home(tmp_path_factory) -> str:
    """临时 HOME + 用真实 CLI 安装 / 启用示例插件(写真实表与 config)。"""
    cli = _find_cli()
    if cli is None:
        pytest.skip(
            "submodule 的 reflect CLI 未构建(需 cargo build -p reflect-cli);插件组跳过"
        )
    if not _EXAMPLE_PLUGIN.is_dir():
        pytest.skip(f"示例插件不存在:{_EXAMPLE_PLUGIN}")

    home = tmp_path_factory.mktemp("plugin-home")
    env = {**os.environ, "HOME": str(home)}
    for args in (
        ["plugin", "install", str(_EXAMPLE_PLUGIN)],
        ["plugin", "enable", "demo@inline"],
    ):
        r = subprocess.run(
            [str(cli), *args],
            env=env,
            capture_output=True,
            text=True,
            timeout=120,
        )
        if r.returncode != 0:
            pytest.skip(f"示例插件安装失败({args}): {r.stdout}{r.stderr}")
    return str(home)


def _plugin_env(home: str) -> dict[str, str]:
    return {
        "HOME": home,
        "REFLECT_MODEL": "mock/mock-1",
        "REFLECT_TUI_UPDATE_CHECK": "0",
    }


def test_plugin_loaded_notice(plugin_home, sandbox) -> None:
    """G1 启动通知:scrollback 出现 `Plugin 'demo@inline' loaded`。"""
    from tui_pty import TuiSession

    env = _plugin_env(plugin_home)
    # 让 sandbox 的 cwd 也用 plugin_home(插件在 HOME 下)。
    with TuiSession(sandbox, env=env, cols=120, rows=40) as s:
        # 插件挂载(含 MCP server 连接)会推迟首帧渲染,必须按内容轮询。
        deadline = time.monotonic() + 40
        txt = ""
        while time.monotonic() < deadline:
            txt = s.normalized()
            if "Plugin 'demo@inline' loaded" in txt:
                break
            time.sleep(0.4)
        assert "Plugin 'demo@inline' loaded" in txt, (
            f"应出现插件加载通知:\n{txt[-600:]}"
        )


def test_plugin_slash_command_is_listed(plugin_home, sandbox) -> None:
    """G2 slash 弹窗列出插件命令 `demo:hello`。"""
    from tui_pty import TuiSession

    env = _plugin_env(plugin_home)
    with TuiSession(sandbox, env=env, cols=120, rows=40) as s:
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            if "for commands" in s.normalized():
                break
            time.sleep(0.4)
        # 分两段输入,避免整段写入走 paste-burst 而不弹命令窗。
        s.type_text("/")
        s._pump(settle=0.8, max_wait=1.2)
        s.type_text("demo")
        s._pump(settle=1.0, max_wait=1.5)
        txt = s.normalized()
        assert "demo:hello" in txt, f"命令弹窗应列出插件命令:\n{txt[-800:]}"


def test_plugin_slash_command_passes_validation(plugin_home, sandbox) -> None:
    """G3 插件命令提交不被「未知命令」校验拦下。"""
    from tui_pty import TuiSession

    env = _plugin_env(plugin_home)
    with TuiSession(sandbox, env=env, cols=120, rows=40) as s:
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            if "for commands" in s.normalized():
                break
            time.sleep(0.4)
        s.type_text("/demo:hello")
        s.key("enter")
        s._pump(settle=1.5, max_wait=3.0)
        txt = s.normalized()
        assert "Unrecognized" not in txt, (
            f"插件命令不应被未知命令校验拦截:\n{txt[-600:]}"
        )
        assert "/demo:hello" in txt, f"插件命令应上屏:\n{txt[-600:]}"


def test_plugin_overlay_lists_installed_plugin(plugin_home, sandbox) -> None:
    """G4 `/plugin` overlay 展示真实已装插件(id / version / enabled)。"""
    from tui_pty import TuiSession

    env = _plugin_env(plugin_home)
    with TuiSession(sandbox, env=env, cols=120, rows=40) as s:
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            if "for commands" in s.normalized():
                break
            time.sleep(0.4)
        s.type_text("/plugin")
        s.key("enter")
        deadline = time.monotonic() + 20
        txt = ""
        while time.monotonic() < deadline:
            txt = s.normalized()
            if "Installed Plugins" in txt:
                break
            time.sleep(0.4)
        for token in ("Installed Plugins", "demo@inline", "v0.1.0", "enabled"):
            assert token in txt, f"overlay 应含 {token}:\n{txt[-800:]}"
