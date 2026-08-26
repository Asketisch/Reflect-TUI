#!/usr/bin/env bash
# ── scripts/install.sh ───────────────────────────────────────────────
# Reflect-TUI 全局安装脚本:把 reflect-tui 二进制装到 PATH。
#
# 优先从已编译的 target/release/reflect-tui 安装;若不存在,自动
# 触发 cargo build --release。
#
# 安装位置:
#   默认 /usr/local/bin(需 sudo)
#   --local  → ~/.local/bin(无需 sudo,自动追加到 PATH)
#
# 用法:
#   ./scripts/install.sh                  # 装到 /usr/local/bin(需 sudo)
#   ./scripts/install.sh --local          # 装到 ~/.local/bin(无需 sudo)
#   ./scripts/install.sh --install-dir /opt/bin
# ────────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN_NAME="reflect-tui"

INSTALL_DIR="/usr/local/bin"
SUDO=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --local)
            INSTALL_DIR="${HOME}/.local/bin"
            shift ;;
        --install-dir)
            INSTALL_DIR="${2:?--install-dir needs a value}"
            shift 2 ;;
        -y|--yes) shift ;;
        -h|--help)
            sed -n '2,18p' "$0"; exit 0 ;;
        *) echo "未知选项: $1" >&2; exit 1 ;;
    esac
done

# ── 定位源二进制:优先 release,其次 release-fast,都没有则编译 ────────
SOURCE_BIN=""
for profile in release release-fast; do
    candidate="${REPO_ROOT}/target/${profile}/${BIN_NAME}"
    if [[ -x "${candidate}" ]]; then
        SOURCE_BIN="${candidate}"
        break
    fi
done

if [[ -z "${SOURCE_BIN}" ]]; then
    echo "→ 未找到已编译的 ${BIN_NAME},触发 cargo build --release ..."
    (cd "${REPO_ROOT}" && cargo build --release -p "${BIN_NAME}")
    SOURCE_BIN="${REPO_ROOT}/target/release/${BIN_NAME}"
fi

if [[ ! -x "${SOURCE_BIN}" ]]; then
    echo "✗ 二进制不可执行: ${SOURCE_BIN}" >&2
    exit 1
fi

# /usr/local/bin 通常需要 root;~/.local/bin 不需要
if [[ "${INSTALL_DIR}" == /usr/local/bin ]] && [[ ! -w "${INSTALL_DIR}" ]]; then
    SUDO="sudo"
fi
mkdir -p "${INSTALL_DIR}" 2>/dev/null || ${SUDO} mkdir -p "${INSTALL_DIR}"

echo "→ 安装 ${BIN_NAME} → ${INSTALL_DIR}/${BIN_NAME}"
if [[ -n "${SUDO}" ]]; then
    ${SUDO} install -m 0755 "${SOURCE_BIN}" "${INSTALL_DIR}/${BIN_NAME}"
else
    install -m 0755 "${SOURCE_BIN}" "${INSTALL_DIR}/${BIN_NAME}"
fi

# ~/.local/bin 需确保在 PATH 里
if [[ "${INSTALL_DIR}" == "${HOME}/.local/bin" ]]; then
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            for rc in "${HOME}/.zshrc" "${HOME}/.bashrc"; do
                [[ -f "${rc}" ]] || continue
                if ! grep -q "export PATH=\"${INSTALL_DIR}:\$PATH\"" "${rc}"; then
                    printf '\n# Added by Reflect-TUI installer\nexport PATH="%s:$PATH"\n' "${INSTALL_DIR}" >> "${rc}"
                    echo "→ 已把 ${INSTALL_DIR} 加入 $(basename "${rc}") 的 PATH"
                fi
            done
            export PATH="${INSTALL_DIR}:${PATH}"
            ;;
    esac
fi

# 健康检查
if command -v "${BIN_NAME}" >/dev/null 2>&1; then
    echo ""
    echo "✓ 已安装: $(${BIN_NAME} --version 2>/dev/null || echo "${BIN_NAME}")"
    echo "  位置:    $(command -v "${BIN_NAME}")"
    echo ""
    echo "  运行 '${BIN_NAME} --help' 查看用法。"
else
    echo ""
    echo "✓ 已安装到 ${INSTALL_DIR}/${BIN_NAME}"
    echo "  (新开终端或标签页以刷新 PATH)"
fi
