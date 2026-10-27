#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Default)]
pub enum Feature {
    #[default]
    Mcp,
    ShellCommand,
    Goals,
    Plugins,
    Personality,
    MentionsV2,
    FastMode,
    GuardianApproval,
    MemoryTool,
    PreventIdleSleep,
    WebSearch,
    Reflect,
    Apps,
    RemotePlugin,
    Collab,
}
#[derive(Debug, Clone, Default)]
pub struct Features;
impl Features {
    pub fn is_enabled(&self, _feature: Feature) -> bool {
        false
    }
    pub fn enabled(&self, _feature: Feature) -> bool {
        false
    }
    pub fn with_defaults() -> Self {
        Features
    }
}

impl From<&crate::tui_core::legacy_core::config::FeaturesConfig> for Features {
    fn from(_config: &crate::tui_core::legacy_core::config::FeaturesConfig) -> Self {
        Features
    }
}
pub const FEATURES: Features = Features;

impl Feature {
    pub fn key(&self) -> &'static str {
        match self {
            Feature::Mcp => "mcp",
            Feature::ShellCommand => "shell_command",
            Feature::Goals => "goals",
            Feature::Plugins => "plugins",
            Feature::Personality => "personality",
            Feature::MentionsV2 => "mentions_v2",
            Feature::FastMode => "fast_mode",
            Feature::GuardianApproval => "guardian_approval",
            Feature::MemoryTool => "memory_tool",
            Feature::PreventIdleSleep => "prevent_idle_sleep",
            Feature::WebSearch => "web_search",
            Feature::Reflect => "reflect",
            Feature::Apps => "apps",
            Feature::RemotePlugin => "remote_plugin",
            Feature::Collab => "collab",
        }
    }
}

/// 功能阶段（实验性、测试版、稳定版等）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExperimentalFeatureStage;

impl ExperimentalFeatureStage {
    pub fn experimental_menu_name(&self) -> Option<&'static str> {
        None
    }
    pub fn experimental_menu_description(&self) -> Option<&'static str> {
        None
    }
    pub fn experimental_announcement(&self) -> Option<&'static str> {
        None
    }
}

/// 菜单中显示的实验性功能规范。
#[derive(Debug, Clone, Copy, Default)]
pub struct ExperimentalFeatureSpec {
    pub id: Feature,
    pub stage: ExperimentalFeatureStage,
}

pub static FEATURE_SPECS: [ExperimentalFeatureSpec; 0] = [];

impl Features {
    pub fn iter(&self) -> impl Iterator<Item = &'static ExperimentalFeatureSpec> {
        FEATURE_SPECS.iter()
    }
}
