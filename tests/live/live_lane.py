"""实模型演练层 · 免费模型车道设施(清单发现 / 能力探测 / 配置注入)。

与 submodule `reflect-agent/tests/live/` 同款车道,但驱动对象不同:
那边驱动 headless `reflect serve`(JSONL wire),这边驱动**终端 TUI**
(pty + pyte 屏幕快照)—— 断言落在「用户真的看到了什么」。

## 车道是什么

「车道」= (端点, 凭据, 模型集合) 的三元组。

| 车道 | 端点 | 凭据 | 前置条件 |
|---|---|---|---|
| **Kilo 免费池**(默认) | `https://api.kilo.ai/api/gateway` | 免 Key | 无 |
| **dsh 本地网关** | `http://127.0.0.1:18900` | `forwardKey` | 起 submodule 的 `scripts/ofm_gateway.sh` |

Kilo 车道**零外部依赖** —— 只改 `config.toml` 就能用。dsh 网关那条是
OpenCode inference 车道:v2 客户端指纹门(UA / 会话头 / 逐字 read+shell
工具定义)只有网关会补齐,见 submodule `docs/FREE_MODEL_PRACTICE.md`。

## 模型从哪来(不写死)

免费模型集合由**上游**决定,随时可能增删 —— 写死 id 必然腐烂:

1. `REFLECT_LIVE_MODELS` 给了逗号分隔列表 → 用它;
2. 否则调 `reflect-tui models list --free-only --json` 拿清单 ——
   免费判定规则只有一份实现(在 Rust 侧 `reflect_llm::FreeModelRules`),
   不在 Python 里复写;
3. 逐个做**能力探测**(带 tools 的最小请求)→ 不支持 tools 的模型标为
   **跳过**而不是失败;
4. 取前 N 个(`REFLECT_LIVE_MODEL_COUNT`)。

## 隔离与安全

- 每个用例一份独立 HOME + cwd(`~/.zhuye/config.toml` 由本模块写入);
- **清掉宿主环境里的 provider 凭据与 `ZHUYE_MODEL`** —— 否则宿主的
  付费 key 会串进免费车道,宿主的 `ZHUYE_MODEL=mock/...` 会把真模型顶掉;
- 场景只喂**合成数据**,不把真实仓库内容 / 凭据喂给免费池(上游可能记录 prompt)。
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

from tui_pty import REFLECT_TUI_BIN

# 默认车道:Kilo 公共免费池(免 Key、无客户端指纹门)。
DEFAULT_BASE_URL = "https://api.kilo.ai/api/gateway"
DEFAULT_API_KEY = ""

# 清单 / 探测 / 等待预算。免费池实测首字 >30s,思考阶段可静默 60~70s,
# 因此 live 层的事件等待预算比 mock 层宽得多。
PROBE_TIMEOUT = float(os.environ.get("REFLECT_LIVE_PROBE_TIMEOUT", "90"))
DRAIN_TIMEOUT = float(os.environ.get("REFLECT_LIVE_DRAIN_TIMEOUT", "240"))
MODEL_COUNT = int(os.environ.get("REFLECT_LIVE_MODEL_COUNT", "3"))
RETRIES = int(os.environ.get("REFLECT_LIVE_RETRIES", "2"))
RETRY_DELAY = float(os.environ.get("REFLECT_LIVE_RETRY_DELAY", "5"))

# 宿主环境里必须清掉的变量:宿主的付费 key 会串进免费车道,宿主的
# `ZHUYE_MODEL=mock/...` 会把真模型顶掉。置空即可 ——
# `reflect-config` 侧对空串按「未设置」处理。
HOST_CRED_VARS = (
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "ZHUYE_MODEL",
    "ZHUYE_PROVIDER",
)


@dataclass(frozen=True)
class Lane:
    """一条车道:端点 + 凭据。"""

    base_url: str
    api_key: str


@dataclass(frozen=True)
class ModelEntry:
    """矩阵里的一个模型:id + 是否支持 tools + 判定理由。"""

    model: str
    supports_tools: bool
    reason: str


def live_enabled() -> bool:
    """是否启用实模型层(默认关闭 —— 不发起任何网络请求)。"""
    return os.environ.get("REFLECT_LIVE", "").strip() not in ("", "0", "false")


def current_lane() -> Lane:
    """当前车道。

    - 默认:**Kilo 公共免费池**(免 Key、零外部依赖);
    - `REFLECT_LIVE_LANE=dsh`:切到本机 dsh-our-free-model 网关(自动探测
      端口与 forwardKey);网关没起 → 回落到 Kilo 车道(并保持免 Key);
    - `REFLECT_LIVE_BASE_URL` / `REFLECT_LIVE_API_KEY`:显式覆盖(优先)。
    """
    explicit_url = os.environ.get("REFLECT_LIVE_BASE_URL", "").strip()
    explicit_key = os.environ.get("REFLECT_LIVE_API_KEY")
    if explicit_url:
        return Lane(
            base_url=explicit_url,
            api_key=explicit_key if explicit_key is not None else DEFAULT_API_KEY,
        )
    if os.environ.get("REFLECT_LIVE_LANE", "").strip().lower() == "dsh":
        endpoint = dsh_gateway_endpoint()
        if endpoint is not None:
            return endpoint
    return Lane(
        base_url=DEFAULT_BASE_URL,
        api_key=explicit_key if explicit_key is not None else DEFAULT_API_KEY,
    )


def dsh_gateway_endpoint() -> Lane | None:
    """探测本机 dsh-our-free-model 网关 → (base_url, forwardKey)。

    端口与密钥都从 `~/.our-free-model/settings.json` 取 —— **不要写死
    18900**:该端口常被别的应用占用,网关自己会顺延(实测本机跑在 18902,
    而 18900 是另一个 node 服务)。端口没监听 → 返回 None。
    """
    settings = Path(os.path.expanduser("~/.our-free-model/settings.json"))
    try:
        data = json.loads(settings.read_text())
    except Exception:  # noqa: BLE001 - 没装 / 没配置 → 视为无网关
        return None
    port = data.get("standalonePort")
    key = data.get("forwardKey")
    if not port or not key:
        return None
    url = f"http://127.0.0.1:{int(port)}"
    req = urllib.request.Request(
        f"{url}/v1/models",
        headers={"Authorization": f"Bearer {key}"},
    )
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            if resp.status != 200:
                return None
    except Exception:  # noqa: BLE001 - 端口没监听 / 鉴权失败 → 无网关
        return None
    return Lane(base_url=url, api_key=str(key))


def clean_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    """派生不含宿主凭据的子进程 env。

    ⚠️ **必须剔除 `HOME`**:`Sandbox.env()` 的顺序是「先设 `HOME=隔离目录`,
    再叠加 extra」—— extra 里若带宿主 `HOME`,会把隔离 HOME **覆盖掉**,
    子进程就会读到用户真实的 `~/.zhuye/config.toml`(provider / model
    全部串味)。隔离 HOME 由调用方显式指定。
    """
    env = dict(os.environ)
    for var in HOST_CRED_VARS:
        env.pop(var, None)
    # HOME 归沙箱管 —— 绝不能从宿主继承。
    env.pop("HOME", None)
    # 屏蔽启动期 GitHub releases 探测 —— 离线/内网环境不卡启动。
    env["REFLECT_TUI_UPDATE_CHECK"] = "0"
    if extra:
        env.update(extra)
    return env


# ── 模型清单(从 provider 接口获取)──────────────────────────────────────────


def fetch_free_models(lane: Lane | None = None) -> list[str]:
    """拉取 provider 的免费模型清单(返回 model id 列表)。

    走 `reflect-tui models list --free-only --json` —— 免费判定规则只有
    一份实现(在 Rust 侧 `reflect_llm::FreeModelRules`),不在 Python 里
    重写一遍,避免规则漂移。
    """
    lane = lane or current_lane()
    with tempfile.TemporaryDirectory(prefix="reflect-live-models-") as tmp:
        home = Path(tmp) / "home"
        (home / ".zhuye").mkdir(parents=True)
        # 清单命令只需要端点;model 写 "free" 让 `models pick` 语义一致。
        write_lane_config(home, model="free", lane=lane)
        env = clean_env({"HOME": str(home)})
        proc = subprocess.run(
            [str(REFLECT_TUI_BIN), "models", "list", "--free-only", "--json"],
            env=env,
            capture_output=True,
            text=True,
            timeout=180,
        )
        if proc.returncode != 0:
            raise RuntimeError(
                f"reflect-tui models list 失败(rc={proc.returncode}):{proc.stderr.strip()}"
            )
        payload = json.loads(proc.stdout)
    ids = [m.get("id") for m in payload.get("models", []) if m.get("id")]
    return [i for i in ids if isinstance(i, str)]


def probe_tool_support(lane: Lane, model: str) -> tuple[bool, str]:
    """能力探测:该模型是否支持 tools。

    返回 `(supported, reason)`。**关键区分**:provider 拒绝 tools(400)
    与上游限流 / 故障(429/5xx/网络)是两回事 —— 前者说明「该模型不支持
    tools」,后者只说明「现在问不了」。把 429 当成「不支持 tools」会误杀
    本来可用的模型。
    """
    body = json.dumps(
        {
            "model": model,
            "messages": [{"role": "user", "content": "Say OK"}],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "probe_tool",
                        "description": "probe",
                        "parameters": {
                            "type": "object",
                            "properties": {"x": {"type": "string"}},
                        },
                    },
                }
            ],
            "max_tokens": 64,
            "stream": False,
        }
    ).encode()
    req = urllib.request.Request(
        f"{lane.base_url.rstrip('/')}/v1/chat/completions",
        data=body,
        method="POST",
        headers={"Content-Type": "application/json"},
    )
    if lane.api_key:
        req.add_header("Authorization", f"Bearer {lane.api_key}")
    try:
        with urllib.request.urlopen(req, timeout=PROBE_TIMEOUT) as resp:
            return resp.status == 200, f"http_{resp.status}"
    except urllib.error.HTTPError as exc:
        detail = ""
        try:
            detail = exc.read().decode("utf-8", "replace")[:160]
        except Exception:  # noqa: BLE001 - 诊断信息尽力而为
            pass
        if exc.code == 429 or exc.code >= 500:
            # 限流 / 上游故障 —— 「暂时问不了」,不是「不支持 tools」。
            return True, f"unavailable_http_{exc.code}"
        return False, f"http_{exc.code}:{detail}"
    except Exception as exc:  # noqa: BLE001 - 网络异常:同样按「暂时问不了」处理
        return True, f"unavailable:{exc}"


def resolve_matrix(lane: Lane | None = None) -> list[ModelEntry]:
    """解析本次要跑的模型矩阵。

    返回 `[ModelEntry]`(顺序确定:清单按 id 字典序)。
    """
    lane = lane or current_lane()
    explicit = os.environ.get("REFLECT_LIVE_MODELS", "").strip()
    if explicit:
        candidates = [m.strip() for m in explicit.split(",") if m.strip()]
        probed: list[ModelEntry] = []
        for model in candidates:
            ok, reason = probe_tool_support(lane, model)
            probed.append(ModelEntry(model, ok, reason))
        return probed

    entries = fetch_free_models(lane)
    if not entries:
        raise RuntimeError(
            "provider 清单里没有免费模型;检查 REFLECT_LIVE_BASE_URL / 出口地区"
        )
    picked: list[ModelEntry] = []
    for model in sorted(entries):
        if len(picked) >= MODEL_COUNT:
            break
        ok, reason = probe_tool_support(lane, model)
        picked.append(ModelEntry(model, ok, reason))
    return picked


def select_model(matrix: list[ModelEntry]) -> ModelEntry:
    """从矩阵里挑一个**能跑工具**的模型。

    优先 `REFLECT_LIVE_MODEL` 指定的;否则取第一个支持 tools 的;都没有
    就取第一个(让用例自己撞上 400,而不是在这里静默换模型)。
    """
    wanted = os.environ.get("REFLECT_LIVE_MODEL", "").strip()
    if wanted:
        for entry in matrix:
            if entry.model == wanted:
                return entry
        return ModelEntry(wanted, False, "not_in_matrix")
    for entry in matrix:
        if entry.supports_tools:
            return entry
    return matrix[0]


# ── 配置注入 ───────────────────────────────────────────────────────────────


def write_lane_config(
    home: Path,
    model: str,
    lane: Lane | None = None,
    fallbacks: list[str] | None = None,
) -> Path:
    """把车道配置写进隔离 HOME 的 `~/.zhuye/config.toml`。

    - `model` 写具体 id(由矩阵选出)或 `"free"`(运行时从清单挑);
    - `fallbacks` 是**跨模型**降级链(429 / 全冷却时自动换模型继续本轮),
      元素写具体 model id;
    - `timeout_secs = 300` —— reqwest 的 timeout 是「建连→body 读完」的
      **总**时长,默认 60s 会把慢思考的流式响应腰斩。
    """
    lane = lane or current_lane()
    fallbacks = fallbacks or []
    lines = [
        "# 实模型演练层配置(合成数据专用,不含真实仓库内容)",
        "[hooks]",
        "enabled = []",
        "",
        "[active]",
        'provider = "openai"',
        "",
        "[model_discovery]",
        "enabled = true",
        "",
        "[openai]",
        f'api_key = "{lane.api_key}"',
        f'base_url = "{lane.base_url}"',
        "timeout_secs = 300",
        f'model = "{model}"',
        "",
        "[[openai.credentials]]",
        'label = "live"',
        f'api_key = "{lane.api_key}"',
        f'base_url = "{lane.base_url}"',
        f'model = "{model}"',
    ]
    if fallbacks:
        specs = ", ".join(f'"openai/{fb}"' for fb in fallbacks)
        lines += [
            "",
            "[routing]",
            "max_attempts = 8",
            "",
            "[routing.main]",
            f'primary = "openai/{model}"',
            f"fallbacks = [{specs}]",
        ]
    path = home / ".zhuye" / "config.toml"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n")
    return path


# ── 会话产物解析(演练报告 / 回合判据用)──────────────────────────────────

# HUD 忙碌 spinner 的字形(history_render/statusline.rs:20 `SPINNER_FRAMES`)。
SPINNER_FRAMES = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"


def is_busy(screen: str) -> bool:
    """屏幕是否处于「回合进行中」。

    HUD 的 identity 行在忙碌时前置一个 spinner(`⠹ model · … Ctrl+C to quit`),
    空闲时没有 —— 这是唯一稳定的「忙 / 不忙」可观测点。
    """
    for line in screen.splitlines():
        s = line.strip()
        if "Ctrl+C to quit" in s and s:
            return s[0] in SPINNER_FRAMES
    return False


def safe_text(session) -> str:
    """读屏幕归一化文本;第三方屏幕解析器(pyte)偶发抛异常时**不**带崩用例。

    live 层是轮询式读屏(每 0.5s 一次),一次解析异常不该让整条用例失败 ——
    读不到就当「这一轮没变化」,下一轮继续。
    """
    try:
        return session.normalized()
    except Exception:  # noqa: BLE001 - 读屏尽力而为
        return ""


# 上游限流的屏幕锚点。免费池是**全局配额**,429 是常态 ——
# 测的是代码,不是配额,所以限流一律判「跳过」而不是失败。
RATE_LIMIT_MARKERS = (
    "rate limited",
    "all credentials exhausted",
    "credentials exhausted",
    "too many requests",
    "retry after",
    # Kilo 免费池的**按出口 IP 推广上限**(实测:所有模型一起 429)。
    "promotion_model_limit",
    # dsh 网关的匿名通道配额用尽 / 地区门。
    "free usage limit reached",
    "region_blocked",
    "not available in your country",
)


def rate_limited(screen: str) -> bool:
    """屏幕是否显示上游限流 / 凭据耗尽。"""
    low = screen.lower()
    return any(marker in low for marker in RATE_LIMIT_MARKERS)


def session_files(home: Path) -> list[Path]:
    """隔离 HOME 下的会话 JSONL 列表。"""
    return sorted((home / ".zhuye" / "sessions").glob("*/*/*/*.jsonl"))


def assistant_message_count(home: Path) -> int:
    """已落盘的 assistant 消息条数(回合收尾的判据之一)。"""
    return session_stats(session_files(home))["assistant_messages"]


def session_stats(session_file_list: list[Path]) -> dict:
    """从会话 JSONL 汇总 token / 工具调用 / 实际服务模型。

    记录形状(见 `reflect_protocol::RolloutRecord`):
    - `{"type":"token_count","usage":{...},"model":"openai/<id>"}`;
    - `{"type":"message","role":"assistant","content":...}`。

    `model` 取自 token_count 记录 —— 它是**实际服务该轮**的模型(降级链
    换模型后这里会变),比配置里写的 primary 更可信。
    """
    usage = {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}
    models: list[str] = []
    assistant_messages = 0
    for path in session_file_list:
        try:
            lines = path.read_text(errors="replace").splitlines()
        except OSError:
            continue
        for line in lines:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except Exception:  # noqa: BLE001 - 坏行跳过,统计不因单行失败
                continue
            rtype = rec.get("type")
            if rtype == "token_count":
                u = rec.get("usage") or {}
                for key in usage:
                    usage[key] += int(u.get(key) or 0)
                model = rec.get("model")
                if model and model not in models:
                    models.append(model)
            elif rtype == "message" and rec.get("role") == "assistant":
                assistant_messages += 1
    return {
        "usage": usage,
        "models": models,
        "assistant_messages": assistant_messages,
    }
