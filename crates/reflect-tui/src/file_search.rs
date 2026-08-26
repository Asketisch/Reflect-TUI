//! reflect_file_search crate 的桩实现。
#[derive(Debug, Clone, Default)]
pub struct FileMatch {
    pub path: std::path::PathBuf,
    pub line_number: Option<usize>,
    pub indices: Option<Vec<usize>>,
    pub match_type: MatchType,
    pub score: i32,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum MatchType {
    #[default]
    Filename,
    Content,
    Directory,
    File,
}
