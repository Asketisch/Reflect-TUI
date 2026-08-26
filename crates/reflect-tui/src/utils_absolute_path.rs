//! reflect_utils_absolute_path crate 的桩。

use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct AbsolutePathBuf(pub String);

impl AbsolutePathBuf {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(&self.0)
    }
}

impl std::ops::Deref for AbsolutePathBuf {
    type Target = Path;
    fn deref(&self) -> &Self::Target {
        self.as_path()
    }
}

impl AsRef<Path> for AbsolutePathBuf {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl std::fmt::Display for AbsolutePathBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for AbsolutePathBuf {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for AbsolutePathBuf {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<PathBuf> for AbsolutePathBuf {
    fn from(p: PathBuf) -> Self {
        Self(p.to_string_lossy().into_owned())
    }
}

impl serde::Serialize for AbsolutePathBuf {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for AbsolutePathBuf {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self(s))
    }
}

pub fn canonicalize_existing_preserving_symlinks(path: &str) -> AbsolutePathBuf {
    AbsolutePathBuf(path.to_string())
}

impl AbsolutePathBuf {
    pub fn display(&self) -> String {
        self.0.clone()
    }
}

impl AbsolutePathBuf {
    pub fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(&self.0)
    }
}

pub trait ToInferredAbsPath {
    fn to_inferred_abs_path(self) -> AbsolutePathBuf;
}

impl ToInferredAbsPath for String {
    fn to_inferred_abs_path(self) -> AbsolutePathBuf {
        AbsolutePathBuf(self)
    }
}

impl ToInferredAbsPath for PathBuf {
    fn to_inferred_abs_path(self) -> AbsolutePathBuf {
        AbsolutePathBuf::from(self)
    }
}
