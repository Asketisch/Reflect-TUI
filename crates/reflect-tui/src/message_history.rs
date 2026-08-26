#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryBatchCursor {
    pub end_offset: usize,
}

impl HistoryBatchCursor {
    pub fn new(end_offset: usize) -> Self {
        Self { end_offset }
    }

    pub fn end_offset(&self) -> usize {
        self.end_offset
    }
}
