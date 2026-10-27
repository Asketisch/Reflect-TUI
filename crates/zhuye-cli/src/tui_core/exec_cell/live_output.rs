use itertools::Either;
use std::borrow::Cow;
use std::collections::VecDeque;

const LIVE_COMMAND_OUTPUT_MAX_BYTES: usize = 1024 * 1024;
const LIVE_COMMAND_OUTPUT_MAX_LINES: usize = 50;
const LIVE_COMMAND_OUTPUT_MAX_LINE_BYTES: usize =
    LIVE_COMMAND_OUTPUT_MAX_BYTES / (2 * LIVE_COMMAND_OUTPUT_MAX_LINES + 2);
const LIVE_COMMAND_OUTPUT_LINE_HEAD_BYTES: usize = LIVE_COMMAND_OUTPUT_MAX_LINE_BYTES / 2;
const LIVE_COMMAND_OUTPUT_LINE_TAIL_BYTES: usize =
    LIVE_COMMAND_OUTPUT_MAX_LINE_BYTES - LIVE_COMMAND_OUTPUT_LINE_HEAD_BYTES;

/// 流式命令输出的有界增量预览。
///
/// 所有输出都会被保留，直到超出字节预算。一旦发生截断，会保留首尾各 50 行已完成的行以及
/// 当前正在进行中的行，每一行独立地保留前缀和后缀，这样永不输出换行符的命令也不会让
/// 当前活跃单元无限增长。
#[derive(Debug, Default)]
pub(super) struct LiveCommandOutput {
    full_output: String,
    truncated: bool,
    head: Vec<String>,
    tail: VecDeque<String>,
    current: LiveCommandOutputLine,
    completed_lines: usize,
    has_partial_line: bool,
    pending_carriage_return: bool,
}

impl LiveCommandOutput {
    /// 追加一段增量内容，跨任意分块边界时保持与 `str::lines` 一致的语义。
    pub(super) fn push_str(&mut self, chunk: &str) {
        if !self.truncated {
            if self.full_output.len().saturating_add(chunk.len()) <= LIVE_COMMAND_OUTPUT_MAX_BYTES {
                self.full_output.push_str(chunk);
                self.completed_lines = self
                    .completed_lines
                    .saturating_add(chunk.bytes().filter(|byte| *byte == b'\n').count());
                if !chunk.is_empty() {
                    self.has_partial_line = !chunk.ends_with('\n');
                }
                return;
            }

            self.truncated = true;
            let full_output = std::mem::take(&mut self.full_output);
            self.completed_lines = 0;
            self.has_partial_line = false;
            self.push_truncated_str(&full_output);
        }

        self.push_truncated_str(chunk);
    }

    /// 追加有界输出，将跨分块的 CRLF 视作单个终止符，同时保留孤立的 CR。
    fn push_truncated_str(&mut self, chunk: &str) {
        for part in chunk.split_inclusive('\n') {
            let Some(part) = part.strip_suffix('\n') else {
                if part.is_empty() {
                    continue;
                }
                if self.pending_carriage_return {
                    self.current.push_str("\r");
                    self.pending_carriage_return = false;
                }
                let part = if let Some(part) = part.strip_suffix('\r') {
                    self.pending_carriage_return = true;
                    part
                } else {
                    part
                };
                self.current.push_str(part);
                self.has_partial_line |= !part.is_empty() || self.pending_carriage_return;
                continue;
            };

            let has_carriage_return = part.ends_with('\r');
            let part = part.strip_suffix('\r').unwrap_or(part);
            if self.pending_carriage_return && (has_carriage_return || !part.is_empty()) {
                self.current.push_str("\r");
            }
            self.pending_carriage_return = false;
            self.current.push_str(part);
            self.completed_lines = self.completed_lines.saturating_add(1);
            self.has_partial_line = false;

            let line = std::mem::take(&mut self.current).render();
            if self.head.len() < LIVE_COMMAND_OUTPUT_MAX_LINES {
                self.head.push(line);
            } else {
                if self.tail.len() == LIVE_COMMAND_OUTPUT_MAX_LINES {
                    self.tail.pop_front();
                }
                self.tail.push_back(line);
            }
        }
    }

    pub(super) fn total_lines(&self) -> usize {
        self.completed_lines
            .saturating_add(usize::from(self.has_partial_line))
    }

    pub(super) fn retained_lines(&self) -> usize {
        if self.truncated {
            self.head
                .len()
                .saturating_add(self.tail.len())
                .saturating_add(usize::from(self.has_partial_line))
        } else {
            self.total_lines()
        }
    }

    /// 返回支持反向遍历的预览行，即未达到字节预算时也会对过长行进行缩写。
    pub(super) fn lines(&self) -> impl DoubleEndedIterator<Item = Cow<'_, str>> {
        if self.truncated {
            Either::Left(
                self.head
                    .iter()
                    .chain(self.tail.iter())
                    .map(|line| Cow::Borrowed(line.as_str()))
                    .chain(
                        self.has_partial_line
                            .then(|| Cow::Owned(self.render_partial_line())),
                    ),
            )
        } else {
            Either::Right(self.full_output.lines().map(|line| {
                if line.len() <= LIVE_COMMAND_OUTPUT_MAX_LINE_BYTES {
                    Cow::Borrowed(line)
                } else {
                    let mut truncated = LiveCommandOutputLine::default();
                    truncated.push_str(line);
                    Cow::Owned(truncated.render())
                }
            }))
        }
    }

    /// 返回无损的转录行，截断后插入“被省略的行”标记。
    pub(super) fn transcript_lines(&self) -> impl Iterator<Item = Cow<'_, str>> {
        let omitted = self.total_lines().saturating_sub(self.retained_lines());
        if self.truncated {
            Either::Left(
                self.head
                    .iter()
                    .map(|line| Cow::Borrowed(line.as_str()))
                    .chain((omitted > 0).then(|| Cow::Owned(format!("… +{omitted} lines"))))
                    .chain(self.tail.iter().map(|line| Cow::Borrowed(line.as_str())))
                    .chain(
                        self.has_partial_line
                            .then(|| Cow::Owned(self.render_partial_line())),
                    ),
            )
        } else {
            Either::Right(self.full_output.lines().map(Cow::Borrowed))
        }
    }

    fn render_partial_line(&self) -> String {
        let mut line = self.current.render();
        if self.pending_carriage_return {
            line.push('\r');
        }
        line
    }
}

#[derive(Debug, Default)]
struct LiveCommandOutputLine {
    head: String,
    tail: String,
    omitted_bytes: usize,
}

impl LiveCommandOutputLine {
    fn push_str(&mut self, chunk: &str) {
        let head_remaining = if self.tail.is_empty() && self.omitted_bytes == 0 {
            LIVE_COMMAND_OUTPUT_LINE_HEAD_BYTES.saturating_sub(self.head.len())
        } else {
            0
        };
        let mut head_end = head_remaining.min(chunk.len());
        while !chunk.is_char_boundary(head_end) {
            head_end -= 1;
        }
        self.head.push_str(&chunk[..head_end]);
        let chunk = &chunk[head_end..];
        if chunk.is_empty() {
            return;
        }

        if chunk.len() >= LIVE_COMMAND_OUTPUT_LINE_TAIL_BYTES {
            let mut tail_start = chunk.len() - LIVE_COMMAND_OUTPUT_LINE_TAIL_BYTES;
            while !chunk.is_char_boundary(tail_start) {
                tail_start += 1;
            }
            self.omitted_bytes = self
                .omitted_bytes
                .saturating_add(self.tail.len())
                .saturating_add(tail_start);
            self.tail.clear();
            self.tail.push_str(&chunk[tail_start..]);
            return;
        }

        self.tail.push_str(chunk);
        let mut tail_start = self
            .tail
            .len()
            .saturating_sub(LIVE_COMMAND_OUTPUT_LINE_TAIL_BYTES);
        while !self.tail.is_char_boundary(tail_start) {
            tail_start += 1;
        }
        if tail_start > 0 {
            self.tail.drain(..tail_start);
            self.omitted_bytes = self.omitted_bytes.saturating_add(tail_start);
        }
    }

    /// 渲染被保留的行，在省略标记之前闭合被截断的 ANSI 转义序列。
    fn render(&self) -> String {
        let omission_marker = (self.omitted_bytes > 0)
            .then(|| format!("... {} bytes omitted ...", self.omitted_bytes));
        let mut line = String::with_capacity(
            self.head
                .len()
                .saturating_add(self.tail.len())
                .saturating_add(omission_marker.as_ref().map_or(0, String::len))
                .saturating_add(1),
        );
        line.push_str(&self.head);
        if let Some(omission_marker) = omission_marker {
            let terminator = self.head.rfind('\x1b').and_then(|start| {
                let escape = &self.head.as_bytes()[start + 1..];
                match escape.first() {
                    Some(b']') if !escape.contains(&b'\x07') => Some('\x07'),
                    Some(b'[') if !escape[1..].iter().any(u8::is_ascii_alphabetic) => Some('m'),
                    _ => None,
                }
            });
            if let Some(terminator) = terminator {
                line.push(terminator);
            }
            line.push_str("\x1b[0m");
            line.push_str(&omission_marker);
        }
        line.push_str(&self.tail);
        line
    }
}

#[cfg(test)]
#[path = "live_output_tests.rs"]
mod tests;
