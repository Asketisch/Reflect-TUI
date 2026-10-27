"""L 组 —— live 端到端(真实 LLM + 真实工具)。

**这不是回归门禁**:它需要真实 provider 与 API key,默认不跑
(`./tests/run_all.sh` 不含 live;`make test-suite-live` 单独跑)。
没有 API key 时整组 SKIP。

覆盖四条真实链路:
- **L1 `/loop <secs> <cmd>`**:定时器真的触发,`/loop stop` 能停;
- **L2 `/goal set` + `/goal clear`**:目标模式进 / 出;
- **L3 子代理派发**:模型调 `call_explorer` 搜索仓库;
- **L4 Plan 模式**:提 plan → `[3]` 修订 → `[1]` 批准(Auto ⇒ AcceptEdits)
  → 真的把文件写出来。

每条都把渲染后的屏幕 dump 出来,便于人工复核模型到底做了什么。

⚠️ 工作目录是**临时目录**(pytest `tmp_path`),不再往仓库根写文件 ——
旧脚本把 `WORKDIR` 硬编码成 `~/Code/CNB/ZhuYe-CLI`,会污染工作区。
"""

from __future__ import annotations

import os
import time
from pathlib import Path

import pytest

from tui_pty import Sandbox, TuiSession

# 需要真实 provider 的凭据(任一即可)。
_HAS_KEY = bool(
    os.environ.get("OPENAI_API_KEY") or os.environ.get("ANTHROPIC_API_KEY")
)
pytestmark = pytest.mark.skipif(
    not _HAS_KEY, reason="需要 OPENAI_API_KEY / ANTHROPIC_API_KEY 才能跑 live 端到端"
)

PROOF_FILE = "WORKFLOW_PROOF.txt"


def _dump(s: TuiSession, tag: str, max_lines: int = 44) -> None:
    """打印当前屏幕 —— live 用例失败时人工复核的唯一线索。"""
    print(f"\n      ── screen dump: {tag} ──")
    shown = 0
    for i, ln in enumerate(s.display_lines()):
        if ln.strip():
            print(f"      {i:2d}|{ln}")
            shown += 1
        if shown >= max_lines:
            break


def _submit(s: TuiSession, text: str, settle: float = 1.0) -> None:
    s.type_text(text)
    s.key("enter")
    time.sleep(settle)


@pytest.fixture()
def live_sandbox(tmp_path: Path) -> Sandbox:
    """隔离 HOME + 临时工作目录(不污染仓库)。"""
    sb = Sandbox.create(prefix="zhuye-cli-live-")
    # 工作目录直接用 sb.cwd —— plan / 文件产物都落在这里。
    yield sb
    sb.cleanup()


def test_live_loop_fires_and_stops(live_sandbox: Sandbox) -> None:
    """L1 `/loop 2 /status` 定时触发,`/loop stop` 能停。"""
    with TuiSession(live_sandbox, cols=120, rows=38) as s:
        s.wait_stable(max_wait=15.0)
        _submit(s, "/loop 2 /status")
        s.wait_text("started", timeout=20)

        # 让它跑 ~7s → 期望 ≥2 次触发。
        deadline = time.monotonic() + 7
        while time.monotonic() < deadline:
            s._pump(settle=0.6, max_wait=1.0)
        fired = s.screen_text().count("▶ /status")
        _dump(s, f"after ~7s of /loop 2 /status (fired={fired})", max_lines=26)
        assert fired >= 2, f"/loop 应至少触发 2 次,实际 {fired}"

        before = s.screen_text().count("▶ /status")
        _submit(s, "/loop stop")
        time.sleep(4)
        s._pump(settle=0.5, max_wait=1.0)
        after = s.screen_text().count("▶ /status")
        assert after <= before + 1, f"/loop stop 应停住: before={before} after={after}"


def test_live_goal_set_and_clear(live_sandbox: Sandbox) -> None:
    """L2 `/goal <objective>` 进入目标模式,`/goal clear` 清空。"""
    with TuiSession(live_sandbox, cols=120, rows=38) as s:
        s.wait_stable(max_wait=15.0)
        _submit(s, "/goal verify the release binary renders its banner")
        s.wait_text("goal", timeout=25)
        _dump(s, "after /goal set", max_lines=20)

        _submit(s, "/goal clear")
        s.wait_text("goal", timeout=25)


def test_live_subagent_dispatch(live_sandbox: Sandbox) -> None:
    """L3 模型可派发子代理(`call_explorer`)并拿到结果。"""
    with TuiSession(live_sandbox, cols=130, rows=44) as s:
        s.wait_stable(max_wait=15.0)
        _submit(
            s,
            "Dispatch a sub-agent (the explorer sub-agent / call_explorer tool) to "
            "search the repo for every Cargo.toml file and report the count. "
            "When it returns, state the count in one line.",
        )
        s.wait(
            lambda t: any(
                k in t.lower()
                for k in ("cargo.toml", "explorer", "sub-agent", "subagent", "call_explorer")
            ),
            timeout=180,
            label="subagent search",
        )
        _dump(s, "subagent/search outcome", max_lines=48)


def test_live_plan_revise_approve_execute(live_sandbox: Sandbox) -> None:
    """L4 Plan 模式:提 plan → `[3]` 修订 → `[1]` 批准 → 真的写出文件。"""
    proof = live_sandbox.cwd / PROOF_FILE
    with TuiSession(live_sandbox, cols=130, rows=44) as s:
        s.wait_stable(max_wait=15.0)
        _submit(s, "/mode plan")
        s.wait_text("[plan]", timeout=20)

        _submit(
            s,
            f"Create a file named {PROOF_FILE} in the working directory whose single "
            "line of content is: hello-plan. Propose a plan first.",
        )
        s.wait(
            lambda t: ("approve" in t.lower() and "plan" in t.lower()) or "[1]" in t,
            timeout=180,
            label="PlanReady approval overlay",
        )
        _dump(s, "PlanReady overlay before revise")

        # [3] Revise → 补充要求 → 重新提 plan。
        s.type_text("3")
        s._pump(settle=1.2, max_wait=3.0)
        _submit(
            s,
            "Change the content to two lines: hello-plan and REVIFIED-OK.",
        )
        s.wait(
            lambda t: ("approve" in t.lower() and "plan" in t.lower()) or "[1]" in t,
            timeout=180,
            label="re-plan after revise",
        )
        _dump(s, "PlanReady after revise")

        # [1] 批准(Auto ⇒ AcceptEdits ⇒ 可写)。
        s.type_text("1")
        s._pump(settle=2.0, max_wait=5.0)
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline and not proof.exists():
            s._pump(settle=1.0, max_wait=1.5)
        _dump(s, "after approve [1] execution")

    assert proof.exists(), f"{PROOF_FILE} 应被创建:{live_sandbox.cwd}"
    assert "REVIFIED-OK" in proof.read_text(), "文件应含修订后的内容"
