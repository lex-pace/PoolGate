//! 幂等指纹（03 §4 契约，W1 实现）。
//!
//! - 幂等：`source_fingerprint`（02 §5 规则）由 `fingerprint()` 统一生成，W3 各 Adapter
//!   不得自行实现；`usage_event.source_fingerprint` 上的 `UNIQUE` 约束兜底。
//!
//! 注：早期设计中的「跨面去重（本地事件 vs 网关 request_logs）」已随
//! 「网关与 Token Monitor 独立」决策移除 —— 网关流量只由网关仪表盘统计，
//! Token Monitor 只统计本地工具用量（usage_event），二者数据源分离、不重复，
//! 无需交叉去重。

use crate::token_monitor::model::NormalizedUsageEvent;

/// 生成幂等指纹（02 §5）。
///
/// ```text
/// fingerprint = sha256_hex(
///     tool_id | normalized_model | rounded_timestamp_to_sec |
///     input_tokens : output_tokens : cache_tokens |
///     session_id.unwrap_or("") | request_id_if_available.unwrap_or("")
/// )
/// ```
/// `NormalizedUsageEvent` 契约未含 `request_id` 字段，该段保留为空串（预留）。
pub fn fingerprint(event: &NormalizedUsageEvent) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(event.tool_id.as_bytes());
    hasher.update(b"|");
    hasher.update(event.model_normalized.as_deref().unwrap_or("").as_bytes());
    hasher.update(b"|");
    hasher.update(rounded_timestamp_to_sec(&event.occurred_at).as_bytes());
    hasher.update(b"|");
    hasher.update(event.input_tokens.unwrap_or(0).to_string().as_bytes());
    hasher.update(b":");
    hasher.update(event.output_tokens.unwrap_or(0).to_string().as_bytes());
    hasher.update(b":");
    let cache = event.cache_read_tokens.unwrap_or(0) + event.cache_write_tokens.unwrap_or(0);
    hasher.update(cache.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(event.session_id.as_deref().unwrap_or("").as_bytes());
    hasher.update(b"|");
    // request_id 预留段（契约无此字段，恒空）
    hasher.update(b"");
    // 同一工具的多个本地数据源可能复用 thread/seq（例如多个 Freebuff workspace）。
    // 有 locator 时纳入指纹，避免跨源事件被错误合并；旧事件 locator=None 的行为不变。
    hasher.update(b"|");
    hasher.update(
        event
            .source_locator_hash
            .as_deref()
            .unwrap_or("")
            .as_bytes(),
    );
    hex::encode(hasher.finalize())
}

/// 把 UTC ISO8601 时间截断到秒（"2026-08-08T00:00:00.123Z" → "2026-08-08T00:00:00"）。
/// 非预期格式原样返回（恒等、不 panic）。
fn rounded_timestamp_to_sec(iso: &str) -> String {
    let bytes = iso.as_bytes();
    if bytes.len() >= 19 && bytes[10] == b'T' && bytes[13] == b':' && bytes[16] == b':' {
        iso[..19].to_string()
    } else {
        iso.to_string()
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::token_monitor::model::{SourceType, UsageAccuracy};

    fn event(occurred_at: &str, input: i64, output: i64) -> NormalizedUsageEvent {
        NormalizedUsageEvent {
            source_type: SourceType::LocalDiscovered,
            tool_id: "claude_code".into(),
            device_id: "local".into(),
            model_raw: Some("claude-3-5-sonnet-20241022".into()),
            model_normalized: Some("claude-3.5-sonnet".into()),
            session_id: Some("sess-1".into()),
            project_id: None,
            account_id: None,
            input_tokens: Some(input),
            output_tokens: Some(output),
            cache_read_tokens: Some(1),
            cache_write_tokens: Some(2),
            reasoning_tokens: None,
            message_count: None,
            session_started_at: None,
            session_last_active_at: None,
            total_tokens: None,
            cost_amount: None,
            cost_currency: None,
            usage_accuracy: UsageAccuracy::Exact,
            occurred_at: occurred_at.into(),
            source_locator_hash: None,
        }
    }

    #[test]
    fn fingerprint_is_deterministic_and_sensitive_to_input() {
        let a = event("2026-08-08T00:00:00Z", 10, 5);
        let same = event("2026-08-08T00:00:00Z", 10, 5);
        let other_time = event("2026-08-08T00:00:01Z", 10, 5);
        let other_tokens = event("2026-08-08T00:00:00Z", 11, 5);
        assert_eq!(fingerprint(&a), fingerprint(&same));
        assert_ne!(fingerprint(&a), fingerprint(&other_time));
        assert_ne!(fingerprint(&a), fingerprint(&other_tokens));
    }

    #[test]
    fn fingerprint_ignores_fractional_seconds_and_cache_split() {
        // 同秒不同毫秒 → 同一指纹；cache 拆分不同但合计相同 → 同一指纹
        let a = event("2026-08-08T00:00:00.123Z", 10, 5);
        let b = event("2026-08-08T00:00:00.999Z", 10, 5);
        assert_eq!(fingerprint(&a), fingerprint(&b));
        let mut c = event("2026-08-08T00:00:00Z", 10, 5);
        c.cache_read_tokens = Some(3);
        c.cache_write_tokens = Some(0);
        assert_eq!(fingerprint(&a), fingerprint(&c));
    }
}
