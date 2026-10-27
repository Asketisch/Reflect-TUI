"""P 组 —— 会话持久化:落盘、恢复、fork / rename / export、统计。

锚点与路径规则来源(证据见 `tests/COVERAGE.md` §P 组):
  - 会话文件:`~/.zhuye/sessions/YYYY/MM/DD/<thread_id>.jsonl`
  - 会话名:`~/.zhuye/sessions/_names/<thread_id>.name`
  - prompt 历史:`~/.zhuye/prompt_history.jsonl`
  - 恢复回填标记「已恢复历史会话」  tui/mod.rs(bootstrap 侧回填)
  - fork 提示「Resume with: zhuye-cli --resume <child_id>」  cli/session.rs:383
"""

from __future__ import annotations

import json
import time
import uuid

from tui_pty import text_reply


def _session_files(sandbox):
    return sandbox.session_files()


def test_session_file_is_written(tui, sandbox, mock_script) -> None:
    """P1 对话落盘到 `~/.zhuye/sessions/YYYY/MM/DD/*.jsonl`。

    注意:引擎的 agent loop 会多次调用模型(mock 脚本被逐行消费,耗尽后
    回退默认回复),落盘的 assistant 消息是**最后一轮**的回复,不是脚本
    第一行。因此这里断言「用户消息落盘 + 存在 assistant 消息」,不锁
    具体是哪一轮。
    """
    script = mock_script([text_reply("P1_REPLY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("persist me")
        s.key("enter")
        # 会话落盘把 assistant 消息写在回合提交时:先等文本上屏,再等
        # 回合收尾,否则慢机器上读到的文件只有 user 消息。
        s.wait_text("P1_REPLY", timeout=30)
        s.wait_idle(timeout=30)
        s._pump(settle=1.5, max_wait=3.0)

    files = _session_files(sandbox)
    assert files, f"应生成会话文件:{sandbox.home}/.zhuye/sessions"
    records = [
        json.loads(line)
        for line in files[-1].read_text().splitlines()
        if line.strip()
    ]
    # 首行应是 session_meta。
    assert records[0].get("type") == "session_meta", f"首行应为 session_meta:{records[0]}"

    messages = [r for r in records if r.get("type") == "message"]
    roles = [m.get("role") for m in messages]
    assert "user" in roles, f"用户消息应落盘:{roles}"

    user_msg = next(m for m in messages if m.get("role") == "user")
    assert user_msg["content"][0]["text"] == "persist me", (
        f"用户消息正文应落盘:{user_msg}"
    )
    assert "assistant" in roles, f"应有 assistant 消息:{roles}"

    # token_count 记录带嵌套 usage 对象。
    tokens = [r for r in records if r.get("type") == "token_count"]
    assert tokens, "应落盘 token_count 记录"
    assert "usage" in tokens[0] and "total_tokens" in tokens[0]["usage"], (
        f"token_count 应带 usage:{tokens[0]}"
    )


def test_prompt_history_is_persisted(tui, sandbox) -> None:
    """P2 prompt 历史落盘到 `~/.zhuye/prompt_history.jsonl`。"""
    with tui() as s:
        s.wait_boot(timeout=30)
        s.type_text("P2_PROMPT_MARKER")
        s.key("enter")
        time.sleep(1.5)

    hist = sandbox.home / ".zhuye" / "prompt_history.jsonl"
    assert hist.is_file(), f"应生成 prompt 历史文件:{hist}"
    assert "P2_PROMPT_MARKER" in hist.read_text()


def test_resume_continues_last_session(tui, sandbox, mock_script) -> None:
    """P3 `-c` 恢复最近会话:历史回填 + 旧消息上屏。"""
    script = mock_script([text_reply("P3_FIRST_REPLY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("P3_PRIOR_MESSAGE")
        s.key("enter")
        s.wait_text("P3_FIRST_REPLY", timeout=30)
        s.quit(timeout=10)

    with tui(args=["-c"]) as s:
        txt = s.wait_text("已恢复历史会话", timeout=25)
        assert "P3_PRIOR_MESSAGE" in txt, f"既往消息应回填:\n{txt[-800:]}"


def test_resume_by_id(tui, sandbox, mock_script) -> None:
    """P4 `--resume <thread_id>` 按 id 恢复会话。"""
    script = mock_script([text_reply("P4_FIRST_REPLY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("P4_PRIOR_MESSAGE")
        s.key("enter")
        s.wait_text("P4_FIRST_REPLY", timeout=30)
        s.quit(timeout=10)

    files = _session_files(sandbox)
    assert files, "应生成会话文件"
    session_id = files[-1].stem

    with tui(args=["--resume", session_id]) as s:
        txt = s.wait_text("已恢复历史会话", timeout=25)
        assert "P4_PRIOR_MESSAGE" in txt, f"既往消息应回填:\n{txt[-800:]}"


def test_resume_with_invalid_id_fails_fast(tui, sandbox) -> None:
    """P5 `--resume <非法 id>` 报错退出(不挂死)。"""
    with tui(args=["--resume", "not-a-uuid"]) as s:
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and s.returncode() is None:
            time.sleep(0.2)
        rc = s.returncode()
        assert rc is not None, "非法 id 应快速退出而不是挂死"
        assert rc != 0, f"应非 0 退出,实际 {rc}"
        # 诊断文案要指名道姓地说明「这个 id 不合法、应该长什么样」。
        # 实现侧的文案是中文(`无效的 session id '…':应为 UUID 或其前缀`),
        # 故断言锚点用「session id」这个稳定片段,而不是某一种语言的措辞。
        out = s.raw_output().lower()
        assert (
            "session id" in out or "invalid" in out or "thread id" in out
        ), f"报错文案应指明 id 无效:\n{s.raw_output()[-400:]}"
        assert "not-a-uuid" in s.raw_output(), (
            f"报错文案应回显出错的 id:\n{s.raw_output()[-400:]}"
        )


def test_resume_old_date_session_keeps_history_and_appends_to_original_file(
    tui, sandbox, mock_script
) -> None:
    """P8 旧日期会话恢复不丢历史,新轮次续写原日期目录的原文件。

    链路:submodule e4560bc 起 `reader::replay` 全盘定位(任意日期桶 +
    轮转副本),`JsonlRolloutWriter` 恢复后继续写原文件 —— 此前回放只扫
    近 3 天(旧会话恢复为空历史),且恢复后的写入会落到当天日期桶的新
    文件(同一会话被拆成两份)。把会话文件挪进 8 月的日期桶模拟
    「很久以前创建的会话」,两条行为都要在 TUI 侧成立。
    """
    script = mock_script([text_reply("P8_FIRST_REPLY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("P8_PRIOR_MESSAGE")
        s.key("enter")
        s.wait_text("P8_FIRST_REPLY", timeout=30)
        s.quit(timeout=10)

    files = _session_files(sandbox)
    assert files, "应生成会话文件"
    src = files[-1]
    session_id = src.stem

    # 挪到旧日期桶(2026/08/01),当天桶不再有该会话的文件。
    old_dir = sandbox.home / ".zhuye" / "sessions" / "2026" / "08" / "01"
    old_dir.mkdir(parents=True, exist_ok=True)
    original = old_dir / src.name
    src.rename(original)
    assert original.is_file()

    script2 = mock_script([text_reply("P8_SECOND_REPLY")])
    with tui(args=["--resume", session_id], script=script2) as s:
        txt = s.wait_text("已恢复历史会话", timeout=25)
        assert "P8_PRIOR_MESSAGE" in txt, f"旧日期会话的既往消息应回填:\n{txt[-800:]}"
        s.type_text("P8_NEW_MESSAGE")
        s.key("enter")
        s.wait_text("P8_SECOND_REPLY", timeout=30)
        s._pump(settle=1.5, max_wait=3.0)

    # 新轮次续写原日期目录的原文件,而不是在当天桶另立新文件。
    kept = [p for p in _session_files(sandbox) if p.stem == session_id]
    assert kept == [original], f"会话应仍只有原日期桶的一份文件:{kept}"
    body = original.read_text()
    assert "P8_NEW_MESSAGE" in body, f"新消息应追加进原文件:{original}"


def test_resume_by_internal_session_id_of_externally_named_file(
    tui, sandbox, mock_script
) -> None:
    """P9 文件名与内部 session id 脱钩的会话,按内部 id 恢复。

    TUI 自建会话文件名 thread id 与首行 `SessionMeta.session_id` 一致
    (bootstrap 注入);其它客户端(headless exec 等)写的会话两者不同。
    把文件改成另一个 uuid 名(首行内部 id 不动)模拟外部会话 —— 恢复
    入口必须支持「文件名 thread id / 首行内部 session id」双 id。
    """
    script = mock_script([text_reply("P9_FIRST_REPLY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("P9_PRIOR_MESSAGE")
        s.key("enter")
        s.wait_text("P9_FIRST_REPLY", timeout=30)
        s.quit(timeout=10)

    files = _session_files(sandbox)
    assert files, "应生成会话文件"
    src = files[-1]
    meta = json.loads(src.read_text().splitlines()[0])
    internal_id = meta["session_id"]
    assert internal_id, "首行 session_meta 应带 session_id"

    # 改名成另一个 uuid:文件名 id 与内部 id 脱钩。
    decoy = src.with_name(f"{uuid.uuid4()}.jsonl")
    src.rename(decoy)

    with tui(args=["--resume", internal_id]) as s:
        txt = s.wait_text("已恢复历史会话", timeout=25)
        assert "P9_PRIOR_MESSAGE" in txt, f"按内部 id 恢复应回填历史:\n{txt[-800:]}"


def test_session_name_is_written_by_rename(tui, sandbox, mock_script) -> None:
    """P6 `/rename <name>` 写 `~/.zhuye/sessions/_names/<id>.name`。"""
    script = mock_script([text_reply("P6_REPLY")])
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("go")
        s.key("enter")
        s.wait_text("P6_REPLY", timeout=30)
        s._pump(settle=1.5, max_wait=3.0)

    files = _session_files(sandbox)
    assert files, "应生成会话文件"

    # `/rename <name>` 在当前构建未接线(见 COVERAGE.md §S 组),改用
    # headless CLI 的 `session rename` 验证落盘路径。
    import subprocess

    from tui_pty import ZHUYE_CLI_BIN

    r = subprocess.run(
        [str(ZHUYE_CLI_BIN), "session", "rename", files[-1].stem, "P6_RENAME"],
        capture_output=True,
        text=True,
        timeout=30,
        env=sandbox.env(),
    )
    assert r.returncode == 0, f"session rename 应成功: {r.stdout}{r.stderr}"
    name_file = sandbox.home / ".zhuye" / "sessions" / "_names" / f"{files[-1].stem}.name"
    assert name_file.is_file(), f"应写会话名文件:{name_file}"
    assert "P6_RENAME" in name_file.read_text()


def test_plan_mode_writes_plan_file(tui, sandbox, mock_script) -> None:
    """P7 Plan 模式下 plan markdown 落盘到 `<workspace>/.zhuye/plan/`。

    链路:`/plan <task>` → `Op::EnterPlanMode` → 引擎发 PlanRequest →
    TUI 弹「Enter Plan Mode」确认框(按 `1` 进入)→ **再发一条消息**才
    真正起轮次 → 模型调 `PlanWrite` 工具 → 写
    `<ws>/.zhuye/plan/<name>.md`。

    注意:`/plan` 本身**不提交任务**(引擎只切模式并发 PlanRequest),
    所以必须再发一条消息,模型才会被调用。
    """
    from tui_pty import tool_call

    script = mock_script(
        [
            tool_call(
                "PlanWrite",
                {"path": "probe-plan.md", "content": "# P7_PLAN_BODY\n\n- step one\n"},
            ),
            text_reply("plan written"),
        ]
    )
    with tui(script=script) as s:
        s.wait_boot(timeout=30)
        s.type_text("/plan ship the release")
        s.key("enter")
        # 等待「Enter Plan Mode」确认框,按 1 进入 plan 模式。
        s.wait_text("Enter Plan Mode", timeout=25)
        s.type_text("1")
        s.key("enter")
        s.wait_text("Entering plan mode", timeout=15)
        # plan 模式下再发一条消息,模型才会被调用 → PlanWrite 落盘。
        s.type_text("write the plan now")
        s.key("enter")
        s.wait_text("PlanWrite", timeout=30)

        deadline = time.monotonic() + 20
        plans: list = []
        while time.monotonic() < deadline:
            plans = sandbox.workspace_files(".zhuye/plan/*.md")
            if plans:
                break
            time.sleep(0.5)

    assert plans, f"应生成 plan 文件:{sandbox.cwd}/.zhuye/plan/"
    body = plans[-1].read_text()
    assert "P7_PLAN_BODY" in body, f"plan 正文应落盘:\n{body}"
