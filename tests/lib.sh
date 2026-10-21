#!/usr/bin/env bash
# tests/lib.sh —— 测试套件公共助手。
#
# 所有 tests/ 下的脚本 source 本文件,获得:
#   - 隔离环境:mktemp HOME + 独立 cwd,不污染真实 ~/.reflect;
#   - mock provider 环境(REFLECT_MODEL=mock/mock-1,离线、免 API key);
#   - 断言助手(assert_contains / assert_not_contains / assert_matches);
#   - 便携超时(macOS 无 timeout/gtimeout,用后台 pid + kill 模拟);
#   - 用例统计与失败汇总(tcase / tfinish 汇总退出码)。
#
# 约定:注释一律中文;脚本自身可独立执行,退出码 0 = 全部通过。
#
# 与 submodule `reflect-agent/tests/lib.sh` 的差异:
#   submodule 测的是 headless `reflect serve`(JSONL wire),本仓测的是
#   终端 TUI —— 因此这里额外提供 mock 脚本生成与 pty 相关的基础环境变量,
#   但**不**提供 wire 会话设施(那由 tests/functional/ 的 pty 层负责)。

# ── 仓库根与二进制定位 ────────────────────────────────────────────────────
TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$TESTS_DIR/.." && pwd)"
REFLECT_TUI_BIN="${REFLECT_TUI_BIN:-$ROOT/target/release/reflect-tui}"

# ── 全局状态 ──────────────────────────────────────────────────────────────
declare -i PASS_COUNT=0
declare -i FAIL_COUNT=0
declare -a FAIL_NAMES=()

# tcase <名称> <命令...>:执行一条用例,记录通过/失败。
tcase() {
    local name="$1"; shift
    if "$@" >/dev/null 2>&1; then
        PASS_COUNT+=1
        echo "  [ok]   $name"
    else
        FAIL_COUNT+=1
        FAIL_NAMES+=("$name")
        echo "  [FAIL] $name"
    fi
}

# tfinish:打印汇总并返回汇总退出码(0=全过)。
tfinish() {
    local script_name="${1:-$(basename "${BASH_SOURCE[1]:-unknown}")}"
    echo
    echo "──────────────────────────────────────────"
    echo "  $script_name: $PASS_COUNT passed, $FAIL_COUNT failed"
    if [ "$FAIL_COUNT" -gt 0 ]; then
        printf '  失败用例: %s\n' "${FAIL_NAMES[*]}"
        return 1
    fi
    return 0
}

# ── 断言助手(供脚本内的函数体使用)──────────────────────────────────────
# 每个断言**同时记账**:通过 → PASS_COUNT,失败 → FAIL_COUNT + FAIL_NAMES。
# 断言失败时打印失败原因,便于在长脚本里定位。

# _record <名称> <0/1 结果> <失败细节>
_record() {
    local name="$1" rc="$2" detail="${3:-}"
    if [ "$rc" -eq 0 ]; then
        PASS_COUNT+=1
        echo "  [ok]   $name"
    else
        FAIL_COUNT+=1
        FAIL_NAMES+=("$name")
        if [ -n "$detail" ]; then
            echo "  [FAIL] $name — $detail"
        else
            echo "  [FAIL] $name"
        fi
    fi
}

# assert_contains <名称> <子串> <文件>:文件包含子串。
assert_contains() {
    local name="$1" needle="$2" file="$3"
    if grep -qF -- "$needle" "$file"; then
        _record "$name" 0
        return 0
    fi
    _record "$name" 1 "未找到子串 '$needle'"
    return 1
}

# assert_not_contains <名称> <子串> <文件>:文件不含子串。
assert_not_contains() {
    local name="$1" needle="$2" file="$3"
    if grep -qF -- "$needle" "$file"; then
        _record "$name" 1 "不应出现的子串 '$needle' 出现了"
        return 1
    fi
    _record "$name" 0
    return 0
}

# assert_matches <名称> <正则> <文件>:文件匹配正则(grep -E)。
assert_matches() {
    local name="$1" pattern="$2" file="$3"
    if grep -qE -- "$pattern" "$file"; then
        _record "$name" 0
        return 0
    fi
    _record "$name" 1 "未匹配正则 '$pattern'"
    return 1
}

# assert_not_matches <名称> <正则> <文件>:文件不匹配正则。
assert_not_matches() {
    local name="$1" pattern="$2" file="$3"
    if grep -qE -- "$pattern" "$file"; then
        _record "$name" 1 "不应匹配的正则 '$pattern' 匹配了"
        return 1
    fi
    _record "$name" 0
    return 0
}

# assert_exit <名称> <期望退出码> <实际退出码>:退出码相等。
assert_exit() {
    local name="$1" want="$2" got="$3"
    if [ "$want" -eq "$got" ]; then
        _record "$name" 0
        return 0
    fi
    _record "$name" 1 "期望退出码 $want,实际 $got"
    return 1
}

# assert_nonempty <名称> <文件>:文件非空。
assert_nonempty() {
    local name="$1" file="$2"
    if [ -s "$file" ]; then
        _record "$name" 0
        return 0
    fi
    _record "$name" 1 "文件为空或不存在: $file"
    return 1
}

# assert_empty <名称> <文件>:文件为空。
assert_empty() {
    local name="$1" file="$2"
    if [ -s "$file" ]; then
        _record "$name" 1 "文件非空: $file"
        return 1
    fi
    _record "$name" 0
    return 0
}

# ── 便携超时:run_with_timeout <秒> <命令...> ─────────────────────────────
# macOS 无 coreutils timeout;后台起进程,轮询等待,超时 SIGTERM+SIGKILL。
run_with_timeout() {
    local secs="$1"; shift
    "$@" &
    local pid=$!
    local waited=0
    while kill -0 "$pid" 2>/dev/null; do
        if [ "$waited" -ge "$secs" ]; then
            kill -TERM "$pid" 2>/dev/null
            sleep 1
            kill -9 "$pid" 2>/dev/null
            wait "$pid" 2>/dev/null
            return 124
        fi
        sleep 1
        waited=$((waited + 1))
    done
    wait "$pid"
}

# ── 隔离环境 ──────────────────────────────────────────────────────────────
# make_isolated_env:创建并进入隔离环境,设置:
#   ISO_HOME / ISO_CWD(导出 HOME、cd 进去)
#   预写 ~/.reflect/config.toml —— 空的最小配置。
#   说明:reflect_config::load_default() 对缺文件是「回落默认配置」而非报错,
#   但显式写入可让「配置文件存在」这一前提在用例里显式成立,避免依赖
#   宿主机残留配置(宿主机若有 provider 配置,会改变启动行为)。
make_isolated_env() {
    ISO_HOME="$(mktemp -d "${TMPDIR:-/tmp}/reflect-tui-test-home.XXXXXX")"
    ISO_CWD="$(mktemp -d "${TMPDIR:-/tmp}/reflect-tui-test-cwd.XXXXXX")"
    mkdir -p "$ISO_HOME/.reflect"
    cat > "$ISO_HOME/.reflect/config.toml" <<'EOF'
# 测试用最小配置:空配置(不指定 provider,靠 REFLECT_MODEL 环境变量)。
EOF
    export HOME="$ISO_HOME"
    cd "$ISO_CWD" || exit 1
}

# cleanup_isolated_env:清掉隔离目录(脚本 trap 里调用)。
cleanup_isolated_env() {
    [ -n "${ISO_HOME:-}" ] && rm -rf "$ISO_HOME"
    [ -n "${ISO_CWD:-}" ] && rm -rf "$ISO_CWD"
}

# ── mock provider 环境 ────────────────────────────────────────────────────
# mock_env <mock 脚本路径(可空)>:以命令前缀形式输出 mock 环境变量。
#   用法:mock_env "$TMP/script.jsonl" "$BIN" session ls > out.txt
# mock 脚本为 JSONL,每行一次模型回复:
#   {"type":"text","text":"回复"}            —— 纯文本
#   {"type":"tool_call","name":"t","args":{}} —— 发起工具调用
mock_env() {
    local script="${1:-}"
    if [ -n "$script" ]; then
        echo "env" "REFLECT_MODEL=mock/mock-1" "REFLECT_MOCK_SCRIPT=$script"
    else
        echo "env" "REFLECT_MODEL=mock/mock-1"
    fi
}

# make_mock_script <输出路径> <行...>:写一个 mock 脚本文件。
make_mock_script() {
    local path="$1"; shift
    : > "$path"
    local line
    for line in "$@"; do
        printf '%s\n' "$line" >> "$path"
    done
}

# require_binary:确认 release 二进制存在,否则提示构建命令后退出。
require_binary() {
    if [ ! -x "$REFLECT_TUI_BIN" ]; then
        echo "FAIL: $REFLECT_TUI_BIN 不存在;先执行 cargo build --release -p reflect-tui" >&2
        exit 2
    fi
}

# require_cmd <命令...>:缺少任一依赖命令则退出(提示 SKIP 语义由调用方处理)。
require_cmd() {
    local c
    for c in "$@"; do
        command -v "$c" >/dev/null 2>&1 || return 1
    done
    return 0
}
