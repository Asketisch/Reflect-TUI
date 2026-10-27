#!/usr/bin/env bash
# tests/functional/run.sh —— 能力覆盖层入口。
#
# 依次执行:
#   1. pytest(能力覆盖层):真二进制 + pty 的行为级断言
#      - test_boot.py          启动 / 渲染锚点 / 退出生命周期 / 全屏模式
#      - test_slash.py         slash 命令全量矩阵(84 条)
#      - test_overlay.py       弹层 / 浮层 / 选择器 + 反向门禁
#      - test_turn.py          对话轮次 / 流式 / 工具 / 统计
#      - test_session.py       会话持久化 / 恢复 / plan 落盘
#      - test_cli.py           headless CLI 的语义断言
#      - test_keys.py          键盘交互(权限循环 / 历史召回 / 编辑器 / vim)
#      - test_render.py        渲染与视觉(宽字符 / 折行 / markdown / 主题 / resize)
#      - test_flow.py          工作流与容错(审批 / AskUser / loop / 恢复边界 / 信号)
#      - test_plugin.py        插件面
#      - test_http_mock.py     真实 HTTP 链路(Anthropic SSE + 确定性 busy 窗口)
#      - test_pty_helpers.py   测试设施自检
#      - test_coverage_audit.py 覆盖自审门禁
#   2. tests/rust/cli_core.sh + cli_modules.sh  CLI 参数矩阵
#
# 前置:release 二进制(入口自动补建)、Python 3 + pytest + pyte。
# 缺依赖的批次自动 SKIP,不算失败。
#
# 全程离线(ZHUYE_MODEL=mock/mock-1)+ 隔离 HOME。退出码 0 = 全部通过。

set -u

# Windows 无 pty(pty 模块仅 Unix),能力覆盖层整体跳过,保持与
# 「缺依赖自动 SKIP,不算失败」的套件哲学一致。移植到 Windows pty
# 方案后再移除此守卫。
if [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* || "$(uname -s)" == CYGWIN* ]]; then
    echo "  [SKIP] Windows 无 pty;能力覆盖层跳过(移植期)"
    exit 0
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../lib.sh"

cd "$ROOT"

TOTAL_RC=0

# release 二进制不存在则构建(幂等)
if [ ! -x "$ZHUYE_CLI_BIN" ]; then
    echo "── 构建 release 二进制 ──"
    cargo build --release -p zhuye-cli --quiet || exit 2
fi

echo
echo "════════════════════════════════════════════════"
echo "  1/2 能力覆盖层(pytest,真二进制 + pty)"
echo "════════════════════════════════════════════════"

if require_cmd python3; then
    if python3 -c "import pytest, pyte" 2>/dev/null; then
        # pytest 从本目录跑,保证 `from tui_pty import ...` 可解析。
        ( cd "$HERE" && python3 -m pytest -q --tb=short ) || TOTAL_RC=1
    elif python3 -c "import pytest" 2>/dev/null; then
        # pyte 缺失会让 TuiSession 在用例里抛 ImportError(逐条失败而非跳过),
        # 与 README 声称的「缺依赖自动 SKIP」不符 —— 在门口整层拦下。
        echo "  [SKIP] 未安装 pyte(pip install pyte);能力覆盖层跳过"
    else
        echo "  [SKIP] 未安装 pytest(pip install pytest);能力覆盖层跳过"
    fi
else
    echo "  [SKIP] 未找到 python3;能力覆盖层跳过"
fi

echo
echo "════════════════════════════════════════════════"
echo "  2/2 CLI 参数矩阵"
echo "════════════════════════════════════════════════"
bash "$HERE/../rust/cli_core.sh" || TOTAL_RC=1
bash "$HERE/../rust/cli_modules.sh" || TOTAL_RC=1

echo
if [ "$TOTAL_RC" -eq 0 ]; then
    echo "  ✅ 能力覆盖层全部通过"
else
    echo "  ❌ 能力覆盖层存在失败(见上方 [FAIL] 行)"
fi
exit "$TOTAL_RC"
