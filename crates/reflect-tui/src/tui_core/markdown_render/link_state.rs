//! Markdown 链接状态子系统。从 markdown_render/mod.rs 抽出。

use super::*;

pub(super) fn never_hide_link_destination(_: &str) -> bool {
    false
}

pub(crate) fn render_markdown_lines_with_width_cwd_and_hidden_link_destinations(
    input: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
    is_hidden_link_destination: &dyn Fn(&str) -> bool,
) -> Vec<HyperlinkLine> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let parser = DecodedTextMerge::new(Parser::new_ext(input, options).into_offset_iter());
    let mut w = Writer::new(input, parser, width, cwd, is_hidden_link_destination);
    w.run();
    w.text
}

#[derive(Clone, Debug)]
pub(super) struct LinkState {
    pub(super) destination: String,
    pub(super) show_destination: bool,
    pub(super) style_label: bool,
/// 本地文件链接的预渲染显示文本。
///
/// 当此字段存在时，markdown label 会被有意抑制，确保渲染出的转写始终反映真实的目标路径。
    pub(super) local_target_display: Option<String>,
}

pub(super) fn should_render_link_destination(dest_url: &str) -> bool {
    !is_local_path_like_link(dest_url)
}

pub(super) static COLON_LOCATION_SUFFIX_RE: LazyLock<Regex> =
    LazyLock::new(
        || match Regex::new(r":\d+(?::\d+)?(?:[-–]\d+(?::\d+)?)?$") {
            Ok(regex) => regex,
            Err(error) => panic!("invalid location suffix regex: {error}"),
        },
    );

// 已由 load_location_suffix_regexes 覆盖。
pub(super) static HASH_LOCATION_SUFFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| match Regex::new(r"^L\d+(?:C\d+)?(?:-L\d+(?:C\d+)?)?$") {
        Ok(regex) => regex,
        Err(error) => panic!("invalid hash location regex: {error}"),
    });
