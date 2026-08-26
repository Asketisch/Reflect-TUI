//! 无 bracketed paste 终端的粘贴突发检测。
//!
//! 在某些平台（尤其是 Windows）上，粘贴常以 `KeyCode::Char` 和 `KeyCode::Enter` 按键事件的
//! 快速流到达，而不是作为单个 "粘贴" 事件。该模式下，编辑器需要：
//!
//! - 防止短暂的 UI 副作用（如绑定到 `?` 的切换）在粘贴文本上触发。
//! - 确保 Enter 被视为粘贴内*的换行*，而不是 "提交消息"。
//! - 避免由插入已输入前缀然后立即将其重新分类为粘贴引起的闪烁。
//!
//! 本模块提供 `PasteBurst` 状态机。`ChatComposer` 仅向其馈送 "普通" 字符事件（无 Ctrl/Alt）
//! 并使用完整缓冲决策来：
//!
//! - 短暂保留第一个 ASCII 字符（闪烁抑制），
//! - 将突发缓冲为单个粘贴字符串，或
//! - 让输入作为正常键入通过。
//!
//! # 调用模式
//!
//! `PasteBurst` 是纯状态机：它不直接修改 textarea。调用者馈送事件然后应用所选操作：
//!
//! - 对每个普通 `KeyCode::Char`，调用 [`PasteBurst::on_plain_char`]（ASCII）或
//!   [`PasteBurst::on_plain_char_no_hold`]（非 ASCII/IME）。
//! - 如果决策指示缓冲，调用者通过 [`PasteBurst::append_char_to_buffer`] 追加到 `PasteBurst.buffer`。
//! - 在 UI 刻度上，调用 [`PasteBurst::flush_if_due`]。如果返回 [`FlushResult::Typed`]，将该字符作为
//!   正常键入插入。如果返回 [`FlushResult::Paste`]，将返回的字符串视为显式粘贴。
//! - 在应用非字符输入（箭头键、Ctrl/Alt 修饰符等）之前，使用 [`PasteBurst::flush_before_modified_input`]
//!   避免缓冲文本 "卡住"，然后 [`PasteBurst::clear_window_after_non_char`] 使后续键入不分组到前一次突发中。
//! - 直接插入调用者可跳过缓冲，在 Enter 处理程序中使用 [`PasteBurst::direct_insert_newline_should_insert`]，
//!   并在 Enter 或 [`PasteBurst::on_plain_char_no_hold`] 报告突发流时调用 [`PasteBurst::extend_window`]。
//!
//! # 状态变量
//!
//! 此状态机编码在几个字段中，含义略有不同：
//!
//! - `active`：在仍*主动*接受字符到当前突发时为 true。
//! - `buffer`：累积的突发文本，最终将作为单个 `Paste(String)` 刷新。
//!   即使 `active` 已清除，非空缓冲区也视为 "在突发上下文中"。
//! - `pending_first_char`：单个保留的 ASCII 字符，用于闪烁抑制。调用者不应渲染此字符，
//!   直到它成为突发的一部分（`BeginBufferFromPending`）或作为正常键入字符刷新（`FlushResult::Typed`）。
//! - `last_plain_char_time`/`consecutive_plain_char_burst`："粘贴状"流的定时/计数启发式。
//! - `burst_window_until`：超出缓冲区本身的 Enter 抑制窗口（"Enter 插入换行"）。
//!
//! # 定时模型
//!
//! 有两个超时：
//!
//! - `PASTE_BURST_CHAR_INTERVAL`：连续 "普通" 字符被视为单次突发一部分的最大延迟。
//!   它还限制 `pending_first_char` 的保留时间。
//! - `PASTE_BURST_ACTIVE_IDLE_TIMEOUT`：缓冲激活后，最后一个字符后等待多久将累积缓冲区刷新为粘贴。
//!
//! `flush_if_due()` 有意使用 `>`（而非 `>=`）比较经过的时间，因此测试和 UI 刻度应至少越过阈值 1ms
//!（见 `recommended_flush_delay()`）。
//!
//! # 回溯捕获细节
//!
//! 回溯捕获用于处理初始将字符插入为 "正常键入" 但后来决定流为粘贴状的情况。
//! 当这种情况发生时，我们回溯性地从 textarea 中移除已插入文本的前缀并将其移到突发缓冲区中，
//! 以便最终的 `handle_paste(...)` 看到连续的粘贴字符串。
//!
//! 回溯捕获主要在不保留第一个字符的路径上重要（非 ASCII/IME 输入和回溯捕获场景）。
//! ASCII 路径通常更喜欢 `RetainFirstChar -> BeginBufferFromPending`，这避免了需要回溯捕获。
//!
//! 回溯捕获以字符（而非字节）表示：
//!
//! - `CharDecision::BeginBuffer { retro_chars }` 将 `retro_chars` 用作字符计数。
//! - `decide_begin_buffer(now, before_cursor, retro_chars)` 通过调用 `retro_start_index()`
//!   将其转换为 UTF-8 字节范围。
//! - `RetroGrab.start_byte` 是 `before_cursor` 切片中的字节索引；调用者必须在切片之前将光标夹紧到字符边界，
//!   使 `start_byte..cursor` 始终是有效的 UTF-8。
//!
//! # 清除与刷新
//!
//! 调用者有两种结束突发处理方式，它们不可互换：
//!
//! - `flush_before_modified_input()` 返回缓冲的文本（和/或挂起的第一个 ASCII 字符），
//!   以便调用者可通过正常粘贴路径应用它，然后再处理不相关的输入。
//! - `clear_window_after_non_char()` 清除*分类窗口*，使后续键入不分组到前一次突发中。
//!   它假定调用者已刷新任何缓冲区，因为它清除 `last_plain_char_time`，
//!   这意味着 `flush_if_due()` 不会在另一个普通字符更新时间戳之前刷新非空缓冲区。
//!
//! # 状态（概念上）
//!
//! - **空闲**：无缓冲文本，无挂起字符。
//! - **挂起第一个字符**：`pending_first_char` 保留一个 ASCII 字符最多 `PASTE_BURST_CHAR_INTERVAL`，
//!   等待查看是否有突发跟随。
//! - **活动缓冲区**：`active`/`buffer` 保存粘贴状内容直到超时并刷新。
//! - **Enter 抑制窗口**：`burst_window_until` 在突发活动后短暂保持 Enter 被视为换行，使多行粘贴保持分组。
//!
//! # ASCII 与非 ASCII
//!
//! - [`PasteBurst::on_plain_char`] 可能返回 [`CharDecision::RetainFirstChar`] 以保留第一个
//!   ASCII 字符并避免闪烁。
//! - [`PasteBurst::on_plain_char_no_hold`] 从不保留（用于 IME/非 ASCII 路径），因为
//!   保留非 ASCII 字符会感觉像输入丢失。
//!
//! # 与调用者的约定
//!
//! `PasteBurst` 不自行修改 UI 文本缓冲区。调用者必须解释决策并应用相应的 UI 编辑。
//! `ChatComposer` 使用完整的缓冲约定：
//!
//! - 对每个普通 ASCII `KeyCode::Char`，调用 [`PasteBurst::on_plain_char`]。
//!   - [`CharDecision::RetainFirstChar`]：**不要**将字符插入 textarea。
//!   - [`CharDecision::BeginBufferFromPending`]：对当前字符调用 [`PasteBurst::append_char_to_buffer`]
//!     （之前保留的字符已在突发缓冲区中）。
//!   - [`CharDecision::BeginBuffer { retro_chars }`]：考虑通过调用 [`PasteBurst::decide_begin_buffer`]
//!     回溯捕获已插入的前缀。如果返回 `Some`，从 textarea 移除返回的 `start_byte..cursor` 范围，
//!     然后对当前字符调用 [`PasteBurst::append_char_to_buffer`]。如果返回 `None`，回退到正常插入。
//!   - [`CharDecision::BufferAppend`]：调用 [`PasteBurst::append_char_to_buffer`]。
//!
//! - 对每个普通非 ASCII `KeyCode::Char`，调用 [`PasteBurst::on_plain_char_no_hold`]，然后：
//!   - 如果返回 `Some(CharDecision::BufferAppend)`，调用 [`PasteBurst::append_char_to_buffer`]。
//!   - 如果返回 `Some(CharDecision::BeginBuffer { retro_chars })`，如上调用 [`PasteBurst::decide_begin_buffer`]
//!     （如果缓冲开始，从 textarea 移除捕获的前缀，然后将当前字符追加到缓冲区）。
//!   - 如果返回 `None`，正常插入。
//!
//! - 在应用非字符输入（或不应加入突发的任何输入）之前，调用 [`PasteBurst::flush_before_modified_input`]
//!   并将返回的字符串（如果有）通过正常粘贴路径传递。
//!
//! - 定期（例如在 UI 刻度上）调用 [`PasteBurst::flush_if_due`]。
//!   - [`FlushResult::Typed`]：将单个字符作为正常键入插入。
//!   - [`FlushResult::Paste`]：将返回的字符串视为显式粘贴。
//!
//! - 当按下非普通键（Ctrl/Alt 修饰输入、箭头等）时，调用者应使用 [`PasteBurst::clear_window_after_non_char`]
//!   防止下一个击键被错误地分组到前一次突发中。

use std::time::Duration;
use std::time::Instant;

// 用于检测粘贴状输入突发的启发式阈值。
// 快速检测，以避免在识别出粘贴之前显示已键入的前缀。
const PASTE_BURST_MIN_CHARS: u16 = 3;
const PASTE_ENTER_SUPPRESS_WINDOW: Duration = Duration::from_millis(120);

// 连续字符被视为同一次粘贴突发一部分的最大延迟。
const PASTE_BURST_CHAR_INTERVAL: Duration = Duration::from_millis(8);

// 刷新缓冲的粘贴内容之前的空闲超时。
// 在 Windows 环境中观察到较慢的粘贴突发。
#[cfg(not(windows))]
const PASTE_BURST_ACTIVE_IDLE_TIMEOUT: Duration = Duration::from_millis(8);
#[cfg(windows)]
const PASTE_BURST_ACTIVE_IDLE_TIMEOUT: Duration = Duration::from_millis(60);

#[derive(Default)]
pub(crate) struct PasteBurst {
    last_plain_char_time: Option<Instant>,
    consecutive_plain_char_burst: u16,
    burst_window_until: Option<Instant>,
    buffer: String,
    active: bool,
    // 短暂保留第一个快速字符以避免渲染闪烁
    pending_first_char: Option<(char, Instant)>,
}

pub(crate) enum CharDecision {
    /// 开始缓冲，并回溯捕获一些已插入的字符。
    BeginBuffer { retro_chars: u16 },
    /// 当前正在缓冲；将当前字符追加到缓冲区中。
    BufferAppend,
    /// 暂不插入/渲染此字符；在等待查看是否会有粘贴状突发跟随
    /// 时，临时保存第一个快速字符。
    RetainFirstChar,
    /// 使用之前保存的第一个字符开始缓冲（无需回溯抓取）。
    BeginBufferFromPending,
}

pub(crate) struct RetroGrab {
    pub start_byte: usize,
    pub grabbed: String,
}

pub(crate) enum FlushResult {
    Paste(String),
    Typed(char),
    None,
}

impl PasteBurst {
    /// 建议在模拟按键之间等待的延迟（或在调度 UI 刻度之前），
    /// 以便挂起的快速击键能从突发检测器中作为正常键入输入被刷新。
    ///
    /// 主要由测试和 TUI 使用，以可靠地越过粘贴突发的定时阈值。
    pub fn recommended_flush_delay() -> Duration {
        PASTE_BURST_CHAR_INTERVAL + Duration::from_millis(1)
    }

    #[cfg(test)]
    pub(crate) fn recommended_active_flush_delay() -> Duration {
        PASTE_BURST_ACTIVE_IDLE_TIMEOUT + Duration::from_millis(1)
    }

    /// 入口点：根据当前计时决定如何处理一个普通字符。
    pub fn on_plain_char(&mut self, ch: char, now: Instant) -> CharDecision {
        self.note_plain_char(now);

        if self.active {
            self.burst_window_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
            return CharDecision::BufferAppend;
        }

        // 如果我们已经保留了第一个字符并收到第二个快速字符，
        // 则开始缓冲而无需回溯抓取（我们从未渲染第一个字符）。
        if let Some((held, held_at)) = self.pending_first_char
            && now.duration_since(held_at) <= PASTE_BURST_CHAR_INTERVAL
        {
            self.active = true;
            // 使用 take() 清除挂起状态；我们已经在上面捕获了保留的字符
            let _ = self.pending_first_char.take();
            self.buffer.push(held);
            self.burst_window_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
            return CharDecision::BeginBufferFromPending;
        }

        if self.consecutive_plain_char_burst >= PASTE_BURST_MIN_CHARS {
            return CharDecision::BeginBuffer {
                retro_chars: self.consecutive_plain_char_burst.saturating_sub(1),
            };
        }

        // 非常短暂地保存第一个快速字符，以查看是否会有突发跟随。
        self.pending_first_char = Some((ch, now));
        CharDecision::RetainFirstChar
    }

    /// 类似于 on_plain_char()，但从不保留第一个字符。
    ///
    /// 用于非 ASCII 输入路径（例如 IME），在这些路径中保留字符会让人感觉输入丢失，
    /// 同时仍允许基于突发的粘贴检测。
    ///
    /// 注意：此方法只会返回 BufferAppend 或 BeginBuffer。
    pub fn on_plain_char_no_hold(&mut self, now: Instant) -> Option<CharDecision> {
        self.note_plain_char(now);

        if self.active {
            self.burst_window_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
            return Some(CharDecision::BufferAppend);
        }

        if self.consecutive_plain_char_burst >= PASTE_BURST_MIN_CHARS {
            return Some(CharDecision::BeginBuffer {
                retro_chars: self.consecutive_plain_char_burst.saturating_sub(1),
            });
        }

        None
    }

    fn note_plain_char(&mut self, now: Instant) {
        match self.last_plain_char_time {
            Some(prev) if now.duration_since(prev) <= PASTE_BURST_CHAR_INTERVAL => {
                self.consecutive_plain_char_burst =
                    self.consecutive_plain_char_burst.saturating_add(1)
            }
            _ => self.consecutive_plain_char_burst = 1,
        }
        self.last_plain_char_time = Some(now);
    }

    /// 如果按键间超时已过，则刷新任何缓冲的突发。
    ///
    /// 返回值：
    ///
    /// - 当粘贴突发处于活动状态且缓冲的文本作为单个粘贴字符串发出时，返回 [`FlushResult::Paste`]。
    /// - 当单个快速的首个 ASCII 字符被保留（闪烁抑制）且超时之前没有突发跟随，
    ///   返回 [`FlushResult::Typed`]。
    /// - 当超时尚未过去，或没有可刷新的内容时，返回 [`FlushResult::None`]。
    pub fn flush_if_due(&mut self, now: Instant) -> FlushResult {
        let timeout = if self.is_active_internal() {
            PASTE_BURST_ACTIVE_IDLE_TIMEOUT
        } else {
            PASTE_BURST_CHAR_INTERVAL
        };
        let timed_out = self
            .last_plain_char_time
            .is_some_and(|t| now.duration_since(t) > timeout);
        if timed_out && self.is_active_internal() {
            self.active = false;
            let out = std::mem::take(&mut self.buffer);
            FlushResult::Paste(out)
        } else if timed_out {
            // 如果我们正在保存单个快速字符且没有突发跟随，
            // 则将其作为正常键入输入刷新。
            if let Some((ch, _at)) = self.pending_first_char.take() {
                FlushResult::Typed(ch)
            } else {
                FlushResult::None
            }
        } else {
            FlushResult::None
        }
    }

    /// 突发过程中：将换行符累积到缓冲区中，而不是提交 textarea。
    ///
    /// 如果追加了换行符（我们处于突发上下文中）则返回 true，
    /// 否则返回 false。
    pub fn append_newline_if_active(&mut self, now: Instant) -> bool {
        if self.is_active() {
            self.buffer.push('\n');
            self.burst_window_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
            true
        } else {
            false
        }
    }

    /// 决定 Enter 应插入换行符（突发上下文）还是提交。
    pub fn newline_should_insert_instead_of_submit(&self, now: Instant) -> bool {
        let in_burst_window = self.burst_window_until.is_some_and(|until| now <= until);
        self.is_active() || in_burst_window
    }

    /// 为立即插入字符的调用者决定 Enter 是否应插入换行符。
    pub fn direct_insert_newline_should_insert(&self, now: Instant) -> bool {
        self.newline_should_insert_instead_of_submit(now)
            || self
                .last_plain_char_time
                .is_some_and(|t| now.duration_since(t) <= PASTE_BURST_CHAR_INTERVAL)
    }

    /// 保持突发窗口有效。
    pub fn extend_window(&mut self, now: Instant) {
        self.burst_window_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
    }

    /// 使用回溯抓取的文本开始缓冲。
    pub fn begin_with_retro_grabbed(&mut self, grabbed: String, now: Instant) {
        if !grabbed.is_empty() {
            self.buffer.push_str(&grabbed);
        }
        self.active = true;
        self.burst_window_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
    }

    /// 将一个字符追加到突发缓冲区中。
    pub fn append_char_to_buffer(&mut self, ch: char, now: Instant) {
        self.buffer.push(ch);
        self.burst_window_until = Some(now + PASTE_ENTER_SUPPRESS_WINDOW);
    }

    /// 仅当突发已处于活动状态时，才尝试将字符追加到突发缓冲区。
    ///
    /// 当字符被捕获到现有突发中时返回 true，否则返回 false。
    pub fn try_append_char_if_active(&mut self, ch: char, now: Instant) -> bool {
        if self.active || !self.buffer.is_empty() {
            self.append_char_to_buffer(ch, now);
            true
        } else {
            false
        }
    }

    /// 决定是否通过从光标前的切片回溯捕获最近的字符来开始缓冲。
    ///
    /// 启发式规则：如果回溯抓取的切片包含任何空白或足够长（>= 16 个字符），
    /// 则将其视为粘贴状，以避免在识别出粘贴之前短暂渲染已键入的前缀。
    /// 这有利于响应性，并防止典型粘贴（URL、文件路径、多行文本）出现闪烁，
    /// 同时不会在短单词上触发。
    ///
    /// 当我们决定回溯缓冲时，返回带有起始字节和抓取文本的 Some(RetroGrab)；
    /// 否则返回 None。
    pub fn decide_begin_buffer(
        &mut self,
        now: Instant,
        before: &str,
        retro_chars: usize,
    ) -> Option<RetroGrab> {
        let start_byte = retro_start_index(before, retro_chars);
        let grabbed = before[start_byte..].to_string();
        let looks_pastey =
            grabbed.chars().any(char::is_whitespace) || grabbed.chars().count() >= 16;
        if looks_pastey {
            // 注意：调用者负责从 UI 文本中移除该切片。
            self.begin_with_retro_grabbed(grabbed.clone(), now);
            Some(RetroGrab {
                start_byte,
                grabbed,
            })
        } else {
            None
        }
    }

    /// 在应用修饰/非字符输入之前：立即刷新缓冲的突发。
    pub fn flush_before_modified_input(&mut self) -> Option<String> {
        if !self.is_active() {
            return None;
        }
        self.active = false;
        let mut out = std::mem::take(&mut self.buffer);
        if let Some((ch, _at)) = self.pending_first_char.take() {
            out.push(ch);
        }
        Some(out)
    }

    /// 仅清除定时窗口和任何挂起的首个字符。
    ///
    /// 不会发出或清除缓冲文本本身；调用者应已通过上述某个刷新方法
    /// 刷新（如有必要）。
    pub fn clear_window_after_non_char(&mut self) {
        self.consecutive_plain_char_burst = 0;
        self.last_plain_char_time = None;
        self.burst_window_until = None;
        self.active = false;
        self.pending_first_char = None;
    }

    /// 如果我们处于任何与粘贴突发相关的瞬态状态（正在积极缓冲、
    /// 拥有非空缓冲区，或在等待潜在突发时已保存第一个快速字符），
    /// 则返回 true。
    pub fn is_active(&self) -> bool {
        self.is_active_internal() || self.pending_first_char.is_some()
    }

    fn is_active_internal(&self) -> bool {
        self.active || !self.buffer.is_empty()
    }

    pub fn clear_after_explicit_paste(&mut self) {
        self.last_plain_char_time = None;
        self.consecutive_plain_char_burst = 0;
        self.burst_window_until = None;
        self.active = false;
        self.buffer.clear();
        self.pending_first_char = None;
    }
}

fn retro_start_index(before: &str, retro_chars: usize) -> usize {
    if retro_chars == 0 {
        return before.len();
    }
    before
        .char_indices()
        .rev()
        .nth(retro_chars.saturating_sub(1))
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// 行为：对于 ASCII 输入，我们会短暂"保留"第一个快速字符。如果没有突发跟随，
    /// 该保留的字符最终应作为正常键入输入刷新（而不是作为粘贴）。
    #[test]
    fn ascii_first_char_is_held_then_flushes_as_typed() {
        let mut burst = PasteBurst::default();
        let t0 = Instant::now();
        assert!(matches!(
            burst.on_plain_char('a', t0),
            CharDecision::RetainFirstChar
        ));

        let t1 = t0 + PasteBurst::recommended_flush_delay() + Duration::from_millis(1);
        assert!(matches!(burst.flush_if_due(t1), FlushResult::Typed('a')));
        assert!(!burst.is_active());
    }

    /// 行为：如果两个 ASCII 字符快速到达，应开始缓冲而从不渲染第一个字符，
    /// 然后将整个缓冲的有效负载作为粘贴刷新。
    #[test]
    fn ascii_two_fast_chars_start_buffer_from_pending_and_flush_as_paste() {
        let mut burst = PasteBurst::default();
        let t0 = Instant::now();
        assert!(matches!(
            burst.on_plain_char('a', t0),
            CharDecision::RetainFirstChar
        ));

        let t1 = t0 + Duration::from_millis(1);
        assert!(matches!(
            burst.on_plain_char('b', t1),
            CharDecision::BeginBufferFromPending
        ));
        burst.append_char_to_buffer('b', t1);

        let t2 = t1 + PasteBurst::recommended_active_flush_delay() + Duration::from_millis(1);
        assert!(matches!(
            burst.flush_if_due(t2),
            FlushResult::Paste(ref s) if s == "ab"
        ));
    }

    /// 行为：当即将应用非字符输入时，立即刷新任何瞬态突发状态（包括单个挂起的 ASCII 字符），
    /// 使状态不会跨输入泄漏。
    #[test]
    fn flush_before_modified_input_includes_pending_first_char() {
        let mut burst = PasteBurst::default();
        let t0 = Instant::now();
        assert!(matches!(
            burst.on_plain_char('a', t0),
            CharDecision::RetainFirstChar
        ));

        assert_eq!(burst.flush_before_modified_input(), Some("a".to_string()));
        assert!(!burst.is_active());
    }

    /// 行为：仅当已插入的前缀看起来像粘贴（包含空白或"足够长"）时才启用回溯抓取缓冲，
    /// 这样短的 IME 突发不会被错误分类。
    #[test]
    fn decide_begin_buffer_only_triggers_for_pastey_prefixes() {
        let mut burst = PasteBurst::default();
        let now = Instant::now();

        assert!(
            burst
                .decide_begin_buffer(now, "ab", /* retro_chars: 要回溯捕获的字符数 */ 2)
                .is_none()
        );
        assert!(!burst.is_active());

        let grab = burst
            .decide_begin_buffer(now, "a b", /* retro_chars: 要回溯捕获的字符数 */ 2)
            .expect("whitespace should be considered paste-like");
        assert_eq!(grab.start_byte, 1);
        assert_eq!(grab.grabbed, " b");
        assert!(burst.is_active());
    }

    /// 行为：在粘贴状突发之后，我们会短暂保持"回车抑制窗口"有效，
    /// 使稍晚的 Enter 仍能插入换行符而不是提交。
    #[test]
    fn newline_suppression_window_outlives_buffer_flush() {
        let mut burst = PasteBurst::default();
        let t0 = Instant::now();
        assert!(matches!(
            burst.on_plain_char('a', t0),
            CharDecision::RetainFirstChar
        ));

        let t1 = t0 + Duration::from_millis(1);
        assert!(matches!(
            burst.on_plain_char('b', t1),
            CharDecision::BeginBufferFromPending
        ));
        burst.append_char_to_buffer('b', t1);

        let t2 = t1 + PasteBurst::recommended_active_flush_delay() + Duration::from_millis(1);
        assert!(matches!(burst.flush_if_due(t2), FlushResult::Paste(ref s) if s == "ab"));
        assert!(!burst.is_active());

        assert!(burst.newline_should_insert_instead_of_submit(t2));
        let t3 = t1 + PASTE_ENTER_SUPPRESS_WINDOW + Duration::from_millis(1);
        assert!(!burst.newline_should_insert_instead_of_submit(t3));
    }
}
