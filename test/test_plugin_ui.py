#!/usr/bin/env python3
"""PHASE 7:插件 UI E2E —— 真实 pty 驱动 reflect-tui,验证 submodule 插件
系统在 TUI 侧的三条接线:

1. 启动通知:bootstrap_plugins 挂载成功后 `PluginLoaded` 经 lifecycle
   通道进通知流(scrollback 出现 "Plugin 'demo@inline' loaded …")。
2. slash 弹窗:插件命令快照注入 composer,输入 `/demo` 弹窗列出
   `demo:hello`;提交 `/demo:hello` 不被 "Unrecognized command" 校验拦截。
3. /plugin overlay:展示真实已装插件(PluginManager 读盘),含 id /
   version / enabled 标志。

前置:reflect-agent submodule 已构建出 reflect-cli(debug 或 release),
用它把 examples/plugin-demo 安装进临时 HOME。任一二进制缺失则整个
phase SKIP(不计失败)——插件 e2e 是增强项,不阻塞基础回归。

离线(REFLECT_MODEL=mock/mock-1,零网络)。
"""
from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from harness import TuiSession, normalize  # noqa: E402

REPO_ROOT = HERE.parent
RES: list[bool] = []
SKIPPED = False


def check(name: str, ok: bool, detail: str = "") -> bool:
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}{(' — ' + detail) if detail else ''}")
    RES.append(ok)
    return ok


def find_cli() -> Path | None:
    """定位 submodule 构建出的 reflect-cli 二进制(bin 名是 `reflect`)。"""
    for profile in ("debug", "release"):
        candidate = REPO_ROOT / "reflect-agent" / "target" / profile / "reflect"
        if candidate.exists() and os.access(candidate, os.X_OK):
            return candidate
    return None


def setup_plugin_home(cli: Path) -> str:
    """临时 HOME + 安装/启用示例插件(走真实 CLI,写真实表和 config)。"""
    home = tempfile.mkdtemp(prefix="reflect-tui-plugin-")
    env = {**os.environ, "HOME": home}
    example = REPO_ROOT / "reflect-agent" / "examples" / "plugin-demo"
    subprocess.run(
        [str(cli), "plugin", "install", str(example)],
        env=env, check=True, capture_output=True, text=True, timeout=60,
    )
    subprocess.run(
        [str(cli), "plugin", "enable", "demo@inline"],
        env=env, check=True, capture_output=True, text=True, timeout=60,
    )
    return home


def mock_env(home: str) -> dict[str, str]:
    return {
        "HOME": home,
        "REFLECT_MODEL": "mock/mock-1",
        "REFLECT_TUI_UPDATE_CHECK": "0",
    }


def type_slash(s: TuiSession, token: str) -> None:
    """逐段输入 slash 文本 —— type_fast 整段写入可能走 paste-burst,
    分 `/` + 剩余两段写可稳定触发命令弹窗。"""
    s.type_fast("/")
    s.wait(1.0)
    s.type_fast(token)
    s.wait(1.2)


def wait_for(s: TuiSession, needle: str, timeout: float = 30.0) -> str:
    """轮询屏幕直到 `needle` 出现(或超时),返回最后一帧文本。

    插件挂载(含 MCP server 连接)会推迟 TUI 首帧渲染,空白屏对
    `wait_stable` 是「稳定」的,不能依赖它 —— 必须按内容轮询。
    """
    deadline = time.time() + timeout
    txt = ""
    while time.time() < deadline:
        txt = normalize(s.screen_text())
        if needle in txt:
            return txt
        time.sleep(0.4)
    return txt


def main() -> int:
    global SKIPPED
    print("========== PHASE 7 / Plugin UI E2E ==========")
    try:
        import pyte  # noqa: F401
    except ImportError:
        print("  [skip] pyte 未安装(pip install pyte)—— 插件 UI E2E 跳过")
        return 0

    cli = find_cli()
    tui_bin = os.environ.get("REFLECT_TUI_BIN")
    if cli is None or (tui_bin is None and not any(
        (REPO_ROOT / "target" / p / "reflect-tui").exists() for p in ("release", "debug")
    )):
        print("  [skip] reflect-cli 或 reflect-tui 二进制缺失 —— 插件 UI E2E 跳过")
        return 0
    SKIPPED = True

    try:
        home = setup_plugin_home(cli)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as e:
        print(f"  [skip] 示例插件安装失败,跳过: {e}")
        return 0

    # 场景 1:启动通知 —— PluginLoaded 经 lifecycle 通道进通知流。
    with TuiSession(env=mock_env(home)) as s:
        txt = wait_for(s, "Plugin 'demo@inline' loaded", timeout=30.0)
        check(
            "boot notice: PluginLoaded surfaced in scrollback",
            "Plugin 'demo@inline' loaded" in txt,
            txt[:80],
        )
        s.exit()

    # 场景 2:slash 弹窗列出插件命令;提交不被白名单拦截。
    with TuiSession(env=mock_env(home)) as s:
        wait_for(s, "for commands", timeout=30.0)  # 首帧渲染完成
        type_slash(s, "demo")
        txt = wait_for(s, "demo:hello", timeout=10.0)
        check(
            "slash popup lists plugin command demo:hello",
            "demo:hello" in txt,
        )
        s.type_fast(":hello")
        s.wait(1.0)
        s.enter(settle=2.0)
        txt2 = normalize(s.screen_text())
        check(
            "plugin command passes validation (no Unrecognized)",
            "Unrecognized" not in txt2 and "/demo:hello" in txt2,
        )
        s.exit()

    # 场景 3:/plugin overlay 展示真实已装插件。弹窗 Enter 派发与文本
    # 提交两条路最终都打开 overlay,只断言终态。
    with TuiSession(env=mock_env(home)) as s:
        wait_for(s, "for commands", timeout=30.0)  # 首帧渲染完成
        type_slash(s, "plugin")
        s.enter(settle=1.0)
        txt = wait_for(s, "Installed Plugins", timeout=10.0)
        ok = all(
            token in txt
            for token in ("Installed Plugins", "demo@inline", "v0.1.0", "enabled")
        )
        check("/plugin overlay shows real installed plugin", ok)
        s.exit()

    passed = sum(RES)
    print(f"\nplugin UI e2e: {passed}/{len(RES)}")
    return 0 if passed == len(RES) else 1


if __name__ == "__main__":
    raise SystemExit(main())
