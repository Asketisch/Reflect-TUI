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

    pub fn to_abs_path(&self) -> Result<PathUri, ()> {
        Ok(self.clone())
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}
