//! reflect_otel crate 的桩。

#[derive(Debug, Clone, Default)]
pub struct SessionTelemetry;

#[derive(Debug, Clone, Default)]
pub struct TelemetryAuthMode;

#[derive(Debug, Clone, Default)]
pub struct RuntimeMetricTotals {
    pub count: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeMetricsSummary {
    pub api_calls: RuntimeMetricTotals,
    pub tool_calls: RuntimeMetricTotals,
    pub websocket_calls: RuntimeMetricTotals,
    pub websocket_events: RuntimeMetricTotals,
    pub streaming_events: RuntimeMetricTotals,
    pub responses_api_overhead_ms: f64,
    pub responses_api_inference_time_ms: f64,
    pub responses_api_engine_iapi_ttft_ms: f64,
    pub responses_api_engine_iapi_tbt_ms: f64,
    pub responses_api_engine_service_ttft_ms: f64,
    pub responses_api_engine_service_tbt_ms: f64,
}

impl SessionTelemetry {
    pub fn counter(&self, _name: &str, _inc: u64, _attrs: &[(&str, &str)]) -> u64 {
        0
    }
    pub fn reset_runtime_metrics(&mut self) {}
    pub fn runtime_metrics_summary(&self) -> RuntimeMetricsSummary {
        RuntimeMetricsSummary::default()
    }
}
impl RuntimeMetricsSummary {
    /// 当没有记录任何指标时为 true。
    ///
    /// `turn_runtime.rs` 根据 `(!is_empty()).then_some(...)` 来决定是否挂上
    /// `FinalMessageSeparator` 的 metrics 标签。一个恒返回 `false` 的桩
    /// 会每回合都附一个空/无意义的分隔符。因此本函数与 `separators.rs::runtime_metrics_label`
    /// (逐字段 `count > 0` 守卫) 及 `helpers.rs::has_websocket_timing_metrics`
    /// (针对 `responses_api_*` 计时的守卫) 的非空判定保持一致,
    /// 只有当这些字段都会被判为不渲染时,本函数才认为 summary 为空。
    pub fn is_empty(&self) -> bool {
        self.api_calls.count == 0
            && self.tool_calls.count == 0
            && self.websocket_calls.count == 0
            && self.websocket_events.count == 0
            && self.streaming_events.count == 0
            && self.responses_api_overhead_ms == 0.0
            && self.responses_api_inference_time_ms == 0.0
            && self.responses_api_engine_iapi_ttft_ms == 0.0
            && self.responses_api_engine_iapi_tbt_ms == 0.0
            && self.responses_api_engine_service_ttft_ms == 0.0
            && self.responses_api_engine_service_tbt_ms == 0.0
    }
    pub fn merge(&mut self, _other: &RuntimeMetricsSummary) {}
    pub fn responses_api_summary(&self) -> String {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_summary_is_empty() {
        // 默认(全零)summary 必须 is_empty == true,否则 turn 结束时
        // turn_runtime.rs:159 会误挂一个空 metrics 分隔符。
        assert!(RuntimeMetricsSummary::default().is_empty());
    }

    #[test]
    fn any_count_makes_summary_non_empty() {
        // 任意一个 count > 0 即应判定非空(对应 separators.rs 会渲染该字段)。
        let mut s = RuntimeMetricsSummary::default();
        s.tool_calls.count = 3;
        assert!(!s.is_empty());

        let mut s = RuntimeMetricsSummary::default();
        s.streaming_events.count = 1;
        assert!(!s.is_empty());
    }

    #[test]
    fn any_responses_api_timing_makes_summary_non_empty() {
        // 任意一个 responses_api_* 计时字段 > 0 即应判定非空
        // (对应 helpers.rs::has_websocket_timing_metrics 的判定集)。
        let mut s = RuntimeMetricsSummary::default();
        s.responses_api_overhead_ms = 1.5;
        assert!(!s.is_empty());

        let mut s = RuntimeMetricsSummary::default();
        s.responses_api_engine_service_tbt_ms = 0.1;
        assert!(!s.is_empty());
    }
}
