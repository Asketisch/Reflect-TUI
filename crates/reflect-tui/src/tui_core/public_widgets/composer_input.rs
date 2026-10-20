//! 内部 ChatComposer 的公开封装，提供简单、可复用的文本输入能力。
//!
//! 这里暴露一个最小化接口，供其他 crate（例如 Reflect 任务）复用成熟的
//! composer 行为：多行输入、粘贴启发式检测、Enter 提交以及 Shift+Enter 换行。

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::time::Duration;

use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::bottom_pane::ChatComposer;
use crate::tui_core::bottom_pane::InputResult;
use crate::tui_core::bottom_pane::slash_commands::PluginCommandEntry;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::slash_command::SlashCommand;

/// 将按键事件送入 ComposerInput 后返回的动作。
#[derive(Debug)]
pub enum ComposerAction {
    /// 用户提交了当前文本（通常通过 Enter）。包含已提交的文本。
    Submitted(String),
    /// 用户选择或输入了一个斜杠命令（无参数）。
    /// 在 composer 解析出 `InputResult::Command` 时产生。
    Command(SlashCommand),
    /// 用户输入了带参数的斜杠命令（例如 `/plan refactor`）。
    /// 在 composer 解析出 `InputResult::CommandWithArgs` 时产生。
    CommandWithArgs(SlashCommand, String),
    /// 未发生提交；若 `needs_redraw()` 返回 true，UI 可能需要重绘。
    None,
}

/// 内部 `ChatComposer` 的最小化公开封装，表现为一个
/// 可复用的、带提交语义的文本输入框。
pub struct ComposerInput {
    inner: ChatComposer,
    _tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    rx: tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    /// 跨会话 prompt 历史(Up / Ctrl+P / Ctrl+R 召回)。
    ///
    /// 自包含实现而非 vendored `ChatComposerHistory`:后者跑在
    /// `LookupMessageHistoryEntry` → `ThreadHistoryEntryResponse` 往返上,
    /// 对面是未接线的 vendored `App`,本路径把那些 AppEvent 全当空操作丢掉。
    history: crate::prompt_history::PromptHistory,
    /// 历史落盘路径(`None` = 不落盘,仅内存)。
    history_path: Option<std::path::PathBuf>,
    /// Ctrl+R 反向搜索会话。`Some` 时按键先归搜索,不进编辑器。
    search: Option<crate::prompt_history::HistorySearch>,
}

impl ComposerInput {
    /// 使用中性占位文本创建一个新的 composer 输入。
    pub fn new() -> Self {
        Self::new_with_placeholder("Compose new task".to_string())
    }

    /// 使用自定义占位文本创建一个新的 composer 输入。
    pub fn new_with_placeholder(placeholder: String) -> Self {
        Self::new_with_config(placeholder, /*disable_paste_burst*/ false)
    }

    /// 使用显式的粘贴突发（paste-burst）配置创建一个新的 composer 输入。
    /// 当 `disable_paste_burst` 为 true 时，每个字符会被立即插入，
    /// 不进行突发检测（适用于测试或非交互场景）。
    pub fn new_with_config(placeholder: String, disable_paste_burst: bool) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AppEventSender::new(tx.clone());
        // `enhanced_keys_supported=true` 启用 Shift+Enter 换行的提示/行为。
        let mut inner = ChatComposer::new(
            /*has_input_focus*/ true,
            sender,
            /*enhanced_keys_supported*/ true,
            placeholder,
            disable_paste_burst,
        );
        // Plan 模式(`/plan`、`/exit-plan`、Plan mode nudge)是本分支的核心能力,
        // 默认开启。ChatComposer 的构造默认值是 false(留给 ChatWidget 自行计算),
        // 而 ComposerInput 是 stable TUI 主循环使用的 composer,这里显式打开。
        inner.set_collaboration_modes_enabled(true);
        // 启用 @ 文件提及（mention）弹窗。ChatComposer 的默认值是 false；
        // stable TUI 主循环使用的是 ComposerInput，因此在这里显式启用。
        inner.set_mentions_v2_enabled(true);
        // 为长时运行任务的目标模式启用 /goal 命令。ChatWidget 通过
        // Feature::Goals 特性开关来控制该命令，但 stable TUI 路径没有
        // 特性标志系统，因此无条件启用。
        inner.set_goal_command_enabled(true);
        // 启用 /personality 命令，让用户可以切换沟通风格。
        inner.set_personality_command_enabled(true);
        // 启用 /plugin 命令 —— 打开真实已装插件 overlay(与 submodule 的
        // 插件系统接线配套;ChatComposer 默认 false,留给宿主自行开启)。
        inner.set_plugins_command_enabled(true);
        // 跨会话 prompt 历史:从磁盘加载(坏文件退化为空,不阻断启动)。
        let history_path = crate::prompt_history::default_path();
        let history = match &history_path {
            Some(p) => crate::prompt_history::PromptHistory::load(p),
            None => crate::prompt_history::PromptHistory::default(),
        };
        Self {
            inner,
            _tx: tx,
            rx,
            history,
            history_path,
            search: None,
        }
    }

    /// 用显式的历史文件路径创建(测试用;生产路径走 `default_path()`)。
    #[cfg(test)]
    pub(crate) fn new_with_history_path(path: std::path::PathBuf) -> Self {
        let mut s = Self::new_with_config(String::new(), /*disable_paste_burst*/ true);
        s.history = crate::prompt_history::PromptHistory::load(&path);
        s.history_path = Some(path);
        s
    }

    /// 若输入为空则返回 true。
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// 获取当前的 composer 文本（bash 模式处于激活状态时包含 `!` 前缀）。
    ///
    /// 用于将现有草稿内容填充给外部编辑器（Ctrl+G），这样用户编辑的是
    /// 自己已输入的内容，而不是一个空缓冲区。委托给底层的
    /// `ChatComposer::current_text`。
    pub fn current_text(&self) -> String {
        self.inner.current_text()
    }

    /// 清空输入文本。
    pub fn clear(&mut self) {
        self.inner
            .set_text_content(String::new(), Vec::new(), Vec::new());
    }

    /// 在运行时更新占位文本（对内部 `ChatComposer::set_placeholder_text` 的薄封装）。
    ///
    /// plan 模式用它来在默认提示与 plan 模式提示（"describe the task or /exit-plan …"）之间
    /// 切换 composer 的提示内容。
    pub fn set_placeholder(&mut self, placeholder: String) {
        self.inner.set_placeholder_text(placeholder);
    }

    /// 注入插件命令快照(对内部 `ChatComposer::set_plugin_commands` 的薄封装)。
    /// 命令会出现在 slash 弹窗补全中,并通过 `validate_submission` 白名单。
    pub(crate) fn set_plugin_commands(&mut self, commands: Vec<PluginCommandEntry>) {
        self.inner.set_plugin_commands(commands);
    }

    /// 将一个按键事件送入 composer，并返回高层动作。
    ///
    /// 当用户输入或选择不带参数的斜杠命令（例如 `/compact`、`/vim`）时返回
    /// `Command(cmd)`。当用户输入带参数的斜杠命令（例如 `/plan refactor`）时返回
    /// `CommandWithArgs(cmd, args)`。当用户提交普通文本消息时返回 `Submitted(text)`。
    pub fn input(&mut self, key: KeyEvent) -> ComposerAction {
        // Ctrl+R 反向搜索:必须在 vendored composer 之前拦截。它自带的
        // `begin_history_search` 会开一个搜索会话,然后把遍历委托给
        // `ChatComposerHistory` —— 后者要 `ThreadHistoryEntryResponse` 往返,
        // 而本路径把那些 AppEvent 当空操作丢掉,于是会话开着但永远出不来匹配。
        // 自己实现一套,让 Ctrl+R 真的能用。
        if self.try_history_search(&key) {
            self.handle_app_events();
            return ComposerAction::None;
        }
        // Up / Down / Ctrl+P / Ctrl+N 在**单行**输入时归 prompt 历史;
        // 多行时仍归 composer 做光标移动(否则多行草稿没法用方向键编辑)。
        // 正在浏览历史时同样优先归历史,保证能一路 Down 回到草稿。
        if self.try_history_navigation(&key) {
            self.handle_app_events();
            return ComposerAction::None;
        }
        let action = match self.inner.handle_key_event(key).0 {
            InputResult::Submitted { text, .. } => {
                self.record_submitted(&text);
                ComposerAction::Submitted(text)
            }
            InputResult::Command(cmd) => ComposerAction::Command(cmd),
            InputResult::CommandWithArgs(cmd, args, _) => {
                ComposerAction::CommandWithArgs(cmd, args)
            }
            _ => ComposerAction::None,
        };
        self.handle_app_events();
        action
    }

    /// 是否单行输入(无换行)。多行时 Up/Down 必须留给 composer 移动光标。
    fn is_single_line(&self) -> bool {
        !self.inner.current_text().contains('\n')
    }

    /// 尝试把按键当历史导航消费。命中返回 `true`(调用方不应再交给 composer)。
    ///
    /// 键位对齐 shell / readline / claude code:
    /// - `Up` / `Ctrl+P` → 更旧的一条
    /// - `Down` / `Ctrl+N` → 更新的一条(越过最新一条还原草稿)
    fn try_history_navigation(&mut self, key: &KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        // 带 Alt/Ctrl(除 P/N)修饰的组合键不抢 —— 留给 composer 的词级移动等。
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if alt {
            return false;
        }
        let go_older = match key.code {
            KeyCode::Up => true,
            KeyCode::Char('p') if ctrl => true,
            _ => false,
        };
        let go_newer = match key.code {
            KeyCode::Down => true,
            KeyCode::Char('n') if ctrl => true,
            _ => false,
        };
        if !go_older && !go_newer {
            return false;
        }
        // 多行且不在浏览历史 → 方向键归 composer。
        if !self.is_single_line() && !self.history.is_browsing() {
            return false;
        }
        if self.history.is_empty() && !self.history.is_browsing() {
            return false;
        }
        if go_older {
            let text = self.inner.current_text();
            match self.history.navigate_up(&text) {
                Some(entry) => {
                    self.set_text(&entry);
                    true
                }
                None => false, // 已到最旧:不消费,让 composer 正常处理(无副作用)
            }
        } else {
            match self.history.navigate_down() {
                Some(entry) => {
                    self.set_text(&entry);
                    true
                }
                None => false,
            }
        }
    }

    /// 整段替换 composer 文本(历史导航用)。
    fn set_text(&mut self, text: &str) {
        self.inner
            .set_text_content(text.to_string(), Vec::new(), Vec::new());
    }

    /// Ctrl+R 反向搜索。命中(消费了按键)返回 `true`。
    ///
    /// 键位对齐 readline / zsh / claude code:
    /// - `Ctrl+R` 开搜索;搜索中再按 = 更旧的匹配
    /// - `Ctrl+S` / `Down` = 更新的匹配
    /// - 普通字符追加到 query;`Backspace` 退格
    /// - `Enter` 接受当前匹配并退出;`Esc` / `Ctrl+C` 还原草稿并退出
    fn try_history_search(&mut self, key: &KeyEvent) -> bool {
        use crate::prompt_history::SearchStep;
        if key.kind == KeyEventKind::Release {
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if alt {
            return false;
        }
        let is_ctrl_r = ctrl && matches!(key.code, KeyCode::Char('r' | 'R'));
        let is_ctrl_s = ctrl && matches!(key.code, KeyCode::Char('s' | 'S'));

        // 开搜索:仅当有历史可搜,且当前没有 slash/mention 弹窗占用字符键。
        if is_ctrl_r && self.search.is_none() {
            if self.history.is_empty() {
                return false;
            }
            self.search = Some(crate::prompt_history::HistorySearch::new(
                self.inner.current_text(),
            ));
            return true;
        }

        let Some(search) = self.search.as_mut() else {
            return false;
        };
        let entries = self.history.entries().to_vec();

        // 取消:还原草稿。
        if key.code == KeyCode::Esc || (ctrl && matches!(key.code, KeyCode::Char('c' | 'C'))) {
            let draft = search.original_draft.clone();
            self.search = None;
            self.set_text(&draft);
            return true;
        }
        // 接受当前匹配。
        if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE {
            let accepted = match search.match_idx {
                Some(i) => entries.get(i).cloned(),
                None => None,
            };
            self.search = None;
            if let Some(text) = accepted {
                self.set_text(&text);
            }
            return true;
        }
        // 更旧 / 更新。
        if is_ctrl_r || key.code == KeyCode::Up {
            if let SearchStep::Matched(t) = search.older(&entries) {
                self.set_text(&t);
            }
            return true;
        }
        if is_ctrl_s || key.code == KeyCode::Down {
            if let SearchStep::Matched(t) = search.newer(&entries) {
                self.set_text(&t);
            }
            return true;
        }
        // query 编辑。
        match key.code {
            KeyCode::Char(c) if !ctrl => {
                if let SearchStep::Matched(t) = search.push_char(c, &entries) {
                    self.set_text(&t);
                }
                true
            }
            KeyCode::Backspace if !ctrl => {
                if let SearchStep::Matched(t) = search.pop_char(&entries) {
                    self.set_text(&t);
                }
                true
            }
            _ => true, // 搜索中吞掉其余键,不让它们漏进编辑器
        }
    }

    /// 当前是否处于 Ctrl+R 搜索会话(供宿主渲染 `(reverse-i-search)` 提示)。
    pub fn history_search_query(&self) -> Option<&str> {
        self.search.as_ref().map(|s| s.query())
    }

    /// 记录一条已提交的提示词进历史并落盘。
    fn record_submitted(&mut self, text: &str) {
        self.history.reset_cursor();
        if let Some(p) = self.history_path.clone() {
            self.history.record(&p, text);
        }
    }

    /// 历史条目数(测试 / 诊断用)。
    #[cfg(test)]
    pub(crate) fn history_len(&self) -> usize {
        self.history.entries().len()
    }

    pub fn handle_paste(&mut self, pasted: String) -> bool {
        let handled = self.inner.handle_paste(pasted);
        self.handle_app_events();
        handled
    }

    /// 覆盖 composer 下方显示的页脚提示项。
    /// 每个元组渲染为 "<key> <label>"，其中按键部分会带样式。
    pub fn set_hint_items(&mut self, items: Vec<(impl Into<String>, impl Into<String>)>) {
        let mapped: Vec<(String, String)> = items
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self.inner.set_footer_hint_override(Some(mapped));
    }

    /// 清除之前设置的自定义提示项，并恢复默认提示。
    pub fn clear_hint_items(&mut self) {
        self.inner.set_footer_hint_override(/*items*/ None);
    }

    /// 给定宽度下期望的高度（以行为单位）。
    pub fn desired_height(&self, width: u16) -> u16 {
        self.inner.desired_height(width)
    }

    /// 计算给定区域下光标在屏幕上的位置。
    pub fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.inner.cursor_pos(area)
    }

    /// 将输入渲染到给定 `area` 的缓冲区中。
    pub fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        self.inner.render(area, buf);
    }

    /// 切换底层文本区的 Vim 编辑模式（NORMAL/INSERT），并返回新的启用状态。
    /// Reflect 侧钩子：`/vim` 斜杠命令调用此方法来开关 Vim；返回的状态
    /// 驱动状态栏中的 `[NORMAL]` 指示器。
    ///
    /// 封装 `ChatComposer::toggle_vim_enabled`（Phase 9.2 薄封装）。
    pub fn toggle_vim_enabled(&mut self) -> bool {
        self.inner.toggle_vim_enabled()
    }

    /// 若当前正在进行粘贴突发检测则返回 true。
    pub fn is_in_paste_burst(&self) -> bool {
        self.inner.is_in_paste_burst()
    }

    /// 若按键间隔超时已到期，则刷新挂起的粘贴突发。
    /// 若文本发生变化且需要重绘，则返回 true。
    pub fn flush_paste_burst_if_due(&mut self) -> bool {
        let flushed = self.inner.flush_paste_burst_if_due();
        self.handle_app_events();
        flushed
    }

    /// 将任何挂起的粘贴突发缓冲区强制刷新为文本区中的实际文本。
    ///
    /// 与 `flush_paste_burst_if_due` 不同，此方法绕过了基于时间的
    /// `flush_if_due` 检查。它的原理是向 composer 注入一个非字符按键事件
    /// （例如 `KeyCode::Null`），从而触发
    /// `ChatComposer::handle_input_basic_with_time` 内部的
    /// `flush_before_modified_input`（任何不是 `Char` 或 `Enter` 的按键码都会触发刷新）。
    ///
    /// 这在提交前至关重要：如果用户输入速度足够快从而触发了粘贴突发检测，
    /// 缓冲的字符仍被 `PasteBurst::buffer` 持有。若不强制刷新，`Enter` 会被
    /// 当作换行处理（经由 `append_newline_if_active`）而不是提交，
    /// 用户消息将永远不会进入历史记录。
    pub fn force_flush_paste_burst(&mut self) {
        let key = KeyEvent {
            code: KeyCode::Null,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let _ = self.inner.handle_key_event(key);
        self.handle_app_events();
    }

    /// 粘贴突发处于激活状态时，建议用于调度下一个微刷新帧的延迟。
    pub fn recommended_flush_delay() -> Duration {
        crate::tui_core::bottom_pane::ChatComposer::recommended_paste_flush_delay()
    }

    /// 收集并从内部通道返回所有待处理的 `AppEvent`。
    ///
    /// 与内部的 `drain_app_events` 不同，此方法会返回事件，
    /// 以便调用方处理它们（例如文件搜索结果、历史搜索批次）。
    /// 在主循环中调用此方法来消费 composer 内部派发的事件。
    #[allow(private_interfaces)]
    pub fn take_app_events(&mut self) -> Vec<crate::tui_core::app_event::AppEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            events.push(ev);
        }
        events
    }

    /// 排空内部 app-event 通道，通过扫描当前目录来处理文件搜索请求，
    /// 并将结果回传给 composer 的提及（mention）弹窗。
    fn handle_app_events(&mut self) {
        use crate::tui_core::app_event::AppEvent;
        let events = self.take_app_events();
        for event in events {
            if let AppEvent::StartFileSearch(query) = event {
                let matches = file_search(&query);
                self.inner.on_file_search_result(query, matches);
            }
            // 在 stable TUI 路径中，其他 app 事件均为空操作。
        }
    }
}

/// 遵循 .gitignore 的模糊文件搜索。返回按分数排序的最多 50 条匹配结果。
/// 用于填充 stable TUI 路径中的 @ 提及（mention）弹窗。
fn file_search(query: &str) -> Vec<crate::file_search::FileMatch> {
    use crate::file_search::{FileMatch, MatchType};
    use crate::utils_fuzzy_match::fuzzy_match;
    use std::path::PathBuf;

    if query.is_empty() {
        return Vec::new();
    }
    let query_lower = query.to_lowercase();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut results: Vec<(PathBuf, i32)> = Vec::new();
    let walker = ignore::WalkBuilder::new(&cwd)
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(&cwd).unwrap_or(path);
        let rel_str = rel.to_string_lossy();
        // 跳过常见的大目录/二进制目录
        if rel_str.starts_with("target/")
            || rel_str.starts_with("node_modules/")
            || rel_str.starts_with(".git/")
        {
            continue;
        }
        let filename = rel
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| rel_str.to_string());
        let filename_lower = filename.to_lowercase();
        let rel_lower = rel_str.to_lowercase();
        let score = fuzzy_match(&filename_lower, &query_lower)
            .map(|(_, s)| s)
            .max(fuzzy_match(&rel_lower, &query_lower).map(|(_, s)| s))
            .unwrap_or(0);
        if score > 0 {
            results.push((rel.to_path_buf(), score));
        }
        if results.len() >= 200 {
            break;
        }
    }
    results.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    results.truncate(50);
    results
        .into_iter()
        .map(|(path, score)| FileMatch {
            path,
            line_number: None,
            indices: None,
            match_type: MatchType::File,
            score,
        })
        .collect()
}

impl Default for ComposerInput {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_text_reflects_typed_input() {
        // current_text() 是外部编辑器 (Ctrl+G) 的 draft seed,必须真实回读
        // composer 内容,否则用户已有输入会被空 buffer 覆盖丢失。
        let mut input =
            ComposerInput::new_with_config(String::new(), /*disable_paste_burst*/ true);
        assert!(input.is_empty());
        assert_eq!(input.current_text(), "");

        input.handle_paste("hello 世界".to_string());
        assert_eq!(input.current_text(), "hello 世界");
    }

    #[test]
    fn current_text_clears_to_empty() {
        let mut input =
            ComposerInput::new_with_config(String::new(), /*disable_paste_burst*/ true);
        input.handle_paste("draft".to_string());
        input.clear();
        assert_eq!(input.current_text(), "");
        assert!(input.is_empty());
    }

    fn up() -> KeyEvent {
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
    }
    fn down() -> KeyEvent {
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    }
    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    #[test]
    fn up_down_recalls_submitted_prompts_and_restores_draft() {
        // 回归:Up / Ctrl+P / Ctrl+R 此前全是死键 —— vendored 历史机制跑在
        // 未接线的 AppEvent 往返上,外层主循环把那些事件当空操作丢掉。
        let dir = std::env::temp_dir().join("reflect-composer-history-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prompt_history.jsonl");

        let mut input = ComposerInput::new_with_history_path(path.clone());
        // 提交两条提示词(经 input() 走 Submitted 路径,自动入历史)。
        input.handle_paste("first prompt".to_string());
        assert!(matches!(
            input.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            ComposerAction::Submitted(_)
        ));
        input.handle_paste("second prompt".to_string());
        input.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(input.history_len(), 2);
        assert!(input.is_empty(), "提交后 composer 应清空");

        // 写个草稿,然后 Up 召回。
        input.handle_paste("wip draft".to_string());
        input.input(up());
        assert_eq!(input.current_text(), "second prompt");
        input.input(up());
        assert_eq!(input.current_text(), "first prompt");
        // 已到最旧:再 Up 不改变内容。
        input.input(up());
        assert_eq!(input.current_text(), "first prompt");
        // Down 回到 second,再 Down 越过最新一条 → 还原草稿。
        input.input(down());
        assert_eq!(input.current_text(), "second prompt");
        input.input(down());
        assert_eq!(
            input.current_text(),
            "wip draft",
            "必须还原用户原本在写的草稿"
        );
    }

    #[test]
    fn ctrl_p_and_ctrl_n_alias_up_down() {
        let dir = std::env::temp_dir().join("reflect-composer-history-ctrlnp");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prompt_history.jsonl");

        let mut input = ComposerInput::new_with_history_path(path);
        input.handle_paste("only one".to_string());
        input.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        input.input(ctrl('p'));
        assert_eq!(input.current_text(), "only one", "Ctrl+P 应等价 Up");
        input.input(ctrl('n'));
        assert_eq!(input.current_text(), "", "Ctrl+N 越过最新一条应还原空草稿");
    }

    #[test]
    fn multiline_input_keeps_arrow_keys_for_cursor() {
        // 多行草稿里 Up/Down 必须归 composer 移动光标,不能被历史抢走 ——
        // 否则用户没法用方向键编辑多行提示词。
        let dir = std::env::temp_dir().join("reflect-composer-history-multiline");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prompt_history.jsonl");

        let mut input = ComposerInput::new_with_history_path(path);
        input.handle_paste("history entry".to_string());
        input.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        // 造一个两行草稿。
        input.handle_paste("line one".to_string());
        input.input(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT, // Shift+Enter = 换行
        ));
        input.handle_paste("line two".to_string());
        assert!(input.current_text().contains('\n'), "应为多行草稿");

        // Up 不得把历史条目灌进来。
        input.input(up());
        assert!(
            !input.current_text().contains("history entry"),
            "多行时 Up 必须归 composer: {}",
            input.current_text()
        );
    }

    #[test]
    fn empty_history_leaves_arrows_to_composer() {
        let dir = std::env::temp_dir().join("reflect-composer-history-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prompt_history.jsonl");

        let mut input = ComposerInput::new_with_history_path(path);
        input.handle_paste("draft".to_string());
        input.input(up());
        assert_eq!(input.current_text(), "draft", "空历史不得改写草稿");
    }

    #[test]
    fn ctrl_r_reverse_search_accepts_and_cancels() {
        // 回归:Ctrl+R 此前开的是 vendored 搜索会话,遍历委托给
        // ChatComposerHistory,而后者要 ThreadHistoryEntryResponse 往返 ——
        // 本路径把那些 AppEvent 当空操作丢掉,于是会话开着但永远出不来匹配。
        let dir = std::env::temp_dir().join("reflect-composer-history-ctrlr");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prompt_history.jsonl");

        let mut input = ComposerInput::new_with_history_path(path);
        for p in ["fix the parser", "add tests for parser", "write docs"] {
            input.handle_paste(p.to_string());
            input.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }

        // 开搜索,输入 "par" → 最新匹配 "add tests for parser"。
        input.input(ctrl('r'));
        assert_eq!(input.history_search_query(), Some(""));
        input.input(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        input.input(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        input.input(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert_eq!(input.history_search_query(), Some("par"));
        assert_eq!(input.current_text(), "add tests for parser");

        // 再按 Ctrl+R → 更旧的匹配。
        input.input(ctrl('r'));
        assert_eq!(input.current_text(), "fix the parser");

        // Enter 接受并退出搜索。
        input.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(input.history_search_query(), None, "Enter 后应退出搜索");
        assert_eq!(input.current_text(), "fix the parser");
    }

    #[test]
    fn ctrl_r_esc_restores_original_draft() {
        let dir = std::env::temp_dir().join("reflect-composer-history-ctrlesc");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prompt_history.jsonl");

        let mut input = ComposerInput::new_with_history_path(path);
        input.handle_paste("old prompt".to_string());
        input.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        input.handle_paste("my wip draft".to_string());
        input.input(ctrl('r'));
        input.input(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(input.current_text(), "old prompt", "搜索命中应预览");
        // Esc 必须还原进搜索前的草稿,不能把用户正在写的东西弄丢。
        input.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(input.history_search_query(), None);
        assert_eq!(input.current_text(), "my wip draft");
    }
}
