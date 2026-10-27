# 实模型演练层(`tests/live/`)

这一层用**真实模型**跑 Reflect-TUI 的完整 agent 链路,补上 mock provider
永远测不到的行为:真实 SSE 帧、真实工具调用质量、真实限流(429)、
真实思考过程、真实截断续作。

默认车道是**免费模型池** —— **零 API key、零费用**。

## 与能力覆盖层的边界

| | 能力覆盖层 `tests/functional/` | 实模型层 `tests/live/` |
|---|---|---|
| 模型 | mock(`ZHUYE_MODEL=mock/mock-1`) | 真实模型(清单接口实时拉取) |
| 断言 | 字段级、可逐字 | **行为级**,不断言回答文本 |
| 网络 | 全程离线 | 需要外网 + 免费额度 |
| 门禁 | **CI 强制**(`run_all.sh functional`) | 可选(`run_all.sh live`),默认 SKIP |

⚠️ **能力断言仍然只写在 `tests/functional/`** —— 本层只断言运行时行为
(工具真的被调了 / 文件真的落盘了 / 上下文真的带上了),不重复能力覆盖。
两层不重叠、不互相替代。

## 跑法

```bash
# 默认跳过(不发起任何网络请求,退出码 0)
./tests/run_all.sh live

# 真跑(默认车道:Kilo 免费池,免 Key)
REFLECT_LIVE=1 ./tests/run_all.sh live

# 指定模型(逗号分隔);不指定则从清单里取
REFLECT_LIVE=1 REFLECT_LIVE_MODELS=cohere/north-mini-code:free ./tests/run_all.sh live

# 指定单个模型(跳过矩阵)
REFLECT_LIVE=1 REFLECT_LIVE_MODEL=cohere/north-mini-code:free ./tests/run_all.sh live
```

### 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `REFLECT_LIVE` | 空 | 设 `1` 才真跑(否则整层 SKIP) |
| `REFLECT_LIVE_BASE_URL` | `https://api.kilo.ai/api/gateway` | provider 端点 |
| `REFLECT_LIVE_API_KEY` | 空 | 免 Key 端点留空;dsh 网关填 `forwardKey` |
| `REFLECT_LIVE_MODELS` | 空 | 逗号分隔模型 id;空则从清单取 |
| `REFLECT_LIVE_MODEL` | 空 | 指定本轮唯一使用的模型 |
| `REFLECT_LIVE_MODEL_COUNT` | `3` | 从清单取几个 |
| `REFLECT_LIVE_PROBE_TIMEOUT` | `90` | 能力探测超时(秒) |
| `REFLECT_LIVE_DRAIN_TIMEOUT` | `240` | 回合等待预算(秒) |
| `REFLECT_LIVE_RETRIES` | `2` | 单个用例重试次数(限流是常态) |
| `REFLECT_LIVE_RETRY_DELAY` | `5` | 重试间隔(秒) |

## 两条车道

| 车道 | 端点 | 凭据 | 前置条件 | 开关 |
|---|---|---|---|---|
| **Kilo 免费池**(默认) | `https://api.kilo.ai/api/gateway` | 免 Key | 无 | 默认 |
| **dsh 本地网关** | `http://127.0.0.1:<端口>` | `forwardKey` | 网关在跑 | `REFLECT_LIVE_LANE=dsh` |

Kilo 车道**零外部依赖** —— 只改 `config.toml` 就能用。

dsh 车道 = 本仓用户自己的 `dsh-our-free-model` 实例(26 个模型,多为 `-free`
结尾,靠**免费判定的 id 模式**命中)。`REFLECT_LIVE_LANE=dsh` 会**自动探测**:
端口与密钥都从 `~/.our-free-model/settings.json` 取(`standalonePort` +
`forwardKey`)—— **不要写死 18900**,该端口常被别的应用占用,网关会自己顺延
(实测本机跑在 18902,18900 是另一个 node 服务)。网关没起 → 回落到 Kilo 车道。

也可用 `REFLECT_LIVE_BASE_URL` / `REFLECT_LIVE_API_KEY` 显式指定任意端点。

## 模型从哪来(不写死)

1. `REFLECT_LIVE_MODELS` 给了 → 用它;
2. 否则调 `reflect-tui models list --free-only --json` → 拿 provider 的实时清单
   (免费判定规则只有一份实现,在 Rust 侧 `reflect_llm::FreeModelRules`);
3. 逐个做**能力探测**(带 `tools` 的最小请求)→ 不支持 tools 的模型标为
   **跳过**而不是失败;
4. 取前 N 个(`REFLECT_LIVE_MODEL_COUNT`)。

免费模型集合由上游决定,随时可能下架 —— **写死模型 id 必然腐烂**。

### 跨模型降级链

配置里的 `[routing].fallbacks` 从**免费清单**取(主模型之外的前 2 个),
不是从本次矩阵取 —— 即使用 `REFLECT_LIVE_MODELS` 只钉了一个模型,降级链
也不是空的:主模型被限流(429)时,引擎自动换模型把这一轮跑完。

### 限流(429)怎么判

免费池是**全局配额**,429 / `all credentials exhausted` 是常态。屏幕上一旦
出现限流锚点(`rate limited` / `all credentials exhausted` / `retry after`),
本条用例**跳过**而不是失败 —— 测的是代码,不是配额。

## 覆盖的真实链路(L 组)

| # | 用例 | 断言的**行为** |
|---|---|---|
| L1 | `/loop 2 /status` | 定时器真的触发 ≥2 次;`/loop stop` 后不再增长 |
| L2 | `/goal set` / `/goal clear` | 目标模式进 / 出 |
| L3 | 子代理派发 | 模型能派发 `call_explorer` 并拿到结果 |
| L4 | Plan 模式 | 提 plan → `[3]` 修订 → **第二个 plan 浮层** → `[1]` 批准 → **真的写出文件**且含计划内容 |
| L5 | 工具调用回合 | 模型调 `bash` → 审批条 → 批准 → 输出上屏 |
| L6 | 会话落盘 | `~/.zhuye/sessions/**.jsonl` 含 assistant 消息 + token 记录 |
| L7 | 多轮上下文 | 第二轮能引用第一轮给的信息 |

**不断言逐字回答文本** —— 真实模型的输出不确定,逐字断言只会制造假失败。
模型**选择不调工具 / 不走 plan 流程**时按**跳过**处理(这些路径的接线已由
能力覆盖层用 mock 覆盖)。

## 全功能实战组(F 组,`test_full_feature_live.py`)

把**能力覆盖层的全部功能面**搬到真实模型的会话里跑一遍:

| 域 | 内容 | 条数 |
|---|---|---|
| **slash 全量矩阵** | 直接复用 `tests/functional/test_slash.py` 的 `MATRIX`(单一事实源),逐条独立会话验证结局 | 85 |
| **弹层** | 可达 picker(Transcript / Diff / Tasks / Plugins / Rewind)+ 不可达 picker 的反向门禁 | 2 |
| **真实轮次** | 流式上屏、真实 token 统计、工具调用 + 审批链、AskUser 面板(结构化)+ 自由文本条 | 4 |
| **持久化 + headless CLI** | 真实会话落盘 → `session ls/show/export/stats` 读出真实模型与真实 token;`fork/rename/rm` 全链路 | 2 |
| **Plan 落盘** | `.zhuye/plan/*.md` 真有内容 | 1 |
| **启动 / 生命周期 / model 解析** | HUD 显示真实模型 spec、终端标题、Ctrl+C / Ctrl+D、全屏 `--fullscreen`、resize、transcript 翻页、`model = "free"` 解析、`enabled=false` 启动即报错 | 9 |
| **键盘交互** | 权限循环 / vim / Ctrl+R 反搜 / ↑ 召回 / Ctrl+G 编辑器 / Ctrl+O 复制 / Alt+Enter 多行 / 大文本与分段粘贴 / Ctrl+A 行首编辑 | 10 |
| **渲染与视觉** | 中文 + emoji / markdown 代码块与列表 / 长回复折行 / NO_COLOR / 浅色主题 / 极小窗口 | 7 |
| **会话恢复** | `--resume` 续作 / 非法 id 快速失败 / prompt 历史落盘 | 3 |
| **插件面** | 启动通知 / 命令注入 / `/plugin` overlay / 插件命令提交 | 3 |
| **slash 弹窗细节** | 未知命令静默拒绝 / 弹窗列命令 / 前缀过滤 / 带斜杠按普通文本 / 行首空格两条语义 | 6 |
| **回退与中断** | `/rewind` 选择器(真模型回合后)/ Esc 中断真实回合(等 busy 再按) | 2 |

> 合计 **134 条**(85 条矩阵 + 49 条专项)。

> 合计 **53 条**(4 条矩阵 + 49 条专项)。

合计 **134 条**(85 条矩阵 + 49 条专项)。

```bash
REFLECT_LIVE=1 ./tests/run_all.sh live-full     # 全功能实战(较重,约 20 分钟量级)
REFLECT_LIVE=1 ./tests/live/run.sh -k full_feature   # 等价写法
```

矩阵条目里只有 `/model` 需要按车道改写(原矩阵切到 `mock/gpt-4o`,真实车道里
切到当前模型,否则整条会话掉回 mock 车道)—— 期望文案同步替换。

## 横向对比演练

```bash
# 场景 × 免费模型 → Markdown 对比表(终态 / 耗时 / token / 工具调用 / 实际模型)
python3 tests/live/practice.py --count 3
python3 tests/live/practice.py --models a,b --out report.md
python3 tests/live/practice.py --dry-run

# 或经统一入口(选项走环境变量,避免和套件名抢位置)
REFLECT_LIVE_MODEL_COUNT=2 PRACTICE_OUT=report.md ./tests/run_all.sh live-practice
```

| 变量 | 作用 |
|---|---|
| `REFLECT_LIVE_MODELS` | 指定模型(逗号分隔) |
| `REFLECT_LIVE_MODEL_COUNT` | 从清单取几个模型(默认 3) |
| `PRACTICE_OUT` | 报告落盘路径 |

**不是门禁** —— 退出码只反映脚本自身错误;模型表现差异只体现在报告里。

## 已知限制

| 限制 | 说明 | 缓解 |
|---|---|---|
| **配额是全局的** | 免费池 429 常见 | 用例自带重试;429 判为**跳过**;**矩阵必须串行** |
| **Kilo 的推广上限是按出口 IP 的** | 触顶后**所有**模型一起返回 `PROMOTION_MODEL_LIMIT_REACHED`(提示「Sign up for free to continue … 500 other models」) | 判跳过;等窗口恢复;换出口 IP |
| **dsh 匿名通道也有配额** | 触顶返回 `RATE_LIMIT: Free usage limit reached`(提示订阅 OpenCode Go) | 判跳过;登录账号通道可解 |
| **dsh 有地区门** | 部分模型返回 `REGION_BLOCKED: This model is not available in your country` | 该模型跳过;换模型 |
| **地区门 403** | 免费车道按出口 IP 判地区 | 换出口后重跑 |
| **模型集合随时变** | 上游随时下架某个 id | 清单每次实时拉取;`[routing].fallbacks` 跨模型降级兜底 |
| **慢首字 / 思考静默** | 实测首字 >30s、思考阶段可静默 60~70s | `timeout_secs = 300`(reqwest 的 timeout 是**总**时长) |
| **部分模型不支持 tools** | provider 直接 400 | 能力探测识别并跳过 |
| **prompt 可能被上游记录** | Kilo 模型卡明示 | **只用合成数据**,不要喂真实仓库内容 / 凭据 |
| **部分 provider 对 tool_results 更严** | 实测 Cohere 系模型报 `invalid request: all elements in tool_results must have the 'outputs' property specified`(400) | 工具回合偶发失败;不影响「工具被调用 / 结果上屏」的断言 |
| **`/loop` 的触发频率由模型延迟决定** | 每次触发都是一次**完整 agent 回合**,回合未收尾时的再次触发会被丢弃 | 断言口径改为「触发过」+「stop 后不增长」 |
| **宿主凭据泄漏** | 宿主的 `OPENAI_API_KEY` 会串进免费车道 | 子进程 env 清空 `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / `ZHUYE_MODEL` / `ZHUYE_PROVIDER` |

## 目录

| 路径 | 内容 |
|---|---|
| `run.sh` | 入口(未启用则整层 SKIP;额外 pytest 参数透传) |
| `conftest.py` | pytest fixture:模型矩阵 / 降级链 / 隔离沙箱 / TUI 会话工厂 |
| `live_lane.py` | 车道设施:清单发现 / 能力探测 / 配置注入 / 限流判定 / 会话统计 |
| `test_workflow_live.py` | L 组 7 条真实链路用例 |
| `test_full_feature_live.py` | F 组:全功能面在真模型下逐条跑(134 条) |
| `practice.py` | 横向对比演练 driver(场景 × 模型 → Markdown) |
