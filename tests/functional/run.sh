#!/usr/bin/env bash
# tests/functional/run.sh —— 能力覆盖层入口。
#
# 依次执行:
#   1. pytest(能力覆盖层):真二进制 + pty 的行为级断言
#      - test_boot.py          启动 / 渲染锚点 / 退出生命周期
#      - test_slash.py         slash 命令全量矩阵(84 条)
#      - test_overlay.py       弹层 / 浮层 / 选择器
#      - test_turn.py          对话轮次 / 流式 / 工具 / 统计
#      - test_session.py       会话持久化 / 恢复 / plan 落盘
#      - test_cli.py           headless CLI 的语义断言
#      - test_coverage_audit.py 覆盖自审门禁
#   2. tests/rust/cli_core.sh + cli_modules.sh  CLI 参数矩阵
#
# 前置:release 二进制(入口自动补建)、Python 3 + pytest + pyte。
# 缺依赖的批次自动 SKIP,不算失败。
#
# 全程离线(REFLECT_MODEL=mock/mock-1)+ 隔离 HOME。退出码 0 = 全部通过。

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../lib.sh"

cd "$ROOT"

TOTAL_RC=0

# release 二进制不存在则构建(幂等)
if [ ! -x "$REFLECT_TUI_BIN" ]; then
    echo "── 构建 release 二进制 ──"
    cargo build --release -p reflect-tui --quiet || exit 2
fi

echo
echo "════════════════════════════════════════════════"
echo "  1/2 能力覆盖层(pytest,真二进制 + pty)"
echo "════════════════════════════════════════════════"

if require_cmd python3; then
    if python3 -c "import pytest" 2>/dev/null; then
        # pytest 从本目录跑,保证 `from tui_pty import ...` 可解析。
        ( cd "$HERE" && python3 -m pytest -q --tb=short ) || TOTAL_RC=1
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
