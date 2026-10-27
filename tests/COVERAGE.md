# 功能覆盖矩阵(测试 ↔ 功能面映射)

本矩阵回答一个问题:**当前测试是否完整覆盖了全部对外功能**(而非冒烟)。

覆盖口径:每条功能面至少有一个**字段级 / 行为级断言**(非「退出码 0」)
的用例;断言失败即失败。全程离线(`ZHUYE_MODEL=mock/mock-1`,零网络、
零 API key)+ 隔离 HOME(mktemp,不污染真实 `~/.zhuye`)。

> 本文件的结构与判据对齐 submodule `reflect-agent/tests/COVERAGE.md`,
> 但**覆盖对象不同**:submodule 是 agent 基座(测 headless `reflect serve`
> 的 JSONL wire);本仓是终端 TUI(测 pty 上的真实渲染、真实按键路由、
> 真实文件落盘)。

---

## 0. 覆盖分层:谁测什么

| 层 | 位置 | 测谁 | 依赖 SDK? | 规模 |
|---|---|---|---|---|
| **能力覆盖层**(主) | `tests/functional/` | **Rust 代码在真产物上的行为**(真二进制 + pty + pyte 屏幕快照) | ❌ | 215 条 |
| **覆盖自审层** | `tests/tools/coverage_audit.py` | 「声明了什么」× 「测了什么」的**机械差集** | ❌ | 4 条门禁 |
| CLI 参数面 | `tests/rust/cli_core.sh`、`cli_modules.sh` | 17 个子命令的**参数矩阵**与输出契约(bash,含退出码) | ❌ | 101 断言 |
| CLI 语义面 | `tests/functional/test_cli.py` | CLI 输出的**语义**(列结构、预览条数、统计口径) | ❌ | 10 条 |
| Rust 单测 | `crates/reflect-tui/src/**` | 内部单元与集成(既有基线) | — | 332 条(自有)+ vendored |
| live 端到端 | `tests/live/` | 真实 LLM + 真实工具(需 API key) | — | 手动跑 |

**判据**:
- 断言「用户真的看到了什么 / 真的落了什么盘」→ **能力覆盖层**;
- 断言「某个 CLI 参数组合的输出契约」→ bash 参数面;
- 断言「CLI 输出的语义口径」→ CLI 语义面;
- **新增能力断言只落能力覆盖层一处** —— 不要在多处重复写。

> 为什么要把能力面与参数面分开:参数面只验「命令跑通了」,能力面验
> 「跑通之后的**语义**正确」。例如 `session show` 能跑通(cli_modules 绿),
> 但对真实会话(ContentBlock 数组形态)的消息预览曾恒为空 —— 只有能力面
> 用**真实落盘形态**的 fixture 才测得出来(见 §6 D1,已修)。

---

## 1. 能力 × 覆盖层矩阵

| 能力域 | 能力覆盖层 | CLI 参数面 | 覆盖自审 |
|---|---|---|---|
| 启动 / 渲染锚点 / 退出生命周期 / 全屏模式 / model 解析 | B1–B15 | cli_core(`--help`/`--version`) | — |
| slash 命令面(86 条命令) | S1–S8 | — | ✅ 全覆盖门禁 |
| 弹层 / 浮层 / 选择器 / 鼠标反向门禁 | O1–O12 | — | ✅ picker 数门禁 |
| 对话轮次 / 流式 / 工具 / 统计 | T1–T11 | — | — |
| 会话持久化 / 恢复 / plan 落盘 | P1–P9 | cli_modules(session) | — |
| headless CLI 语义 | C1–C10 | cli_modules(traces) | — |
| 键盘交互(权限循环 / 历史召回 / 编辑器 / 复制 / 多行 / vim) | K1–K11(无 K6) | — | — |
| 渲染与视觉(宽字符 / 折行 / markdown / 主题 / resize / 小窗) | V1–V9 | — | — |
| 工作流与容错(审批 / AskUser / loop / 恢复边界 / 坏输入 / 信号) | W1–W12 | — | — |
| 真实 HTTP 链路(Anthropic SSE / 跨天恢复 / busy 窗口中断) | H1–H3 | — | — |
| Rust 编译面 | — | workspace_tests(cargo test/clippy/fmt/doc) | — |

---

## 2. 能力覆盖层明细(`tests/functional/`)

### B 组 启动与生命周期(`test_boot.py`)

| # | 用例 | 断言锚点 |
|---|---|---|
| B1 | 启动到稳定首屏 | `Reflect` + `/help for commands` |
| B2 | banner 块字形画 | `█` / `╗` |
| B3 | HUD identity 行 | `mock/mock-1` + `Ctrl+C to quit`(statusline.rs:218) |
| B4 | HUD metrics 行 | `no tool calls yet`(statusline.rs:273) |
| B5 | composer 占位符 | `Send a message… (shift+enter for newline)`(events.rs:593) |
| B6 | 终端标题(OSC 0) | `Reflect · mock/mock-1 · …`(tui/mod.rs:958-971) |
| B7 | Ctrl+C 退出 | 退出码 0 + 无 panic |
| B8 | Ctrl+D 退出 | 退出码 0 |
| B9 | 窗口 resize 重绘 | 进程存活 + 无 panic |
| B10 | 缺 provider 快速失败 | 非 0 退出 + 错误含 `provider` |
| B11 | `--fullscreen` 进备用屏 | raw 含 `ESC[?1049h` + banner 块字 + composer 占位符(`draw_fullscreen` 独立渲染路径) |
| B12 | 全屏模式退出还原主屏 | raw 含 `ESC[?1049l` + 退出码 0 |
| B13 | `model = "free"` 走清单发现 | 非 0 退出 + 错误含 `模型清单`(未接线时会拿占位串 `provider/free` 照常启动) |
| B14 | `model = "free"` 且 `[model_discovery] enabled = false` | 非 0 退出 + 错误含 `model_discovery` / `free` |
| B15 | 静态 model 走原路径不回归 | 正常启动 + HUD 显示 `anthropic/claude-test-model` |

### S 组 slash 命令面(`test_slash.py`,**数据驱动全量矩阵**)

84 条命令逐条断言运行时结局(矩阵 85 行 —— `/side` 测了带参与不带参两种)。
矩阵是**单一事实源**,由 `tests/tools/coverage_audit.py` 静态核对「Rust 里
声明的命令」与「矩阵覆盖的命令」,缺口直接判失败。

四类结局(实测于 release 二进制):

| 类别 | 行数 | 运行时表现 | 断言 |
|---|---|---|---|
| `wired` | 25 | 有真实行为 | 屏幕出现指定文本 |
| `placeholder` | 34 | 命令被识别,但本构建未接线 | `⚠ /<cmd>: 此命令在当前构建中尚未接入分发` |
| `unrecognized` | 17 | 命令未被识别,提交被静默吞掉 | 输入留在框内 + **无任何 notice** |
| `panel` | 7 | 打开弹层 | 面板标题出现 |
| `exit` | 2 | 退出 TUI | 退出码 0 |

另有 7 条独立用例:命令弹窗列出 / 过滤 / 未知命令静默拒绝 /
`/mode` 切权限模式 / 含 `/` 的名字不按命令解析 / 行首空格的两种结局。

**关键机制(测试必须知道,否则会写错断言)**:

1. **两条分发通道**。composer 敲下的命令名命中
   `is_legacy_passthrough_slash` 白名单(`help ? tasks agents cost
   exit-plan exitplan mode effort ref rm-session`)→ 原样提交走
   `history_render::dispatch_slash`;否则命中可见的 `SlashCommand`
   枚举 → 走 `tui/mod.rs` 的 `dispatch_slash_command`。
2. **枚举通道只实现了部分命令**。其余落到兜底文案
   「`/<cmd>: 此命令在当前构建中尚未接入分发`」。
3. **两条通道的行为不一致**。例:`/mode` 在 legacy 通道真切权限模式;
   `/model` 在枚举通道真切模型(`switched to '…'`),在 legacy 通道才
   校验 registry。
4. **不可见命令 = 未识别**。feature 门控隐藏的命令(`/apps` `/rollout`
   `/test-approval` `/setup-default-sandbox` `/sandbox-add-read-dir`)
   与 legacy 死代码臂(`/version` `/doctor` `/history` `/files` `/branch`
   `/image` `/img` `/checkpoint` `/traces` `/think` `/thinking` `/retry`)
   都在 composer 层被拒收,输入留在框里。

### O 组 弹层 / 浮层(`test_overlay.py`)

| # | 用例 | 断言锚点 |
|---|---|---|
| O1 | Ctrl+T 打开 transcript | `Transcript (Esc / Ctrl+T to close)` |
| O2 | `/help` 打开帮助页 | 同上 + 命令说明 |
| O3 | `/diff` 打开 diff | `Diff (Esc / Ctrl+T to close · /diff)` + `no diffs in this session` |
| O4 | Ctrl+Shift+D 不可达 | `0x04` 被解析成 Ctrl+D → 退出,不是打开 diff(见 §6 D8) |
| O5 | `/tasks` 打开任务面板 | `Tasks & Agents (Esc / Ctrl+T to close · /tasks)` + 分区 |
| O6 | `/plugin` 打开插件面板 | `Installed Plugins` + `No plugins installed` + 安装指引 |
| O7 | `/rewind` 需历史 | `no user prompts to rewind to yet` |
| O8 | `/clear` 二次确认 | `Clear scrollback?` → `Clear cancelled.` |
| O9 | `/rm-session` 二次确认 | `Permanently delete current session files and exit?` |
| O10 | overlay 遮蔽全局键 | overlay 打开时 Ctrl+T 不生效 |
| O11 | 9 条未接线弹层保持不可达 | 标题**不**出现 |
| O12 | 鼠标序列无副作用 | SGR/X10 鼠标序列注入后:进程存活、无 panic、composer 完好、无弹层打开 |

> O11 / O12 是**反向门禁**:Image / Keymap / Statusline / Theme / Checkpoint /
> Skills Hub / MCP / Traces / Sessions 八个弹层的渲染代码存在且有 Rust
> 单测,但运行时不可达;鼠标则整体未接入(无 `EnableMouseCapture`)。
> 将来接线后对应用例会失败并提醒更新矩阵。

### T 组 对话轮次(`test_turn.py`)

| # | 用例 | 断言锚点 |
|---|---|---|
| T1 | 一次往返 | 用户消息 + mock 回复上屏 |
| T2 | agent 流式前缀 | `• `(marker.rs:28-29;`● ` 只作用于整段 `Agent` 历史项) |
| T2b | 用户消息前缀 | `▶ `(marker.rs:28) |
| T3 | 流式逐行落 scrollback | 三个标记都在 |
| T4 | 工具调用成功行 | `✓ echo (<N>ms)` + 输出正文 |
| T5 | HUD metrics 更新 | `×1` |
| T6 | 回合分隔线 | `─ ─ ─`(cells.rs:182) |
| T7 | 多轮保留 | 两轮回复都在 |
| T8 | 脚本耗尽回退 | `Hello from mock LLM!` |
| T9 | Esc 打断回合 | 回合收尾 / 中断提示 |
| T10 | 回合后 HUD 用量段 | `context left` |
| T11 | 工具参数分片下发 | 调用行带参数摘要(`✓ glob (…) · *.rs`) |

> **mock provider 的语义(测试必须知道)**:
> `ZHUYE_MOCK_SCRIPT` 是 JSONL,**每次模型调用消费一行**
> (mock.rs:163 `script.pop_front()`);脚本耗尽回退
> `Hello from mock LLM!`。一次用户提交会让引擎**多次**调用模型
> (实测 4 次),所以:
> - 屏幕上的回复是**多次调用结果的拼接**;
> - 落盘的 assistant 消息是**最后一轮**的回复 —— 要断言落盘正文,脚本须
>   给足**同款行**(如 `[text_reply(x)] * 6`),否则落盘的是回退文本;
> - 要让工具在第一轮就执行,`tool_call` 必须是脚本**第一行**。
> - `echo` 工具的参数名是 `text`(不是 `message`)。
> - **工具名全小写且大小写敏感**:`bash` / `ask_user` / `ask_user_question`;
>   写成 `Bash` 会得到 `✗ Bash (0ms) tool not found: Bash`。

### P 组 会话持久化(`test_session.py`)

| # | 用例 | 断言 |
|---|---|---|
| P1 | 会话落盘 | `~/.zhuye/sessions/YYYY/MM/DD/*.jsonl`;首行 `session_meta`;user/assistant 消息;`token_count` 带嵌套 `usage` |
| P2 | prompt 历史落盘 | `~/.zhuye/prompt_history.jsonl` |
| P3 | `-c` 恢复 | 「已恢复历史会话」+ 既往消息上屏 |
| P4 | `--resume <id>` 恢复 | 同上 |
| P5 | 非法 `--resume` id | 快速失败(非 0),不挂死 |
| P6 | 会话名落盘 | `~/.zhuye/sessions/_names/<id>.name` |
| P7 | Plan 落盘 | `<workspace>/.zhuye/plan/*.md` + 正文 |
| P8 | 旧日期会话恢复 + 续写原文件 | 会话文件挪进旧日期桶后 `--resume` 历史回填;新轮次追加进原文件,当天桶不另立新文件 |
| P9 | 双 id 恢复(外部命名会话) | 文件名与首行 `SessionMeta.session_id` 脱钩后,按内部 id 恢复仍回填历史 |

### C 组 headless CLI 语义(`test_cli.py`)
| # | 用例 | 断言 |
|---|---|---|
| C1 | `session ls` 六列 | session_id/model/started/msgs/tokens/cost |
| C2 | token 列 | 累计 token 可见 |
| C3 | `session show` 预览 | 元数据 + 消息正文(**真实 ContentBlock 数组形态**) |
| C4 | `session show` 统计 | `Tokens:` + in/out 拆分 |
| C5 | `session show` per-model 表 | `Per-model usage:` + 模型名 |
| C6 | `session export` markdown | 角色标题 + 正文 |
| C7 | `session export --out` | 落盘非空 + **无 `.tmp` 残留**(原子写) |
| C8 | `session stats` | per-model 表 + TOTAL 行 + token 总量 |
| C9 | `traces spans` 过滤 | 按 session 前缀命中 / 未命中占位 |
| C10 | `session show` 不存在的 id | 非 0 + 可读错误 |

### K 组 键盘交互(`test_keys.py`)

`KEY_SEQS` 里定义但此前无用例引用的按键,逐一变成行为断言。

| # | 用例 | 断言锚点 |
|---|---|---|
| K1 | Shift+Tab 循环权限模式 | identity 行 `[accept_edits]` → `[plan]` → 无标记(Auto),实测三态循环(见 §6 D11) |
| K2 | Ctrl+O 复制最后回复 | `⚠ Copied last reply (N chars) to clipboard.` |
| K3 | Ctrl+G 外部编辑器往返 | `⚠ External editor closed; content loaded into composer.` + `▶ TEXT_FROM_EDITOR` 提交 |
| K3b | 全屏模式 Ctrl+G 外部编辑器 | 编辑器前真正离开备用屏(`ESC[?1049l`),退出后重进(`ESC[?1049h`) |
| K4 | Ctrl+R 反向搜索历史 | composer 回填命中条目(`beta probe two`) |
| K5 | Up 召回最近历史 | composer 回填 `HIST_LAST_ENTRY` |
| K7 | alt+enter 多行输入 | composer 两行 + 落盘消息体含 `\n`(shift+enter 依赖 kitty 协议,哑 pty 用等价路径) |
| K8 | 大段粘贴 300+ 字符 | 落盘内容逐字符完整 |
| K9 | vim 模式编辑行为 | `⚠ Vim mode enabled` + HUD `[NORMAL]` + `x` 删除末字符 |
| K10 | 行内编辑(Ctrl+A 行首) | 落盘文本 = 前缀 + 原文 |
| K11 | 粘连突发多段输入 | 三段按序拼合,无丢失 |

> K6(Ctrl+C busy 首按中断)不设用例:mock 回复瞬时完成,离线无法
> 确定性制造 busy 窗口(空闲态 Ctrl+C 是退出,语义不可区分)。
> Esc 中断已由 T9 覆盖,空闲退出由 B7 覆盖。

### V 组 渲染与视觉(`test_render.py`)

| # | 用例 | 断言锚点 |
|---|---|---|
| V1 | 中文往返 | 中文输入/回复上屏 + 落盘 + 无 panic |
| V2 | emoji(含 ZWJ 组合) | 上屏 + 无 panic |
| V3 | 超长回复折行 | `V3_HEAD` 与 `V3_TAIL` 都在屏幕上(无整段丢失) |
| V4 | markdown 代码块 | 围栏内容(`fn probe_render_fn`)上屏 |
| V5 | markdown 表格 / 列表 | `CELLKEY42` / `LISTITEM_*` 上屏 |
| V6 | `NO_COLOR=1` 分支 | 首屏锚点齐全(theme.rs `resolve_startup`) |
| V7 | `REFLECT_TUI_THEME=light` 分支 | 首屏锚点齐全 |
| V8 | resize 布局锚点 | 80x24 → 120x40 → 80x24,HUD 模型名 + composer 占位符始终可见(升级 B9) |
| V9 | 极小窗口 40x10 | 存活 + 有渲染输出 + 交互不崩 |

### W 组 工作流与容错(`test_flow.py`)

| # | 用例 | 断言锚点 |
|---|---|---|
| W1 | 审批批准链路 | `⚠ APPROVAL NEEDED` + `Tool: bash({…})` + `[Y]es/[N]o/[A]ll/[S]cope` → y → `✓ bash` + 输出上屏 |
| W2 | 审批拒绝链路 | n → `✗ bash (0ms)` + `Approval denied: denied by user`,命令未执行 |
| W3 | AskUserQuestion 面板 | `❓ QUESTION 1/1 · <header> · 单选` + 选项列表 → 数字勾选 + Enter → `✓ ask_user_question` + 回合继续 |
| W4 | ask_user 自由文本条 | `❓ AGENT 等待你的回答` → 输入回车 → `▶ (回答) <text>` + `✓ ask_user` + 回合继续 |
| W5 | `/loop` 离线定时触发 | raw 流 `LOOPFIRE` 标记 ≥8(≥2 轮,视口计数会漏须用 raw);`/loop stop` → `⚠ /loop: stopped (was: /status every 3s)` |
| W6 | `-c` 无历史会话 | 非 0 退出 + `no sessions found` + `nothing to resume` |
| W7 | `--resume` 前缀过短拒绝 | 非 0 退出 + `前缀过短`(<4 位直接拒绝) |
| W7b | `--resume` 唯一前缀命中 | `↩ 已恢复历史会话 <id> · 回放 1 条消息` |
| W7c | `--resume` 跨天会话(完整 UUID) | `↩ 已恢复历史会话 <id> · 回放 1 条消息` |
| W8 | 200 条消息大会话恢复 | 不挂死、可继续对话、无 panic(旧消息进原生 scrollback,pyte 视口不可见) |
| W9 | 坏 config.toml | 终端给出显式告警(`解析失败` + 路径),并回落默认配置,TUI 照常启动 |
| W10 | 非法 `ZHUYE_MODEL` | 非 0 退出 + 错误含 `provider` |
| W11 | SIGTERM | rc=-15(死于信号)+ 无 panic + 会话文件已落盘 |
| W12 | stdin 非 tty(管道) | 非 0 退出 + `Error: Device not configured`,不挂死 |

> W5 的 stop 需要**空闲窗口**:busy 期间 composer 的提交会被吞且草稿
> 保留(§6 D10),用例内先等 HUD spinner 消失再提交(带重试)。

---

## 2b. 其余两组

### G 组 插件面(`test_plugin.py`)

| # | 用例 | 断言 |
|---|---|---|
| G1 | 启动通知 | scrollback 出现 `Plugin 'demo@inline' loaded …` |
| G2 | slash 弹窗注入插件命令 | 弹窗列出 `demo:hello` |
| G3 | 插件命令不被未知命令校验拦下 | 无 `Unrecognized`,命令上屏 |
| G4 | `/plugin` overlay | 展示真实已装插件(id / version / enabled) |

> 前置:submodule 构建出的 `reflect` CLI(用于安装示例插件)。缺失则整组 SKIP。

### U 组 测试设施自身(`test_pty_helpers.py`)

| # | 用例 | 断言 |
|---|---|---|
| U1 | `normalize` 折叠空白 | 折行/多空白 → 单空格 |
| U2 | Sandbox 隔离 | 两个沙箱 HOME / cwd 互不相同 + 预写空 config |
| U3 | `Sandbox.env()` | HOME 指向隔离目录,TERM 可渲染,宿主 HOME 不泄漏 |
| U4 | `cleanup()` | 隔离目录整体删除,不留残留 |
| U5 | `session_files()` | 按 `YYYY/MM/DD/<id>.jsonl` 层级找会话文件 |
| U6 | 按键表覆盖 | TUI 用到的全部快捷键都有非空字节序列 |
| U7 | mock 脚本助手 | 产出引擎能解析的 JSONL 行 |
| U8 | 无参 `tool_call` | 也产出合法 JSON |

> 这一组守的是**测试基础设施的正确性** —— 设施坏了,上面所有能力断言都会
> 变成「假绿」。

### H 组 真实 HTTP 链路(`test_http_mock.py`)

| # | 用例 | 断言 |
|---|---|---|
| H1 | 真实 Anthropic SSE 往返 | 请求打到本地 mock server,`content_block_delta` 聚合出的回复上屏 |
| H2 | 跨天会话 resume(真实 HTTP 产出) | 会话搬到 `2026/01/01` 桶后 `--resume` 仍回放既往消息 |
| H3 | 确定性 busy 窗口下 Esc 中断 | 出现「已发送中断请求」,且连按 3 次只出现一次(去抖生效) |

> **为什么要有这一组**:其余用例都用 `ZHUYE_MODEL=mock/mock-1` —— 那是
> reflect-llm `ModelClient` 层的内置 provider,**完全绕过** HTTP 传输、
> SSE 分帧与 Anthropic 协议形态;内置 mock 还「瞬时完成」,离线造不出
> **确定性 busy 窗口**(这正是 §6 里「Ctrl+C busy 首按中断」等条目长期
> 锁在「未设用例」的原因)。本组用 `mock_anthropic_server.py` 起一个本地
> HTTP server,补上那一层,并用 `MOCK_ANTHROPIC_DELAY` 拉长模型响应来
> 制造 busy 窗口。

---

## 3. CLI 子命令(14 个,不含 help)

| 子命令 | 参数矩阵(bash) | 语义断言(Python) |
|---|---|---|
| `session ls` | cli_modules | C1, C2 |
| `session show` | cli_modules | C3, C4, C5, C10 |
| `session rm` | cli_modules | — |
| `session fork` | cli_modules | — |
| `session rename` | cli_modules | P6 |
| `session export` | cli_modules | C6, C7 |
| `session stats` | cli_modules | C8 |
| `session search` | cli_core | — |
| `traces ls` | cli_modules | — |
| `traces show` | cli_modules | — |
| `traces spans` | cli_modules | C9 |
| `workspace clone` | cli_core | — |
| `discussion run` | cli_core | — |
| `security audit` | cli_core | — |

顶层:`--help` / `-h` / `--version` / `--cwd` / `-C` / TUI 参数
(`--prompt` / `-c` / `-r` / `--resume` / `--plan-mode` /
`--ephemeral-tasks` / `--ephemeral-teams` / `--auto-root` / `--fullscreen`)
→ cli_core。

---

## 4. 集成面

本仓是表现层,引擎能力由 submodule 提供。**接线面**的可观测点:

| 能力 | 接线位置 | 黑盒可观测点 |
|---|---|---|
| 内置工具(18 个) | `bootstrap.rs:119-142` | `✓ <tool> (<N>ms)` 行 |
| Task / Team 工具 | `bootstrap.rs:168` | `/tasks` 面板的 Tasks 分区 |
| Skills 工具 | `bootstrap_wiring.rs:313-320` | `/skills`(当前未接线) |
| Session note 工具 | `bootstrap_wiring.rs:332-334` | 工具行 |
| 子代理 `call_<role>` | `bootstrap_wiring.rs:487-511` | `/subagents` 面板 + `[子代理 <role>]` 通知 |
| MCP 资源 / Prompts 工具 | `bootstrap.rs:392-407` | `/mcp` 面板 |
| Plan 工具组 | `bootstrap.rs:130-140` | Plan 审批条 + `.zhuye/plan/*.md` |
| LSP 工具 | `bootstrap_wiring.rs:691-693` | `LSP server 'x' started` 通知 |
| 插件能力 | `bootstrap.rs:409-426` | `Plugin 'x' loaded (…)` 通知 + `/plugin` 面板 |
| Telemetry | `bootstrap_wiring.rs:75-129` | `~/.zhuye/traces/` 产物 + `traces spans` |
| 压缩 | `bootstrap.rs` + `reflect-compact` | `🗜 已压缩上下文:…` 通知 |
| 权限模式 | `Op::CyclePermissionMode` | HUD `[plan]` / `[accept_edits]` 段 |
| 工具参数流(`ToolCallArgsDelta`) | `tui/conversion.rs` + `adapter.rs` | 调用行参数摘要 `✓ glob (…) · *.rs` |
| 会话恢复回放(任意日期 + 轮转副本 + 双 id) | `resume.rs::replay_all`(`reflect_rollout::reader::replay`) | 旧日期会话 `--resume` 历史回填 + 续写原文件(P8);按内部 session id 恢复(P9) |
| 路由事件(failover / 冷却 / 跨模型降级) | `tui/conversion.rs` + `routing_status.rs` | `↻ 路由切换 · main: …` / `↘ 模型降级 · …` 通知行 |
| 目标模式状态快照 | `tui/mod.rs`(`AgentThread::goal_snapshot`) | `/goal` 无参 → `目标模式 · 进行中` + 进度行 |
| provider 模型清单 + 免费判定 | `reflect_config::fetch_catalog` + `reflect_llm::model_catalog` | `/model` 无参 → 清单行 + `免费` / `免费?` 标记 |
| 配置热重载 | `reflect_config::ConfigWatcher` + `config_reload.rs` | `🔄 配置已重载(…)· 变更段: …` 通知行 |
| 分析遥测(OTLP) | `logging.rs`(`reflect_config::init_exporter`) | `[analytics] enabled=true` → OTLP span 导出 |
| LSP 诊断 | `reflect_lsp::DiagnosticRegistry` | `/lsp` → `诊断文档: N · 诊断条数: M` |
| Agent 待办清单 | `AgentConfig::todos_slot` | `/todo` → `N 项(✅ … 完成 · 🔄 … 进行中 · ⬜ … 待办)` |
| MCP 精选目录 | `reflect_mcp::curated_catalog` | `/mcp` overlay 的 `Curated catalog` 段 + 安装片段 |
| 目标模式校验 / 预算 | `reflect_goal` + `Op::EnterGoalMode` | `/goal <目标> --verify <cmd> --budget <n>` |
| OS 级沙箱状态 | `reflect_sandbox::OsSandbox::status_line` | `/status` 的 `sandbox:` 段 |
| 终端通知开关 / 通道 | `reflect_config::TuiNotificationsSection` | `[notifications.tui]` 的 `enabled` / `on_*` / `channels` 真正生效 |

---

## 5. 已知产品限制(测试断言当前行为,防退化)

- **大部分 slash 命令未接线**:枚举通道的 `dispatch_slash_command` 只实现了
  少数命令,其余落到「尚未接入分发」兜底(36 条)。这是**当前构建的真实
  状态**,不是测试缺陷。
- **8 个弹层运行时不可达**(O11):渲染代码 + Rust 单测都在,但命令到不了。
- **`/theme` `/keymap` `/statusline` 等的配置持久化是桩**
  (`tui_core/legacy_core.rs:362-372` 的 `ConfigEditsBuilder`):`/theme` 的
  apply 只回写内存态并提示「display only」。
- **feature flag 恒全关**(`features.rs:22-31`):15 个 flag 的 `is_enabled`
  恒 `false`,且**没有任何切换途径**(不读 config / env / CLI)。
- **`login.rs` / `cloud_config.rs` / `models_manager.rs` / `connectors.rs` /
  `core_plugins.rs` 都是桩**:鉴权只有 API key 一条路径。
- **`tui_core/chatwidget/` 整体不可达**:live TUI 只用 `tui_core` 的呈现
  原语;模型弹窗 / service tier / onboarding 都在未接线路径上。
  `tui_core/app.rs:1-5` 自述是「桩」。
- **鼠标完全未接入**:全 crate 无 `EnableMouseCapture`,`Event::Mouse`
  被 `handle_event` 的兜底分支静默吞掉。
- **行首空格不阻止已知命令被解析**:`validate_submission` 的
  `input_starts_with_space` 只影响「未知命令」的拦截;legacy 通道的
  `dispatch_slash` 入口做了 `trim()`,所以 `" /clear"` 仍会弹二次确认。
- **`/prompt` 子命令**(`list` / `reload` / `rescan`)在当前构建未接线。
- **`--resume` 只认完整 UUID**:3 字符或 8 字符前缀都报 `invalid thread id
  … invalid length`;前缀解析只在 headless `session show` 里有(W7 锁定)。
  → **已修**:TUI 侧 `--resume` 支持唯一前缀(≥4 位,语义与 `session show`
  对齐),过短前缀仍拒绝并提示「前缀过短」;唯一前缀命中后回放既往消息
  (W7b),跨天会话也照常回放(W7c)。
- **`--resume` 只扫近 3 天的日期桶**:`reader::replay` 用
  `Utc::now() - days(0..3)` 拼路径,3 天以上前创建的会话回放为空 ——
  续作「成功」但历史静默丢失;轮转副本(`<id>.N.jsonl`)也不参与合并。
  → **已修**:先由本仓库维护全盘扫描 workaround;submodule `e4560bc` 起
  共享 `reader::replay` 具备同等定位能力(任意日期桶 + 轮转副本合并 +
  文件名/内部 id 双入口),workaround 已删除、消费点回归共享实现
  (P8、P9),恢复后的写入也续写原日期目录的原文件(writer 侧同步修复)。
- **Shift+Tab 实际是三态循环**(Auto → AcceptEdits → Plan → Auto):
  `keymap.rs` 注释宣称五模式(含 Prompt / Bypass),但循环到不了后两者
  (K1 锁定,见 §6 D11)。
- **坏 `config.toml` 静默回落默认配置**:解析失败无任何用户可见告警,
  TUI 照常启动(W9 锁定「不崩」,见 §6 D9)。
  → **已修**:bootstrap 启动前显式体检 config.toml,解析失败即在终端打印
  `⚠ 警告:<路径> 解析失败(...)`,同时仍回落默认配置不挡启动(W9)。

---

## 6. 已知缺陷

| # | 缺陷 | 证据 | 状态 |
|---|---|---|---|
| D1 | **`session show` 对真实会话的消息预览恒为空**:`preview_message_body` 只认 `content` 为字符串 / `{text}` 对象,认不出 v1.2 起的 **ContentBlock 数组**形态 —— 而那正是引擎的实际落盘形态 | `crates/reflect-tui/src/cli/session.rs:607`(修复前) | **已修**(`crates/reflect-tui/src/cli/session.rs:607-641` + 4 条单测)。同款缺陷在 submodule `reflect-cli` 亦存在,未同步修复 |
| D2 | **未知 slash 命令的拒绝提示被丢弃**:`chat_composer.rs:2801-2806` 发出的 `AppEvent::InsertHistoryCell("Unrecognized command '/x'")` 没有消费方(`ComposerInput::handle_app_events` 只处理 `StartFileSearch`),`src/tui/mod.rs` 也从不调 `take_app_events` | `tui_core/public_widgets/composer_input.rs:483-493` | 未修(记录)。用户只看到「输入还在框里、没发出去」 |
| D3 | **`TurnAborted` 与 `TurnComplete` 折叠**:协议两者都映射 `UiEventKind::TurnCompleted`,屏幕无法区分「正常结束」与「被 Esc/Ctrl-C 中断」 | `tui/conversion.rs:53-54` | 未修(记录)。只能靠中断瞬间的 `⏹ 已发送中断请求…` notice 间接区分 |
| D4 | **4 个协议事件被静默丢弃**:`McpToolInvoked` / `PermissionBubble` / `ConfigReloaded` / `CollabMessage` / `Routing` 在 `convert_event` 里返回 `None`,屏幕无任何痕迹 | `tui/conversion.rs:226-241` | 未修(记录) |
| D5 | **picker 的 `Nop` 键位不吞键**:在 image/keymap/statusline/theme/skills_hub 里按 `j`/`k` 既移动选择又往 composer 输入框打字;keymap / skills_hub / statusline 的 `Enter` 也是 `Nop`(会提交草稿) | `tui/mod.rs:2594, 2721, 2737, 2772, 2788, 2804, 2848` | 未修(记录)。本套件一律用 `Esc` 关闭弹层以规避 |
| D6 | **`REFLECT_TUI_UPDATE_CHECK` 无人读取**:旧 harness 设它以屏蔽启动期更新探测,但本 fork 的 `update_action.rs:29-33` 恒返回 `None` | `crates/reflect-tui/src/tui_core/update_action.rs:29-33` | 未修(记录)。无害,但属残留 |
| D7 | **mock provider 一次提交触发多次模型调用**:实测 4 次,导致脚本行与最终落盘的 assistant 消息不是一一对应 | `reflect-agent/crates/abilities/reflect-llm/src/providers/mock.rs:163` | 引擎侧行为,记录 |
| D8 | **`Ctrl+Shift+D` 快捷键在真实终端不可达**:终端无法区分 Ctrl+D 与 Ctrl+Shift+D(同为字节 `0x04`),crossterm 把 `0x04` 解析成 `Char('d') + CONTROL`(**不带 SHIFT**),而 `keymap.rs` 的 `OpenDiff` 分支要求 `CONTROL|SHIFT` —— 该分支永远命中不了,实际落到 `Ctrl+D → Exit` | `crates/reflect-tui/src/keymap.rs:88-94`(要求 SHIFT)vs `:60-65`(Ctrl+D 排除 SHIFT);实测见 `test_overlay.py::test_ctrl_shift_d_is_unreachable_from_a_terminal` | 未修(记录)。diff overlay 的实际入口只有 `/diff` |
| D9 | ~~**坏 `config.toml` 静默回落**~~:`this is [ not valid toml {{{` 这类非法内容不产生任何用户可见告警,TUI 以默认配置照常启动 —— 用户改了配置却不生效时无从得知 | W9 实证(2026-10):进程存活、无 panic、raw 无 config 相关输出 | **已修**:bootstrap 启动前显式体检 config.toml,解析失败在终端打印 `⚠ 警告:<路径> 解析失败(...)`,仍回落默认配置不挡启动 |
| D10 | **busy 窗口内的 composer 提交被吞且草稿保留**:回合进行中按 Enter,提交内容不上传也不清框;`/loop stop` 在迭代 busy 窗口内提交即失效,且残留草稿会污染后续输入(与 `/loop` 的自动提交叠加尤其明显) | W5 调试实证(2026-10):第 2 轮触发后立刻 stop 无效,拖到空闲窗口才生效;残留草稿使后续 stop 拼成非法命令 | 未修(记录) |
| D11 | **Shift+Tab 权限模式循环与注释不符**:`keymap.rs:45-100` 注释宣称 Plan/Auto/Prompt/AcceptEdits/Bypass 五模式循环,实测 HUD 可见循环只有三态 Auto → AcceptEdits → Plan → Auto,Prompt / Bypass 到不了 | K1 实证(2026-10):连续 6 次 Shift+Tab 交替出现 `[accept_edits]`/`[plan]`/无标记 | 未修(记录) |
| D12 | **整段输入(paste-burst)会丢字符**:一次 `write` 写入的连续按键被当作 bracketed paste 处理,部分字符被吞 —— 实测 `/demo:hello` → `/emo:hello`、`/plugin` → `/lugin`,表现为 slash 弹窗不弹、命令不被识别 | G 组实证(2026-10):`test_plugin.py` 的 G2/G3/G4 在整组跑时**间歇**失败(单跑恒过),屏上可见缺字;与 submodule 升级无关(升级前后各复现一次,基线代码同样复现) | 未修(记录)。测试侧规避:分两段写入 + `_pump` 让出(G2/G3/G4 已按此改);产品侧 paste-burst 的字符丢弃待定 |
| D13 | **满载下 Ctrl+G 按键间歇丢失**:套件满载连续跑时 `ctrl+g`(0x07)偶发未被处理 —— 编辑器未拉起、无任何通知、composer 草稿原样保留;单跑稳定复现不了 | K3 / K3b 实证(2026-10-27,合并 submodule `e4560bc` 后):全套件跑复现 2 次(K3+K3b 同批)+ 单跑复现 1 次,随后单跑连 7 次全过;失败批次均在 7 分钟连续 pty 压测的高负载窗口。 ede1639 长驻阻塞读取任务事件循环下出现,基线(每轮 spawn poll 任务)同批次未复现 | 未修(记录)。疑与高负载下 fork 版 crossterm poll/read 或输入通道交接时序有关;复现率低,先挂观察 |

---

## 7. 用例规模汇总

| 文件 | 规模 |
|---|---|
| `functional/test_slash.py` | 96 条(89 行命令矩阵覆盖 86 条命令 + 7 条独立用例) |
| `functional/test_overlay.py` | 20 条(10 条可达弹层 + 9 条不可达弹层反向门禁 + 1 条鼠标反向门禁) |
| `functional/test_boot.py` | 15 条(含 2 条全屏模式 + 3 条 model 解析) |
| `functional/test_flow.py` | 14 条(审批 / AskUser / loop / 恢复边界 / 容错 / 信号) |
| `functional/test_turn.py` | 12 条 |
| `functional/test_keys.py` | 11 条(键盘交互) |
| `functional/test_cli.py` | 10 条 |
| `functional/test_render.py` | 9 条(渲染与视觉) |
| `functional/test_pty_helpers.py` | 8 条 |
| `functional/test_session.py` | 9 条 |
| `functional/test_plugin.py` | 4 条 |
| `functional/test_coverage_audit.py` | 4 条 |
| `functional/test_http_mock.py` | 3 条(真实 HTTP + SSE + 确定性 busy 窗口) |
| **能力覆盖层合计** | **215 条** |
| `rust/cli_core.sh` | 45 断言(含退出码) |
| `rust/cli_modules.sh` | 56 断言(含退出码) |
| 另有 `cargo test --workspace` / clippy / fmt / doc | 既有基线 |

---

## 8. 未覆盖项(诚实清单)

- **真实 LLM provider 的流式 / 重试 / 限流路径**:mock provider 覆盖协议
  语义,真实 provider 行为(429 退避、连接中断重试)需要网络。
- **一次性提交触发多次模型调用的原因**:实测引擎在一次用户提交里调用
  模型 4 次(落盘 4 条 `token_count`),但未定位到是哪几条辅助调用
  (title 生成 / compact 判定 / auto-review?),故未对调用次数做断言。
- **Ctrl+C busy 首按中断**:mock 回复瞬时完成,离线无法确定性制造 busy
  窗口;空闲态 Ctrl+C 是退出,两种结局不可区分。Esc 中断由 T9 覆盖,
  确定性 busy 窗口下的 Esc 中断(含去抖)由 H3 覆盖 —— 但 **Ctrl+C**
  仍缺用例:它与空闲退出同键,需另设退出判据。
- **真实 shift+enter 序列**:依赖 kitty 键盘协议(`ESC[13;2u`),哑 pty
  发不出;K7 用 composer 支持的等价路径 alt+enter 锁定多行语义。
- **系统剪贴板真实内容**:K2 只断言 `Copied last reply` 通知,未读回
  系统剪贴板(跨平台剪贴板后端在 CI 哑终端下不可靠)。
- **并行远程工具调用**:TUI 不注册远程工具(仅 serve 模式有)。
- **resize 后的具体版面**:V8 断言锚点仍可见,不断言行列位置。
- **终端能力分支**(kitty 图形协议 / sixel 图片 / OSC 9 通知 / 真实
  alt-screen 语义):依赖真实终端;B11/B12 只验 `ESC[?1049h/l` 序列发出。
- **subagent 真实派发**:需引擎侧的 `call_<role>` 工具被模型选中,离线
  mock 只能脚本化单个 `tool_call`,派发链路未覆盖。
- **鼠标正向交互**:鼠标整体未接入,O12 只做反向门禁。
- **vendored 上游测试(约 760 条)**:受 feature `tui-upstream-tests` 门控,
  且测的是**运行时不可达**的 `tui_core/bottom_pane` / `chatwidget` 一族,
  不宜当作「已覆盖」。
- **`model = "free"` 的清单发现**:mock provider 不提供模型清单端点
  (`list_models` 返回 `Ok(None)`),离线路径只能验到「如实报错」这一支;
  真正的「从清单里挑一个免费模型」需要带 `/models` 端点的 provider。
  判定规则(`reflect_llm::model_catalog::FreeModelRules`)本身有 Rust 单测。
- **路由 failover / `SpecFallback` 跨模型降级**:需要同 provider 的多个
  凭据(至少一个失效)+ `[routing] fallbacks` 配置,离线 mock 只有单凭据。
  事件格式化(`routing_status.rs`)有 Rust 单测,真实切换路径未覆盖。
- **配置热重载**:watcher 已接上(`ConfigWatcher` → 重建 provider client →
  推 `🔄 配置已重载` 通知),但「改文件 → 界面出现通知」这条端到端路径
  未加 pty 用例(依赖文件系统事件时序,易 flaky)。
- **LSP 诊断**:需要真实语言服务器(rust-analyzer 等)推送
  `publishDiagnostics`;`/lsp` 的空态路径有用例覆盖。
- **分析遥测(OTLP)**:需要真实的 OTLP 端点;`[analytics]` 段的解析与
  exporter 启动失败降级有 Rust 单测,导出内容未验证。
- **终端通知的开关 / 通道**:策略判定(`enabled` / `on_*` / `channels`)有
  Rust 单测;实际 OSC9 / BEL 序列发出依赖真实终端,pty 环境下不可观测。

> 已从本清单移出(2026-10 离线实证可测,已纳入 W 组):审批链路
> `⚠ APPROVAL NEEDED`(W1/W2)、`AskUserQuestion` / `ask_user` 面板
> (W3/W4)—— 此前登记为「离线难稳定触发」,实际 mock 脚本注入小写
> 工具名的 `tool_call` 即可稳定触发。
