pub fn path_utils() -> PathUtils {
    PathUtils
}
pub struct PathUtils;
impl PathUtils {
    pub fn is_hidden(&self, _path: &std::path::Path) -> bool {
        false
    }
}
