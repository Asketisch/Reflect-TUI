#!/usr/bin/env bash
# tests/rust/workspace_tests.sh —— Rust 编译面基线。
#
# 依次执行:
#   1. cargo test --workspace --all-targets   —— 单元 + 集成测试
#   2. cargo clippy --workspace --all-targets -- -D warnings
#   3. cargo fmt --all -- --check
#   4. cargo doc --no-deps                      —— 文档可编译
#
# 这四项是 CI 已有的门槛;本套件把它们收拢到一处,保证「发布前审查」
# 跑的是同一组命令。
#
# `REFLECT_SKIP_WORKSPACE=1` 时整体跳过(CI 的功能门禁入口用此模式:
# 那部分由既有 CI job 覆盖,避免重复消耗 CI 时间)。
#
# 退出码 0 = 全部通过。

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../lib.sh"

cd "$ROOT"

if [ "${REFLECT_SKIP_WORKSPACE:-0}" = "1" ]; then
    echo "  workspace 测试 —— 跳过(REFLECT_SKIP_WORKSPACE=1)"
    exit 0
fi

TOTAL_RC=0
LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/reflect-tui-workspace.XXXXXX")"
trap 'rm -rf "$LOG_DIR"' EXIT

# step <名称> <命令...>:跑一步,落日志,按**命令自身**的退出码判定成败。
#
# 注意:不能写成 `if cmd | tail -30; then` —— 那样取到的是 `tail` 的退出码,
# 永远为 0,失败会被吞掉。这里把输出重定向到文件再看 `$?`。
step() {
    local name="$1"; shift
    local log="$LOG_DIR/$(echo "$name" | tr ' /' '__').log"
    echo
    echo "── $name ──"
    if "$@" > "$log" 2>&1; then
        tail -20 "$log"
        echo "  [ok] $name"
        return 0
    fi
    local rc=$?
    echo "  [FAIL] $name(退出码 $rc)"
    tail -40 "$log"
    return 1
}

step "cargo test --workspace --all-targets" \
    cargo test --workspace --all-targets --quiet || TOTAL_RC=1

step "cargo clippy --workspace --all-targets -- -D warnings" \
    cargo clippy --workspace --all-targets -- -D warnings || TOTAL_RC=1

step "cargo fmt --all -- --check" \
    cargo fmt --all -- --check || TOTAL_RC=1

step "cargo doc --no-deps" \
    cargo doc --no-deps || TOTAL_RC=1

echo
if [ "$TOTAL_RC" -eq 0 ]; then
    echo "  ✅ workspace 测试全部通过"
else
    echo "  ❌ workspace 测试存在失败"
fi
exit "$TOTAL_RC"
