use super::*;

/// 保守的上限，防止单条用户消息独占庞大的上下文窗口。
/// 与上游的 `1 << 20` 保持一致。
pub const MAX_USER_INPUT_TEXT_CHARS: usize = 1 << 20;

/// 父 UTF-8 文本缓冲区内部的字节区间。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl From<std::ops::Range<usize>> for ByteRange {
    fn from(range: std::ops::Range<usize>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }
}

impl From<crate::app_server_protocol::ByteRange> for ByteRange {
    fn from(value: crate::app_server_protocol::ByteRange) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

/// 用户文本消息中由 UI 定义的、应以特殊元素（如图片占位符）
/// 渲染或持久化的区间。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextElement {
    pub byte_range: ByteRange,
    /// 元素的可选人读占位符。在桩实现中保持公开，
    /// 以便外部调用方（输入框状态、历史单元格）无需经过访问器 API
    /// 即可克隆/检查它。
    pub placeholder: Option<String>,
}

impl TextElement {
    pub fn new(byte_range: ByteRange, placeholder: Option<String>) -> Self {
        Self {
            byte_range,
            placeholder,
        }
    }

    /// 返回字节区间被重映射后的元素副本。
    pub fn map_range<F>(&self, map: F) -> Self
    where
        F: FnOnce(ByteRange) -> ByteRange,
    {
        Self {
            byte_range: map(self.byte_range),
            placeholder: self.placeholder.clone(),
        }
    }

    pub fn set_placeholder(&mut self, placeholder: Option<String>) {
        self.placeholder = placeholder;
    }

    /// 用于显示的占位符，回退为原样文本区间。
    pub fn placeholder<'a>(&'a self, text: &'a str) -> Option<&'a str> {
        self.placeholder
            .as_deref()
            .or_else(|| text.get(self.byte_range.start..self.byte_range.end))
    }
}
