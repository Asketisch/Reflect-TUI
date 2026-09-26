use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathConvention {
    Local,
    Browser,
    Native,
}

impl PathConvention {
    pub fn native() -> Self {
        Self::Native
    }
}
pub enum LegacyAppPathString {
    Local(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathUri(pub String);

impl LegacyAppPathString {
    pub fn to_inferred_path_uri(&self) -> Option<PathUri> {
        match self {
            Self::Local(s) => Some(PathUri(s.clone())),
        }
    }
}

impl LegacyAppPathString {
    pub fn into_string(&self) -> String {
        match self {
            Self::Local(s) => s.clone(),
        }
    }
}

impl PathUri {
    pub fn infer_path_convention(&self) -> Option<PathConvention> {
        Some(PathConvention::Native)
    }

    pub fn inferred_native_path_string(&self) -> String {
        self.0.clone()
    }

    /// 恒成功的克隆(历史签名是 `Result<PathUri, ()>`,但实现从不失败;
    /// clippy result_unit_err 下改为不可失败返回)。
    pub fn to_abs_path(&self) -> PathUri {
        self.clone()
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}
