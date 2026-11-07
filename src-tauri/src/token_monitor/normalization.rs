//! 模型名规范化 + usage 语义标准化（03 §4 契约，W1 实现）。
//!
//! 硬约束：`total_tokens` 按各 Provider 的 usage 语义标准化，**不双算 cache**
//! （Anthropic：total = input + output，cache 单列保留；OpenAI：total = prompt + completion）。

use crate::token_monitor::model::NormalizedUsageEvent;

/// 已知模型别名 → 规范名（大小写不敏感）。
///
/// 合并同一底层模型在不同来源里的不同写法，避免统计中裂成多张卡片：
/// - Freebuff 前端把 Claude Opus 4.8 展示为 `claude-fable-5`（fable 为内部代号）；
/// - Claude Code 新版 jsonl 用 MAAS 端点名 `MaaS_Cl_Opus_4.8_<date>_cache` 报告同一模型。
/// 两者统一归一到 `claude-opus-4-8`（与 Claude Code 旧版 jsonl 的模型名一致）。
/// - DeepSeek Harness 渠道包把上游模型写成内部代号（`dfmodel` = DeepSeek-Flash 等），
///   代号会原样落进 DSH 会话文件，需归一到真实模型名。
pub fn alias_model(raw: &str) -> Option<&'static str> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower == "claude-fable-5" || lower == "fable-5" {
        return Some("claude-opus-4-8");
    }
    // 带日期/变体后缀的 MAAS 端点名（如 maas_cl_opus_4.8_20260528_cache）都归一。
    if lower.starts_with("maas_cl_opus_4.8") {
        return Some("claude-opus-4-8");
    }
    dsh_channel_pack_model(&lower)
}

/// DSH 渠道包内部模型代号 → 真实模型名（与 channel-pack 的 `id`/`name` 一一对应）。
///
/// 背景：`dsh-our-free-model` 插件的 `vendor/channel-pack/pack.js` 用代号向上游请求
/// （如 `dfmodel` 实为 DeepSeek-Flash），DSH 把该代号写进会话文件的
/// `request/header.config.model`，PoolGate 采集后 `model_normalized` 就是代号——
/// 用户看到「用的是 DeepSeek Flash，统计里却写着 dfmodel」。
///
/// 只映射**有稳定真实名**的代号；`cmodel`/`smodel` 这类厂商自造代号保留原名
/// （能力诚实：不猜测真实型号）。
fn dsh_channel_pack_model(lower: &str) -> Option<&'static str> {
    match lower {
        // Qoder 渠道（qoder / qodercn）
        "dfmodel" => Some("deepseek-flash"),
        "dmodel" => Some("deepseek-v4-pro"),
        "qfmodel" => Some("qwen3.8-flash"),
        "q37fmodel" => Some("qwen3.7-flash"),
        "qmodel" => Some("qwen3.7-plus"),
        "qmodel_latest" => Some("qwen3.7-max"),
        "qmodel_38max" => Some("qwen3.8-max"),
        "gmodel" => Some("glm-5.3"),
        "gfmodel" => Some("glm-5.3-flash"),
        "gm51model" => Some("glm-5.2"),
        "kmodel" => Some("kimi-k2.8-preview"),
        "kmodel_latest" => Some("kimi-k3"),
        "mmodel" => Some("minimax-m3"),
        _ => None,
    }
}

/// 归一模型名：去掉厂商模型名中的日期后缀等易变部分。
///
/// - "claude-3-5-sonnet-20241022" → "claude-3.5-sonnet"
/// - "gpt-4o-2024-08-06" → "gpt-4o"
/// - "gemini-1.5-pro-001" → "gemini-1.5-pro"
/// 未识别模式原样返回（能力诚实：不做有损猜测）。
pub fn normalize_model(raw: &str) -> String {
    if let Some(canonical) = alias_model(raw) {
        return canonical.to_string();
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // 家族前缀列表（按最长优先匹配）
    const FAMILIES: &[&str] = &[
        "claude-3-5-",
        "claude-3-7-",
        "claude-3-",
        "claude-sonnet-",
        "claude-opus-",
        "claude-haiku-",
        "claude-",
        "gpt-4o-mini-",
        "gpt-4o-",
        "gpt-4.1-",
        "gpt-4-",
        "gpt-5-",
        "o4-mini-",
        "o3-mini-",
        "o3-",
        "o1-mini-",
        "o1-",
        "gemini-2.5-",
        "gemini-2.0-",
        "gemini-1.5-",
        "gemini-",
        "deepseek-",
        "qwen-",
        "mistral-",
        "llama-",
        "grok-",
        "kimi-",
    ];

    for family in FAMILIES {
        if let Some(rest) = trimmed.strip_prefix(family) {
            // 去掉结尾的日期段（YYYYMMDD / YYYY-MM-DD / 纯数字版本段）
            let core = strip_date_suffix(rest);
            return format!("{family}{core}").trim_end_matches('-').to_string();
        }
    }
    trimmed.to_string()
}

/// 去掉末尾的日期段（"-20241022"、"-2024-08-06"、"-001"）。
/// 只剥离 `>= 3 位` 的纯数字段或完整日期形态；1–2 位的版本段（如 "claude-opus-4-8"
/// 的 "4"/"8"）保留，避免把用户可识别的模型版本号折没（与开源 Token Monitor 的
/// 原始模型名展示保持一致）。
fn strip_date_suffix(rest: &str) -> &str {
    // 完整日期形态（YYYY-MM-DD）整体剥离（逐段会因末段 "06" 长度不足而漏剥）
    let is_date10 = |s: &str| {
        s.len() == 10
            && s.chars()
                .enumerate()
                .all(|(i, c)| c.is_ascii_digit() || (c == '-' && (i == 4 || i == 7)))
    };
    if is_date10(rest) {
        return &rest[..0];
    }
    let mut end = rest.len();
    for part in rest.split('-').rev() {
        // 只剥离 >= 3 位的纯数字段；1–2 位版本段（"claude-opus-4-8" 的 "4"/"8"）保留
        let is_date_like = part.len() >= 3 && part.chars().all(|c| c.is_ascii_digit());
        if !is_date_like {
            break;
        }
        end = end.saturating_sub(part.len());
        if end > 0 && rest.as_bytes()[end - 1] == b'-' {
            end -= 1;
        }
    }
    &rest[..end]
}

/// 按 Provider 语义计算 total（不双算 cache），并回填 model_normalized。
///
/// - model_normalized 为空时由 model_raw 经 `normalize_model` 回填。
/// - total_tokens 为空时：Anthropic/OpenAI 均按 `input + output` 计算（cache 单列保留、
///   不并入 total）；无任何 token 数据时保持 `None`（UI 显示「不可用」）。
pub fn finalize(event: &mut NormalizedUsageEvent) {
    // 别名归一先行：即使 model_normalized 已由适配器直填（如 tokscale 保留原始模型名），
    // 也要把已知别名归一到规范名，避免同一底层模型裂成多张模型卡片。
    if let Some(normalized) = event.model_normalized.as_deref() {
        if let Some(canonical) = alias_model(normalized) {
            event.model_normalized = Some(canonical.to_string());
        }
    }
    if event.model_normalized.is_none() {
        if let Some(raw) = event.model_raw.clone() {
            event.model_normalized = Some(normalize_model(&raw));
        }
    }
    if event.total_tokens.is_none() {
        match (event.input_tokens, event.output_tokens) {
            (Some(input), Some(output)) => {
                event.total_tokens = Some(input + output);
            }
            (Some(input), None) => event.total_tokens = Some(input),
            (None, Some(output)) => event.total_tokens = Some(output),
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_monitor::model::{SourceType, UsageAccuracy};

    fn event(model_raw: Option<&str>) -> NormalizedUsageEvent {
        NormalizedUsageEvent {
            source_type: SourceType::LocalDiscovered,
            tool_id: "claude_code".into(),
            device_id: "local".into(),
            model_raw: model_raw.map(ToString::to_string),
            model_normalized: None,
            session_id: None,
            project_id: None,
            account_id: None,
            input_tokens: Some(10),
            output_tokens: Some(5),
            cache_read_tokens: Some(2),
            cache_write_tokens: Some(3),
            reasoning_tokens: None,
            message_count: None,
            session_started_at: None,
            session_last_active_at: None,
            total_tokens: None,
            cost_amount: None,
            cost_currency: None,
            usage_accuracy: UsageAccuracy::Exact,
            occurred_at: "2026-08-08T00:00:00Z".into(),
            source_locator_hash: None,
        }
    }

    #[test]
    fn normalize_known_families_strips_dates() {
        assert_eq!(
            normalize_model("claude-3-5-sonnet-20241022"),
            "claude-3-5-sonnet"
        );
        assert_eq!(
            normalize_model("claude-3-7-sonnet-20250219"),
            "claude-3-7-sonnet"
        );
        assert_eq!(normalize_model("gpt-4o-2024-08-06"), "gpt-4o");
        assert_eq!(normalize_model("gemini-1.5-pro-001"), "gemini-1.5-pro");
        assert_eq!(normalize_model("deepseek-chat"), "deepseek-chat");
        assert_eq!(
            normalize_model("unknown-custom-model"),
            "unknown-custom-model"
        );
        // 1–2 位版本段保留（对齐开源版模型名展示，如 claude-opus-4-8 / claude-sonnet-5）
        assert_eq!(normalize_model("claude-opus-4-8"), "claude-opus-4-8");
        assert_eq!(normalize_model("claude-sonnet-5"), "claude-sonnet-5");
        assert_eq!(normalize_model("gpt-5.6-sol"), "gpt-5.6-sol");
    }

    #[test]
    fn normalize_aliases_fable_and_maas_opus_to_canonical() {
        // Freebuff 前端展示名与 Claude Code MAAS 端点名都是 Claude Opus 4.8，
        // 必须归一到同一个规范名，避免同一模型裂成多张卡片。
        assert_eq!(normalize_model("claude-fable-5"), "claude-opus-4-8");
        assert_eq!(normalize_model("fable-5"), "claude-opus-4-8");
        assert_eq!(
            normalize_model("MaaS_Cl_Opus_4.8_20260528_cache"),
            "claude-opus-4-8"
        );
        assert_eq!(
            normalize_model("maas_cl_opus_4.8_20260528_cache"),
            "claude-opus-4-8"
        );
        // thinking 变体是不同模型，不能被别名吞掉
        assert_ne!(
            normalize_model("claude-opus-4-8-thinking"),
            "claude-opus-4-8"
        );
        assert_eq!(
            normalize_model("claude-opus-4-8-thinking"),
            "claude-opus-4-8-thinking"
        );
    }

    #[test]
    fn normalize_dsh_channel_pack_codenames_to_real_models() {
        // DSH 渠道包用内部代号请求上游，代号会原样写进会话文件并落库；
        // 必须归一到真实模型名，否则用户看到「dfmodel」而非 DeepSeek-Flash。
        assert_eq!(normalize_model("dfmodel"), "deepseek-flash");
        assert_eq!(normalize_model("dmodel"), "deepseek-v4-pro");
        assert_eq!(normalize_model("qfmodel"), "qwen3.8-flash");
        assert_eq!(normalize_model("gmodel"), "glm-5.3");
        assert_eq!(normalize_model("gfmodel"), "glm-5.3-flash");
        assert_eq!(normalize_model("kmodel_latest"), "kimi-k3");
        assert_eq!(normalize_model("mmodel"), "minimax-m3");
        // 大小写与空白同样归一（DB 里可能带空格）
        assert_eq!(normalize_model(" DFModel "), "deepseek-flash");
        // 未映射的自造代号保持原名（能力诚实，不猜测）
        assert_eq!(normalize_model("cmodel"), "cmodel");
    }

    #[test]
    fn finalize_aliases_dsh_codename_preset_model_normalized() {
        // tokscale/DSH 直填 model_normalized=dfmodel 时也要归一到真实模型名。
        let mut e = event(Some("dfmodel"));
        e.model_normalized = Some("dfmodel".into());
        finalize(&mut e);
        assert_eq!(e.model_normalized.as_deref(), Some("deepseek-flash"));
        // model_raw 保留代号，便于诊断上游来源
        assert_eq!(e.model_raw.as_deref(), Some("dfmodel"));
    }

    #[test]
    fn finalize_fills_model_and_total_without_double_counting_cache() {
        let mut e = event(Some("claude-3-5-sonnet-20241022"));
        finalize(&mut e);
        assert_eq!(e.model_normalized.as_deref(), Some("claude-3-5-sonnet"));
        // total = input + output，不含 cache（2+3）
        assert_eq!(e.total_tokens, Some(15));
    }

    #[test]
    fn finalize_aliases_preset_model_normalized() {
        // tokscale 直填 model_normalized 时，finalize 也要把别名归一（不能只依赖 normalize_model 路径）。
        let mut e = event(Some("MaaS_Cl_Opus_4.8_20260528_cache"));
        e.model_normalized = Some("maas_cl_opus_4.8_20260528_cache".into());
        finalize(&mut e);
        assert_eq!(e.model_normalized.as_deref(), Some("claude-opus-4-8"));
        // model_raw 保留原始值，便于诊断
        assert_eq!(
            e.model_raw.as_deref(),
            Some("MaaS_Cl_Opus_4.8_20260528_cache")
        );
    }

    #[test]
    fn finalize_keeps_none_total_when_no_token_data() {
        let mut e = event(None);
        e.input_tokens = None;
        e.output_tokens = None;
        finalize(&mut e);
        assert_eq!(e.total_tokens, None);
    }
}
