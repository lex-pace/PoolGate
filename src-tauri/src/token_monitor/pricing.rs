//! 价格表 + 费用估算（03 §4 契约，W1 实现）。
//!
//! 能力诚实：未知价格返回 `None`，UI 标注「未知价格」，**不猜不填 0**。
//! 价格按 USD / 1M tokens，缓存读取价 (cache_read) 与写入价 (cache_write) 单列。

/// 价格条目：$ / 1M tokens。`cache_read` 同时覆盖缓存读取（estimate_cost 只收单值）。
#[derive(Clone, Copy)]
struct PriceEntry {
    input: f64,
    output: f64,
    cache_read: Option<f64>,
}

/// 已知模型价格表（2026-08 基线，前缀匹配）。未收录 → None（不猜测）。
const PRICE_TABLE: &[(&str, PriceEntry)] = &[
    // Anthropic
    (
        "claude-sonnet-4",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: Some(0.30),
        },
    ),
    (
        "claude-3-7-sonnet",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: Some(0.30),
        },
    ),
    (
        "claude-3-5-sonnet",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: Some(0.30),
        },
    ),
    (
        "claude-3-5-haiku",
        PriceEntry {
            input: 0.80,
            output: 4.0,
            cache_read: Some(0.08),
        },
    ),
    (
        "claude-3-opus",
        PriceEntry {
            input: 15.0,
            output: 75.0,
            cache_read: Some(1.50),
        },
    ),
    (
        "claude-3-haiku",
        PriceEntry {
            input: 0.25,
            output: 1.25,
            cache_read: Some(0.03),
        },
    ),
    // Anthropic 归一名兜底（model_normalized 常剥掉版本号：claude-opus / claude-sonnet / claude-haiku）。
    // 最长前缀优先，故上面的精确项不受影响；仅在无精确匹配时命中这里。
    (
        "claude-opus",
        PriceEntry {
            input: 15.0,
            output: 75.0,
            cache_read: Some(1.50),
        },
    ),
    (
        "claude-sonnet",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: Some(0.30),
        },
    ),
    (
        "claude-haiku",
        PriceEntry {
            input: 0.80,
            output: 4.0,
            cache_read: Some(0.08),
        },
    ),
    (
        "claude",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: Some(0.30),
        },
    ),
    // OpenAI
    (
        "gpt-5",
        PriceEntry {
            input: 1.25,
            output: 10.0,
            cache_read: Some(0.125),
        },
    ),
    (
        "gpt-4o-mini",
        PriceEntry {
            input: 0.15,
            output: 0.60,
            cache_read: Some(0.075),
        },
    ),
    (
        "gpt-4o",
        PriceEntry {
            input: 2.50,
            output: 10.0,
            cache_read: Some(1.25),
        },
    ),
    (
        "gpt-4.1",
        PriceEntry {
            input: 2.0,
            output: 8.0,
            cache_read: Some(0.50),
        },
    ),
    (
        "gpt-4",
        PriceEntry {
            input: 30.0,
            output: 60.0,
            cache_read: None,
        },
    ),
    (
        "o4-mini",
        PriceEntry {
            input: 1.10,
            output: 4.40,
            cache_read: Some(0.55),
        },
    ),
    (
        "o3-mini",
        PriceEntry {
            input: 1.10,
            output: 4.40,
            cache_read: None,
        },
    ),
    (
        "o3",
        PriceEntry {
            input: 2.0,
            output: 8.0,
            cache_read: None,
        },
    ),
    (
        "o1-mini",
        PriceEntry {
            input: 1.10,
            output: 4.40,
            cache_read: None,
        },
    ),
    (
        "o1",
        PriceEntry {
            input: 15.0,
            output: 60.0,
            cache_read: None,
        },
    ),
    // DeepSeek
    (
        "deepseek-reasoner",
        PriceEntry {
            input: 0.55,
            output: 2.19,
            cache_read: Some(0.07),
        },
    ),
    (
        "deepseek-chat",
        PriceEntry {
            input: 0.27,
            output: 1.10,
            cache_read: Some(0.07),
        },
    ),
    // Google
    (
        "gemini-2.5-pro",
        PriceEntry {
            input: 1.25,
            output: 10.0,
            cache_read: Some(0.3125),
        },
    ),
    (
        "gemini-2.0-flash",
        PriceEntry {
            input: 0.10,
            output: 0.40,
            cache_read: Some(0.025),
        },
    ),
    (
        "gemini-1.5-pro",
        PriceEntry {
            input: 1.25,
            output: 5.0,
            cache_read: Some(0.3125),
        },
    ),
    (
        "gemini-1.5-flash",
        PriceEntry {
            input: 0.35,
            output: 1.05,
            cache_read: Some(0.0875),
        },
    ),
    // Qwen
    (
        "qwen-max",
        PriceEntry {
            input: 1.60,
            output: 6.40,
            cache_read: Some(0.16),
        },
    ),
    (
        "qwen-plus",
        PriceEntry {
            input: 0.80,
            output: 2.0,
            cache_read: Some(0.08),
        },
    ),
];

/// 估算费用（USD）；未知价格返回 `None`。
///
/// 前缀最长优先匹配；估算结果属 `derived` 精度（UI 标注，不当作官方计费）。
pub fn estimate_cost(model_normalized: &str, input: i64, output: i64, cache: i64) -> Option<f64> {
    let entry = PRICE_TABLE
        .iter()
        .filter(|(prefix, _)| model_normalized.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, entry)| entry)?;

    let input_m = input as f64 / 1_000_000.0;
    let output_m = output as f64 / 1_000_000.0;
    let cache_m = cache as f64 / 1_000_000.0;
    let mut cost = input_m * entry.input + output_m * entry.output;
    if cache_m > 0.0 {
        cost += cache_m * entry.cache_read.unwrap_or(0.0);
    }
    Some(cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_yields_estimate() {
        // 1M input + 1M output claude-3-5-sonnet = 3 + 15 = 18
        let cost = estimate_cost("claude-3-5-sonnet", 1_000_000, 1_000_000, 0);
        assert!((cost.unwrap() - 18.0).abs() < 0.001);
    }

    #[test]
    fn cache_read_counts_when_known() {
        // claude-3-5-sonnet: 1M cache_read = 0.30
        let base = estimate_cost("claude-3-5-sonnet", 1_000_000, 1_000_000, 0).unwrap();
        let with_cache =
            estimate_cost("claude-3-5-sonnet", 1_000_000, 1_000_000, 1_000_000).unwrap();
        assert!((with_cache - base - 0.30).abs() < 0.001);
    }

    #[test]
    fn unknown_model_yields_none() {
        assert_eq!(estimate_cost("totally-unknown-model", 1000, 500, 0), None);
    }

    #[test]
    fn normalized_claude_names_match_fallback() {
        // 归一名（剥掉版本号）也应命中：claude-opus / claude-sonnet
        // 1M input + 1M output opus = 15 + 75 = 90
        let opus = estimate_cost("claude-opus", 1_000_000, 1_000_000, 0).unwrap();
        assert!((opus - 90.0).abs() < 0.001);
        // sonnet = 3 + 15 = 18
        let sonnet = estimate_cost("claude-sonnet", 1_000_000, 1_000_000, 0).unwrap();
        assert!((sonnet - 18.0).abs() < 0.001);
        // 精确项仍优先于兜底：claude-3-5-haiku 用自己的价（0.80+4.0=4.8），非 claude-haiku 也非 claude
        let haiku = estimate_cost("claude-3-5-haiku", 1_000_000, 1_000_000, 0).unwrap();
        assert!((haiku - 4.8).abs() < 0.001);
    }

    #[test]
    fn prefix_match_prefers_longest() {
        // gpt-4o vs gpt-4o-mini 必须各自匹配
        let a = estimate_cost("gpt-4o-mini", 1_000_000, 1_000_000, 0).unwrap();
        let b = estimate_cost("gpt-4o", 1_000_000, 1_000_000, 0).unwrap();
        assert!(a < b, "mini should be cheaper than full");
    }
}
