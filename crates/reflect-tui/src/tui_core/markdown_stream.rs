//! 在换行边界处收集 markdown 流式源码。
//!
//! `MarkdownStreamCollector` 缓冲进入的 token 增量，并在每个换行处暴露一个提交边界。
//! 流控制器（`streaming/controller.rs`）在每个含换行的增量之后调用
//! `commit_complete_source()`，取得可重新渲染的已完成前缀，并把末尾不完整的那一行
//! 留在缓冲区中等待下一个增量。
//!
//! 在收尾时，`finalize_and_take_source()` 会把完整的源码缓冲区转交给控制器
//! （包括可能缺少行尾换行符的最后一行）。

#[cfg(test)]
use ratatui::text::Line;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use crate::tui_core::markdown;

/// 以换行为门控的累加器，缓冲原始 markdown 源码，且只提交已完成的行。
///
/// 缓冲区通过 `committed_source_len` 记录已提交的源码字节数，因此每次调用
/// `commit_complete_source()` 都会返回新完成部分的区间。这种设计让流控制器可以
/// 重新渲染全部累积的源码，同时只追加新增内容。
///
/// 该收集器在生产环境中不解析 markdown。它只负责界定稳定的源码边界；渲染逻辑位于
/// 流控制器中，这样宽度变化时可以基于同一份累积的源码字符串重新渲染。
pub(crate) struct MarkdownStreamCollector {
    buffer: String,
    committed_source_len: usize,
    #[cfg(test)]
    committed_line_count: usize,
    width: Option<usize>,
    #[cfg(test)]
    cwd: PathBuf,
}

impl MarkdownStreamCollector {
    /// 创建一个累积原始 markdown 增量的收集器。
    ///
    /// `width` 与 `cwd` 只被仅用于测试的渲染辅助函数使用；生产环境的流提交
    /// 只基于原始源码边界运作。收集器会对 `cwd` 做快照，使测试渲染在多次增量提交
    /// 之间保持本地文件链接展示的一致性。
    pub fn new(width: Option<usize>, cwd: &Path) -> Self {
        #[cfg(not(test))]
        let _ = cwd;

        Self {
            buffer: String::new(),
            committed_source_len: 0,
            #[cfg(test)]
            committed_line_count: 0,
            width,
            #[cfg(test)]
            cwd: cwd.to_path_buf(),
        }
    }

    /// 更新仅用于测试的行提交辅助函数所使用的渲染宽度。
    pub fn set_width(&mut self, width: Option<usize>) {
        self.width = width;
    }

    /// 重置全部缓冲的源码和提交记账信息。
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.committed_source_len = 0;
        #[cfg(test)]
        {
            self.committed_line_count = 0;
        }
    }

    /// 将一个原始的流式增量追加到内部源码缓冲区。
    pub fn push_delta(&mut self, delta: &str) {
        tracing::trace!("push_delta: {delta:?}");
        self.buffer.push_str(delta);
    }

    /// 提交截至最后一个换行符为止新完成的原始 markdown 源码。
    ///
    /// 返回此前提交尚未返回过的源码区间。若在不含换行的增量之后调用，则返回 `None`，
    /// 这可以避免实时流渲染那些在该行余下部分到达后语义可能发生改变的不完整
    /// markdown 块。
    pub fn commit_complete_source(&mut self) -> Option<std::ops::Range<usize>> {
        let commit_end = self.buffer.rfind('\n').map(|idx| idx + 1)?;
        let commit_start = self.committed_source_len;
        if commit_end <= commit_start {
            return None;
        }

        self.committed_source_len = commit_end;
        Some(commit_start..commit_end)
    }

    /// 返回以换行结尾、可以安全渲染的源码。
    pub fn committed_source(&self) -> &str {
        &self.buffer[..self.committed_source_len]
    }

    /// 收尾该流并转交其完整的原始源码。
    ///
    /// 当返回的源码块非空时确保它以换行结尾，这样调用方就能安全地对最终块运行
    /// markdown 块解析。该方法会清空收集器；调用方不应在流真正完成，或需要有意
    /// 合并被中断的输出之前调用它。
    pub fn finalize_and_take_source(&mut self) -> String {
        let mut out = std::mem::take(&mut self.buffer);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        self.clear();
        out
    }

    /// 渲染整个缓冲区，并只返回自上次提交以来新完成的逻辑行。
    /// 当缓冲区不以换行结尾时，最后渲染出的那一行被视为不完整，不会输出。
    ///
    /// 此辅助函数有意使用 `append_markdown`（而非 `append_markdown_agent`），
    /// 使测试可以在不涉及流控制器保留（holdback）语义的情况下，单独验证收集器的
    /// 换行边界行为。
    #[cfg(test)]
    pub fn commit_complete_lines(&mut self) -> Vec<Line<'static>> {
        let Some(commit_end) = self.buffer.rfind('\n').map(|idx| idx + 1) else {
            return Vec::new();
        };
        if commit_end <= self.committed_source_len {
            return Vec::new();
        }
        let source = self.buffer[..commit_end].to_string();
        let mut rendered: Vec<Line<'static>> = Vec::new();
        markdown::append_markdown(&source, self.width, Some(self.cwd.as_path()), &mut rendered);
        let mut complete_line_count = rendered.len();
        if complete_line_count > 0
            && crate::tui_core::render::line_utils::is_blank_line_spaces_only(
                &rendered[complete_line_count - 1],
            )
        {
            complete_line_count -= 1;
        }

        if self.committed_line_count >= complete_line_count {
            return Vec::new();
        }

        let out_slice = &rendered[self.committed_line_count..complete_line_count];

        let out = out_slice.to_vec();
        self.committed_source_len = commit_end;
        self.committed_line_count = complete_line_count;
        out
    }

    /// 收尾该流：输出最后一次提交之后剩余的全部行。
    /// 如果缓冲区不以换行结尾，会临时追加一个换行以供渲染。
    #[cfg(test)]
    pub fn finalize_and_drain(&mut self) -> Vec<Line<'static>> {
        let mut source = self.buffer.clone();
        if source.is_empty() {
            self.clear();
            return Vec::new();
        }
        if !source.ends_with('\n') {
            source.push('\n');
        };
        tracing::debug!(
            raw_len = self.buffer.len(),
            source_len = source.len(),
            "markdown finalize (raw length: {}, rendered length: {})",
            self.buffer.len(),
            source.len()
        );
        tracing::trace!("markdown finalize (raw source):\n---\n{source}\n---");

        let mut rendered: Vec<Line<'static>> = Vec::new();
        markdown::append_markdown(&source, self.width, Some(self.cwd.as_path()), &mut rendered);

        let out = if self.committed_line_count >= rendered.len() {
            Vec::new()
        } else {
            rendered[self.committed_line_count..].to_vec()
        };

        // 为下一次流重置收集器状态。
        self.clear();
        out
    }
}

#[cfg(test)]
fn test_cwd() -> PathBuf {
    // 这些测试只需要一个稳定的绝对工作目录；使用 temp_dir() 可以避免把 Unix 或
    // Windows 特有的根目录语义固化到测试固件中。
    std::env::temp_dir()
}

#[cfg(test)]
pub(crate) fn simulate_stream_markdown_for_tests(
    deltas: &[&str],
    finalize: bool,
) -> Vec<Line<'static>> {
    let mut collector = MarkdownStreamCollector::new(/*width*/ None, &test_cwd());
    let mut out = Vec::new();
    for d in deltas {
        collector.push_delta(d);
        if d.contains('\n') {
            out.extend(collector.commit_complete_lines());
        }
    }
    if finalize {
        out.extend(collector.finalize_and_drain());
    }
    out
}

#[cfg(test)]
#[cfg(test)]
mod tests;
