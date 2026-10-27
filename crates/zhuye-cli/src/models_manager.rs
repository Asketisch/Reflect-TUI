pub fn bundled_models_response() -> String {
    "{}".to_string()
}
pub mod collaboration_mode_presets {
    use crate::protocol_compat::config_types::CollaborationModeMask;

    pub fn builtin_collaboration_mode_presets() -> Vec<CollaborationModeMask> {
        vec![]
    }
}
pub mod model_presets {
    pub const HIDE_GPT5_1_MIGRATION_PROMPT_CONFIG: bool = false;
    pub const HIDE_GPT_5_1_PRO_MAX_MIGRATION_PROMPT_CONFIG: bool = false;
}
pub mod test_support {
    pub fn construct_model_info_offline_for_tests() -> String {
        "{}".to_string()
    }
    pub fn get_model_offline_for_tests() -> String {
        "{}".to_string()
    }
}
