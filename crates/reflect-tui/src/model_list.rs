//! provider 模型清单的人读渲染(v1.5 模型发现接线)。
//!
//! 背景:submodule 的 `zhuye_llm::model_catalog` 提供了 provider 的
//! `/models` 清单端点解析 + 免费模型判定规则(`FreeModelRules`),而
//! `zhuye_config::fetch_catalog` 负责真正去拉清单。此前 TUI 的
//! `/model` 只回一句「display only」,**没有任何真实模型清单** —— 用户
//! 看不到 provider 实际支持哪些模型、哪些是免费的。
//!
//! 本模块只做纯格式化(无 IO、无全局状态),便于单测覆盖。

use zhuye_llm::model_catalog::{FreeModelRules, ModelInfo};

/// 把 provider 模型清单格式化成给人看的多行文本。
///
/// `current` 是当前正在用的模型 spec(如 `"openai/gpt-4o"`),命中时该行
/// 前缀 `→ ` 标记。免费判定三态:
/// - `Some(true)` → 标 `免费`;
/// - `Some(false)` → 不标;
/// - `None`(provider 未表态且规则未命中)→ 标 `免费?`(存疑)。
///
/// 清单为空时返回一句明确的「取不到」说明,而不是空串 —— 静默的空输出
/// 会让用户以为「这个 provider 没有模型」。
pub fn format_catalog(
    provider: &str,
    models: &[ModelInfo],
    rules: &FreeModelRules,
    current: Option<&str>,
) -> String {
    if models.is_empty() {
        return format!(
            "/model: provider '{provider}' 的模型清单为空(该 provider 可能不提供模型清单端点)"
        );
    }
    let mut out = format!(
        "/model: provider '{provider}' 可用模型(共 {} 个)",
        models.len()
    );
    for m in models {
        let mut marks: Vec<&str> = Vec::new();
        if let Some(c) = current
            && model_matches(c, &m.id)
        {
            marks.push("当前");
        }
        match rules.classify(m) {
            Some(true) => marks.push("免费"),
            Some(false) => {}
            None => marks.push("免费?"),
        }
        let ctx = match m.context_window {
            Some(w) => format!("ctx {}", format_context_window(w)),
            None => String::new(),
        };
        // 尾注 = 上下文窗口 + 标记,空段不占位。
        let mut tail_parts: Vec<String> = Vec::new();
        if !ctx.is_empty() {
            tail_parts.push(ctx);
        }
        if !marks.is_empty() {
            tail_parts.push(marks.join(" · "));
        }
        let tail = tail_parts.join(" · ");
        if tail.is_empty() {
            out.push_str(&format!("\n  {}", m.id));
        } else {
            out.push_str(&format!("\n  {:<28}  {}", m.id, tail));
        }
    }
    out
}

/// 判断当前模型 spec 是否指向清单里的这条模型。
///
/// spec 形如 `"provider/model"`,清单里的 id 不含 provider 前缀 —— 这里
/// 只比 model 部分(后一段),并容忍 spec 直接就是裸 id 的写法。
fn model_matches(current: &str, id: &str) -> bool {
    if current == id {
        return true;
    }
    let model_part = current.rsplit('/').next().unwrap_or(current);
    model_part == id
}

/// 上下文窗口的人读形式。
fn format_context_window(tokens: u32) -> String {
    if tokens >= 1000 {
        format!("{}k", tokens / 1000)
    } else {
        format!("{tokens}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str) -> ModelInfo {
        ModelInfo::new(id)
    }

    #[test]
    fn empty_catalog_says_so_explicitly() {
        let s = format_catalog("openai", &[], &FreeModelRules::default(), None);
        assert!(s.contains("模型清单为空"));
        assert!(!s.trim().is_empty());
    }

    #[test]
    fn free_models_are_marked() {
        let models = vec![info("gpt-4o"), info("gpt-4o-mini:free")];
        let s = format_catalog("openai", &models, &FreeModelRules::default(), None);
        // 免费的那条:命中 `:free` 模式 → 标「免费」。
        let free_line = s
            .lines()
            .find(|l| l.contains("gpt-4o-mini:free"))
            .expect("免费模型行应存在");
        assert!(free_line.contains("免费"));
        assert!(
            !free_line.contains("免费?"),
            "确定免费不该标存疑:{free_line}"
        );
        // 不免费的那条:provider 未表态 → 标「免费?」(存疑)。
        let paid_line = s
            .lines()
            .find(|l| l.contains("gpt-4o") && !l.contains("mini"))
            .expect("非免费模型行应存在");
        assert!(paid_line.contains("免费?"), "{paid_line}");
    }

    #[test]
    fn unknown_free_state_is_marked_as_uncertain() {
        let models = vec![info("some-model")];
        let s = format_catalog("openai", &models, &FreeModelRules::default(), None);
        assert!(s.contains("免费?"));
    }

    #[test]
    fn explicit_is_free_false_is_not_marked() {
        let mut m = info("paid-model");
        m.is_free = Some(false);
        let s = format_catalog("openai", &[m], &FreeModelRules::default(), None);
        // 明确不免费 → 不标「免费」,也不标「免费?」。
        assert!(!s.contains("免费"), "{s}");
    }

    #[test]
    fn current_model_is_highlighted() {
        let models = vec![info("gpt-4o"), info("gpt-4o-mini")];
        let s = format_catalog(
            "openai",
            &models,
            &FreeModelRules::default(),
            Some("openai/gpt-4o"),
        );
        let line = s
            .lines()
            .find(|l| l.contains("gpt-4o") && !l.contains("mini"))
            .unwrap();
        assert!(line.contains("当前"));
    }

    #[test]
    fn context_window_is_rendered() {
        let mut m = info("big-model");
        m.context_window = Some(200_000);
        let s = format_catalog("openai", &[m], &FreeModelRules::default(), None);
        assert!(s.contains("ctx 200k"));
    }

    #[test]
    fn always_free_whitelist_is_honored() {
        let rules = FreeModelRules::new(vec!["union-alpha".into()], vec![]);
        let s = format_catalog("openai", &[info("union-alpha")], &rules, None);
        assert!(s.contains("免费"));
    }

    #[test]
    fn model_matches_handles_bare_and_prefixed_specs() {
        assert!(model_matches("openai/gpt-4o", "gpt-4o"));
        assert!(model_matches("gpt-4o", "gpt-4o"));
        assert!(!model_matches("openai/gpt-4o", "gpt-4o-mini"));
    }
}
