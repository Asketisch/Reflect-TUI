#!/usr/bin/env bash
# tests/rust/run.sh —— Rust 侧测试总入口。
#
# 依次执行:
#   1. workspace_tests.sh   cargo test / clippy / fmt / doc
#   2. cli_core.sh          顶层 CLI 参数矩阵(--help / --version / --cwd / 分派)
#   3. cli_modules.sh       session / traces 参数矩阵与输出契约
#
# `REFLECT_SKIP_WORKSPACE=1` 时跳过第 1 步 —— CI 的功能门禁入口用此模式:
# 那部分由既有 CI job 覆盖,避免重复消耗 CI 时间。
#
# 全程离线(ZHUYE_MODEL=mock/mock-1)+ 隔离 HOME,不污染真实 ~/.zhuye。
# 退出码 0 = 全部通过。

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../lib.sh"

cd "$ROOT"

# release 二进制不存在则构建(幂等)
if [ ! -x "$REFLECT_TUI_BIN" ]; then
    echo "── 构建 release 二进制 ──"
    cargo build --release -p reflect-tui --quiet || exit 2
fi

TOTAL_RC=0

if [ "${REFLECT_SKIP_WORKSPACE:-0}" = "1" ]; then
    echo
    echo "════════════════════════════════════════════════"
    echo "  1/3 workspace 测试 —— 跳过(REFLECT_SKIP_WORKSPACE=1)"
    echo "════════════════════════════════════════════════"
else
    echo
    echo "════════════════════════════════════════════════"
    echo "  1/3 workspace 测试(cargo test + clippy + fmt + doc)"
    echo "════════════════════════════════════════════════"
    bash "$HERE/workspace_tests.sh" || TOTAL_RC=1
fi

echo
echo "════════════════════════════════════════════════"
echo "  2/3 顶层 CLI 参数矩阵"
echo "════════════════════════════════════════════════"
bash "$HERE/cli_core.sh" || TOTAL_RC=1

echo
echo "════════════════════════════════════════════════"
echo "  3/3 session / traces 参数矩阵"
echo "════════════════════════════════════════════════"
bash "$HERE/cli_modules.sh" || TOTAL_RC=1

echo
if [ "$TOTAL_RC" -eq 0 ]; then
    echo "  ✅ Rust 测试套件全部通过"
else
    echo "  ❌ Rust 测试套件存在失败(见上方 [FAIL] 行)"
fi
exit "$TOTAL_RC"
