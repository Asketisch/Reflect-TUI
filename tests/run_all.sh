#!/usr/bin/env bash
# tests/run_all.sh —— 全量测试入口:能力覆盖层 + Rust 侧 + 覆盖自审。
#
# 用法:
#   ./tests/run_all.sh              # 全部
#   ./tests/run_all.sh functional   # 能力覆盖层(真二进制 + pty 行为断言)
#   ./tests/run_all.sh rust         # Rust 侧(cargo test/clippy/fmt/doc + CLI 参数矩阵)
#   ./tests/run_all.sh audit        # 覆盖自审(slash 命令 × 测试矩阵的差集核对)
#   ./tests/run_all.sh live         # live 端到端(需真实 LLM + API key)
#
# `functional` 是 CI 的门禁入口:跳过 cargo test / clippy / fmt / doc
# (那部分由既有 CI job 覆盖),跑真正的功能覆盖。
#
# 全程离线(REFLECT_MODEL=mock/mock-1)+ 隔离 HOME,不污染真实 ~/.reflect。
# 退出码 0 = 全部通过。

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/lib.sh"

SUITES=("functional" "rust" "audit")
if [ $# -gt 0 ]; then
    SUITES=("$@")
fi

cd "$ROOT"

# release 二进制不存在则构建(幂等)
if [ ! -x "$REFLECT_TUI_BIN" ]; then
    echo "── 构建 release 二进制 ──"
    cargo build --release -p reflect-tui --quiet || exit 2
fi

OVERALL=0
for s in "${SUITES[@]}"; do
    case "$s" in
        functional)
            echo
            echo "╔══════════════════════════════════════════════════╗"
            echo "  套件:functional(能力覆盖层)"
            echo "╚══════════════════════════════════════════════════╝"
            # 能力覆盖层只跑 pytest + CLI 参数矩阵,不含 cargo 编译面 ——
            # 编译面由 rust 套件负责,两者不重复。
            if bash "$HERE/functional/run.sh"; then
                echo "  ✅ functional 通过"
            else
                echo "  ❌ functional 失败"
                OVERALL=1
            fi
            ;;
        rust)
            echo
            echo "╔══════════════════════════════════════════════════╗"
            echo "  套件:rust"
            echo "╚══════════════════════════════════════════════════╝"
            if bash "$HERE/rust/run.sh"; then
                echo "  ✅ rust 通过"
            else
                echo "  ❌ rust 失败"
                OVERALL=1
            fi
            ;;
        audit)
            echo
            echo "╔══════════════════════════════════════════════════╗"
            echo "  套件:audit(覆盖自审)"
            echo "╚══════════════════════════════════════════════════╝"
            if python3 "$HERE/tools/coverage_audit.py" --strict; then
                echo "  ✅ audit 通过"
            else
                echo "  ❌ audit 失败(有未覆盖的 slash 命令)"
                OVERALL=1
            fi
            ;;
        live)
            echo
            echo "╔══════════════════════════════════════════════════╗"
            echo "  套件:live(需真实 LLM + API key)"
            echo "╚══════════════════════════════════════════════════╝"
            if python3 "$HERE/live/test_workflow_live.py"; then
                echo "  ✅ live 通过"
            else
                echo "  ❌ live 失败"
                OVERALL=1
            fi
            ;;
        *)
            echo "未知套件:$s(可选:functional / rust / audit / live)" >&2
            exit 2
            ;;
    esac
done

echo
if [ "$OVERALL" -eq 0 ]; then
    echo "══════════════ 全部测试套件通过 ══════════════"
else
    echo "══════════════ 存在失败套件(见上)══════════════"
fi
exit "$OVERALL"
