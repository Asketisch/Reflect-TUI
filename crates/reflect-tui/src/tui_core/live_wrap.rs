use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

/// 由 RowBuilder 生成的单个可视化行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    /// 如果此行以显式换行符结尾（而非强制换行），则为 true。
    pub explicit_break: bool,
}

impl Row {
    pub fn width(&self) -> usize {
        self.text.width()
    }
}

/// 将输入文本增量地换行为最多 `width` 个单元格的可视行。
///
/// 步骤 1：仅纯文本。ANSI 转义携带和样式化区间后续再添加。
pub struct RowBuilder {
    target_width: usize,
    /// 当前逻辑行的缓冲区（直到遇到 '\n' 为止）。
    current_line: String,
    /// 目前已构建的输出行（包括当前逻辑行与之前各行）。
    rows: Vec<Row>,
}

impl RowBuilder {
    pub fn new(target_width: usize) -> Self {
        Self {
            target_width: target_width.max(1),
            current_line: String::new(),
            rows: Vec::new(),
        }
    }

    pub fn set_width(&mut self, width: usize) {
        self.target_width = width.max(1);
        // 重新换行所有内容（步骤 1 的简单方法）。
        let mut all = String::new();
        for row in self.rows.drain(..) {
            all.push_str(&row.text);
            if row.explicit_break {
                all.push('\n');
            }
        }
        all.push_str(&self.current_line);
        self.current_line.clear();
        self.push_fragment(&all);
    }

    /// 推送一段输入片段。可能包含换行符。
    pub fn push_fragment(&mut self, fragment: &str) {
        if fragment.is_empty() {
            return;
        }
        let mut start = 0usize;
        for (i, ch) in fragment.char_indices() {
            if ch == '\n' {
                // 刷新换行符之前所有待处理的内容。
                if start < i {
                    self.current_line.push_str(&fragment[start..i]);
                }
                self.flush_current_line(/*explicit_break*/ true);
                start = i + ch.len_utf8();
            }
        }
        if start < fragment.len() {
            self.current_line.push_str(&fragment[start..]);
            self.wrap_current_line();
        }
    }

    /// 返回已生成行的快照（非消费式读取）。
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// 适合显示的行，包括当前未完成的行（如有）。
    pub fn display_rows(&self) -> Vec<Row> {
        let mut out = self.rows.clone();
        if !self.current_line.is_empty() {
            out.push(Row {
                text: self.current_line.clone(),
                explicit_break: false,
            });
        }
        out
    }

    /// 消费掉超出 `max_keep` 显示行数限制的最早行（包括
    /// 当前未完成的行，如有）。按顺序返回被消费的行。
    pub fn drain_commit_ready(&mut self, max_keep: usize) -> Vec<Row> {
        let display_count = self.rows.len() + if self.current_line.is_empty() { 0 } else { 1 };
        if display_count <= max_keep {
            return Vec::new();
        }
        let to_commit = display_count - max_keep;
        let commit_count = to_commit.min(self.rows.len());
        let mut drained = Vec::with_capacity(commit_count);
        for _ in 0..commit_count {
            drained.push(self.rows.remove(0));
        }
        drained
    }

    fn flush_current_line(&mut self, explicit_break: bool) {
        // 将当前行剩余内容换行，然后通过 explicit_break 完成处理。
        self.wrap_current_line();
        // 如果当前行正好在宽度边界处结束且非空，则使用
        // 一个空的显式行来表示换行，以维持分块不变性。
        if explicit_break {
            if self.current_line.is_empty() {
                // 之前已在边界处结束；添加一个空的显式行。
                self.rows.push(Row {
                    text: String::new(),
                    explicit_break: true,
                });
            } else {
                // 存在尚未换行的剩余内容；现在就将其以显式标记推送。
                let mut s = String::new();
                std::mem::swap(&mut s, &mut self.current_line);
                self.rows.push(Row {
                    text: s,
                    explicit_break: true,
                });
            }
        }
        // 重置当前行缓冲区以处理下一个逻辑行。
        self.current_line.clear();
    }

    fn wrap_current_line(&mut self) {
        // 当 current_line 超出宽度时，截取前缀。
        loop {
            if self.current_line.is_empty() {
                break;
            }
            let (prefix, suffix, taken) =
                take_prefix_by_width(&self.current_line, self.target_width);
            if taken == 0 {
                // 避免在异常输入上陷入无限循环；取一个标量值后继续。
                if let Some((i, ch)) = self.current_line.char_indices().next() {
                    let len = i + ch.len_utf8();
                    let p = self.current_line[..len].to_string();
                    self.rows.push(Row {
                        text: p,
                        explicit_break: false,
                    });
                    self.current_line = self.current_line[len..].to_string();
                    continue;
                }
                break;
            }
            if suffix.is_empty() {
                // 完全适应；保留在缓冲区中（暂不推送），以便稍后追加更多内容。
                break;
            } else {
                // 将换行后的前缀作为非显式行输出，继续处理剩余部分。
                self.rows.push(Row {
                    text: prefix,
                    explicit_break: false,
                });
                self.current_line = suffix.to_string();
            }
        }
    }
}

/// 截取 `text` 的前缀，使其可见宽度不超过 `max_cols`。
/// 返回 (前缀, 后缀, 前缀宽度)。
pub fn take_prefix_by_width(text: &str, max_cols: usize) -> (String, &str, usize) {
    if max_cols == 0 || text.is_empty() {
        return (String::new(), text, 0);
    }
    let mut cols = 0usize;
    let mut end_idx = 0usize;
    for (i, ch) in text.char_indices() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if cols.saturating_add(ch_width) > max_cols {
            break;
        }
        cols += ch_width;
        end_idx = i + ch.len_utf8();
        if cols == max_cols {
            break;
        }
    }
    let prefix = text[..end_idx].to_string();
    let suffix = &text[end_idx..];
    (prefix, suffix, cols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn rows_do_not_exceed_width_ascii() {
        let mut rb = RowBuilder::new(/*target_width*/ 10);
        rb.push_fragment("hello whirl this is a test");
        let rows = rb.rows().to_vec();
        assert_eq!(
            rows,
            vec![
                Row {
                    text: "hello whir".to_string(),
                    explicit_break: false
                },
                Row {
                    text: "l this is ".to_string(),
                    explicit_break: false
                }
            ]
        );
    }

    #[test]
    fn rows_do_not_exceed_width_emoji_cjk() {
        // 😀 的宽度为 2；你/好 的宽度也为 2。` → `// 😀 的宽度为 2；你/好 的宽度也为 2。
        let mut rb = RowBuilder::new(/*target_width*/ 6);
        rb.push_fragment("😀😀 你好");
        let rows = rb.rows().to_vec();
        // 在宽度为 6 时，期望第一行恰好容纳两个表情和一个空格
        //（2 + 2 + 1 = 5）再加上一个 CJK 字符的列（宽度 2 会超出），
        // 因此只有两个表情和一个空格能容纳，其余内容留在缓冲区中。
        assert_eq!(
            rows,
            vec![Row {
                text: "😀😀 ".to_string(),
                explicit_break: false
            }]
        );
    }

    #[test]
    fn fragmentation_invariance_long_token() {
        let s = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"; // 26 个字符
        let mut rb_all = RowBuilder::new(/*target_width*/ 7);
        rb_all.push_fragment(s);
        let all_rows = rb_all.rows().to_vec();

        let mut rb_chunks = RowBuilder::new(/*target_width*/ 7);
        for i in (0..s.len()).step_by(3) {
            let end = (i + 3).min(s.len());
            rb_chunks.push_fragment(&s[i..end]);
        }
        let chunk_rows = rb_chunks.rows().to_vec();

        assert_eq!(all_rows, chunk_rows);
    }

    #[test]
    fn newline_splits_rows() {
        let mut rb = RowBuilder::new(/*target_width*/ 10);
        rb.push_fragment("hello\nworld");
        let rows = rb.display_rows();
        assert!(rows.iter().any(|r| r.explicit_break));
        assert_eq!(rows[0].text, "hello");
        // 第二行应以 'world' 开头
        assert!(rows.iter().any(|r| r.text.starts_with("world")));
    }

    #[test]
    fn rewrap_on_width_change() {
        let mut rb = RowBuilder::new(/*target_width*/ 10);
        rb.push_fragment("abcdefghijK");
        assert!(!rb.rows().is_empty());
        rb.set_width(/*width*/ 5);
        for r in rb.rows() {
            assert!(r.width() <= 5);
        }
    }
}
