//! Markdown Writer 核心状态机：消费 pulldown-cmark 事件并输出 styled ratatui 行。
//!
//! 注：本文件超过 800 行红线，但 `impl Writer` 为内聚的完整状态机（表格布局/换行/链接/
//! 代码块渲染），与上游逐行对应，按 AGENTS.md「内聚完整状态机例外」保留。

//! Markdown Writer 核心状态机：消费 pulldown-cmark 事件并输出 styled ratatui 行。
//! 从 markdown_render/mod.rs 抽出。

use super::*;

pub(super) struct Writer<'a, 'policy, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    pub(super) input: &'a str,
    pub(super) iter: I,
    pub(super) text: Vec<HyperlinkLine>,
    pub(super) styles: MarkdownStyles,
    pub(super) inline_styles: Vec<Style>,
    pub(super) indent_stack: Vec<IndentContext>,
    pub(super) list_indices: Vec<Option<u64>>,
    pub(super) list_needs_blank_before_next_item: Vec<bool>,
    pub(super) list_item_start_line_counts: Vec<usize>,
    pub(super) link: Option<LinkState>,
    pub(super) needs_newline: bool,
    pub(super) pending_marker_line: bool,
    pub(super) in_paragraph: bool,
    pub(super) in_code_block: bool,
    pub(super) code_block_lang: Option<String>,
    pub(super) code_block_buffer: String,
    pub(super) wrap_width: Option<usize>,
    pub(super) cwd: Option<PathBuf>,
    pub(super) is_hidden_link_destination: &'policy dyn Fn(&str) -> bool,
    pub(super) line_ends_with_local_link_target: bool,
    pub(super) pending_local_link_soft_break: bool,
    pub(super) current_line_content: Option<HyperlinkLine>,
    pub(super) current_initial_indent: Vec<Span<'static>>,
    pub(super) current_subsequent_indent: Vec<Span<'static>>,
    pub(super) current_line_style: Style,
    pub(super) current_line_in_code_block: bool,
    pub(super) table_state: Option<TableState>,
}

impl<'a, 'policy, I> Writer<'a, 'policy, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    pub(super) fn new(
        input: &'a str,
        iter: I,
        wrap_width: Option<usize>,
        cwd: Option<&Path>,
        is_hidden_link_destination: &'policy dyn Fn(&str) -> bool,
    ) -> Self {
        Self {
            input,
            iter,
            text: Vec::new(),
            styles: MarkdownStyles::for_terminal(),
            inline_styles: Vec::new(),
            indent_stack: Vec::new(),
            list_indices: Vec::new(),
            list_needs_blank_before_next_item: Vec::new(),
            list_item_start_line_counts: Vec::new(),
            link: None,
            needs_newline: false,
            pending_marker_line: false,
            in_paragraph: false,
            in_code_block: false,
            code_block_lang: None,
            code_block_buffer: String::new(),
            wrap_width,
            cwd: cwd.map(Path::to_path_buf),
            is_hidden_link_destination,
            line_ends_with_local_link_target: false,
            pending_local_link_soft_break: false,
            current_line_content: None,
            current_initial_indent: Vec::new(),
            current_subsequent_indent: Vec::new(),
            current_line_style: Style::default(),
            current_line_in_code_block: false,
            table_state: None,
        }
    }

    pub(super) fn run(&mut self) {
        while let Some((ev, range)) = self.iter.next() {
            self.handle_event(ev, range);
        }
        self.flush_current_line();
    }

    fn handle_event(&mut self, event: Event<'a>, range: Range<usize>) {
        self.prepare_for_event(&event);
        match event {
            Event::Start(tag) => self.start_tag(tag, range),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.text(text),
            Event::Code(code) => self.code(code),
            Event::SoftBreak => self.soft_break(),
            Event::HardBreak => self.hard_break(),
            Event::Rule => {
                self.flush_current_line();
                if !self.text.is_empty() {
                    self.push_blank_line();
                }
                self.push_line(Line::from(Span::styled("———", self.styles.rule)));
                self.needs_newline = true;
            }
            Event::Html(html) => self.html(html, /*inline*/ false),
            Event::InlineHtml(html) => self.html(html, /*inline*/ true),
            Event::FootnoteReference(_) => {}
            Event::TaskListMarker(_) => {}
        }
    }

    fn prepare_for_event(&mut self, event: &Event<'a>) {
        if !self.pending_local_link_soft_break {
            return;
        }

        // 本地文件链接从 `TagEnd::Link` 的目标地址渲染，因此紧接在描述性 `: ...` 之前的
        // Markdown 软换行应保持内联，而不是将列表项拆成两行。
        if matches!(event, Event::Text(text) if text.trim_start().starts_with(':')) {
            self.pending_local_link_soft_break = false;
            return;
        }

        self.pending_local_link_soft_break = false;
        self.push_line(Line::default());
    }

    fn start_tag(&mut self, tag: Tag<'a>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { level, .. } => self.start_heading(level),
            Tag::BlockQuote => self.start_blockquote(),
            Tag::CodeBlock(kind) => {
                let indent = match kind {
                    CodeBlockKind::Fenced(_) => None,
                    CodeBlockKind::Indented => Some(Span::from(" ".repeat(4))),
                };
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => Some(lang.to_string()),
                    CodeBlockKind::Indented => None,
                };
                self.start_codeblock(lang, indent)
            }
            Tag::List(start) => self.start_list(start),
            Tag::Item => self.start_item(),
            Tag::Emphasis => self.push_inline_style(self.styles.emphasis),
            Tag::Strong => self.push_inline_style(self.styles.strong),
            Tag::Strikethrough => self.push_inline_style(self.styles.strikethrough),
            Tag::Link { dest_url, .. } => self.push_link(dest_url.to_string()),
            Tag::Table(alignments) => self.start_table(alignments),
            Tag::TableHead => self.start_table_head(),
            Tag::TableRow => self.start_table_row(range),
            Tag::TableCell => self.start_table_cell(),
            Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::Image { .. }
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::Heading(_) => self.end_heading(),
            TagEnd::BlockQuote => self.end_blockquote(),
            TagEnd::CodeBlock => self.end_codeblock(),
            TagEnd::List(_) => self.end_list(),
            TagEnd::Item => {
                self.flush_current_line();
                let start_line_count = self.list_item_start_line_counts.pop().unwrap_or_default();
                if self.text.len().saturating_sub(start_line_count) > 1
                    && let Some(needs_blank) = self.list_needs_blank_before_next_item.last_mut()
                {
                    *needs_blank = true;
                }
                self.indent_stack.pop();
                self.pending_marker_line = false;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_inline_style(),
            TagEnd::Link => self.pop_link(),
            TagEnd::Table => self.end_table(),
            TagEnd::TableHead => self.end_table_head(),
            TagEnd::TableRow => self.end_table_row(),
            TagEnd::TableCell => self.end_table_cell(),
            TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::Image
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    fn start_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline {
            self.push_blank_line();
        }
        self.push_line(Line::default());
        self.needs_newline = false;
        self.in_paragraph = true;
    }

    fn end_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.needs_newline = true;
        self.in_paragraph = false;
        self.pending_marker_line = false;
    }

    fn start_heading(&mut self, level: HeadingLevel) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline {
            self.push_line(Line::default());
            self.needs_newline = false;
        }
        let heading_style = match level {
            HeadingLevel::H1 => self.styles.h1,
            HeadingLevel::H2 => self.styles.h2,
            HeadingLevel::H3 => self.styles.h3,
            HeadingLevel::H4 => self.styles.h4,
            HeadingLevel::H5 => self.styles.h5,
            HeadingLevel::H6 => self.styles.h6,
        };
        let content = format!("{} ", "#".repeat(level as usize));
        self.push_line(Line::from(vec![Span::styled(content, heading_style)]));
        self.push_inline_style(heading_style);
        self.needs_newline = false;
    }

    fn end_heading(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.needs_newline = true;
        self.pop_inline_style();
    }

    fn start_blockquote(&mut self) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        self.indent_stack.push(IndentContext::new(
            vec![Span::styled("> ", self.styles.blockquote_prefix)],
            /*marker*/ None,
            /*is_list*/ false,
        ));
    }

    fn end_blockquote(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.indent_stack.pop();
        self.needs_newline = true;
    }

    fn text(&mut self, text: CowStr<'a>) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            self.push_text_to_table_cell(&text);
            return;
        }

        if self.pending_marker_line {
            self.push_line(Line::default());
        }
        self.pending_marker_line = false;

        // 在已知语言的围栏代码块内，将文本累积到缓冲区，
        // 以便在 end_codeblock() 中批量高亮。
        // 按原样追加 —— pulldown-cmark 的 text 事件已包含原始换行，
        // 因此插入分隔符会导致重复换行。
        if self.in_code_block && self.code_block_lang.is_some() {
            self.code_block_buffer.push_str(&text);
            return;
        }

        if self.in_code_block && !self.needs_newline {
            let has_content = self
                .current_line_content
                .as_ref()
                .map(|line| !line.line.spans.is_empty())
                .unwrap_or_else(|| {
                    self.text
                        .last()
                        .map(|line| !line.line.spans.is_empty())
                        .unwrap_or(false)
                });
            if has_content {
                self.push_line(Line::default());
            }
        }
        for (i, line) in text.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if i > 0 {
                self.push_line(Line::default());
            }
            let content = line.to_string();
            let style = self.inline_styles.last().copied().unwrap_or_default();
            self.push_text_spans(&content, style);
        }
        self.needs_newline = false;
    }

    fn code(&mut self, code: CowStr<'a>) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            self.push_span_to_table_cell(Span::from(code.into_string()).style(self.styles.code));
            return;
        }

        if self.pending_marker_line {
            self.push_line(Line::default());
            self.pending_marker_line = false;
        }
        let span = Span::from(code.into_string()).style(self.styles.code);
        self.push_span(span);
    }

    fn html(&mut self, html: CowStr<'a>, inline: bool) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            let style = self.inline_styles.last().copied().unwrap_or_default();
            for (i, line) in html.lines().enumerate() {
                if i > 0 {
                    self.push_table_cell_hard_break();
                }
                self.push_span_to_table_cell(Span::styled(line.to_string(), style));
            }
            if !inline {
                self.push_table_cell_hard_break();
            }
            return;
        }
        self.pending_marker_line = false;
        for (i, line) in html.lines().enumerate() {
            if self.needs_newline {
                self.push_line(Line::default());
                self.needs_newline = false;
            }
            if i > 0 {
                self.push_line(Line::default());
            }
            let style = self.inline_styles.last().copied().unwrap_or_default();
            self.push_span(Span::styled(line.to_string(), style));
        }
        self.needs_newline = !inline;
    }

    fn hard_break(&mut self) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            self.push_table_cell_hard_break();
            return;
        }
        self.push_line(Line::default());
    }

    fn soft_break(&mut self) {
        if self.suppressing_local_link_label() {
            return;
        }
        if self.in_table_cell() {
            let style = self.inline_styles.last().copied().unwrap_or_default();
            self.push_span_to_table_cell(Span::styled(" ".to_string(), style));
            return;
        }
        if self.line_ends_with_local_link_target {
            self.pending_local_link_soft_break = true;
            self.line_ends_with_local_link_target = false;
            return;
        }
        self.line_ends_with_local_link_target = false;
        self.push_line(Line::default());
    }

    fn start_list(&mut self, index: Option<u64>) {
        if self.list_indices.is_empty() && self.needs_newline {
            self.push_line(Line::default());
        }
        self.list_indices.push(index);
        self.list_needs_blank_before_next_item.push(false);
    }

    fn end_list(&mut self) {
        self.list_indices.pop();
        self.list_needs_blank_before_next_item.pop();
        self.needs_newline = true;
    }

    fn start_item(&mut self) {
        if self
            .list_needs_blank_before_next_item
            .last_mut()
            .map(std::mem::take)
            .unwrap_or(false)
        {
            self.push_blank_line();
        }
        self.flush_current_line();
        self.list_item_start_line_counts.push(self.text.len());
        self.pending_marker_line = true;
        let depth = self.list_indices.len();
        let is_ordered = self
            .list_indices
            .last()
            .map(Option::is_some)
            .unwrap_or(false);
        let width = depth * 4 - 3;
        let marker = if let Some(last_index) = self.list_indices.last_mut() {
            match last_index {
                None => Some(vec![Span::styled(
                    " ".repeat(width - 1) + "- ",
                    self.styles.unordered_list_marker,
                )]),
                Some(index) => {
                    *index += 1;
                    Some(vec![Span::styled(
                        format!("{:width$}. ", *index - 1),
                        self.styles.ordered_list_marker,
                    )])
                }
            }
        } else {
            None
        };
        let indent_prefix = if depth == 0 {
            Vec::new()
        } else {
            let indent_len = if is_ordered { width + 2 } else { width + 1 };
            vec![Span::from(" ".repeat(indent_len))]
        };
        self.indent_stack.push(IndentContext::new(
            indent_prefix,
            marker,
            /*is_list*/ true,
        ));
        self.needs_newline = false;
    }

    fn start_codeblock(&mut self, lang: Option<String>, indent: Option<Span<'static>>) {
        self.flush_current_line();
        if !self.text.is_empty() {
            self.push_blank_line();
        }
        self.in_code_block = true;

        // 从 info string 中提取语言标记。CommonMark 的 info string 在语言之后
        // 还可能包含元数据，由逗号、空格或其他分隔符分隔
        //（例如 "rust,no_run"、"rust title=demo"）。
        // 仅取第一个标记，以确保语法查找成功。
        let lang = lang
            .as_deref()
            .and_then(|s| s.split([',', ' ', '\t']).next())
            .filter(|s| !s.is_empty())
            .map(std::string::ToString::to_string);
        self.code_block_lang = lang;
        self.code_block_buffer.clear();

        self.indent_stack.push(IndentContext::new(
            vec![indent.unwrap_or_default()],
            /*marker*/ None,
            /*is_list*/ false,
        ));
        self.needs_newline = true;
    }

    fn end_codeblock(&mut self) {
        // 如果为已知语言缓冲了代码，现在对其进行语法高亮。
        if let Some(lang) = self.code_block_lang.take() {
            let code = std::mem::take(&mut self.code_block_buffer);
            if !code.is_empty() {
                let highlighted = highlight_code_to_lines(&code, &lang);
                for hl_line in highlighted {
                    self.push_line(Line::default());
                    for span in hl_line.spans {
                        self.push_span(span);
                    }
                }
            }
        }

        self.needs_newline = true;
        self.in_code_block = false;
        self.indent_stack.pop();
    }

    fn start_table(&mut self, alignments: Vec<Alignment>) {
        self.flush_current_line();
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        self.table_state = Some(TableState::new(alignments));
    }

    fn end_table(&mut self) {
        let Some(table_state) = self.table_state.take() else {
            return;
        };

        let RenderedTableLines {
            table_lines,
            table_lines_prewrapped,
            spillover_lines,
        } = self.render_table_lines(table_state);
        let mut pending_marker_line = self.pending_marker_line;
        for line in table_lines {
            if table_lines_prewrapped {
                self.push_prewrapped_line(line, pending_marker_line);
            } else {
                self.push_hyperlink_line(line);
                self.flush_current_line();
            }
            pending_marker_line = false;
        }
        self.pending_marker_line = false;
        for spillover_line in spillover_lines {
            self.push_hyperlink_line(spillover_line);
            self.flush_current_line();
        }
        self.needs_newline = true;
    }

    fn start_table_head(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.in_header = true;
            table_state.current_row = Some(Vec::new());
        }
    }

    fn end_table_head(&mut self) {
        let Some(table_state) = self.table_state.as_mut() else {
            return;
        };
        if let Some(current_cell) = table_state.current_cell.take() {
            table_state
                .current_row
                .get_or_insert_with(Vec::new)
                .push(current_cell);
        }
        if let Some(row) = table_state.current_row.take() {
            table_state.header = Some(row);
        }
        table_state.in_header = false;
    }

    fn start_table_row(&mut self, source_range: Range<usize>) {
        let has_table_pipe_syntax = self.has_table_row_boundary_pipe(source_range);
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.current_row = Some(Vec::new());
            table_state.current_row_has_table_pipe_syntax = has_table_pipe_syntax;
        }
    }

    fn has_table_row_boundary_pipe(&self, source_range: Range<usize>) -> bool {
        let Some(source) = self.input.get(source_range) else {
            return false;
        };
        let source = source.trim();
        source.starts_with('|') || source.ends_with('|')
    }

    fn end_table_row(&mut self) {
        let Some(table_state) = self.table_state.as_mut() else {
            return;
        };

        if let Some(current_cell) = table_state.current_cell.take() {
            table_state
                .current_row
                .get_or_insert_with(Vec::new)
                .push(current_cell);
        }

        let Some(row) = table_state.current_row.take() else {
            return;
        };

        if table_state.in_header {
            table_state.header = Some(row);
        } else {
            table_state.rows.push(TableBodyRow {
                cells: row,
                has_table_pipe_syntax: table_state.current_row_has_table_pipe_syntax,
            });
        }
        table_state.current_row_has_table_pipe_syntax = false;
    }

    fn start_table_cell(&mut self) {
        if let Some(table_state) = self.table_state.as_mut() {
            table_state.current_cell = Some(TableCell::default());
        }
    }

    fn end_table_cell(&mut self) {
        let Some(table_state) = self.table_state.as_mut() else {
            return;
        };

        if let Some(cell) = table_state.current_cell.take() {
            table_state
                .current_row
                .get_or_insert_with(Vec::new)
                .push(cell);
        }
    }

    fn in_table_cell(&self) -> bool {
        self.table_state
            .as_ref()
            .and_then(|table_state| table_state.current_cell.as_ref())
            .is_some()
    }

    fn push_span_to_table_cell(&mut self, span: Span<'static>) {
        if let Some(table_state) = self.table_state.as_mut()
            && let Some(cell) = table_state.current_cell.as_mut()
        {
            cell.push_span(span);
        }
    }

    fn push_table_cell_hard_break(&mut self) {
        if let Some(table_state) = self.table_state.as_mut()
            && let Some(cell) = table_state.current_cell.as_mut()
        {
            cell.hard_break();
        }
    }

    fn push_text_to_table_cell(&mut self, text: &str) {
        let style = self.inline_styles.last().copied().unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            if i > 0 {
                self.push_table_cell_hard_break();
            }
            self.push_text_spans_to_table_cell(line, style);
        }
    }

    fn push_text_spans_to_table_cell(&mut self, text: &str, style: Style) {
        let span = Span::styled(text.to_string(), style);
        let destination = self
            .link
            .as_ref()
            .and_then(|link| web_destination(&link.destination));
        let mut annotated = if let Some(destination) = destination {
            let mut annotated = HyperlinkLine::new(Line::default());
            annotated.push_span(span, Some(&destination));
            annotated
        } else if self.link.is_some() || self.in_code_block {
            HyperlinkLine::new(Line::from(span))
        } else {
            annotate_web_urls_in_line(Line::from(span))
        };
        if let Some(table_state) = self.table_state.as_mut()
            && let Some(cell) = table_state.current_cell.as_mut()
        {
            cell.push_annotated(std::mem::take(&mut annotated));
        }
    }

    /// 将已完成的 `TableState` 转换为带样式的表格 `Line`。
    ///
    /// 流水线：过滤溢出行 -> 标准化列数 -> 计算列宽 ->
    /// 渲染对齐的行，或在数值系统性失去可读性、或宽大单元格在多行内
    /// 退化为细窄条时退化为键值记录。溢出行以纯文本追加到表格之后。
    ///
    /// 当正文行无法放入对齐网格时，退化为键值记录；
    /// 仅含表头的表格保留原始管道输出，因为其中没有可转置的记录。
    fn render_table_lines(&self, mut table_state: TableState) -> RenderedTableLines {
        let column_count = table_state.alignments.len();
        if column_count == 0 {
            return RenderedTableLines {
                table_lines: Vec::new(),
                table_lines_prewrapped: true,
                spillover_lines: Vec::new(),
            };
        }

        let mut spillover_rows: Vec<TableCell> = Vec::with_capacity(4);
        let mut rows: Vec<Vec<TableCell>> = Vec::with_capacity(table_state.rows.len());
        for (row_idx, row) in table_state.rows.iter().enumerate() {
            let next_row = table_state.rows.get(row_idx + 1);
            // pulldown-cmark 允许不带管道符的正文行，这可能将后续段落变成单格表格行。
            // 对于多列表格，将其视为表格后渲染的溢出文本。
            if column_count > 1 && Self::is_spillover_row(row, next_row) {
                if let Some(cell) = row.cells.first().cloned() {
                    spillover_rows.push(cell);
                }
            } else {
                rows.push(row.cells.clone());
            }
        }

        let mut header = table_state
            .header
            .take()
            .unwrap_or_else(|| vec![TableCell::default(); column_count]);
        Self::normalize_row(&mut header, column_count);
        for row in &mut rows {
            Self::normalize_row(row, column_count);
        }

        let metrics = Self::collect_table_column_metrics(&header, &rows, column_count);
        let available_width = self.available_table_width(column_count);
        let widths = Self::compute_column_widths(&metrics, available_width);
        let spillover_lines: Vec<HyperlinkLine> = spillover_rows
            .into_iter()
            .flat_map(|spillover| spillover.lines)
            .collect();
        let header_style =
            foreground_style_for_scopes(&["entity.name.type", "support.type", "variable"])
                .unwrap_or(self.styles.strong)
                .bold();
        let separator_style = table_separator_style();

        let Some(column_widths) = widths else {
            if !rows.is_empty() {
                return RenderedTableLines {
                    table_lines: table_key_value::render_records(
                        &header,
                        &rows,
                        &metrics,
                        self.available_record_width(),
                        header_style,
                        separator_style,
                    ),
                    table_lines_prewrapped: true,
                    spillover_lines,
                };
            }
            return RenderedTableLines {
                table_lines: self.render_table_pipe_fallback(
                    &header,
                    &rows,
                    &table_state.alignments,
                ),
                table_lines_prewrapped: false,
                spillover_lines,
            };
        };

        if table_key_value::should_render_records(&rows, &column_widths, &metrics) {
            return RenderedTableLines {
                table_lines: table_key_value::render_records(
                    &header,
                    &rows,
                    &metrics,
                    self.available_record_width(),
                    header_style,
                    separator_style,
                ),
                table_lines_prewrapped: true,
                spillover_lines,
            };
        }

        let mut out = Vec::with_capacity(2 + rows.len() * 2);
        out.extend(self.render_table_row(
            &header,
            &column_widths,
            &table_state.alignments,
            header_style,
        ));
        out.push(Self::render_table_separator(
            &column_widths,
            TABLE_HEADER_SEPARATOR_CHAR,
            separator_style,
        ));
        for (row_idx, row) in rows.iter().enumerate() {
            out.extend(self.render_table_row(
                row,
                &column_widths,
                &table_state.alignments,
                Style::default(),
            ));
            if row_idx + 1 < rows.len() {
                out.push(Self::render_table_separator(
                    &column_widths,
                    TABLE_BODY_SEPARATOR_CHAR,
                    separator_style,
                ));
            }
        }
        RenderedTableLines {
            table_lines: out,
            table_lines_prewrapped: true,
            spillover_lines,
        }
    }

    fn normalize_row(row: &mut Vec<TableCell>, column_count: usize) {
        row.truncate(column_count);
        row.resize(column_count, TableCell::default());
    }

    /// 从内容预算中减去水平间隙和每个单元格的内边距。
    fn available_table_width(&self, column_count: usize) -> Option<usize> {
        self.wrap_width.map(|wrap_width| {
            let prefix_width =
                Self::spans_display_width(&self.prefix_spans(self.pending_marker_line));
            let reserved = prefix_width
                + (column_count.saturating_sub(1) * TABLE_COLUMN_GAP)
                + (column_count * TABLE_CELL_PADDING * 2);
            wrap_width.saturating_sub(reserved)
        })
    }

    /// 返回用于记录式回退渲染的完整内容预算。
    fn available_record_width(&self) -> Option<usize> {
        self.wrap_width.map(|wrap_width| {
            let prefix_width =
                Self::spans_display_width(&self.prefix_spans(self.pending_marker_line));
            wrap_width.saturating_sub(prefix_width)
        })
    }

    /// 为按行分隔的对齐表格渲染分配列宽。
    ///
    /// 每列从其固有（最大单元格内容）宽度开始，然后按优先级收缩列，
    /// 直到总宽度适应 `available_width`。Token 密集列先让出多余宽度，
    /// 然后是叙述性文本列；紧凑列最后保留。当即使最小宽度（每列 3 个字符）
    /// 仍无法放下时返回 `None`。
    pub(crate) fn compute_column_widths(
        metrics: &[TableColumnMetrics],
        available_width: Option<usize>,
    ) -> Option<Vec<usize>> {
        let min_column_width = 3usize;
        let mut widths: Vec<usize> = metrics
            .iter()
            .map(|col| col.max_width.max(min_column_width))
            .collect();

        let Some(max_width) = available_width else {
            return Some(widths);
        };
        let minimum_total = metrics.len() * min_column_width;
        if max_width < minimum_total {
            return None;
        }

        let mut floors: Vec<usize> = metrics
            .iter()
            .map(|col| Self::preferred_column_floor(col, min_column_width))
            .collect();
        let floor_total: usize = floors.iter().sum();
        if floor_total > max_width {
            let minimums = vec![min_column_width; floors.len()];
            Self::shrink_columns(&mut floors, &minimums, metrics, floor_total - max_width);
        }

        let total_width: usize = widths.iter().sum();
        if total_width > max_width {
            let remaining =
                Self::shrink_columns(&mut widths, &floors, metrics, total_width - max_width);
            if remaining > 0 {
                return None;
            }
        }

        Some(widths)
    }

    pub(crate) fn collect_table_column_metrics(
        header: &[TableCell],
        rows: &[Vec<TableCell>],
        column_count: usize,
    ) -> Vec<TableColumnMetrics> {
        let mut metrics = Vec::with_capacity(column_count);
        for column in 0..column_count {
            let header_cell = &header[column];
            let header_plain = header_cell.plain_text();
            let header_token_width = Self::longest_token_width(&header_plain);
            let mut max_width = Self::cell_display_width(header_cell);
            let mut body_token_width = 0usize;
            let mut body_token_count = 0usize;
            let mut long_body_token_count = 0usize;
            let mut total_words = 0usize;
            let mut total_cells = 0usize;
            let mut total_cell_width = 0usize;

            for row in rows {
                let cell = &row[column];
                max_width = max_width.max(Self::cell_display_width(cell));
                let plain = cell.plain_text();
                let mut word_count = 0usize;
                for token in plain.split_whitespace() {
                    let token_width = token.width();
                    body_token_width = body_token_width.max(token_width);
                    long_body_token_count += usize::from(token_width >= 20);
                    word_count += 1;
                }
                if word_count > 0 {
                    body_token_count += word_count;
                    total_words += word_count;
                    total_cells += 1;
                    total_cell_width += plain.width();
                }
            }

            let avg_words_per_cell = if total_cells == 0 {
                header_plain.split_whitespace().count() as f64
            } else {
                total_words as f64 / total_cells as f64
            };
            let avg_cell_width = if total_cells == 0 {
                header_plain.width() as f64
            } else {
                total_cell_width as f64 / total_cells as f64
            };
            let kind = if long_body_token_count > 0
                && long_body_token_count >= body_token_count.saturating_sub(long_body_token_count)
            {
                TableColumnKind::TokenHeavy
            } else if avg_words_per_cell >= 4.0 || avg_cell_width >= 28.0 {
                TableColumnKind::Narrative
            } else {
                TableColumnKind::Compact
            };

            metrics.push(TableColumnMetrics {
                max_width,
                header_token_width,
                body_token_width,
                kind,
            });
        }

        metrics
    }

    /// 在收缩循环开始进一步压缩之前，计算列的优选最小宽度。
    ///
    /// 叙述性和 token 密集列保留可读的 16 字符软下限。
    /// 紧凑列以表头和正文 token 宽度的较大者作为下限（正文上限为 16）。
    /// 结果被钳制到 `[min_column_width, max_width]` 区间。
    pub(crate) fn preferred_column_floor(
        metrics: &TableColumnMetrics,
        min_column_width: usize,
    ) -> usize {
        let token_target = match metrics.kind {
            TableColumnKind::Narrative | TableColumnKind::TokenHeavy => 16,
            TableColumnKind::Compact => metrics
                .header_token_width
                .max(metrics.body_token_width.min(16)),
        };
        token_target.max(min_column_width).min(metrics.max_width)
    }

    /// 按优先级顺序收缩列，并在同一优先级内平衡余量。
    ///
    /// 优先级：先收缩 TokenHeavy 列，再 Narrative，最后 Compact。
    /// 在同一类别内，余量高于下限最多的列先收缩，
    /// 使形状相似的列保持平衡。这与反复收缩单个显示单元格的结果一致，
    /// 同时无需反复扫描每列的长 token。
    pub(crate) fn shrink_columns(
        widths: &mut [usize],
        floors: &[usize],
        metrics: &[TableColumnMetrics],
        mut amount: usize,
    ) -> usize {
        for kind in [
            TableColumnKind::TokenHeavy,
            TableColumnKind::Narrative,
            TableColumnKind::Compact,
        ] {
            let slack_total = widths
                .iter()
                .enumerate()
                .filter(|(idx, _)| metrics[*idx].kind == kind)
                .map(|(idx, width)| width.saturating_sub(floors[idx]))
                .sum::<usize>();
            let to_remove = amount.min(slack_total);
            if to_remove == 0 {
                continue;
            }

            let mut low = 0usize;
            let mut high = widths
                .iter()
                .enumerate()
                .filter(|(idx, _)| metrics[*idx].kind == kind)
                .map(|(idx, width)| width.saturating_sub(floors[idx]))
                .max()
                .unwrap_or(/*default*/ 0);
            while low < high {
                let cap = low + (high - low) / 2;
                let removed = widths
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| metrics[*idx].kind == kind)
                    .map(|(idx, width)| width.saturating_sub(floors[idx]).saturating_sub(cap))
                    .sum::<usize>();
                if removed > to_remove {
                    low = cap + 1;
                } else {
                    high = cap;
                }
            }

            let cap = low;
            let mut removed = 0usize;
            for (idx, width) in widths.iter_mut().enumerate() {
                if metrics[idx].kind != kind {
                    continue;
                }
                let reduction = width.saturating_sub(floors[idx]).saturating_sub(cap);
                *width -= reduction;
                removed += reduction;
            }

            let mut remainder = to_remove - removed;
            for (idx, width) in widths.iter_mut().enumerate() {
                if remainder == 0 {
                    break;
                }
                if metrics[idx].kind == kind && width.saturating_sub(floors[idx]) == cap {
                    *width -= 1;
                    remainder -= 1;
                }
            }

            amount -= to_remove;
            if amount == 0 {
                break;
            }
        }

        amount
    }

    fn render_table_separator(
        column_widths: &[usize],
        separator_char: char,
        style: Style,
    ) -> HyperlinkLine {
        let segment_char = separator_char.to_string();
        let gap = " ".repeat(TABLE_COLUMN_GAP);
        let text = column_widths
            .iter()
            .map(|width| segment_char.repeat(*width + (TABLE_CELL_PADDING * 2)))
            .collect::<Vec<_>>()
            .join(&gap);
        HyperlinkLine::new(Line::from(Span::styled(text, style)))
    }

    fn render_table_row(
        &self,
        row: &[TableCell],
        column_widths: &[usize],
        alignments: &[Alignment],
        row_style: Style,
    ) -> Vec<HyperlinkLine> {
        let wrapped_cells: Vec<Vec<HyperlinkLine>> = row
            .iter()
            .zip(column_widths)
            .map(|(cell, width)| self.wrap_cell(cell, *width))
            .collect();
        let row_height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1);

        let mut out = Vec::with_capacity(row_height);
        for row_line in 0..row_height {
            let Some(last_visible_column) = wrapped_cells.iter().rposition(|lines| {
                lines
                    .get(row_line)
                    .is_some_and(|line| Self::line_display_width(&line.line) > 0)
            }) else {
                out.push(HyperlinkLine::new(Line::default().style(row_style)));
                continue;
            };
            let mut spans = Vec::new();
            for (column, width) in column_widths
                .iter()
                .enumerate()
                .take(last_visible_column + 1)
            {
                spans.push(Span::raw(" ".repeat(TABLE_CELL_PADDING)));
                let mut line = wrapped_cells[column]
                    .get(row_line)
                    .cloned()
                    .unwrap_or_default();
                let line_width = Self::line_display_width(&line.line);
                let remaining = width.saturating_sub(line_width);
                let (left_padding, right_padding) = match alignments[column] {
                    Alignment::Left | Alignment::None => (0, remaining),
                    Alignment::Center => (remaining / 2, remaining - (remaining / 2)),
                    Alignment::Right => (remaining, 0),
                };
                if left_padding > 0 {
                    spans.push(Span::raw(" ".repeat(left_padding)));
                }
                spans.append(&mut line.line.spans);
                let is_last_column = column == last_visible_column;
                if right_padding > 0 && !is_last_column {
                    spans.push(Span::raw(" ".repeat(right_padding)));
                }
                if !is_last_column {
                    spans.push(Span::raw(" ".repeat(TABLE_CELL_PADDING)));
                }
                if !is_last_column {
                    spans.push(Span::raw(" ".repeat(TABLE_COLUMN_GAP)));
                }
            }
            let mut out_line = HyperlinkLine::new(Line::from(spans).style(row_style));
            let mut column_start = 0usize;
            for (column, width) in column_widths
                .iter()
                .enumerate()
                .take(last_visible_column + 1)
            {
                column_start += TABLE_CELL_PADDING;
                if let Some(line) = wrapped_cells[column].get(row_line) {
                    let remaining = width.saturating_sub(Self::line_display_width(&line.line));
                    let left_padding = match alignments[column] {
                        Alignment::Left | Alignment::None => 0,
                        Alignment::Center => remaining / 2,
                        Alignment::Right => remaining,
                    };
                    out_line
                        .hyperlinks
                        .extend(line.hyperlinks.iter().cloned().map(|mut link| {
                            link.columns = link.columns.start + column_start + left_padding
                                ..link.columns.end + column_start + left_padding;
                            link
                        }));
                }
                column_start += *width + TABLE_CELL_PADDING + TABLE_COLUMN_GAP;
            }
            out.push(out_line);
        }
        out
    }

    /// 将仅含表头的表格渲染为原始的竖线分隔行（`| A | B |`）。
    ///
    /// 在 `compute_column_widths` 返回 `None` 且没有可转置的正文记录时使用。
    /// 单元格内容中的竖线字符会被转义为 `\|`，
    /// 以确保下游解析器保持单元格边界完整。
    fn render_table_pipe_fallback(
        &self,
        header: &[TableCell],
        rows: &[Vec<TableCell>],
        alignments: &[Alignment],
    ) -> Vec<HyperlinkLine> {
        let mut out = Vec::new();
        out.push(Self::row_to_pipe_line(header));
        out.push(HyperlinkLine::new(Line::from(
            Self::alignments_to_pipe_delimiter(alignments),
        )));
        out.extend(rows.iter().map(|row| Self::row_to_pipe_line(row)));
        out
    }

    fn row_to_pipe_line(row: &[TableCell]) -> HyperlinkLine {
        let mut out = HyperlinkLine::new(Line::default());
        out.push_span("|".into(), /*destination*/ None);
        for cell in row {
            out.push_span(" ".into(), /*destination*/ None);
            for (index, line) in cell.lines.iter().enumerate() {
                if index > 0 {
                    out.push_span(" ".into(), /*destination*/ None);
                }
                let text = line
                    .line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>();
                let mut column = 0usize;
                let mut current_destination = None;
                let mut current_text = String::new();
                let flush = |out: &mut HyperlinkLine,
                             current_text: &mut String,
                             destination: Option<&str>| {
                    if !current_text.is_empty() {
                        out.push_span(Span::raw(std::mem::take(current_text)), destination);
                    }
                };
                for ch in text.chars() {
                    let destination = line
                        .hyperlinks
                        .iter()
                        .find(|link| link.columns.contains(&column))
                        .map(|link| link.destination.as_str());
                    if destination != current_destination {
                        flush(&mut out, &mut current_text, current_destination);
                        current_destination = destination;
                    }
                    if ch == '|' {
                        current_text.push_str("\\|");
                    } else {
                        current_text.push(ch);
                    }
                    column += UnicodeWidthChar::width(ch).unwrap_or(/*default*/ 0);
                }
                flush(&mut out, &mut current_text, current_destination);
            }
            out.push_span(" |".into(), /*destination*/ None);
        }
        out
    }

    fn alignments_to_pipe_delimiter(alignments: &[Alignment]) -> String {
        let mut out = String::new();
        out.push('|');
        for alignment in alignments {
            let segment = match alignment {
                Alignment::Left => ":---",
                Alignment::Center => ":---:",
                Alignment::Right => "---:",
                Alignment::None => "---",
            };
            out.push_str(segment);
            out.push('|');
        }
        out
    }

    /// 将单个表格单元格的内容折行到 `width` 宽度，并在折行后的各行之间
    /// 保留富文本内联样式（粗体、代码、链接）。
    ///
    /// 单元格内的每个逻辑行（由硬换行分隔）独立折行。
    /// 空单元格产生单个空行，以保持行网格对齐。
    pub(super) fn wrap_cell(&self, cell: &TableCell, width: usize) -> Vec<HyperlinkLine> {
        if cell.lines.is_empty() {
            return vec![HyperlinkLine::new(Line::default())];
        }
        let mut wrapped = Vec::new();
        for source_line in &cell.lines {
            let rendered =
                word_wrap_line(&source_line.line, RtOptions::new(width.max(/*other*/ 1)))
                    .into_iter()
                    .map(|line| line_to_static(&line))
                    .collect::<Vec<_>>();
            if rendered.is_empty() {
                wrapped.push(HyperlinkLine::new(Line::default()));
            } else {
                wrapped.extend(remap_wrapped_line(source_line, rendered));
            };
        }
        if wrapped.is_empty() {
            wrapped.push(HyperlinkLine::new(Line::default()));
        }
        wrapped
    }

    /// 检测由 pulldown-cmark 宽松表格解析产生的伪影行。
    ///
    /// pulldown-cmark 接受不带前导竖线的正文行，这可能将末尾的段落吸收为
    /// 多列表格中的单格行。这些“溢出”（spillover）行会被提取出来，
    /// 作为纯文本渲染在表格网格之后，以免显示为格式错误的表格内容。
    ///
    /// 启发式规则：当一行唯一的非空单元格是第一个单元格，且满足以下任一条件时，
    /// 该行即为溢出行（单格行缺少表格竖线语法、内容看起来像 HTML、
    /// 是后跟 HTML 内容的标签行，或是末尾的 HTML 引导标签行）。
    pub(crate) fn is_spillover_row(row: &TableBodyRow, next_row: Option<&TableBodyRow>) -> bool {
        let Some(first_text) = Self::first_non_empty_only_text(&row.cells) else {
            return false;
        };

        if row.cells.len() == 1 && !row.has_table_pipe_syntax {
            return true;
        }

        if Self::looks_like_html_content(&first_text) {
            return true;
        }

        // 将常见的引导语 + HTML 块溢出行保持在一起：
        // 即 "HTML block:" 后跟 "<div ...>"。
        if first_text.trim_end().ends_with(':') {
            if next_row
                .and_then(|row| Self::first_non_empty_only_text(&row.cells))
                .is_some_and(|text| Self::looks_like_html_content(&text))
            {
                return true;
            }

            // pulldown 可能会在对应的 HTML 块行之前结束表格。
            // 此时，将末尾的 HTML 引导标签（例如 "HTML block:"）视为溢出行，
            // 同时保留真实表格中的显式稀疏标签。
            if next_row.is_none() && Self::looks_like_html_label_line(&first_text) {
                return true;
            }
        }

        false
    }

    fn first_non_empty_only_text(row: &[TableCell]) -> Option<String> {
        let first = row.first()?.plain_text();
        if first.trim().is_empty() {
            return None;
        }
        let rest_empty = row[1..]
            .iter()
            .all(|cell| cell.plain_text().trim().is_empty());
        rest_empty.then_some(first)
    }

    fn looks_like_html_content(text: &str) -> bool {
        let bytes = text.as_bytes();
        for (idx, &byte) in bytes.iter().enumerate() {
            if byte != b'<' {
                continue;
            }

            let mut tag_start = idx + 1;
            if tag_start < bytes.len() && (bytes[tag_start] == b'/' || bytes[tag_start] == b'!') {
                tag_start += 1;
            }

            if bytes.get(tag_start).is_some_and(u8::is_ascii_alphabetic)
                && bytes
                    .get(tag_start + 1..)
                    .is_some_and(|suffix| suffix.contains(&b'>'))
            {
                return true;
            }
        }
        false
    }

    fn looks_like_html_label_line(text: &str) -> bool {
        let trimmed = text.trim();
        if !trimmed.ends_with(':') {
            return false;
        }
        let prefix = trimmed.trim_end_matches(':').trim();
        prefix
            .split_whitespace()
            .any(|word| word.eq_ignore_ascii_case("html"))
    }

    // 宽度测量辅助函数采用内联——在表格列宽计算期间按单元格调用，
    // 而该计算在每次重新渲染时都会运行。

    #[inline]
    fn spans_display_width(spans: &[Span<'_>]) -> usize {
        spans.iter().map(|span| span.content.width()).sum()
    }

    #[inline]
    fn line_display_width(line: &Line<'_>) -> usize {
        Self::spans_display_width(&line.spans)
    }

    #[inline]
    fn cell_display_width(cell: &TableCell) -> usize {
        cell.lines
            .iter()
            .map(|line| Self::line_display_width(&line.line))
            .max()
            .unwrap_or(0)
    }

    #[inline]
    fn longest_token_width(text: &str) -> usize {
        text.split_whitespace().map(str::width).max().unwrap_or(0)
    }

    fn push_inline_style(&mut self, style: Style) {
        let current = self.inline_styles.last().copied().unwrap_or_default();
        let merged = current.patch(style);
        self.inline_styles.push(merged);
    }

    fn pop_inline_style(&mut self) {
        self.inline_styles.pop();
    }

    fn push_link(&mut self, dest_url: String) {
        let style_label = (self.is_hidden_link_destination)(&dest_url);
        if style_label {
            self.push_inline_style(self.styles.link);
        }
        let show_destination = !style_label && should_render_link_destination(&dest_url);
        self.link = Some(LinkState {
            show_destination,
            style_label,
            local_target_display: if is_local_path_like_link(&dest_url) {
                render_local_link_target(&dest_url, self.cwd.as_deref())
            } else {
                None
            },
            destination: dest_url,
        });
    }

    fn pop_link(&mut self) {
        if let Some(link) = self.link.take() {
            if link.style_label {
                self.pop_inline_style();
            }
            if link.show_destination {
                // 链接目标会渲染为 " (url)" 后缀。在解析表格单元格时，
                // 应将该后缀追加到当前单元格缓冲区，而不是外层段落行，
                // 以避免 url 行脱离单元格。
                if self.in_table_cell() {
                    self.push_span_to_table_cell(" (".into());
                    let mut destination = HyperlinkLine::new(Line::default());
                    destination.push_span(
                        Span::styled(link.destination.clone(), self.styles.link),
                        web_destination(&link.destination).as_deref(),
                    );
                    if let Some(table_state) = self.table_state.as_mut()
                        && let Some(cell) = table_state.current_cell.as_mut()
                    {
                        cell.push_annotated(destination);
                    }
                    self.push_span_to_table_cell(")".into());
                } else {
                    self.push_span(" (".into());
                    let mut destination = HyperlinkLine::new(Line::default());
                    destination.push_span(
                        Span::styled(link.destination.clone(), self.styles.link),
                        web_destination(&link.destination).as_deref(),
                    );
                    self.push_annotated(destination);
                    self.push_span(")".into());
                }
            } else if let Some(local_target_display) = link.local_target_display {
                // 本地文件链接会渲染为类似代码的路径文本，使记录中显示
                // 解析后的目标，而不是调用方随意提供的标签文本。
                let style = self
                    .inline_styles
                    .last()
                    .copied()
                    .unwrap_or_default()
                    .patch(self.styles.code);
                let span = Span::styled(local_target_display, style);
                if self.in_table_cell() {
                    self.push_span_to_table_cell(span);
                } else {
                    if self.pending_marker_line {
                        self.push_line(Line::default());
                    }
                    self.push_span(span);
                    self.line_ends_with_local_link_target = true;
                }
            }
        }
    }

    fn suppressing_local_link_label(&self) -> bool {
        self.link
            .as_ref()
            .and_then(|link| link.local_target_display.as_ref())
            .is_some()
    }

    fn flush_current_line(&mut self) {
        if let Some(mut line) = self.current_line_content.take() {
            let style = self.current_line_style;
            // 注意：我们不对代码块中的代码折行，以便保留空白字符供复制/粘贴。
            if !self.current_line_in_code_block
                && let Some(width) = self.wrap_width
            {
                let opts = RtOptions::new(width)
                    .initial_indent(self.current_initial_indent.clone().into())
                    .subsequent_indent(self.current_subsequent_indent.clone().into());
                let wrapped = adaptive_wrap_line(&line.line, opts)
                    .into_iter()
                    .map(|wrapped| line_to_static(&wrapped))
                    .collect();
                for wrapped in remap_wrapped_line(&line, wrapped) {
                    self.push_output_line(wrapped.style(style));
                }
            } else {
                let mut spans = self.current_initial_indent.clone();
                let shift = spans.iter().map(|span| span.content.width()).sum::<usize>();
                spans.append(&mut line.line.spans);
                for hyperlink in &mut line.hyperlinks {
                    hyperlink.columns =
                        hyperlink.columns.start + shift..hyperlink.columns.end + shift;
                }
                line.line = Line::from_iter(spans);
                self.push_output_line(line.style(style));
            }
            self.current_initial_indent.clear();
            self.current_subsequent_indent.clear();
            self.current_line_in_code_block = false;
            self.line_ends_with_local_link_target = false;
        }
    }

    /// 推送一行已按正确宽度排版好的行，跳过自动折行。
    ///
    /// 表格行已使用精确的列宽和分隔线预先格式化。
    /// 让它们经过 `word_wrap_line` 会在任意位置破坏布局。
    /// 本方法在行前添加缩进/引用块前缀，
    /// 然后直接推送到 `self.text`。
    fn is_blockquote_active(&self) -> bool {
        self.indent_stack
            .iter()
            .any(|ctx| ctx.prefix.iter().any(|p| p.content.contains('>')))
    }

    fn push_prewrapped_line(&mut self, mut line: HyperlinkLine, pending_marker_line: bool) {
        self.flush_current_line();
        let blockquote_active = self.is_blockquote_active();
        let style = if blockquote_active {
            self.styles.blockquote.patch(line.line.style)
        } else {
            line.line.style
        };

        let mut spans = self.prefix_spans(pending_marker_line);
        let shift = spans.iter().map(|span| span.content.width()).sum::<usize>();
        spans.append(&mut line.line.spans);
        for hyperlink in &mut line.hyperlinks {
            hyperlink.columns = hyperlink.columns.start + shift..hyperlink.columns.end + shift;
        }
        line.line = Line::from(spans);
        self.push_output_line(line.style(style));
    }

    fn push_line(&mut self, line: Line<'static>) {
        self.flush_current_line();
        let blockquote_active = self.is_blockquote_active();
        let style = if blockquote_active {
            self.styles.blockquote
        } else {
            line.style
        };
        let was_pending = self.pending_marker_line;

        self.current_initial_indent = self.prefix_spans(was_pending);
        self.current_subsequent_indent = self.prefix_spans(/*pending_marker_line*/ false);
        self.current_line_style = style;
        self.current_line_content = Some(HyperlinkLine::new(line));
        self.current_line_in_code_block = self.in_code_block;
        self.line_ends_with_local_link_target = false;

        self.pending_marker_line = false;
    }

    fn push_hyperlink_line(&mut self, line: HyperlinkLine) {
        let hyperlinks = line.hyperlinks;
        self.push_line(line.line);
        if let Some(current) = self.current_line_content.as_mut() {
            current.hyperlinks = hyperlinks;
        }
    }

    fn push_span(&mut self, span: Span<'static>) {
        if let Some(line) = self.current_line_content.as_mut() {
            line.line.push_span(span);
        } else {
            self.push_line(Line::from(vec![span]));
        }
    }

    fn push_annotated(&mut self, mut appended: HyperlinkLine) {
        if self.current_line_content.is_none() {
            self.push_line(Line::default());
        }
        if let Some(line) = self.current_line_content.as_mut() {
            let shift = line.width();
            line.line.spans.append(&mut appended.line.spans);
            line.hyperlinks
                .extend(appended.hyperlinks.into_iter().map(|mut link| {
                    link.columns = link.columns.start + shift..link.columns.end + shift;
                    link
                }));
        }
    }

    fn push_text_spans(&mut self, text: &str, style: Style) {
        let span = Span::styled(text.to_string(), style);
        let destination = self
            .link
            .as_ref()
            .and_then(|link| web_destination(&link.destination));
        let annotated = if let Some(destination) = destination {
            let mut annotated = HyperlinkLine::new(Line::default());
            annotated.push_span(span, Some(&destination));
            annotated
        } else if self.link.is_some() || self.in_code_block {
            HyperlinkLine::new(Line::from(span))
        } else {
            annotate_web_urls_in_line(Line::from(span))
        };
        self.push_annotated(annotated);
    }

    fn push_blank_line(&mut self) {
        self.flush_current_line();
        if self.indent_stack.iter().all(|ctx| ctx.is_list) {
            self.push_output_line(HyperlinkLine::new(Line::default()));
        } else {
            self.push_line(Line::default());
            self.flush_current_line();
        }
    }

    fn push_output_line(&mut self, line: HyperlinkLine) {
        self.text.push(line);
    }

    fn prefix_spans(&self, pending_marker_line: bool) -> Vec<Span<'static>> {
        let mut prefix: Vec<Span<'static>> = Vec::new();
        let last_marker_index = if pending_marker_line {
            self.indent_stack
                .iter()
                .enumerate()
                .rev()
                .find_map(|(i, ctx)| if ctx.marker.is_some() { Some(i) } else { None })
        } else {
            None
        };
        let last_list_index = self.indent_stack.iter().rposition(|ctx| ctx.is_list);

        for (i, ctx) in self.indent_stack.iter().enumerate() {
            if pending_marker_line {
                if Some(i) == last_marker_index
                    && let Some(marker) = &ctx.marker
                {
                    prefix.extend(marker.iter().cloned());
                    continue;
                }
                if ctx.is_list && last_marker_index.is_some_and(|idx| idx > i) {
                    continue;
                }
            } else if ctx.is_list && Some(i) != last_list_index {
                continue;
            }
            prefix.extend(ctx.prefix.iter().cloned());
        }

        prefix
    }
}
