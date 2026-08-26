use super::*;
// 配置层类型
// ===========================================================================

/// 有效 Reflect 配置中某一层来源的说明。
///
/// 建模为带有字符串载荷的枚举，使调用方无需依赖 `AbsolutePathBuf` 即可对上游形态进行模式匹配。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigLayerSource {
    /// 由 MDM 交付的受管偏好设置。
    Mdm { domain: String, key: String },
    /// 从文件加载的主机级配置。
    System { file: PathBuf },
    /// 由企业云捆绑包交付的配置。
    EnterpriseManaged { id: String, name: String },
    /// 用户配置，可选择由所选配置文件（profile）增强。
    User {
        file: PathBuf,
        profile: Option<String>,
    },
    /// 从项目的 .reflect 目录加载的配置。
    Project { dot_reflect_folder: PathBuf },
    /// 为当前会话提供的覆盖项。
    #[default]
    SessionFlags,
    /// 从文件加载的旧版受管配置。
    LegacyManagedConfigTomlFromFile { file: PathBuf },
    /// 由 MDM 交付的旧版受管配置。
    LegacyManagedConfigTomlFromMdm,
}

impl ConfigLayerSource {
    /// 优先级较高的层中的设置会覆盖优先级较低的层中的设置。
    pub fn precedence(&self) -> i16 {
        match self {
            Self::Mdm { .. } => 0,
            Self::System { .. } => 10,
            Self::EnterpriseManaged { .. } => 15,
            Self::User { profile, .. } => {
                if profile.is_some() {
                    21
                } else {
                    20
                }
            }
            Self::Project { .. } => 25,
            Self::SessionFlags => 30,
            Self::LegacyManagedConfigTomlFromFile { .. } => 40,
            Self::LegacyManagedConfigTomlFromMdm => 50,
        }
    }
}

impl std::fmt::Display for ConfigLayerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mdm { domain, key } => write!(f, "MDM ({domain}:{key})"),
            Self::System { file } => write!(f, "system ({})", file.display()),
            Self::EnterpriseManaged { id, name } => {
                write!(f, "enterprise-managed ({name}, {id})")
            }
            Self::User { file, .. } => write!(f, "user ({})", file.display()),
            Self::Project { dot_reflect_folder } => {
                write!(f, "project ({})", dot_reflect_folder.display())
            }
            Self::SessionFlags => write!(f, "session-flags"),
            Self::LegacyManagedConfigTomlFromFile { file } => {
                write!(f, "legacy managed_config.toml ({})", file.display())
            }
            Self::LegacyManagedConfigTomlFromMdm => {
                write!(f, "legacy managed_config.toml (MDM)")
            }
        }
    }
}

/// 一个已物化的配置层及其来源。
///
/// 上游结构体携带解析后的 TOML 值以及版本字符串；此存根保留了内置调试渲染器读取的公共字段。
#[derive(Debug, Clone)]
pub struct ConfigLayerEntry {
    pub name: ConfigLayerSource,
    pub version: String,
    pub disabled_reason: Option<String>,
    /// 该层的原始 TOML 内容（如果它是从文件加载的）。
    pub raw_toml: Option<String>,
    pub config: toml::Value,
}

impl Default for ConfigLayerEntry {
    fn default() -> Self {
        Self {
            name: ConfigLayerSource::default(),
            version: String::new(),
            disabled_reason: None,
            raw_toml: None,
            config: toml::Value::String(String::new()),
        }
    }
}

impl ConfigLayerEntry {
    /// 如果该层已被禁用（例如解析失败），则返回 `true`。
    pub fn is_disabled(&self) -> bool {
        self.disabled_reason.is_some()
    }

    /// 该层的原始 TOML 源（如果有）。
    pub fn raw_toml(&self) -> Option<&str> {
        self.raw_toml.as_deref()
    }
}

/// 遍历 [`ConfigLayerStack`] 各层的顺序。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConfigLayerStackOrdering {
    /// 优先级最低者优先（从基础层到最上层覆盖）。
    #[default]
    LowestPrecedenceFirst,
    /// 优先级最高者优先（从最上层覆盖到基础层）。
    HighestPrecedenceFirst,
}

/// 已物化的配置层栈，以及约束其如何解析为 `Config` 的受管需求。
///
/// 此存根保留了内置调试渲染器使用的方法形态：`get_layers`、`requirements` 和 `requirements_toml`。
#[derive(Debug, Clone, Default)]
pub struct ConfigLayerStack {
    pub layers: Vec<ConfigLayerEntry>,
    pub requirements: ConfigRequirements,
    pub requirements_toml: ConfigRequirementsToml,
}

impl ConfigLayerStack {
    /// 从已物化的层及其约束需求构建层栈。
    pub fn new(
        layers: Vec<ConfigLayerEntry>,
        requirements: ConfigRequirements,
        requirements_toml: ConfigRequirementsToml,
    ) -> std::io::Result<Self> {
        Ok(Self {
            layers,
            requirements,
            requirements_toml,
        })
    }

    /// 按请求的优先级顺序返回各层。
    pub fn get_layers(
        &self,
        _ordering: ConfigLayerStackOrdering,
        include_disabled: bool,
    ) -> Vec<&ConfigLayerEntry> {
        self.layers
            .iter()
            .filter(|layer| include_disabled || !layer.is_disabled())
            .collect()
    }

    pub fn requirements(&self) -> &ConfigRequirements {
        &self.requirements
    }

    pub fn requirements_toml(&self) -> &ConfigRequirementsToml {
        &self.requirements_toml
    }

    /// 返回有效的合并后用户配置（如果有）。
    pub fn effective_user_config(&self) -> Option<&toml::Value> {
        self.layers.last().map(|layer| &layer.config)
    }

    /// 返回当前活动（最上层未禁用）的用户配置层。
    pub fn get_active_user_layer(&self) -> Option<&ConfigLayerEntry> {
        self.layers.iter().find(|layer| !layer.is_disabled())
    }
}

/// 由加载器提供、应用于已解析配置栈之上的覆盖项。
#[derive(Debug, Clone, Default)]
pub struct LoaderOverrides {
    pub config_overrides: Option<String>,
    pub cwd: Option<PathBuf>,
}

/// 上游配置加载器产生的错误类型的占位符。
#[derive(Debug, Clone, Default)]
pub struct ConfigLoadError;

impl std::fmt::Display for ConfigLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("config load error")
    }
}

impl std::error::Error for ConfigLoadError {}
