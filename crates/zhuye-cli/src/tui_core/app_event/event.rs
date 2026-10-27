//! AppEvent 核心事件枚举（180 个变体）。
//!
//! 这是 上游 `app_event.rs`（1157 行）的逐字迁移。该枚举是单一内聚的事件类型
//! 定义，其 180 个变体无法按簇拆分到独立文件而不改变 `AppEvent::Variant` 的构造路径
//! （破坏全 crate 的匹配/构造点）。与上游一致保留为单文件。
//!
//! 注：本文件超过 800 行红线，但 AppEvent 为单一内聚枚举定义，拆分将破坏全 crate 的
//! 模式匹配和构造点，按 AGENTS.md「内聚完整状态机例外」保留。

use super::*;

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum AppEvent {
    /// 打开代理选择器以切换当前活动线程。
    OpenAgentPicker,
    /// 将当前线程切换到所选代理。
    SelectAgentThread(ThreadId),

    /// 将当前线程派生为一个临时的侧边对话。
    StartSide {
        parent_thread_id: ThreadId,
        user_message: Option<UserMessage>,
    },

    /// 向指定线程提交一个操作，无论当前焦点在何处。
    SubmitThreadOp {
        thread_id: ThreadId,
        op: AppCommand,
    },

    /// 中断、派生并使用服务端选择的模型重试一次安全缓冲的回合。
    RetrySafetyBufferedTurn {
        thread_id: ThreadId,
        turn_id: String,
        model: String,
        turn: AppCommand,
        prompt: UserMessage,
    },

    /// 将合成的历史查询响应投递到指定线程通道。
    ThreadHistoryEntryResponse {
        thread_id: ThreadId,
        event: HistoryLookupResponse,
    },

    /// 将提交的提示词持久化到跨会话消息历史中。
    AppendMessageHistoryEntry {
        thread_id: ThreadId,
        text: String,
    },

    /// 将从 App git 操作指令中发现的 Git 分支持久化到线程元数据中。
    SyncThreadGitBranch {
        thread_id: ThreadId,
        branch: String,
    },

    /// 按偏移量获取一条持久化的跨会话消息历史记录。
    LookupMessageHistoryEntry {
        thread_id: ThreadId,
        offset: usize,
        log_id: u64,
    },

    /// 获取一批有上限的持久化历史记录，用于反向搜索。
    LookupMessageHistoryBatch {
        thread_id: ThreadId,
        cursor: HistoryBatchCursor,
        log_id: u64,
    },

    /// 开始一个新的会话。
    NewSession,

    /// 输入界面就绪后附加的新启动线程的结果。
    StartupThreadStarted {
        result: Result<AppServerStartedThread, String>,
    },

    /// 清除终端界面（屏幕 + 回滚缓冲区），开启全新会话，并保留
    /// 之前的聊天记录可供恢复。
    ClearUi,

    /// 使用选定的回滚渲染模式重新渲染对话记录。
    RawOutputModeChanged {
        enabled: bool,
    },

    /// 清除当前上下文，开启全新会话，并提交一条初始用户消息。
    ///
    /// 这是 Plan Mode 交接路径：之前的线程仍可恢复，但新会话配置完成后，模型
    /// 只会看到 `text` 中携带的显式提示词。
    ClearUiAndSubmitUserMessage {
        text: String,
    },

    /// 在运行中的 TUI 会话内打开恢复选择器。
    OpenResumePicker,

    /// 在运行中的 TUI 会话内打开外部代理配置迁移选择器。
    OpenExternalAgentConfigMigration,

    /// 在运行中的 TUI 会话内按 UUID 或线程名恢复线程。
    ResumeSessionByIdOrName(String),

    /// 归档当前活动主线程，并在成功后退出。
    ArchiveCurrentThread,

    /// 永久删除当前活动主线程，并在成功后退出。
    DeleteCurrentThread,

    /// 将当前会话派生为一个新线程。
    ForkCurrentSession,

    /// 在选定的提示词之前分支，并在新线程的输入框中重新打开它。
    ForkSessionForPromptEdit {
        thread_id: ThreadId,
        nth_user_message: usize,
        prompt: UserMessage,
    },

    /// 请求退出应用程序。
    ///
    /// 用户发起的退出应使用 `ShutdownFirst`，以便核心清理得以执行，界面
    /// 仅在收到 `ShutdownComplete` 后才退出。`Immediate` 是最后手段的
    /// 逃生通道，它会跳过关闭流程，可能丢弃进行中的工作（例如
    /// 后台任务、输出刷新或子进程清理）。
    Exit(ExitMode),

    /// 请求应用服务器账户登出，成功后再退出。
    Logout,

    /// 因致命错误请求退出应用程序。
    #[allow(dead_code)]
    FatalExitRequest(String),

    /// 将命令转发给 Agent。使用 `AppEvent` 可避免在多层级组件间冒泡传递 channel。
    ReflectOp(AppCommand),

    /// 批准在 TUI 中选中的某次近期自动审阅拒绝的重试。
    ApproveRecentAutoReviewDenial {
        thread_id: ThreadId,
        id: String,
    },

    /// 为给定的查询（`@` 之后的文本）启动一次异步文件搜索。
    /// 应用层可能会取消之前的搜索，以确保同时最多只有一个进行中的搜索。
    StartFileSearch(String),

    /// 已完成的异步文件搜索结果。`query` 回显原始搜索词，
    /// 以便界面判断结果是否仍然相关。
    FileSearchResult {
        query: String,
        matches: Vec<FileMatch>,
    },

    /// 在后台刷新账户速率限制。
    RefreshRateLimits {
        origin: RateLimitRefreshOrigin,
    },

    /// 打开当前线程目标概览/操作菜单。
    OpenThreadGoalMenu {
        thread_id: ThreadId,
    },

    /// 打开当前线程目标对象的编辑器。
    OpenThreadGoalEditor {
        thread_id: Option<ThreadId>,
    },

    /// 具体化并设置或替换当前线程的目标对象。
    SetThreadGoalDraft {
        thread_id: ThreadId,
        draft: GoalDraft,
        mode: ThreadGoalSetMode,
    },

    /// 暂停或恢复当前线程目标。
    SetThreadGoalStatus {
        thread_id: ThreadId,
        status: ThreadGoalStatus,
    },

    /// 清除当前线程目标。
    ClearThreadGoal {
        thread_id: ThreadId,
    },

    /// 刷新速率限制的结果。
    RateLimitsLoaded {
        origin: RateLimitRefreshOrigin,
        hard_stop_generation: u64,
        result: Result<GetAccountRateLimitsResponse, String>,
    },

    /// 打开从 `/usage` 菜单选定的默认 token 活动视图。
    OpenTokenActivity,

    /// 打开从 `/usage` 菜单选定的重置额度流程。
    OpenRateLimitResetCredits,

    /// 确认从重置额度选择器中选定的重置额度。
    OpenRateLimitResetConfirmation {
        picker_request_id: u64,
        confirmation_gate: Arc<AtomicBool>,
        credit_id: Option<String>,
        reset_title: String,
        reset_detail: Option<String>,
        reset_description: String,
    },

    /// 使用稳定的幂等键消耗一个重置额度。
    ConsumeRateLimitResetCredit {
        idempotency_key: String,
        credit_id: Option<String>,
    },

    /// 消耗一个重置额度的结果。
    RateLimitResetCreditConsumed {
        request_id: u64,
        idempotency_key: String,
        credit_id: Option<String>,
        result: Result<ConsumeAccountRateLimitResetCreditResponse, String>,
    },

    /// 为 `/usage` 历史卡片获取账户级 token 活动。
    RefreshTokenActivity {
        request_id: u64,
    },

    /// 获取账户级 token 活动的结果。
    TokenActivityLoaded {
        request_id: u64,
        result: Result<GetAccountTokenUsageResponse, String>,
    },

    /// 为状态栏头条项目获取工作区消息。
    RefreshStatusLineWorkspaceHeadline {
        request_id: u64,
    },

    /// 在活动输出屏障解除后，提交已结算的异步用量输出。
    CommitPendingUsageOutput,

    /// 在流关闭后提交已结算的异步用量输出。
    CommitPendingUsageOutputAfterStreamShutdown,

    /// 发送用户确认过的通知工作区所有者请求。
    SendAddCreditsNudgeEmail {
        credit_type: AddCreditsNudgeCreditType,
    },

    /// 通知工作区所有者的结果。
    AddCreditsNudgeEmailFinished {
        result: Result<AddCreditsNudgeEmailStatus, String>,
    },

    /// 预取连接器的结果。
    ConnectorsLoaded {
        result: Result<ConnectorsSnapshot, String>,
        is_final: bool,
    },

    /// 计算 `/diff` 命令的结果。
    DiffResult(String),

    /// 在底部面板打开应用链接视图。
    OpenAppLink {
        app_id: String,
        title: String,
        description: Option<String>,
        instructions: String,
        url: String,
        is_installed: bool,
        is_enabled: bool,
    },

    /// 在用户的浏览器中打开提供的 URL。
    OpenUrlInBrowser {
        url: String,
    },

    /// 在 Reflect Desktop 中打开当前线程。
    OpenDesktopThread {
        thread_id: ThreadId,
    },

    /// 持久化宠物选择并重新加载环境宠物。
    PetSelected {
        pet_id: String,
    },

    /// 将终端宠物持久化为禁用状态，并移除环境宠物。
    PetDisabled,

    /// 开始加载宠物选择器的侧边预览。
    PetPreviewRequested {
        pet_id: String,
    },

    /// 加载宠物选择器侧边预览的结果。
    PetPreviewLoaded {
        request_id: u64,
        result: Result<crate::tui_core::pets::AmbientPet, String>,
    },

    /// 在配置持久化之前加载所选环境宠物的结果。
    PetSelectionLoaded {
        request_id: u64,
        pet_id: String,
        result: Result<Option<crate::tui_core::pets::AmbientPet>, String>,
    },

    /// 启动期间恢复已配置环境宠物的结果。
    ConfiguredPetLoaded {
        pet_id: String,
        result: Result<Option<crate::tui_core::pets::AmbientPet>, String>,
    },

    /// 刷新应用连接器状态与提及绑定。
    RefreshConnectors {
        force_refetch: bool,
    },

    /// 在组件接受刷新请求后，从应用服务器获取连接器状态。
    FetchConnectorsList {
        force_refetch: bool,
    },

    /// 获取指定工作目录的插件市场状态。
    FetchPluginsList {
        cwd: PathBuf,
    },

    /// 获取指定工作目录的生命周期钩子清单。
    FetchHooksList {
        cwd: PathBuf,
    },

    /// 获取插件市场状态的结果。
    PluginsLoaded {
        cwd: PathBuf,
        result: Result<PluginListResponse, String>,
    },

    /// 从已缓存的响应中打开插件列表。
    OpenPluginsList {
        cwd: PathBuf,
        response: PluginListResponse,
    },

    /// 显式获取远程支持的插件分区的结果。
    PluginRemoteSectionsLoaded {
        cwd: PathBuf,
        marketplaces: Vec<PluginMarketplaceEntry>,
        section_errors: Vec<PluginRemoteSectionError>,
    },

    /// 获取生命周期钩子清单的结果。
    HooksLoaded {
        cwd: PathBuf,
        result: Result<crate::app_server_protocol::HooksListResponse, String>,
    },

    /// 打开添加市场来源的提示框。
    OpenMarketplaceAddPrompt,

    /// 将插件弹窗替换为添加市场时的加载状态。
    OpenMarketplaceAddLoading {
        source: String,
    },

    /// 从提供的来源添加市场。
    FetchMarketplaceAdd {
        cwd: PathBuf,
        source: String,
    },

    /// 添加市场的结果。
    MarketplaceAddLoaded {
        cwd: PathBuf,
        source: String,
        result: Result<MarketplaceAddResponse, String>,
    },

    /// 打开移除市场的确认提示框。
    OpenMarketplaceRemoveConfirm {
        marketplace_name: String,
        marketplace_display_name: String,
    },

    /// 将插件弹窗替换为移除市场时的加载状态。
    OpenMarketplaceRemoveLoading {
        marketplace_display_name: String,
    },

    /// 按名称移除市场。
    FetchMarketplaceRemove {
        cwd: PathBuf,
        marketplace_name: String,
        marketplace_display_name: String,
    },

    /// 移除市场的结果。
    MarketplaceRemoveLoaded {
        cwd: PathBuf,
        marketplace_name: String,
        marketplace_display_name: String,
        result: Result<MarketplaceRemoveResponse, String>,
    },

    /// 将插件弹窗替换为市场升级时的加载状态。
    OpenMarketplaceUpgradeLoading {
        marketplace_name: Option<String>,
    },

    /// 升级已配置的 Git 市场。
    FetchMarketplaceUpgrade {
        cwd: PathBuf,
        marketplace_name: Option<String>,
    },

    /// 升级已配置 Git 市场的结果。
    MarketplaceUpgradeLoaded {
        cwd: PathBuf,
        result: Result<MarketplaceUpgradeResponse, String>,
    },

    /// 将插件弹窗替换为插件详情加载状态。
    OpenPluginDetailLoading {
        plugin_display_name: String,
    },

    /// 从市场获取特定插件的详情。
    FetchPluginDetail {
        cwd: PathBuf,
        params: PluginReadParams,
    },

    /// 获取插件详情的结果。
    PluginDetailLoaded {
        cwd: PathBuf,
        result: Result<PluginReadResponse, String>,
    },

    /// 将插件弹窗替换为安装加载状态。
    OpenPluginInstallLoading {
        plugin_display_name: String,
    },

    /// 将插件弹窗替换为卸载加载状态。
    OpenPluginUninstallLoading {
        plugin_display_name: String,
    },

    /// 从市场安装特定插件。
    FetchPluginInstall {
        cwd: PathBuf,
        location: PluginLocation,
        plugin_name: String,
        plugin_display_name: String,
    },

    /// 安装插件的结果。
    PluginInstallLoaded {
        cwd: PathBuf,
        location: PluginLocation,
        plugin_name: String,
        plugin_display_name: String,
        result: Result<PluginInstallResponse, String>,
    },

    /// 按规范化插件 ID 卸载特定插件。
    FetchPluginUninstall {
        cwd: PathBuf,
        plugin_id: String,
        plugin_display_name: String,
    },

    /// 卸载插件的结果。
    PluginUninstallLoaded {
        cwd: PathBuf,
        plugin_id: String,
        plugin_display_name: String,
        result: Result<PluginUninstallResponse, String>,
    },

    /// 启用或禁用已安装的插件。
    SetPluginEnabled {
        cwd: PathBuf,
        plugin_id: String,
        enabled: bool,
    },

    /// 启用或禁用插件的结果。
    PluginEnabledSet {
        cwd: PathBuf,
        plugin_id: String,
        enabled: bool,
        result: Result<(), String>,
    },

    /// 从当前配置刷新插件提及绑定。
    RefreshPluginMentions,

    /// 刷新插件提及绑定的结果。
    PluginMentionsLoaded {
        plugins: Option<Vec<PluginCapabilitySummary>>,
    },

    /// 推进安装后插件应用授权流程。
    PluginInstallAuthAdvance {
        refresh_connectors: bool,
    },

    /// 中止安装后插件应用授权流程。
    PluginInstallAuthAbandon,

    /// 通过应用服务器 RPC 获取 MCP 清单并将其渲染到历史中。
    FetchMcpInventory {
        detail: McpServerStatusDetail,
        thread_id: Option<ThreadId>,
    },

    /// 通过应用服务器 RPC 获取 MCP 清单的结果。
    McpInventoryLoaded {
        result: Result<Vec<McpServerStatus>, String>,
        detail: McpServerStatusDetail,
        thread_id: Option<ThreadId>,
    },

    /// 在第一帧调度后运行启动技能刷新的结果。
    ///
    /// 此事件仅在启动时触发。交互式技能刷新通过应用命令路径同步处理，
    /// 因为这些调用方期望在其命令完成时，可见的技能状态是当前的。
    SkillsListLoaded {
        result: Result<SkillsListResponse, String>,
    },

    /// 在初始恢复重放行写入回滚缓冲区之前，开始对其进行缓冲。
    BeginInitialHistoryReplayBuffer,

    /// 开始缓冲线程切换的重放单元格，以便最终的回滚写入可以复用
    /// 调整尺寸重排时的尾部渲染器。
    BeginThreadSwitchHistoryReplayBuffer,

    InsertHistoryCell(Box<dyn HistoryCell>),

    /// 在所有重放事件入队后，完成初始恢复重放的缓冲。
    EndInitialHistoryReplayBuffer,

    /// 将对话记录末尾连续的一段流式 `AgentMessageCell` 替换为单个
    /// `AgentMarkdownCell`，后者保存原始 markdown 源码，并在调整尺寸时
    /// 据此重新渲染。
    ///
    /// 由 `ChatWidget::flush_answer_stream_with_separator` 在流结束
    /// 后发出。`App` 处理器会从 `transcript_cells` 末尾向前遍历，
    /// 找到这段 `AgentMessageCell` 并将其拼接为合并单元格。
    /// `cwd` 用于在最终重新渲染时保持本地文件链接显示稳定。
    /// `scrollback_reflow` 允许表格尾部最终化强制已输出的
    /// 终端回滚基于合并的源码单元格重建。
    /// `deferred_history_cell` 允许调用方将最终流尾部加入
    /// 对话记录，而无需先将其临时渲染写入回滚缓冲区。
    ConsolidateAgentMessage {
        source: String,
        cwd: PathBuf,
        inline_visualization_context: Option<InlineVisualizationContext>,
        scrollback_reflow: ConsolidationScrollbackReflow,
        deferred_history_cell: Option<Box<dyn HistoryCell>>,
    },

    /// 将对话记录末尾连续的一段流式 `ProposedPlanStreamCell` 替换为单个
    /// 由源码支持的 `ProposedPlanCell`。
    ///
    /// 由 `ChatWidget::on_plan_item_completed` 在计划流结束
    /// 后发出。
    ConsolidateProposedPlan(String),

    StartCommitAnimation,
    StopCommitAnimation,
    CommitTick,

    /// 更新运行中的应用和组件中的当前推理力度。
    UpdateReasoningEffort(Option<ReasoningEffort>),

    /// 更新运行中的应用和组件中的当前模型标识。
    UpdateModel(String),

    /// 更新运行中的应用和组件中的当前人设。
    UpdatePersonality(Personality),

    /// 在其前置更新事件应用完成后，结束一次设置选择。
    SettingsSelectionClosed,
    /// 在处理关闭事件时发出的所有嵌套设置事件之后运行。
    SettingsSelectionSettled,

    /// 将选定的模型和推理力度持久化到相应的配置中。
    PersistModelSelection {
        model: String,
        effort: Option<ReasoningEffort>,
    },

    /// 将选定的人设持久化到相应的配置中。
    PersistPersonalitySelection {
        personality: Personality,
    },

    /// 将选定的服务等级持久化到相应的配置中。
    PersistServiceTierSelection {
        service_tier: Option<String>,
    },

    /// 选定模型后打开推理选择弹窗。
    OpenReasoningPopup {
        model: ModelPreset,
    },

    /// 为模型打开显式的 Max/Ultra 推理选择弹窗。
    OpenAdvancedReasoningPopup {
        model: ModelPreset,
    },

    /// 在不更改默认值的情况下，将高级推理力度应用到当前对话。
    ApplyAdvancedReasoning {
        model: String,
        effort: ReasoningEffort,
    },

    /// 为选定的模型/推理力度打开 Plan 模式推理范围提示框。
    OpenPlanReasoningScopePrompt {
        model: String,
        effort: Option<ReasoningEffort>,
    },

    /// 打开完整模型选择器（非自动模型）。
    OpenAllModelsPopup {
        models: Vec<ModelPreset>,
    },

    /// 在启用完全访问模式前打开确认提示框。
    OpenFullAccessConfirmation {
        preset: ApprovalPreset,
        return_to_permissions: bool,
        profile_selection: Option<PermissionProfileSelection>,
    },

    /// 打开 Windows 全局可写目录警告。
    /// 若 `preset` 为 `Some`，确认框将在继续时应用提供的
    /// 审批/沙箱配置；若为 `None`，则不进行任何策略更改，
    /// 仅确认/关闭该警告。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    OpenWorldWritableWarningConfirmation {
        preset: Option<ApprovalPreset>,
        profile_selection: Option<PermissionProfileSelection>,
        /// 最多 3 个用于在警告中展示的示例全局可写目录。
        sample_paths: Vec<String>,
        /// 如果数量超过 `sample_paths`，此项携带剩余的数量。
        extra_count: usize,
        /// 当扫描失败（例如 ACL 查询错误）且无法验证保护措施时为 true。
        failed_scan: bool,
    },

    /// 在使用 Agent 模式之前，提示启用 Windows 沙箱功能。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    OpenWindowsSandboxEnablePrompt {
        preset: ApprovalPreset,
        profile_selection: Option<PermissionProfileSelection>,
    },

    /// 在拒绝或提权失败后，打开 Windows 沙箱回退提示框。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    OpenWindowsSandboxFallbackPrompt {
        preset: ApprovalPreset,
        profile_selection: Option<PermissionProfileSelection>,
    },

    /// 开始提权的 Windows 沙箱设置流程。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    BeginWindowsSandboxElevatedSetup {
        preset: ApprovalPreset,
        profile_selection: Option<PermissionProfileSelection>,
    },

    /// 开始非提权的 Windows 沙箱设置流程。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    BeginWindowsSandboxLegacySetup {
        preset: ApprovalPreset,
        profile_selection: Option<PermissionProfileSelection>,
    },

    /// 开始为附加目录授予非提权的读取访问权限。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    BeginWindowsSandboxGrantReadRoot {
        path: String,
    },

    /// 尝试为附加目录授予读取访问权限的结果。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    WindowsSandboxGrantReadRootCompleted {
        path: PathBuf,
        error: Option<String>,
    },

    /// 启用 Windows 沙箱功能并切换到 Agent 模式。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    EnableWindowsSandboxForAgentMode {
        preset: ApprovalPreset,
        mode: WindowsSandboxEnableMode,
        profile_selection: Option<PermissionProfileSelection>,
    },

    /// 在不更改审批预设的情况下更新 Windows 沙箱功能模式。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]

    /// 更新运行中的应用和组件中的当前审批策略。
    UpdateAskForApprovalPolicy(AskForApproval),

    /// 更新运行中的应用和组件中的当前内置活动权限配置。
    UpdateActivePermissionProfile(ActivePermissionProfile),

    /// 选择指定的权限配置，可选地同时应用内置模式设置。
    SelectPermissionProfile(PermissionProfileSelection),

    /// 更新运行中的应用和组件中的当前审批审阅者。
    UpdateApprovalsReviewer(ApprovalsReviewer),

    /// 更新功能开关并将其持久化到顶层配置。
    UpdateFeatureFlags {
        updates: Vec<(Feature, bool)>,
    },

    /// 更新记忆设置并将其持久化到 config.toml。
    UpdateMemorySettings {
        use_memories: bool,
        generate_memories: bool,
    },

    /// 通过应用服务器清除所有已持久化的本地记忆工件。
    ResetMemories,

    /// 更新全局可写目录警告是否已被确认。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    UpdateWorldWritableWarningAcknowledged(bool),

    /// 更新会话的速率限制切换提示是否已被确认。
    UpdateRateLimitSwitchPromptHidden(bool),

    /// 更新内存中 Plan 模式特有的推理力度。
    UpdatePlanModeReasoningEffort(Option<ReasoningEffort>),

    /// 持久化全局可写目录警告的确认标志。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    PersistWorldWritableWarningAcknowledged,

    /// 持久化速率限制切换提示的确认标志。
    PersistRateLimitSwitchPromptHidden,

    /// 持久化 Plan 模式特有的推理力度。
    PersistPlanModeReasoningEffort(Option<ReasoningEffort>),

    /// 持久化模型迁移提示的确认标志。
    PersistModelMigrationPromptAcknowledged {
        from_model: String,
        to_model: String,
    },

    /// 在用户确认继续后，跳过下一次全局可写扫描（一次性）。
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    SkipNextWorldWritableScan,

    /// 重新打开审批预设弹窗。
    OpenApprovalsPopup,

    /// 打开技能列表弹窗。
    OpenSkillsList,

    /// 打开技能启用/禁用选择器。
    OpenManageSkillsPopup,

    /// 按路径启用或禁用技能。
    SetSkillEnabled {
        path: AbsolutePathBuf,
        enabled: bool,
    },

    /// 按连接器 ID 启用或禁用应用。
    SetAppEnabled {
        id: String,
        enabled: bool,
    },

    /// 按稳定的钩子键启用或禁用钩子。
    SetHookEnabled {
        key: String,
        enabled: bool,
    },

    /// 按稳定的钩子键信任某个钩子的当前定义。
    TrustHook {
        key: String,
        current_hash: String,
    },

    /// 按稳定的钩子键信任一个或多个钩子的当前定义。
    TrustHooks {
        updates: Vec<crate::tui_core::hooks_rpc::HookTrustUpdate>,
    },

    /// 持久化钩子启用状态的结果。
    HookEnabledSet {
        key: String,
        enabled: bool,
        result: Result<(), String>,
    },

    /// 持久化钩子信任状态的结果。
    HookTrusted {
        result: Result<(), String>,
    },

    /// 通知管理技能弹窗已关闭。
    ManageSkillsClosed,

    /// 重新打开权限预设弹窗。
    OpenPermissionsPopup,

    /// 从审阅弹窗打开分支选择器选项。
    OpenReviewBranchPicker(PathBuf),

    /// 从审阅弹窗打开提交选择器选项。
    OpenReviewCommitPicker(PathBuf),

    /// 从审阅弹窗打开自定义提示词选项。
    OpenReviewCustomPrompt,

    /// 使用显式的协作掩码提交用户消息。
    SubmitUserMessageWithMode {
        text: String,
        collaboration_mode: CollaborationModeMask,
    },

    /// 打开审批弹窗。
    FullScreenApprovalRequest(ApprovalRequest),

    /// 在用户选择类别后，打开反馈备注输入覆盖层。
    OpenFeedbackNote {
        category: FeedbackCategory,
        include_logs: bool,
    },

    /// 选择类别后，打开反馈上传同意弹窗。
    OpenFeedbackConsent {
        category: FeedbackCategory,
    },

    /// 通过应用服务器的反馈 RPC 为当前线程提交反馈。
    SubmitFeedback {
        category: FeedbackCategory,
        reason: Option<String>,
        turn_id: Option<String>,
        include_logs: bool,
    },

    /// 由 TUI 发起的反馈上传请求的结果。
    FeedbackSubmitted {
        origin_thread_id: Option<ThreadId>,
        category: FeedbackCategory,
        include_logs: bool,
        result: Result<String, String>,
    },

    /// 在正常绘制完成后启动外部编辑器。
    LaunchExternalEditor,

    /// 用于状态栏渲染的当前 Git 分支的异步更新。
    StatusLineBranchUpdated {
        cwd: PathBuf,
        branch: Option<String>,
    },
    /// 用于状态栏渲染的 Git 摘要字段的异步更新。
    StatusLineGitSummaryUpdated {
        cwd: PathBuf,
        summary: crate::tui_core::chatwidget::StatusLineGitSummary,
    },
    /// 用于状态栏渲染的工作区通知头条的异步更新。
    StatusLineWorkspaceHeadlineUpdated {
        request_id: u64,
        result: Result<crate::tui_core::workspace_messages::WorkspaceHeadlineFetchResult, String>,
    },
    /// 应用用户确认的状态栏项目排序/选择。
    StatusLineSetup {
        items: Vec<StatusLineItem>,
        use_theme_colors: bool,
    },
    /// 关闭状态栏设置界面而不更改配置。
    StatusLineSetupCancelled,

    /// 应用用户确认的终端标题项目排序/选择。
    TerminalTitleSetup {
        items: Vec<TerminalTitleItem>,
    },
    /// 在设置界面打开期间应用临时的终端标题预览。
    TerminalTitleSetupPreview {
        items: Vec<TerminalTitleItem>,
    },
    /// 关闭终端标题设置界面而不更改配置。
    TerminalTitleSetupCancelled,

    /// 应用用户确认的语法主题选择。
    SyntaxThemeSelected {
        name: String,
    },

    /// 运行时语法主题预览已更改；刷新由主题派生的界面颜色。
    SyntaxThemePreviewed,

    /// 为选定的键位映射操作打开设置/移除操作。
    OpenKeymapActionMenu {
        context: String,
        action: String,
    },

    /// 在为操作替换某个绑定之前，打开绑定选择。
    OpenKeymapReplaceBindingMenu {
        context: String,
        action: String,
    },

    /// 为选定的键位映射操作打开按键捕获。
    OpenKeymapCapture {
        context: String,
        action: String,
        intent: KeymapEditIntent,
    },

    /// 打开键位映射按键检查器。
    OpenKeymapDebug,

    /// 将捕获到的按键应用到选定的键位映射操作。
    KeymapCaptured {
        context: String,
        action: String,
        key: String,
        intent: KeymapEditIntent,
    },

    /// 移除选定键位映射操作的自定义根绑定。
    KeymapCleared {
        context: String,
        action: String,
    },
}
