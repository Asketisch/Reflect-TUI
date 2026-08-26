//! 状态界面中限流与 credits 的展示塑形。
//!
//! 本模块将 `RateLimitSnapshot` 协议载荷映射为面向展示的行，使 TUI 能够在
//! `/status` 与状态栏等场景中直接渲染，而无需重复格式化逻辑。
//!
//! 关键约定是：所有与时间相关的值都以调用方传入的捕获时间戳为参照来解读，
//! 以保证在同一次绘制周期中，陈旧检测与重置标签保持一致。
use crate::tui_core::chatwidget::fallback_limit_label;
use crate::tui_core::chatwidget::limit_label_for_window;
use crate::tui_core::text_formatting::capitalize_first;

use super::helpers::format_reset_timestamp;
use crate::app_server_protocol::CreditsSnapshot as CoreCreditsSnapshot;
use crate::app_server_protocol::RateLimitSnapshot;
use crate::app_server_protocol::RateLimitWindow;
use crate::app_server_protocol::SpendControlLimitSnapshot as CoreSpendControlLimitSnapshot;
use crate::protocol_compat::num_format::format_with_separators;
use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use chrono::Local;
use chrono::Utc;

const STATUS_LIMIT_BAR_SEGMENTS: usize = 20;
const STATUS_LIMIT_BAR_FILLED: &str = "█";
const STATUS_LIMIT_BAR_EMPTY: &str = "░";

#[derive(Debug, Clone)]
pub(crate) struct StatusRateLimitRow {
    /// 人类可读的行标签，例如 `"5h limit"`、`"Monthly limit"` 或 `"Credits"`。
    pub label: String,
    /// 该行的值载荷。
    pub value: StatusRateLimitValue,
}

/// 单条限速行的展示值变体。
#[derive(Debug, Clone)]
pub(crate) enum StatusRateLimitValue {
    /// 基于百分比的使用窗口，可附带重置时间戳文本。
    Window {
        /// 已使用窗口的百分比。
        percent_used: f64,
        /// 本地化的重置字符串，未知时为 `None`。
        resets_at: Option<String>,
        /// 渲染在进度条下方的可选详情行。
        details: Option<String>,
    },
    /// 用于非窗口行的纯文本值。
    Text(String),
}

/// 状态输出中限流数据的可用性状态。
#[derive(Debug, Clone)]
pub(crate) enum StatusRateLimitData {
    /// 快照数据足够新，可正常渲染。
    Available(Vec<StatusRateLimitRow>),
    /// 快照数据存在，但已超过陈旧阈值。
    Stale(Vec<StatusRateLimitRow>),
    /// 刷新已完成，但响应中未包含可展示的使用数据。
    Unavailable,
    /// 当前没有任何快照数据。
    Missing,
}

/// 状态输出中快照被视为陈旧的最大时长。
pub(crate) const RATE_LIMIT_STALE_THRESHOLD_MINUTES: i64 = 15;

/// 从快照派生的单个使用窗口的展示友好表示。
#[derive(Debug, Clone)]
pub(crate) struct RateLimitWindowDisplay {
    /// 该窗口已使用的百分比。
    pub used_percent: f64,
    /// 人类可读的本地重置时间。
    pub resets_at: Option<String>,
    /// 由服务端提供的窗口长度（分钟）。
    pub window_minutes: Option<i64>,
}

impl RateLimitWindowDisplay {
    fn from_window(window: &RateLimitWindow, captured_at: DateTime<Local>) -> Self {
        let resets_at_utc = window
            .resets_at
            .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0))
            .map(|dt| dt.with_timezone(&Local));
        let resets_at = resets_at_utc.map(|dt| format_reset_timestamp(dt, captured_at));

        Self {
            used_percent: f64::from(window.used_percent),
            resets_at,
            window_minutes: window.window_duration_mins,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RateLimitSnapshotDisplay {
    /// 规范的限额标识符（例如：`reflect` 或 `reflect_other`）。
    pub limit_name: String,
    /// 表示此展示快照捕获时刻的本地时间戳。
    pub captured_at: DateTime<Local>,
    /// 主要使用窗口。
    pub primary: Option<RateLimitWindowDisplay>,
    /// 次要使用窗口。
    pub secondary: Option<RateLimitWindowDisplay>,
    /// 可用时附带的 credits 元数据。
    pub credits: Option<CreditsSnapshotDisplay>,
    /// 来自工作区支出控制的可选有效月度 credit 限额。
    pub individual_limit: Option<SpendControlLimitSnapshotDisplay>,
}

/// 从协议快照中提取的、可直接展示的 credits 状态。
#[derive(Debug, Clone)]
pub(crate) struct CreditsSnapshotDisplay {
    /// 该账户是否启用了 credits 跟踪。
    pub has_credits: bool,
    /// 该账户是否拥有无限 credits。
    pub unlimited: bool,
    /// 后端提供的原始余额文本。
    pub balance: Option<String>,
}

/// 从支出控制中提取的、可直接展示的有效月度限额。
#[derive(Debug, Clone)]
pub(crate) struct SpendControlLimitSnapshotDisplay {
    /// 表示捕获此月度使用值时刻的本地时间戳。
    pub captured_at: DateTime<Local>,
    pub percent_remaining: f64,
    pub used: String,
    pub limit: String,
    pub resets_at: Option<String>,
}

/// 将协议快照转换为对 UI 友好的展示数据。
///
/// 请传入与 `snapshot` 来自同一观测点的时间戳；提供明显更早或更晚的
/// `captured_at` 可能导致具有误导性的重置标签和错误的陈旧分类。
#[cfg(test)]
pub(crate) fn rate_limit_snapshot_display(
    snapshot: &RateLimitSnapshot,
    captured_at: DateTime<Local>,
) -> RateLimitSnapshotDisplay {
    rate_limit_snapshot_display_for_limit(snapshot, "reflect".to_string(), captured_at)
}

pub(crate) fn rate_limit_snapshot_display_for_limit(
    snapshot: &RateLimitSnapshot,
    limit_name: String,
    captured_at: DateTime<Local>,
) -> RateLimitSnapshotDisplay {
    RateLimitSnapshotDisplay {
        limit_name,
        captured_at,
        primary: snapshot
            .primary
            .as_ref()
            .map(|window| RateLimitWindowDisplay::from_window(window, captured_at)),
        secondary: snapshot
            .secondary
            .as_ref()
            .map(|window| RateLimitWindowDisplay::from_window(window, captured_at)),
        credits: snapshot.credits.as_ref().map(CreditsSnapshotDisplay::from),
        individual_limit: snapshot
            .individual_limit
            .as_ref()
            .and_then(|limit| SpendControlLimitSnapshotDisplay::from_limit(limit, captured_at)),
    }
}

impl From<&CoreCreditsSnapshot> for CreditsSnapshotDisplay {
    fn from(value: &CoreCreditsSnapshot) -> Self {
        Self {
            has_credits: value.has_credits,
            unlimited: value.unlimited,
            balance: value.balance.clone(),
        }
    }
}

impl SpendControlLimitSnapshotDisplay {
    fn from_limit(
        value: &CoreSpendControlLimitSnapshot,
        captured_at: DateTime<Local>,
    ) -> Option<Self> {
        Some(Self {
            captured_at,
            percent_remaining: f64::from(value.remaining_percent.clamp(0, 100)),
            used: format_credit_amount(&value.used)?,
            limit: format_credit_amount(&value.limit)?,
            resets_at: DateTime::<Utc>::from_timestamp(value.resets_at, 0)
                .map(|dt| format_reset_timestamp(dt.with_timezone(&Local), captured_at)),
        })
    }
}

/// 从快照构建展示行，并根据捕获时长标记陈旧数据。
///
/// 调用方应在渲染时为 `now` 传入 `Local::now()`；使用缓存的时间戳可能使
/// 新鲜数据显得陈旧，或导致陈旧警告无法出现。
pub(crate) fn compose_rate_limit_data(
    snapshot: Option<&RateLimitSnapshotDisplay>,
    now: DateTime<Local>,
) -> StatusRateLimitData {
    match snapshot {
        Some(snapshot) => compose_rate_limit_data_many(std::slice::from_ref(snapshot), now),
        None => StatusRateLimitData::Missing,
    }
}

pub(crate) fn compose_rate_limit_data_many(
    snapshots: &[RateLimitSnapshotDisplay],
    now: DateTime<Local>,
) -> StatusRateLimitData {
    if snapshots.is_empty() {
        return StatusRateLimitData::Missing;
    }

    let mut rows = Vec::with_capacity(snapshots.len().saturating_mul(3));
    let mut stale = false;

    for snapshot in snapshots {
        stale |= now.signed_duration_since(snapshot.captured_at)
            > ChronoDuration::minutes(RATE_LIMIT_STALE_THRESHOLD_MINUTES);
        stale |= snapshot
            .individual_limit
            .as_ref()
            .map(|limit| {
                now.signed_duration_since(limit.captured_at)
                    > ChronoDuration::minutes(RATE_LIMIT_STALE_THRESHOLD_MINUTES)
            })
            .unwrap_or(false);

        let limit_bucket_label = snapshot.limit_name.clone();
        let show_limit_prefix = !limit_bucket_label.eq_ignore_ascii_case("reflect");
        let primary_label = snapshot
            .primary
            .as_ref()
            .map(|window| {
                limit_label_for_window(window.window_minutes, /*is_secondary*/ false)
            })
            .map(|label| capitalize_first(&label));
        let secondary_label = snapshot
            .secondary
            .as_ref()
            .map(|window| limit_label_for_window(window.window_minutes, /*is_secondary*/ true))
            .map(|label| capitalize_first(&label));
        let window_count =
            usize::from(snapshot.primary.is_some()) + usize::from(snapshot.secondary.is_some());
        let combine_non_reflect_single_limit = show_limit_prefix && window_count == 1;

        if show_limit_prefix && !combine_non_reflect_single_limit {
            rows.push(StatusRateLimitRow {
                label: format!("{limit_bucket_label} limit"),
                value: StatusRateLimitValue::Text(String::new()),
            });
        }

        if let Some(primary) = snapshot.primary.as_ref() {
            let label = if combine_non_reflect_single_limit {
                format!(
                    "{} {} limit",
                    limit_bucket_label,
                    primary_label.clone().unwrap_or_else(|| capitalize_first(
                        fallback_limit_label(/*is_secondary*/ false)
                    ))
                )
            } else {
                format!(
                    "{} limit",
                    primary_label.clone().unwrap_or_else(|| capitalize_first(
                        fallback_limit_label(/*is_secondary*/ false)
                    ))
                )
            };
            rows.push(StatusRateLimitRow {
                label,
                value: StatusRateLimitValue::Window {
                    percent_used: primary.used_percent,
                    resets_at: primary.resets_at.clone(),
                    details: None,
                },
            });
        }

        if let Some(secondary) = snapshot.secondary.as_ref() {
            let label = if combine_non_reflect_single_limit {
                format!(
                    "{} {} limit",
                    limit_bucket_label,
                    secondary_label.clone().unwrap_or_else(|| capitalize_first(
                        fallback_limit_label(/*is_secondary*/ true)
                    ))
                )
            } else {
                format!(
                    "{} limit",
                    secondary_label.clone().unwrap_or_else(|| capitalize_first(
                        fallback_limit_label(/*is_secondary*/ true)
                    ))
                )
            };
            rows.push(StatusRateLimitRow {
                label,
                value: StatusRateLimitValue::Window {
                    percent_used: secondary.used_percent,
                    resets_at: secondary.resets_at.clone(),
                    details: None,
                },
            });
        }

        if let Some(credits) = snapshot.credits.as_ref()
            && let Some(row) = credit_status_row(credits)
        {
            rows.push(row);
        }
        if let Some(individual_limit) = snapshot.individual_limit.as_ref() {
            rows.push(StatusRateLimitRow {
                label: "Monthly credit limit".to_string(),
                value: StatusRateLimitValue::Window {
                    percent_used: 100.0 - individual_limit.percent_remaining,
                    resets_at: individual_limit.resets_at.clone(),
                    details: Some(format!(
                        "{} of {} credits used",
                        individual_limit.used, individual_limit.limit
                    )),
                },
            });
        }
    }

    if rows.is_empty() {
        StatusRateLimitData::Unavailable
    } else if stale {
        StatusRateLimitData::Stale(rows)
    } else {
        StatusRateLimitData::Available(rows)
    }
}

/// 根据剩余百分比渲染固定宽度的进度条。
///
/// 本函数期望剩余值位于 `0..=100` 范围内，并对越界输入进行钳制。
/// 若误传已使用百分比，进度条会反转并误导用户。
pub(crate) fn render_status_limit_progress_bar(percent_remaining: f64) -> String {
    let ratio = (percent_remaining / 100.0).clamp(0.0, 1.0);
    let filled = (ratio * STATUS_LIMIT_BAR_SEGMENTS as f64).round() as usize;
    let filled = filled.min(STATUS_LIMIT_BAR_SEGMENTS);
    let empty = STATUS_LIMIT_BAR_SEGMENTS.saturating_sub(filled);
    format!(
        "[{}{}]",
        STATUS_LIMIT_BAR_FILLED.repeat(filled),
        STATUS_LIMIT_BAR_EMPTY.repeat(empty)
    )
}

/// 根据剩余百分比格式化紧凑的文本摘要。
pub(crate) fn format_status_limit_summary(percent_remaining: f64) -> String {
    format!("{percent_remaining:.0}% left")
}

/// 在工作区 credits 可用时构建单条 `StatusRateLimitRow`。
/// 无限 credits 会被显式标出；有限 credits 展示其四舍五入后的
/// 余额，余额被隐藏时则显示 `Available`。
fn credit_status_row(credits: &CreditsSnapshotDisplay) -> Option<StatusRateLimitRow> {
    if credits.unlimited {
        return Some(StatusRateLimitRow {
            label: "Credits".to_string(),
            value: StatusRateLimitValue::Text("Unlimited".to_string()),
        });
    }
    if !credits.has_credits {
        return None;
    }
    let value = credits
        .balance
        .as_deref()
        .and_then(format_credit_balance)
        .map_or_else(
            || "Available".to_string(),
            |display_balance| format!("{display_balance} credits"),
        );
    Some(StatusRateLimitRow {
        label: "Credits".to_string(),
        value: StatusRateLimitValue::Text(value),
    })
}

fn format_credit_balance(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(int_value) = trimmed.parse::<i64>()
        && int_value > 0
    {
        return Some(int_value.to_string());
    }

    if let Ok(value) = trimmed.parse::<f64>()
        && value.is_finite()
        && value > 0.0
    {
        let rounded = value.round() as i64;
        return Some(rounded.to_string());
    }

    None
}

fn format_credit_amount(raw: &str) -> Option<String> {
    let value = raw.trim().parse::<f64>().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some(format_with_separators(value.round() as i64))
}

#[cfg(test)]
mod tests {
    use super::CreditsSnapshotDisplay;
    use super::RateLimitSnapshotDisplay;
    use super::RateLimitWindowDisplay;
    use super::StatusRateLimitData;
    use super::compose_rate_limit_data_many;
    use chrono::Local;
    use pretty_assertions::assert_eq;

    fn window(used_percent: f64) -> RateLimitWindowDisplay {
        RateLimitWindowDisplay {
            used_percent,
            resets_at: Some("soon".to_string()),
            window_minutes: Some(300),
        }
    }

    #[test]
    fn non_reflect_single_limit_renders_combined_row() {
        let now = Local::now();
        let reflect = RateLimitSnapshotDisplay {
            limit_name: "reflect".to_string(),
            captured_at: now,
            primary: Some(window(/*used_percent*/ 10.0)),
            secondary: None,
            credits: Some(CreditsSnapshotDisplay {
                has_credits: true,
                unlimited: false,
                balance: Some("25".to_string()),
            }),
            individual_limit: None,
        };
        let other = RateLimitSnapshotDisplay {
            limit_name: "reflect-other".to_string(),
            captured_at: now,
            primary: Some(window(/*used_percent*/ 20.0)),
            secondary: None,
            credits: Some(CreditsSnapshotDisplay {
                has_credits: true,
                unlimited: false,
                balance: Some("99".to_string()),
            }),
            individual_limit: None,
        };

        let rows = match compose_rate_limit_data_many(&[reflect, other], now) {
            StatusRateLimitData::Available(rows) => rows,
            other => panic!("unexpected status: {other:?}"),
        };

        let labels: Vec<String> = rows.iter().map(|row| row.label.clone()).collect();
        assert_eq!(
            labels,
            vec![
                "5h limit".to_string(),
                "Credits".to_string(),
                "reflect-other 5h limit".to_string(),
                "Credits".to_string(),
            ]
        );
        assert_eq!(rows.iter().filter(|row| row.label == "Credits").count(), 2);
    }

    #[test]
    fn non_reflect_multi_limit_keeps_group_row() {
        let now = Local::now();
        let other = RateLimitSnapshotDisplay {
            limit_name: "reflect-other".to_string(),
            captured_at: now,
            primary: Some(RateLimitWindowDisplay {
                used_percent: 20.0,
                resets_at: Some("soon".to_string()),
                window_minutes: Some(60),
            }),
            secondary: Some(RateLimitWindowDisplay {
                used_percent: 40.0,
                resets_at: Some("later".to_string()),
                window_minutes: Some(2 * 60),
            }),
            credits: None,
            individual_limit: None,
        };

        let rows = match compose_rate_limit_data_many(&[other], now) {
            StatusRateLimitData::Available(rows) => rows,
            other => panic!("unexpected status: {other:?}"),
        };
        let labels: Vec<String> = rows.iter().map(|row| row.label.clone()).collect();
        assert_eq!(
            labels,
            vec![
                "reflect-other limit".to_string(),
                "Usage limit".to_string(),
                "Secondary usage limit".to_string(),
            ]
        );
    }
}
