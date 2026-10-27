use super::*;

/// 当候选值违反受管约束时抛出的错误。
///
/// 变体与上游枚举一致，调用方可以直接构造它们。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintError {
    InvalidValue {
        field_name: String,
        candidate: String,
        allowed: String,
        requirement_source: RequirementSource,
    },
    EmptyField {
        field_name: String,
    },
    ExecPolicyParse {
        requirement_source: RequirementSource,
        reason: String,
    },
    McpServerRequirementParse {
        server_name: String,
        requirement_source: RequirementSource,
        reason: String,
    },
}

impl ConstraintError {
    pub fn empty_field(field_name: impl Into<String>) -> Self {
        Self::EmptyField {
            field_name: field_name.into(),
        }
    }
}

impl From<String> for ConstraintError {
    fn from(reason: String) -> Self {
        Self::ExecPolicyParse {
            requirement_source: RequirementSource::Unknown,
            reason,
        }
    }
}

impl std::fmt::Display for ConstraintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidValue {
                field_name,
                candidate,
                allowed,
                requirement_source,
            } => {
                write!(
                    f,
                    "invalid value for `{field_name}`: `{candidate}` is not in the allowed set {allowed} (set by {requirement_source})"
                )
            }
            Self::EmptyField { field_name } => {
                write!(f, "field `{field_name}` cannot be empty")
            }
            Self::ExecPolicyParse {
                requirement_source,
                reason,
            } => {
                write!(
                    f,
                    "invalid rules in requirements (set by {requirement_source}): {reason}"
                )
            }
            Self::McpServerRequirementParse {
                server_name,
                requirement_source,
                reason,
            } => {
                write!(
                    f,
                    "invalid requirement for MCP server `{server_name}` (set by {requirement_source}): {reason}"
                )
            }
        }
    }
}

impl std::error::Error for ConstraintError {}

/// 上游约束 API 中广泛使用的便捷别名。
pub type ConstraintResult<T> = Result<T, ConstraintError>;

/// 一个值与验证器（以及可选的规范化器）配对，受管需求用它来拒绝不允许的配置。
///
/// 存根实现总是接受任何值，不做真正的验证。它存在的目的是让内置调用点
/// （`Constrained::allow_any`、`Constrained::new`、`Constrained::value`）继续通过类型检查。
#[derive(Debug, Clone)]
pub struct Constrained<T> {
    pub value: T,
}

impl<T> Constrained<T> {
    pub fn new(
        initial_value: T,
        _validator: impl Fn(&T) -> ConstraintResult<()>,
    ) -> ConstraintResult<Self> {
        Ok(Self {
            value: initial_value,
        })
    }

    pub fn allow_any(initial_value: T) -> Self {
        Self {
            value: initial_value,
        }
    }

    pub fn allow_only(only_value: T) -> Self
    where
        T: Clone,
    {
        Self { value: only_value }
    }

    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn value(&self) -> T
    where
        T: Copy,
    {
        self.value
    }

    pub fn can_set(&self, _candidate: &T) -> ConstraintResult<()> {
        Ok(())
    }

    pub fn set(&mut self, value: T) -> ConstraintResult<()> {
        self.value = value;
        Ok(())
    }
}

impl<T: Default> Default for Constrained<T> {
    fn default() -> Self {
        Self {
            value: T::default(),
        }
    }
}

impl<T> std::ops::Deref for Constrained<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// 与贡献它的 [`RequirementSource`] 配对的 `Constrained<T>`。
#[derive(Debug, Clone)]
pub struct ConstrainedWithSource<T> {
    pub value: T,
    pub source: RequirementSource,
}

impl<T> ConstrainedWithSource<T> {
    pub fn new(value: T, _source: RequirementSource) -> Self {
        Self {
            value,
            source: RequirementSource::Unknown,
        }
    }

    pub fn allow_only(only_value: T) -> Self {
        Self {
            value: only_value,
            source: RequirementSource::Unknown,
        }
    }

    pub fn allow_any(any_value: T) -> Self {
        Self {
            value: any_value,
            source: RequirementSource::Unknown,
        }
    }

    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn can_set(&self, _value: &T) -> bool {
        true
    }

    pub fn set(&mut self, value: T) -> Result<(), String>
    where
        T: Clone,
    {
        self.value = value;
        Ok(())
    }
}

impl<T: Default> Default for ConstrainedWithSource<T> {
    fn default() -> Self {
        Self {
            value: T::default(),
            source: RequirementSource::Unknown,
        }
    }
}

impl<T> std::ops::Deref for ConstrainedWithSource<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// 与贡献它的 [`RequirementSource`]（或层来源）配对的普通值。用于未经校验的受管需求。
#[derive(Debug, Clone)]
pub struct Sourced<T> {
    pub value: T,
    pub source: RequirementSource,
}

impl<T> Sourced<T> {
    pub fn new(value: T, _source: impl Into<RequirementSource>) -> Self {
        Self {
            value,
            source: RequirementSource::Unknown,
        }
    }

    pub fn get(&self) -> &T {
        &self.value
    }
}

impl<T> AsRef<T> for Sourced<T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T: Default> Default for Sourced<T> {
    fn default() -> Self {
        Self {
            value: T::default(),
            source: RequirementSource::Unknown,
        }
    }
}
