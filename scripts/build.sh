#!/usr/bin/env bash
# ── Reflect-TUI 跨平台构建脚本 ───────────────────────────────────────
# 编译 `reflect-tui` 二进制,可选打包成平台分发包。
#
# 核心 crate 由 `reflect-agent/` submodule 提供,打包前会检查 submodule
# 是否已初始化,未初始化时提示 `git submodule update --init --recursive`。
#
# 用法:
#   ./scripts/build.sh                            # 编译(release,默认)
#   ./scripts/build.sh --fast                     # release-fast profile(无 LTO,快)
#   ./scripts/build.sh --install                  # 编译 + 安装到 /usr/local/bin(需 sudo)
#   ./scripts/build.sh --dist                     # 编译 + 打包成平台包到 dist/
#   ./scripts/build.sh --dist --target=<triple>   # 交叉编译 + 打包
#   ./scripts/build.sh --dry-run                  # 只打印命令不执行
#   ./scripts/build.sh --help
#
# --dist 打包格式(三平台统一压缩包,TUI 作为终端工具无需 DMG 拖拽体验):
#   macOS    → tar.gz
#   Linux    → tar.gz
#   Windows  → zip
# ────────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BINARY_NAME="reflect-tui"
INSTALL_DIR="/usr/local/bin"

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

选项:
  --fast             用 release-fast profile(无 LTO,增量编译快)
                     默认: release(LTO,体积小)
  --install          编译 + 安装到 ${INSTALL_DIR}/(需 sudo)
  --dist             编译 + 打包成平台分发包到 dist/
  --target=TRIPLE    指定 rustc target(交叉编译)
  --dry-run          只打印命令不实际执行
  -h, --help         显示本帮助

--dist 各平台打包格式:
  macOS / Linux → tar.gz
  Windows       → zip

示例:
  ./scripts/build.sh                          # 编译(release)
  ./scripts/build.sh --fast                   # 快速编译(开发循环)
  ./scripts/build.sh --dist                   # 打包当前平台分发包
  ./scripts/build.sh --dist --target=x86_64-unknown-linux-gnu
EOF
}

# ── 默认值 ────────────────────────────────────────────────────────────
USE_FAST=false
DO_INSTALL=false
DO_DIST=false
DRY_RUN=false
TARGET_OVERRIDE=""

# ── 参数解析 ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    arg="$1"
    case "$arg" in
        --fast)     USE_FAST=true; shift ;;
        --install)  DO_INSTALL=true; shift ;;
        --dist)     DO_DIST=true; shift ;;
        --dry-run)  DRY_RUN=true; shift ;;
        --target)   TARGET_OVERRIDE="${2:?--target requires a value}"; shift 2 ;;
        --target=*) TARGET_OVERRIDE="${arg#--target=}"; shift ;;
        -h|--help)  usage; exit 0 ;;
        *)          echo "未知选项: $1" >&2; usage; exit 1 ;;
    esac
done

# ── 辅助:打印 + 执行(尊重 DRY_RUN)──────────────────────────────────
run() {
    echo "+ $*"
    if ! $DRY_RUN; then
        eval "$@"
    fi
}

# ── OS 检测 ───────────────────────────────────────────────────────────
OS_NAME="$(uname -s)"
case "$OS_NAME" in
    Darwin)         PLATFORM="macos" ;;
    Linux)          PLATFORM="linux" ;;
    MINGW*|MSYS*|CYGWIN*) PLATFORM="windows" ;;
    *)
        echo "不支持的 OS: $OS_NAME" >&2
        exit 1
        ;;
esac

# ── submodule 检查:TUI 依赖 reflect-agent/ submodule 提供核心 crate ──
SUBMODULE_DIR="${REPO_ROOT}/reflect-agent"
if [[ ! -f "${SUBMODULE_DIR}/Cargo.toml" ]]; then
    echo "✗ reflect-agent submodule 未初始化:缺少 ${SUBMODULE_DIR}/Cargo.toml" >&2
    echo "  请先运行:git submodule update --init --recursive" >&2
    if ! $DRY_RUN; then
        exit 1
    fi
    echo "  (dry-run 模式:继续)" >&2
fi

# ── profile 决策 ──────────────────────────────────────────────────────
if $USE_FAST; then
    PROFILE="release-fast"
    CARGO_PROFILE_FLAG="--profile release-fast"
    BIN_SUBDIR="release-fast"
else
    PROFILE="release"
    CARGO_PROFILE_FLAG="--release"
    BIN_SUBDIR="release"
fi

# ── target 参数(交叉编译)────────────────────────────────────────────
TARGET_ARGS=()
TARGET_SUBDIR=""
if [[ -n "$TARGET_OVERRIDE" ]]; then
    TARGET_ARGS=(--target "$TARGET_OVERRIDE")
    TARGET_SUBDIR="${TARGET_OVERRIDE}/"
fi

# 版本号:从 root Cargo.toml [workspace.package] 提取
extract_version() {
    awk -F'=' '
        /^\[workspace\.package\]/ { in_pkg=1; next }
        /^\[/ { in_pkg=0 }
        in_pkg && $1 ~ /^version[[:space:]]*$/ { gsub(/[ "]/,"",$2); print $2; exit }
    ' "${REPO_ROOT}/Cargo.toml"
}
VERSION="$(extract_version)"

# 二进制产物路径(Windows 带 .exe)
case "$PLATFORM" in
    windows) BIN_PATH="${REPO_ROOT}/target/${TARGET_SUBDIR}${BIN_SUBDIR}/${BINARY_NAME}.exe" ;;
    *)       BIN_PATH="${REPO_ROOT}/target/${TARGET_SUBDIR}${BIN_SUBDIR}/${BINARY_NAME}" ;;
esac

echo "── Reflect-TUI 构建 ──────────────────────────────────────"
echo "平台:          $PLATFORM ($OS_NAME)"
echo "profile:       $PROFILE"
echo "target:        ${TARGET_OVERRIDE:-<host>}"
echo "version:       ${VERSION:-<unknown>}"
echo "dist:          $DO_DIST"
echo "install:       $DO_INSTALL"
echo "dry-run:       $DRY_RUN"
echo "───────────────────────────────────────────────────────────"

# ── 工具链检查 ────────────────────────────────────────────────────────
need_cmd() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "缺少必需工具: $1" >&2
        exit 1
    }
}
need_cmd cargo
need_cmd rustc

# ── 编译 ──────────────────────────────────────────────────────────────
echo "→ 编译 ${BINARY_NAME} (${PROFILE}) ..."
run "cargo build ${CARGO_PROFILE_FLAG} ${TARGET_ARGS[*]:-} -p ${BINARY_NAME}"

# ── 安装分支 ──────────────────────────────────────────────────────────
if $DO_INSTALL; then
    if [[ ! -e "${BIN_PATH}" ]]; then
        echo "✗ 编译产物不存在: ${BIN_PATH}" >&2
        exit 1
    fi
    echo "→ 安装到 ${INSTALL_DIR}/${BINARY_NAME} ..."
    if [[ ! -w "${INSTALL_DIR}" ]]; then
        run "sudo install -m 0755 '${BIN_PATH}' '${INSTALL_DIR}/${BINARY_NAME}'"
    else
        run "install -m 0755 '${BIN_PATH}' '${INSTALL_DIR}/${BINARY_NAME}'"
    fi
    if ! $DRY_RUN; then
        echo ""
        echo "✓ 安装成功。运行 '${BINARY_NAME} --help' 查看用法。"
    fi
    exit 0
fi

# ── 打包分支(--dist)─────────────────────────────────────────────────
if $DO_DIST; then
    if [[ ! -e "${BIN_PATH}" ]]; then
        echo "✗ 编译产物不存在: ${BIN_PATH}" >&2
        exit 1
    fi

    # 确定 target triple
    if [[ -n "$TARGET_OVERRIDE" ]]; then
        TRIPLE="$TARGET_OVERRIDE"
    else
        ARCH="$(uname -m)"
        case "$OS_NAME" in
            Darwin) TRIPLE="${ARCH}-apple-darwin" ;;
            Linux)  TRIPLE="${ARCH}-unknown-linux-gnu" ;;
            *)      TRIPLE="${ARCH}-pc-windows-msvc" ;;
        esac
    fi

    PKG_NAME="${BINARY_NAME}-${VERSION}-${TRIPLE}"
    DIST_DIR="${REPO_ROOT}/dist"
    STAGING_DIR="$(mktemp -d -t reflect-tui-dist)"
    PKG_DIR="${STAGING_DIR}/${PKG_NAME}"
    mkdir -p "${PKG_DIR}" "${DIST_DIR}"

    # staging 内容:二进制 + LICENSE + README.txt
    cp "${BIN_PATH}" "${PKG_DIR}/"
    [[ -f "${REPO_ROOT}/LICENSE" ]] && cp "${REPO_ROOT}/LICENSE" "${PKG_DIR}/" || true
    cat > "${PKG_DIR}/README.txt" <<README_EOF
Reflect-TUI ${VERSION}  (${TRIPLE})
=========================================================

Reflect-TUI 是 Reflect Agent 的终端交互界面(TUI)。

【安装】解压后把二进制放到 PATH:
    tar xzf ${PKG_NAME}.tar.gz        # 或解压 zip
    install -m 0755 ${PKG_NAME}/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}

【验证】
    ${BINARY_NAME} --help
    ${BINARY_NAME}                     # 启动 TUI

【环境变量】需要 OPENAI_API_KEY 或 ANTHROPIC_API_KEY。
详见 README.md。
README_EOF

    case "$PLATFORM" in
        macos|linux)
            ARCHIVE_PATH="${DIST_DIR}/${PKG_NAME}.tar.gz"
            echo "→ 打包 tar.gz ..."
            run "(cd '${STAGING_DIR}' && tar -czf '${ARCHIVE_PATH}' '${PKG_NAME}')"
            ;;
        windows)
            ARCHIVE_PATH="${DIST_DIR}/${PKG_NAME}.zip"
            echo "→ 打包 zip ..."
            if command -v zip >/dev/null 2>&1; then
                run "(cd '${STAGING_DIR}' && zip -r -q '${ARCHIVE_PATH}' '${PKG_NAME}')"
            else
                PS_STAGING="$(cygpath -w "${STAGING_DIR}" 2>/dev/null || echo "${STAGING_DIR}")"
                PS_ARCHIVE="$(cygpath -w "${ARCHIVE_PATH}" 2>/dev/null || echo "${ARCHIVE_PATH}")"
                run "powershell.exe -NoProfile -Command \"Compress-Archive -Path '${PS_STAGING}\\\\${PKG_NAME}' -DestinationPath '${PS_ARCHIVE}' -Force\""
            fi
            ;;
    esac

    if ! $DRY_RUN; then
        rm -rf "${STAGING_DIR}"
        echo ""
        echo "── 产物 ────────────────────────────────────────────────────"
        for entry in "${DIST_DIR}"/*; do
            [[ -e "$entry" ]] || continue
            size="$(du -h "$entry" 2>/dev/null | cut -f1)"
            printf "  %8s  %s\n" "$size" "$entry"
        done
        echo "───────────────────────────────────────────────────────────"
    fi
    exit 0
fi

# ── 默认:仅编译 ──────────────────────────────────────────────────────
if ! $DRY_RUN; then
    echo "✓ 编译完成: ${BIN_PATH}"
fi
