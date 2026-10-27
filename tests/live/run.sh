#!/usr/bin/env bash
# tests/live/run.sh —— 实模型演练层入口(真实模型 + 真实工具)。
#
# 与能力覆盖层(`tests/functional/`)的关系:
#   - 能力覆盖层用 mock provider —— 确定性、离线、**CI 门禁**;
#   - 本层用**真实模型**跑同一条 TUI 链路,断言协议形状与运行时行为,
#     **不断言逐字文本** —— 默认不跑,需显式开启。
#
# 默认车道:**Kilo 公共免费池**(`https://api.kilo.ai/api/gateway`)——
# 免 Key、无需任何 API key、不产生费用。
#
# 用法:
#   ./tests/live/run.sh                                  # 默认跳过(零网络请求)
#   REFLECT_LIVE=1 ./tests/live/run.sh                   # 真跑(免费模型池)
#   REFLECT_LIVE=1 REFLECT_LIVE_MODELS=a,b ./tests/live/run.sh
#   REFLECT_LIVE=1 ./tests/live/run.sh -k full_feature   # 只跑全功能实战组(重)
#   REFLECT_LIVE=1 REFLECT_LIVE_BASE_URL=http://127.0.0.1:18900 \
#     REFLECT_LIVE_API_KEY=<forwardKey> ./tests/live/run.sh   # dsh 网关车道
#
# 退出码:0 = 全部通过(或整层跳过);1 = 有用例失败。

set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../lib.sh"

cd "$ROOT"

# release 二进制不存在则构建(幂等)
if [ ! -x "$ZHUYE_CLI_BIN" ]; then
    echo "── 构建 release 二进制 ──"
    cargo build --release -p zhuye-cli --quiet || exit 2
fi

if ! require_cmd python3; then
    echo "  [SKIP] 未找到 python3;实模型层跳过"
    exit 0
fi
if ! python3 -c "import pytest" 2>/dev/null; then
    echo "  [SKIP] 未安装 pytest(pip install pytest);实模型层跳过"
    exit 0
fi
if ! python3 -c "import pyte" 2>/dev/null; then
    echo "  [SKIP] 未安装 pyte(pip install pyte);实模型层跳过"
    exit 0
fi

# 未开启 → 整层跳过(不发起任何网络请求)。
if [ -z "${REFLECT_LIVE:-}" ] || [ "${REFLECT_LIVE}" = "0" ]; then
    echo "  [SKIP] 实模型层未启用(设 REFLECT_LIVE=1 开启;默认车道=免费模型池)"
    exit 0
fi

echo "  车道:   ${REFLECT_LIVE_BASE_URL:-https://api.kilo.ai/api/gateway}"
echo "  模型:   ${REFLECT_LIVE_MODELS:-(从 provider 清单接口实时拉取)}"

# pytest 从本目录跑,保证 `from tui_pty import ...` / `from live_lane import ...` 可解析。
# 额外参数透传:`./tests/live/run.sh -k full_feature` 只跑全功能实战组。
( cd "$HERE" && python3 -m pytest -q --tb=short "$@" )
