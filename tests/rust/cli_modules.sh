#!/usr/bin/env bash
# tests/rust/cli_modules.sh —— `session` / `traces` 子命令的参数矩阵与输出契约。
#
# 覆盖:
#   session ls      —— 表头 / limit / --model 过滤 / 空目录占位
#   session show    —— 元数据 + 消息预览 / 前缀解析 / 不存在 id 报错
#   session rm      —— --yes 删除 / 无 --yes 时交互确认(取消)
#   session fork    —— 生成子会话 + 打印续作提示
#   session rename  —— 写 `~/.zhuye/sessions/_names/<id>.name`
#   session export  —— stdout markdown / --out 文件落盘(原子写)
#   session stats   —— per-model 用量表 / --workspace 过滤 / TOTAL 行
#   traces ls       —— 表头 / 空目录占位
#   traces show     —— 逐条调用详情 / 前缀解析 / 不存在 id 报错
#   traces spans    —— span 日志 / --days 过滤 / 按 session 前缀过滤
#
# 全程离线 + 隔离 HOME。退出码 0 = 全部通过。

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../lib.sh"

cd "$ROOT"
require_binary

TMP="$(mktemp -d "${TMPDIR:-/tmp}/reflect-tui-cli-modules.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

BIN="$REFLECT_TUI_BIN"
HOME_DIR="$TMP/home"
SESS_DIR="$HOME_DIR/.zhuye/sessions"
TODAY="$(date -u +%Y/%m/%d)"
SESSION_ID="11111111-2222-4333-8444-555555555555"
SESSION_FILE="$SESS_DIR/$TODAY/$SESSION_ID.jsonl"

mkdir -p "$HOME_DIR/.zhuye" "$(dirname "$SESSION_FILE")"
printf '# 测试用最小配置:空配置。\n' > "$HOME_DIR/.zhuye/config.toml"

# seed_session:写一条含 user + assistant 的会话 JSONL。
#
# 形态与引擎实际落盘一致(见 reflect-protocol `RolloutRecord`):
#   - `content` 是 **ContentBlock 数组**(v1.2 起);
#   - `token_count` 的 token 字段嵌在 `usage` 对象里。
# 用真实形态而非简化形态,才能测出「读取侧只认旧形态」这类缺陷。
seed_session() {
    local file="$1"
    local sid="$2"
    local model="${3:-mock/mock-1}"
    local workspace="${4:-}"
    mkdir -p "$(dirname "$file")"
    {
        printf '{"type":"session_meta","session_id":"%s","model":"%s","started_at":"2026-01-01T00:00:00Z"' "$sid" "$model"
        if [ -n "$workspace" ]; then
            printf ',"workspace":"%s"' "$workspace"
        fi
        printf '}\n'
        printf '{"type":"message","turn_id":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee","role":"user","content":[{"type":"text","text":"probe user message"}]}\n'
        printf '{"type":"message","turn_id":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee","role":"assistant","content":[{"type":"text","text":"probe assistant reply"}]}\n'
        printf '{"type":"token_count","turn_id":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee","usage":{"input_tokens":120,"output_tokens":34,"cached_tokens":7,"cache_write_tokens":0,"total_tokens":154},"cost_usd":0.0123,"model":"%s","provider":"mock","at":"2026-01-01T00:00:01Z"}\n' "$model"
    } > "$file"
}

# run_cli <输出文件> <参数...>:在隔离 HOME 下跑 CLI。
# 真实退出码记入 LAST_RC —— 注意 `set -u` 下脚手架不会中断,必须显式捕获,
# 否则后续 `$?` 拿到的是上一条断言(grep)的返回码,形成恒过的伪断言。
run_cli() {
    local out="$1"; shift
    HOME="$HOME_DIR" ZHUYE_MODEL=mock/mock-1 "$BIN" "$@" > "$out" 2>&1
    LAST_RC=$?
}

# ok <名称> <条件结果 0/1>:记一条通过/失败。
ok() {
    if [ "$2" -eq 0 ]; then
        PASS_COUNT+=1; echo "  [ok]   $1"
    else
        FAIL_COUNT+=1; FAIL_NAMES+=("$1"); echo "  [FAIL] $1"
    fi
}

# ── 1. session ls ────────────────────────────────────────────────────────

run_cli "$TMP/ls_empty.txt" session ls
assert_contains "session ls 空目录给占位文案" "no sessions found" "$TMP/ls_empty.txt"
assert_exit "session ls 空目录退出码为 0" 0 "$LAST_RC"

seed_session "$SESSION_FILE" "$SESSION_ID"

run_cli "$TMP/ls.txt" session ls
assert_contains "session ls 表头含 session_id" "session_id" "$TMP/ls.txt"
assert_contains "session ls 表头含 model" "model" "$TMP/ls.txt"
assert_contains "session ls 表头含 tokens" "tokens" "$TMP/ls.txt"
assert_contains "session ls 列出目标会话" "$SESSION_ID" "$TMP/ls.txt"

run_cli "$TMP/ls_limit.txt" session ls --limit 0
assert_not_contains "session ls --limit 0 不列出会话" "$SESSION_ID" "$TMP/ls_limit.txt"

run_cli "$TMP/ls_model.txt" session ls --model mock
assert_contains "session ls --model mock 命中" "$SESSION_ID" "$TMP/ls_model.txt"

run_cli "$TMP/ls_model_miss.txt" session ls --model zzz-no-such-model
assert_not_contains "session ls --model 不命中时为空" "$SESSION_ID" "$TMP/ls_model_miss.txt"

# ── 2. session show ──────────────────────────────────────────────────────

run_cli "$TMP/show.txt" session show "$SESSION_ID"
assert_contains "session show 输出 Session 行" "Session:" "$TMP/show.txt"
assert_contains "session show 输出 Path 行" "Path:" "$TMP/show.txt"
assert_contains "session show 输出 Model 行" "Model:" "$TMP/show.txt"
assert_contains "session show 预览 user 消息" "probe user message" "$TMP/show.txt"
assert_contains "session show 预览 assistant 消息" "probe assistant reply" "$TMP/show.txt"

run_cli "$TMP/show_prefix.txt" session show "11111111"
assert_contains "session show 支持前缀解析" "probe user message" "$TMP/show_prefix.txt"

run_cli "$TMP/show_missing.txt" session show "00000000-0000-4000-8000-000000000000"
assert_exit_nonzero "session show 不存在 id 时非 0 退出" "$LAST_RC"
assert_contains "session show 不存在 id 有错误文案" "not found" "$TMP/show_missing.txt"

run_cli "$TMP/show_badid.txt" session show "not-a-uuid"
assert_exit_nonzero "session show 非法 id 时非 0 退出" "$LAST_RC"
assert_contains "session show 非法 id 有前缀诊断" "no session matches prefix" "$TMP/show_badid.txt"

# ── 3. session rename ────────────────────────────────────────────────────

run_cli "$TMP/rename.txt" session rename "$SESSION_ID" "probe-renamed"
assert_contains "session rename 打印成功行" "Renamed session" "$TMP/rename.txt"
ok "session rename 写 .name 文件" "$([ -f "$HOME_DIR/.zhuye/sessions/_names/$SESSION_ID.name" ] && echo 0 || echo 1)"
run_cli "$TMP/show_after_rename.txt" session show "$SESSION_ID"
assert_contains "session show 显示名称" "probe-renamed" "$TMP/show_after_rename.txt"

# ── 4. session export ────────────────────────────────────────────────────

run_cli "$TMP/export_stdout.txt" session export "$SESSION_ID"
assert_contains "session export stdout 含 user 消息" "probe user message" "$TMP/export_stdout.txt"
assert_contains "session export stdout 含 assistant 消息" "probe assistant reply" "$TMP/export_stdout.txt"

run_cli "$TMP/export_file.txt" session export "$SESSION_ID" --out "$TMP/exported.md"
assert_contains "session export --out 打印字节数" "bytes" "$TMP/export_file.txt"
ok "session export --out 文件落盘" "$([ -s "$TMP/exported.md" ] && echo 0 || echo 1)"
assert_contains "导出 markdown 含用户消息" "probe user message" "$TMP/exported.md"

# ── 5. session fork ──────────────────────────────────────────────────────

run_cli "$TMP/fork.txt" session fork "$SESSION_ID" --branch probe-branch
assert_contains "session fork 打印 Forked 行" "Forked session" "$TMP/fork.txt"
assert_contains "session fork 打印续作提示" "Resume with" "$TMP/fork.txt"
CHILD_ID="$(grep -oE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' "$TMP/fork.txt" | tail -1)"
ok "session fork 生成子会话文件" "$([ -n "$CHILD_ID" ] && [ -f "$(find "$SESS_DIR" -name "$CHILD_ID.jsonl" | head -1)" ] && echo 0 || echo 1)"
if [ -n "$CHILD_ID" ]; then
    run_cli "$TMP/show_child.txt" session show "$CHILD_ID"
    assert_contains "子会话继承了父历史" "probe user message" "$TMP/show_child.txt"
fi

# ── 6. session stats ─────────────────────────────────────────────────────

# 另建一条**带 workspace** 的会话,供 --workspace 过滤用。
# 模型名与主会话不同 —— per-model 表按模型聚合,同名会合并,断言就分不清行数。
WS_SESSION_ID="44444444-5555-4666-8777-888888888888"
WS_SESSION_FILE="$SESS_DIR/$TODAY/$WS_SESSION_ID.jsonl"
seed_session "$WS_SESSION_FILE" "$WS_SESSION_ID" "mock/mock-ws" "$TMP"

run_cli "$TMP/stats.txt" session stats
assert_contains "session stats 输出 Per-model 表头" "Per-model usage" "$TMP/stats.txt"
assert_contains "session stats 含 TOTAL 行" "TOTAL" "$TMP/stats.txt"
assert_contains "session stats 含 token 总量" "154" "$TMP/stats.txt"

run_cli "$TMP/stats_ws.txt" session stats --workspace "$TMP"
assert_contains "session stats --workspace 过滤生效" "session(s) in workspace" "$TMP/stats_ws.txt"
assert_contains "session stats --workspace 含 token 总量" "154" "$TMP/stats_ws.txt"

run_cli "$TMP/stats_ws_miss.txt" session stats --workspace /no/such/workspace
assert_contains "session stats --workspace 未命中给占位" "no sessions found for workspace" "$TMP/stats_ws_miss.txt"

# ── 7. session rm ────────────────────────────────────────────────────────

run_cli "$TMP/rm_abort.txt" session rm "$SESSION_ID" <<< "n"
assert_contains "session rm 交互取消打印 aborted" "aborted" "$TMP/rm_abort.txt"
ok "session rm 取消后文件仍在" "$([ -f "$SESSION_FILE" ] && echo 0 || echo 1)"

run_cli "$TMP/rm_yes.txt" session rm "$SESSION_ID" --yes
assert_contains "session rm --yes 打印 removed" "removed" "$TMP/rm_yes.txt"
ok "session rm --yes 删除文件" "$([ ! -f "$SESSION_FILE" ] && echo 0 || echo 1)"

# ── 8. traces ls / show / spans ──────────────────────────────────────────

run_cli "$TMP/traces_ls_empty.txt" traces ls
assert_contains "traces ls 空目录给占位文案" "no traces found" "$TMP/traces_ls_empty.txt"

TRACE_DIR="$HOME_DIR/.zhuye/traces/model-io"
mkdir -p "$TRACE_DIR"
TRACE_ID="22222222-3333-4444-8555-666666666666"
cat > "$TRACE_DIR/model-io-sess_$TRACE_ID.jsonl" <<EOF
{"started_at":"2026-01-01T00:00:00Z","completed_at":"2026-01-01T00:00:01Z","duration_ms":1000,"attempt":1,"request_id":"r1","trace_id":"t1","session_id":"$TRACE_ID","query_source":"main_turn","model":{"model_id":"mock-1","provider_id":"mock","role":"main","source":"main_turn"},"request":{},"response":{"finish_reason":"stop"},"usage":{"input_tokens":10,"output_tokens":5,"cached_tokens":0,"cache_write_tokens":0,"total_tokens":15}}
EOF

run_cli "$TMP/traces_ls.txt" traces ls
assert_contains "traces ls 表头含 session_id" "session_id" "$TMP/traces_ls.txt"
assert_contains "traces ls 列出 trace 会话" "$TRACE_ID" "$TMP/traces_ls.txt"
assert_contains "traces ls 显示调用次数" "1" "$TMP/traces_ls.txt"

run_cli "$TMP/traces_show.txt" traces show "$TRACE_ID"
assert_contains "traces show 输出 Session 行" "Session:" "$TMP/traces_show.txt"
assert_contains "traces show 输出 model 行" "model:" "$TMP/traces_show.txt"
assert_contains "traces show 输出 usage 行" "usage:" "$TMP/traces_show.txt"
assert_contains "traces show 输出 finish_reason" "finish_reason" "$TMP/traces_show.txt"

run_cli "$TMP/traces_show_prefix.txt" traces show "22222222"
assert_contains "traces show 支持前缀解析" "Session:" "$TMP/traces_show_prefix.txt"

run_cli "$TMP/traces_show_missing.txt" traces show "00000000-0000-4000-8000-000000000000"
assert_exit_nonzero "traces show 不存在 id 时非 0 退出" "$LAST_RC"
assert_contains "traces show 不存在 id 报 no trace session" "no trace session" "$TMP/traces_show_missing.txt"

SPAN_DIR="$HOME_DIR/.zhuye/traces/log"
mkdir -p "$SPAN_DIR"
SPAN_DAY="$(date -u +%Y-%m-%d)"
cat > "$SPAN_DIR/zhuye-$SPAN_DAY.jsonl" <<EOF
{"timestamp":"2026-01-01T00:00:00Z","level":"info","event":"turn.started","module":"m","trace_id":"t1","span_id":"s1","session_id":"$TRACE_ID"}
EOF

run_cli "$TMP/traces_spans.txt" traces spans
assert_contains "traces spans 列出 span 事件" "turn.started" "$TMP/traces_spans.txt"

run_cli "$TMP/traces_spans_id.txt" traces spans "$TRACE_ID"
assert_contains "traces spans 按 id 过滤命中" "turn.started" "$TMP/traces_spans_id.txt"

run_cli "$TMP/traces_spans_miss.txt" traces spans "no-such-session"
assert_contains "traces spans 未命中给占位" "no span events for session" "$TMP/traces_spans_miss.txt"

run_cli "$TMP/traces_spans_days.txt" traces spans --days 1
assert_contains "traces spans --days 1 仍列出今日 span" "turn.started" "$TMP/traces_spans_days.txt"

tfinish "cli_modules"
