#!/usr/bin/env bash
# tests/rust/cli_core.sh —— 顶层 CLI 参数矩阵与输出契约。
#
# 覆盖 `reflect-tui` 顶层分派面:
#   - `--help` / `-h` / `--version` 的输出契约
#   - 全局 `--cwd` / `-C` 的路径切换语义
#   - TUI 模式参数(`--prompt` / `-c` / `-r` / `--resume` / `--plan-mode` /
#     `--ephemeral-tasks` / `--ephemeral-teams` / `--auto-root` / `--fullscreen`)
#     的**参数校验**层(clap 层,不启动 TUI)
#   - 子命令分派:`session` / `traces` 的存在性与二级子命令
#   - 错误路径:未知子命令 / 未知参数 / 缺参数 → 非 0 退出码
#
# 全程离线(REFLECT_MODEL=mock/mock-1)+ 隔离 HOME,不污染真实 ~/.reflect。
# 退出码 0 = 全部通过。

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../lib.sh"

cd "$ROOT"
require_binary

TMP="$(mktemp -d "${TMPDIR:-/tmp}/reflect-tui-cli-core.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

BIN="$REFLECT_TUI_BIN"
HOME_DIR="$TMP/home"
SESS_DIR="$HOME_DIR/.reflect/sessions"
SESSION_ID="11111111-2222-4333-8444-555555555555"

mkdir -p "$HOME_DIR/.reflect" "$SESS_DIR/2026/01/01"
printf '# 测试用最小配置:空配置。\n' > "$HOME_DIR/.reflect/config.toml"
# 预置一条会话 —— `session ls` 的表头断言需要有数据才有意义
# (空目录时输出的是 "(no sessions found …)" 占位,没有表头)。
cat > "$SESS_DIR/2026/01/01/$SESSION_ID.jsonl" <<EOF
{"type":"session_meta","session_id":"$SESSION_ID","model":"mock/mock-1","started_at":"2026-01-01T00:00:00Z"}
{"type":"message","turn_id":"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee","role":"user","content":[{"type":"text","text":"cli core probe"}]}
EOF

# run_cli <输出文件> <参数...>:在隔离 HOME 下跑一次 CLI,落 stdout+stderr。
# 返回 CLI 自身的退出码;真实退出码同时记入 LAST_RC 供断言使用。
run_cli() {
    local out="$1"; shift
    HOME="$HOME_DIR" REFLECT_MODEL=mock/mock-1 "$BIN" "$@" > "$out" 2>&1
    LAST_RC=$?
}

# ── 1. help / version 输出契约 ────────────────────────────────────────────

run_cli "$TMP/help.txt" --help
assert_contains "help 含 usage" "Usage:" "$TMP/help.txt"
assert_contains "help 列出 session 子命令" "session" "$TMP/help.txt"
assert_contains "help 列出 traces 子命令" "traces" "$TMP/help.txt"
assert_contains "help 含 cwd 全局参数" "cwd" "$TMP/help.txt"

run_cli "$TMP/help_short.txt" -h
assert_contains "短选项 -h 同样输出 usage" "Usage:" "$TMP/help_short.txt"

run_cli "$TMP/version.txt" --version
assert_matches "version 输出含版本号形状" "reflect-tui [0-9]+\.[0-9]+" "$TMP/version.txt"

# ── 2. 未知子命令 / 未知参数 → 有诊断输出 ────────────────────────────────

run_cli "$TMP/bad_sub.txt" definitely-not-a-command
assert_contains "未知子命令打印用法" "Usage:" "$TMP/bad_sub.txt"
assert_exit_nonzero "未知子命令非 0 退出" "$LAST_RC"

run_cli "$TMP/bad_flag.txt" --definitely-not-a-flag
assert_nonempty "未知参数给出错误输出" "$TMP/bad_flag.txt"
assert_exit_nonzero "未知参数非 0 退出" "$LAST_RC"

# ── 3. 子命令层级 ─────────────────────────────────────────────────────────

for sub in session traces workspace discussion security; do
    run_cli "$TMP/${sub}_help.txt" "$sub" --help
    assert_contains "$sub --help 含 usage" "Usage:" "$TMP/${sub}_help.txt"
done

run_cli "$TMP/session_help.txt" session --help
assert_contains "session 列出 ls" "ls" "$TMP/session_help.txt"
assert_contains "session 列出 show" "show" "$TMP/session_help.txt"
assert_contains "session 列出 rm" "rm" "$TMP/session_help.txt"
assert_contains "session 列出 fork" "fork" "$TMP/session_help.txt"
assert_contains "session 列出 rename" "rename" "$TMP/session_help.txt"
assert_contains "session 列出 export" "export" "$TMP/session_help.txt"
assert_contains "session 列出 stats" "stats" "$TMP/session_help.txt"
# v1.5 接线:跨会话全文搜索(reflect_rollout::search_sessions)。
assert_contains "session 列出 search" "search" "$TMP/session_help.txt"

# ── 3b. session search:跨会话全文搜索 ──────────────────────────────────────

run_cli "$TMP/search_hit.txt" session search "cli core probe"
assert_contains "session search 命中预置会话" "cli core probe" "$TMP/search_hit.txt"
assert_contains "session search 输出 session 前缀" "11111111" "$TMP/search_hit.txt"

run_cli "$TMP/search_miss.txt" session search "zzz-not-present-zzz"
assert_contains "session search 未命中给出明确说明" "no matches" "$TMP/search_miss.txt"

run_cli "$TMP/search_json.txt" session search --json "cli core probe"
assert_contains "session search --json 输出 JSON 行" "\"session_id\"" "$TMP/search_json.txt"

run_cli "$TMP/traces_help.txt" traces --help
assert_contains "traces 列出 ls" "ls" "$TMP/traces_help.txt"
assert_contains "traces 列出 show" "show" "$TMP/traces_help.txt"
assert_contains "traces 列出 spans" "spans" "$TMP/traces_help.txt"

# v1.x 接线:workspace clone(reflect_stream::workspace_sync 共享 API)。
run_cli "$TMP/workspace_help.txt" workspace --help
assert_contains "workspace 列出 clone" "clone" "$TMP/workspace_help.txt"

run_cli "$TMP/workspace_clone_help.txt" workspace clone --help
assert_contains "workspace clone 列出 --dest" "--dest" "$TMP/workspace_clone_help.txt"
assert_contains "workspace clone 列出 --branch" "--branch" "$TMP/workspace_clone_help.txt"

# 空 URL → 立即失败(git clone 之前就被共享 API 拒掉),不挂死。
run_cli "$TMP/workspace_clone_bad.txt" workspace clone ""
assert_nonempty "workspace clone 空 url 有诊断输出" "$TMP/workspace_clone_bad.txt"
assert_exit_nonzero "workspace clone 空 url 非 0 退出" "$LAST_RC"

# v1.x 接线:多 agent 讨论(reflect_discussion::cli::run)与安全扫描
# (reflect_integration::security_scan)。
run_cli "$TMP/discussion_help.txt" discussion --help
assert_contains "discussion 列出 run" "run" "$TMP/discussion_help.txt"

run_cli "$TMP/discussion_run_help.txt" discussion run --help
assert_contains "discussion run 列出 --config" "--config" "$TMP/discussion_run_help.txt"

run_cli "$TMP/security_help.txt" security --help
assert_contains "security 列出 audit" "audit" "$TMP/security_help.txt"

run_cli "$TMP/security_audit_help.txt" security audit --help
assert_contains "security audit 列出 --workspace" "--workspace" "$TMP/security_audit_help.txt"

# 注:不实际跑 `security audit` —— cargo-audit 存在时要联网拉 advisory
# 数据库,会让本套件(全程离线的参数面门禁)变慢且不稳定。这里只锁参数面。

# ── 4. 缺参数 → 有诊断输出(clap 校验层)──────────────────────────────────

for args in "session show" "session rm" "session fork" "session rename" \
            "session export" "session search" "traces show" "workspace clone" \
            "discussion run"; do
    # shellcheck disable=SC2086
    run_cli "$TMP/missing.txt" $args
    assert_nonempty "缺参数有诊断输出: $args" "$TMP/missing.txt"
    assert_exit_nonzero "缺参数非 0 退出: $args" "$LAST_RC"
done

# ── 5. 全局 --cwd / -C 生效 ──────────────────────────────────────────────

mkdir -p "$TMP/workdir"
run_cli "$TMP/cwd.txt" --cwd "$TMP/workdir" session ls
assert_contains "session ls 输出含会话表头" "session_id" "$TMP/cwd.txt"
assert_contains "--cwd 后仍列出隔离 HOME 的会话" "$SESSION_ID" "$TMP/cwd.txt"

run_cli "$TMP/cwd_short.txt" -C "$TMP/workdir" session ls
assert_contains "-C 短选项等价 --cwd" "session_id" "$TMP/cwd_short.txt"

# ── 6. 非法 --cwd → 有诊断输出 ────────────────────────────────────────────

run_cli "$TMP/cwd_bad.txt" --cwd "$TMP/definitely-missing-dir" session ls
assert_nonempty "非法 --cwd 给出错误输出" "$TMP/cwd_bad.txt"
assert_exit_nonzero "非法 --cwd 非 0 退出" "$LAST_RC"

# ── 7. TUI 模式参数的存在性(clap 解析层)────────────────────────────────

run_cli "$TMP/tui_help.txt" --help
assert_contains "TUI 参数含 prompt" "prompt" "$TMP/tui_help.txt"
assert_contains "TUI 参数含 resume" "resume" "$TMP/tui_help.txt"
assert_contains "TUI 参数含 plan-mode" "plan-mode" "$TMP/tui_help.txt"
assert_contains "TUI 参数含 fullscreen" "fullscreen" "$TMP/tui_help.txt"

# ── 8. TUI 模式参数在子命令存在时被忽略(不报错)──────────────────────────

# `TuiArgs` 被 flatten 进顶层;带子命令时这些参数被忽略(clap 允许,
# 因为 group 只在 TUI 模式下有意义)。断言不因它们而失败。
#
# 注意:`TuiArgs` 的 group 是 `multiple = false` —— `--plan-mode` 与
# `--fullscreen` **互斥**,同时给会被 clap 拒绝(这也是一条契约)。
run_cli "$TMP/tui_with_sub.txt" --plan-mode session ls
assert_contains "TUI 参数与子命令共存时仍走子命令" "session_id" "$TMP/tui_with_sub.txt"

run_cli "$TMP/tui_conflict.txt" --plan-mode --fullscreen session ls
assert_contains "互斥的 TUI 参数被 clap 拒绝" "cannot be used with" "$TMP/tui_conflict.txt"

tfinish "cli_core"
