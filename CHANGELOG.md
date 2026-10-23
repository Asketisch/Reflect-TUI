# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### submodule 能力接线(v1.5 / v1.7 批次)

submodule 指针更新到 `7912323`(归并远端 7 周演进)后,本仓库把此前**未接线**
的引擎能力逐条接到界面上,顺带修掉两处升级带来的编译断点。

**编译面修复**

- `[sandbox].os_level` 由 `bool` 改为 `Option<bool>`(区分「用户未表态」与
  「显式关闭」)—— TUI 按 `reflect-exec` 的同款语义桥接环境变量。
- `EventMsg` 新增 `ToolCallArgsDelta` 变体,TUI 的事件转换 match 补齐分支。

**新接线的引擎能力**

- **工具参数流式预览**(`EventMsg::ToolCallArgsDelta`):模型生成 write / edit
  等长参数工具时,live 区渐进显示「正在对什么动手」;工具完成后,参数摘要
  (`✓ glob (12ms) · *.rs`)留在历史行 —— 此前 `ToolCallBegin.args` 恒为
  `null`,界面完全看不到工具动了哪个文件。整包形态 provider(Ollama 等)用
  `ToolCallBegin.args` 兜底。
- **路由事件可见化**(`EventMsg::Routing`):credential failover / 进冷却 /
  全部失败 / `SpecFallback` 跨模型降级都会在对话流留一行
  (`↻ 路由切换 · main: …` / `↘ 模型降级 · …`)。协议侧文档本就要求
  「TUI 收到后画一行 status」,此前整条被丢弃 —— 配了 `[routing] fallbacks`
  却零提示。
- **跨会话全文搜索**:新增 `reflect-tui session search <query>`
  (`reflect_rollout::search_sessions`)。
- **模型清单发现**:`model = "free"` 这类运行时才知道的 model 值现在在启动期
  解析成真实 id(`reflect_config::resolve_model_spec`),并用
  `routing_policy_with` 把真实 spec 喂给 routing —— 此前会把字面量
  `"free"` 当模型名发给 provider。`/model` 无参列出 provider 的真实模型
  清单(`reflect_config::fetch_catalog` + `reflect_llm::model_catalog` 的
  免费判定三通道),带参则是**真切换**(枚举通道此前只回显「display only」)。
- **目标模式状态读口**:`/goal` 无参显示 `AgentThread::goal_snapshot()` 的
  权威状态(跑到第几轮 / LM 判定 / 累计 token / 3-strike 阻塞计数);
  `/goal <目标> --verify <cmd> --budget <n>` 打通命令式校验与 token 预算
  (此前硬编码 `None`)。
- **配置热重载**:监听 `~/.reflect/config.toml` 变更,重建 provider client、
  必要时切 model,并在对话流提示变更了哪些段(此前改配置必须重启 TUI)。
- **分析遥测**:`[analytics]` 段启动 OTLP/HTTP exporter(此前是死配置)。
- **语言服务诊断**:`/lsp` 显示 `reflect_lsp::DiagnosticRegistry` 的诊断统计
  (此前 LSP manager 被直接丢弃,诊断数据永远读不到)。
- **Agent 待办清单**:`/todo` 显示 `TodoWrite` 写入的清单(引擎侧早已回灌
  模型上下文,界面却一个字都没有)。
- **MCP 精选目录**:`/mcp` overlay 展示 `reflect_mcp::curated_catalog` 与安装
  片段(此前只有「已连接」的服务器,没有「有哪些可选 / 怎么装」)。
- **OS 级沙箱状态**:`/status` 增加 `sandbox:` 段(`reflect_sandbox::OsSandbox`)。
- **终端通知开关**:`[notifications.tui]` 的 `enabled` / `on_turn_complete` /
  `on_approval` / `on_error` / `channels` 真正生效(此前全是死配置)。
- **工作区克隆**:新增 `reflect-tui workspace clone <url>`
  (`reflect_stream::workspace_sync`,与 `reflect workspace clone` 对称)。
- **多 agent 讨论**:新增 `reflect-tui discussion run --config <toml>`
  (`reflect_discussion::cli::run` —— 讨论编排的 CLI 逻辑在库 crate 里,
  是可复用入口)。
- **安全扫描**:新增 `reflect-tui security audit`
  (`reflect_integration::security_scan`;cargo-audit 不可用时如实降级)。

**其它**

- `reflect-tui session search` / `workspace clone` / `discussion run` / `security audit`
  补进 CLI 参数矩阵;`/lsp` / `/todo` / `/goal`(无参)/ `/model`(无参)补进 slash
  全量矩阵(86 条命令,覆盖自审 0 缺口)。
- 修 `test_resume_with_invalid_id_fails_fast` 的断言口径(实现侧早已改成
  中文错误文案,断言仍锚在英文措辞上,是一处先前遗留的红门禁)。

### 全功能测试套件(`tests/`)

对标 submodule `reflect-agent/tests/` 的严格测试标准,新建统一测试入口
`tests/`,用于**发布阶段审查与测试全部功能**。全程离线
(`REFLECT_MODEL=mock/mock-1`,零网络、零 API key)+ 隔离 HOME。

- **能力覆盖层**(`tests/functional/`):真二进制 + pty(pyte 屏幕快照)驱动的
  行为级断言 —— 断言「用户真的看到了什么、真的落了什么盘」,不是「退出码 0」。
  - `test_boot.py`:启动、banner / HUD / composer 占位符锚点、终端标题、
    Ctrl+C / Ctrl+D 退出、resize、缺 provider 的快速失败。
  - `test_slash.py`:**84 条 slash 命令的全量数据驱动矩阵**,逐条锁定运行时
    结局(真行为 / 「尚未接入分发」占位 / 未被识别而静默吞掉 / 打开弹层 /
    退出 TUI)。
  - `test_overlay.py`:可达弹层(transcript / diff / tasks / plugin)与
    **9 条未接线弹层的反向门禁**(防「静默接线」漂移)。
  - `test_turn.py`:对话轮次、流式落屏、工具调用行、HUD 统计、回合分隔线、
    Esc 中断。
  - `test_session.py`:会话落盘、prompt 历史、`-c` / `--resume` 恢复、
    会话名文件、Plan mode 写盘。
  - `test_cli.py`:headless CLI 的**语义**断言(列结构、预览条数、统计口径、
    原子导出)。
  - `test_plugin.py`:插件启动通知、slash 命令注入、`/plugin` overlay。
  - `test_pty_helpers.py`:测试设施自身的单元测试(隔离是否真隔离、按键映射
    是否覆盖、mock 脚本是否合法)。
- **覆盖自审**(`tests/tools/coverage_audit.py`):从 Rust 源码解析
  `SlashCommand` 枚举 + legacy 通道白名单 + 实现臂,与测试矩阵做**机械差集**
  —— 新增 slash 命令却忘了加用例,CI 直接红。另有
  `tests/tools/probe_slash_commands.py` 探针,随时重跑刷新运行时真值表。
- **CLI 参数面**(`tests/rust/cli_core.sh`、`cli_modules.sh`):顶层与
  `session` / `traces` 子命令的参数矩阵与输出契约(100+ 断言)。
- **Rust 编译面**(`tests/rust/workspace_tests.sh`):`cargo test` / `clippy
  -D warnings` / `fmt` / `doc` 收拢到一处。
- **CI 门禁**:新增 `functional-suite` job(ubuntu + macOS),与既有的
  fmt / clippy / test 分工 —— 后者守编译面,前者守行为面。
- **覆盖矩阵**(`tests/COVERAGE.md`):能力 × 覆盖层映射、已知产品限制、
  已知缺陷清单、未覆盖项的诚实清单。

### 缺陷修复

- **`session show` 对真实会话的消息预览恒为空**:`preview_message_body`
  只认 `content` 为字符串 / `{text}` 对象,认不出 v1.2 起的 **ContentBlock
  数组**形态 —— 而那正是引擎的实际落盘形态,导致所有真实会话的预览都是
  空行。改为按数组顺序拼接文本块,并补 4 条单测覆盖三种历史形态。
  (同款缺陷在 submodule `reflect-cli` 亦存在,未同步修复。)

### submodule 接线（reflect-agent 3cb791f → 46dc156）

- **提示词分层库进 TUI（v1.6）**：`build_m4` 对齐 headless `bootstrap_m4` ——
  启动期 `init_library`（home 在前、workspace 在后，另兼容旧版
  `~/.reflect/prompts.md` 降级为 default 层并打 deprecation 日志），默认 agent
  不再携带本地硬编码 `DEFAULT_SYSTEM_PROMPT` 副本（exec 侧已删除），改标
  `SystemPromptSource::BuiltinDefault`，由 pre_loop 按当前模型 spec 从提示词库
  解析 `system.main`（default → 厂商 → 精确模型名三级回退）。TUI 会话由此获得
  9 厂商层（claude / gpt / gemini / kimi / qwen / grok / glm / minimax）与用户
  目录层（`.reflect/prompts/*.md`）的按模型命中。
- **`/prompt` 运行时管理（v1.6.1）**：`/prompt [list|reload|<layer>]` 解析语义
  与 headless `expand_prompt_slash` 一致（关键字大小写不敏感，层名透传保留
  原样），发 `Op::PromptCommand`（不开 turn）；应答 `PromptList` /
  `PromptSwitched` 经会话泵回流为 Notice（可用层 + 当前模型生效层 / 重扫确认）。
  `M4Deps.prompt_dirs` 记录启动目录序供 reload 重扫。per-submission handle
  立即 drop —— 引擎双路发送（per-submission + 会话扇出），TUI 只消费会话泵，
  避免 Notice 重复入列。`/prompt` 已登记进 SlashCommand 枚举（弹窗可发现、
  支持内联参数、任务进行中可用）与 `/help`。

### 能力补齐（对标 codex / claude code / pi 审计后）

- **prompt 历史召回**：Up / Ctrl+P / Ctrl+N / Ctrl+R 此前全是死键 —— vendored
  `ChatComposerHistory` 跑在未接线的 `AppEvent` 往返上（对面是 vendored `App`），
  Ctrl+R 更糟：会真的开一个搜索会话然后永远出不来匹配。新增自包含
  `prompt_history.rs`：JSONL 落盘（temp + rename 原子写，崩溃不会丢全部历史）、
  Up 召回 / Down 越过最新一条还原草稿、跳过空与超长与连续重复、上限 500 条；
  多行草稿时方向键仍归 composer 移动光标；Ctrl+R 反向搜索（大小写不敏感子串、
  重复按找更旧匹配、Enter 接受、Esc 还原草稿）。
- **审批 always-allow 可收窄**：`[A]lways allow` 对 bash 原先写
  `shell_pattern: None` = 放行**所有** shell 命令，而 UI 只显示一行 80 字符
  JSON 摘要。新增 `[S]cope` 键按实际命令派生 `shell_pattern`
  （`git push origin main` → `git push *`，复合命令取第一段），命令解析用
  submodule 新加的 `resolve_bash_command`（按 cmd → command → cmdline 解析别名）。
  Notice 现在明确说出放行范围。
- **审批弹窗补 diff 预览**：Edit / Write 审批显示目标路径 + 最多 3 行 `-` /
  3 行 `+`（按字符截断，CJK 不会 panic）；审批块高度改为按内容动态计算。
- **`/checkpoint` 接 git**：此前每个按钮都是占位通知（"git auto-commit not yet
  wired" / "git reset --hard not yet wired" / "(placeholder)"）。新增
  `git_checkpoint.rs`：create = `git add -A` + 带前缀提交 + tag；list 走
  `for-each-ref refs/tags/reflect-cp/*`（**必须打 tag**：rewind 会先做安全快照
  再 `reset --hard`，那之后安全快照已不在分支历史上，只有 tag 还指着它）；
  rewind 只接受本模块创建的 checkpoint 且**回退前先做安全快照提交**，脏树时
  未提交的工作不会被一次性烧掉。
- **图片附件真正进模型上下文**：`/image` picker 此前只往 composer 插一段
  `@<path>` 文本，而 `Submission::user_input_items` 从未被 TUI 调用 —— 附件
  等于被静默丢掉。改为走 composer 的真实图片附件路径，提交时构建
  `Text` + `LocalImage` items。
- **`/theme` 真正生效**：此前选完只打印 "display only — palette apply not yet
  wired"。新增语义调色板 `theme::Palette`（brand / dim / warn / danger / ok /
  info / plan），statusline 与 metrics 行改引角色而非硬编码颜色；`no-color`
  主题真正剥掉全部颜色，`light` 避开在白底上不可见的 `DarkGray`；选择持久化到
  `<reflect home>/theme`（原子写），启动优先级
  `NO_COLOR` > `REFLECT_TUI_THEME` > 持久化 > 深色默认。

### 修复（对标 codex / claude code 审计后的 TUI 层缺陷）

- **Ctrl-C 不再在回合进行中直接退出 TUI**：此前 Ctrl-C 无条件映射 `Exit`，
  长回合中误触即丢失整个会话。改为对齐 codex / claude code —— busy 时首次
  Ctrl-C 发 `Op::Interrupt` 打断当前回合，空闲时才一次退出；中断请求已发出而
  回合仍未收尾时，1.5s 内第二次 Ctrl-C 放行退出，保证引擎卡死时用户有出路。
  判定逻辑收敛到 `keymap::exit_key_action` 纯函数。
- **`/copy` 与 Ctrl+O 修复**：`adapter::agent_replies_in_order` 早已写好却零调用方，
  而 `copy_last_reply` / `copy_reply_n` / copy-history overlay 都只匹配整段
  `UiHistoryItem::Agent` —— 正常流式路径下 agent 正文全部以 `StreamedAgent`
  片段入 history，导致每次真实对话后 Ctrl+O 与 `/copy` 都报「no agent reply
  yet」。三处统一改走该辅助函数（合并连续流式片段为一条回复）。
- **`/mcp` overlay 不再显示假数据**：此前硬编码一份
  `[("node_repl", "Node.js REPL environment", "connected", 3)]` 虚构清单。改为
  由 `McpServerStarted` / `McpServerFailed` 生命周期事件 upsert 出
  `state.mcp_servers` 连接态快照，overlay 只读快照；没连上就如实显示
  「No MCP servers connected」，连接成功时展开协议事件本就携带的 `tool_names`。
- **全屏模式补绘 ask_user / question 面板**：`draw_fullscreen` 的 layout 漏了
  `question_h` / `ask_user_h` 两段，`--fullscreen` 下引擎阻塞在
  `AskUserQuestion` / `AskUser` 时用户看到的是空白，而键位仍被面板接管 ——
  表现为「界面卡死但按键有反应」。与 inline `draw` 对齐补齐。
- **自动压缩与配额耗尽不再静默**：`ContextCompacted` / `QuotaExhausted` 此前
  整条 `return None` 丢弃，用户看到的是「上下文突然变短」「模型突然不记得
  前面的话」「回合莫名停住」，界面上没有任何解释。改为 Notice，携带压缩策略 /
  移除消息数 / token 前后对比，以及配额 provider / credential / 用量 / 窗口
  重置时间。
- **`/diff` 窄终端下溢 panic**：`diff_render::render_change` 的 `wrap_cols - 4`
  在终端宽度 ≤ 4 时下溢（dev/test profile 开了 overflow-checks）。钳到
  `saturating_sub(4).max(1)`，调用侧 `open_diff_overlay` 的 width 也钳到 ≥ 8。
- `cargo fmt` 收口：`question_panel_key` 内 `apply_select` 签名单行化。

### 正确性收口（潜在 panic / 非原子写 / 无上限增长）

- `/diff` 之外的三处算术缺陷：pager 底栏百分比 `width - pct_w - 1` 在分隔条
  窄于百分比文本时下溢（debug panic / release 回绕成越界 x），装不下就整个不画；
  sixel 缓存 `frame.height()` 为 0 时除零 panic，补零维度守卫；`/usage` 图表
  `value * 4` / 周累加在 overflow-checks 下 panic、release 下回绕成负数让高用量
  渲染成低等级，统一改 `saturating_*`。
- 三处非原子写改 temp + rename：CLI `session export`（崩溃会把用户指定的导出
  路径截断成不完整 markdown）、pets sixel 缓存（`path.exists()` 快路径不校验
  内容，损坏条目会被当有效缓存复用）、prompt 历史与主题持久化。
- `state.history` 此前无上限增长（每个 `StreamedAgent` 片段、每条 `Notice`
  永久留在内存）。钳到 5000 条并同步下调 `history_tail` 游标 —— 它按 history
  下标寻址，不同步会让 scrollback 重复或漏渲染。完整内容本就在 rollout JSONL 里。

### 交互细节收口

- **Ctrl+D 不再吞草稿**：此前 Ctrl-D 在 composer 有内容时也直接退出，写了一半的
  提示词一次误触就丢。改为只在 composer 为空时退出，有草稿时交给 composer
  自己处理。
- **终端标题跟随状态**：此前只在启动时设一次，之后 model 切换 / 权限模式变化
  都不更新。改为每帧比对，变了才重发 OSC 序列（无脑重发是浪费）；非 git 仓库
  时用目录名，多个标签页仍可区分。
- **通知不再每轮都响**：此前每个 `TurnCompleted` 都发 "Reflect finished." ——
  用户刚敲完回车盯着屏幕时这是纯噪音。改为只在干了实事（工具调用 / 子代理）
  或回合跑超过 30 秒时才通知，对齐 claude code。
- **`/retry` 重试入口**：引擎对流式错误自带重试，但硬失败（配额耗尽 / 认证失效 /
  5xx 终结）之后用户此前没有任何重试手段，只能把提示词重新敲一遍。新增
  `/retry` 重发上一条用户提示词（走同一条插件命令展开路径）。

### 能力接线（对齐 headless reflect-exec，引擎能力在 TUI 内真正可用）

- M4 完整装配：agent 定义（`.reflect/agents/`）、技能系统（扫描 + 内置技能包 +
  `load_skill` / `read_skill_resource` 工具）、分层记忆（会话内存 + 项目/用户文件）、
  session notes（JSONL 落盘 + `add_session_note` 工具）、post-compact 文件恢复、
  子代理调用登记表、LLM 摘要压缩（`LlmSummarizer`，Role::Compact 路由，可选
  记忆固化）、prompt 构建器与 coordinator 模式注入。
- `[sanitize]` 脱敏器与 `[hooks]` hook engine（含自定义 shell hooks）注入
  AgentThread —— 此前两段配置在 TUI 内是死信。
- 子代理能力补齐：`[subagent_providers]` 独立 child registry、父技能共享、
  `call_<role>` 工具补入模型 always-on 可见集、`SubagentRuntimeRegistry` +
  会话取消令牌级联（退出 TUI 不留下飞子代理）。
- MCP：`[mcp_servers]` 连接管理器接线，外部工具注册进工具表，生命周期事件
  （started/failed）以 Notice 行进对话流；注册 MCP 资源读取工具。
- LSP：`[lsp_servers]` 连接管理器 + `lsp` 工具接线。
- telemetry：`[telemetry]` 本地 trace sink + `[langfuse]` 云端并行投递。
- 真实分词器：启用 reflect-compact `tokenizer` feature（tiktoken），压缩触发
  阈值与上下文估算不再走 len/3.5 启发式。

### 会话恢复闭环

- `--resume <id>` / `-c` / `-r N` 启动参数真实消费：rollout 索引定位 → replay →
  历史预载注入引擎 + scrollback 回填渲染。
- `/resume` overlay 列出真实历史会话（rollout 索引，名字优先 `/rename` 落盘值）。
- `/archive` 落地：`[archived] ` 前缀重命名（可逆，rollout 暂无独立归档存储），
  y/n 确认后退出时执行；`/delete` 落地：删除当前会话全部落盘文件（JSONL +
  轮转副本 + 名字文件，不可逆），y/n 确认后退出时执行。

### 交互补全

- `/model` 真正切换模型（`AgentConfig::set_model`，registry 校验，下一回合生效）。
- 审批弹窗选 always-allow（a）时，Allow 规则持久化到 `~/.reflect/permissions.toml`，
  重启后同类工具调用继续免审批。
- v1.4 子代理状态快照（`SubagentStatus`）接入 `/tasks` 面板实时刷新。

### 体验

- v1.4 `ToolCallOutputDelta`：长时工具（bash / web_fetch）输出逐段实时上屏
  （live 区 tail，最多 10 行）；`REFLECT_TUI_TOOL_STREAM=0` 可关闭。

### 测试

- 重建 `test/` 离线测试套件（原脚本丢失）：`harness.py`（pty + pyte + TERM/CPR
  自动应答）、`run_all.py`（7 阶段入口）、unit / cli help / binary smoke /
  mock E2E / resume E2E（`-c` 回填断言）。

### 工程

- CI 增加 Windows 测试 job（发布产物已含 Windows 包）。

## [0.1.0] — 初始公开版本

- TUI 表现层（ratatui / crossterm fork）：流式 markdown 渲染、60+ slash 命令、
  审批弹窗、Plan mode 闭环、fork/rewind/checkpoint、会话/任务/技能/主题面板、
  通知、IDE 上下文 IPC、双模式（inline / alt-screen）。
- 核心引擎由 [reflect-agent](https://github.com/Asketisch/Reflect-Agent) 子模块
  提供（协议 v1.5：子代理可观测双通道、request_human_input 持久化、可插拔
  tokenizer 等）。
- 开源基建：LICENSE / NOTICE / CI / release / audit workflows。
