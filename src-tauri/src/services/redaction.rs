const REDACTED: &str = "[REDACTED]";

/// Redact common credential forms without attempting to parse or re-emit the
/// original value. This is intentionally conservative and may hide extra text.
pub fn redact_sensitive(input: &str) -> String {
    let mut output = input.to_string();
    for marker in [
        "authorization:",
        "x-api-key:",
        "x-goog-api-key:",
        "api_key=",
        "api-key=",
        "access_token=",
        "refresh_token=",
        "token=",
        "key=",
    ] {
        output = redact_after_marker(&output, marker);
    }
    output = redact_bearer(&output);
    output
}

fn redact_after_marker(input: &str, marker: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let marker_lower = marker.to_ascii_lowercase();
    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(&marker_lower) {
        let start = cursor + relative;
        let value_start = start + marker.len();
        result.push_str(&input[cursor..value_start]);
        let value_end = input[value_start..]
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '&' | '#' | ',' | '"' | '\'' | '}' | ']')
            })
            .map(|offset| value_start + offset)
            .unwrap_or(input.len());
        result.push_str(REDACTED);
        cursor = value_end;
    }
    result.push_str(&input[cursor..]);
    result
}

fn redact_bearer(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let marker = "bearer ";
    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(marker) {
        let start = cursor + relative;
        let value_start = start + marker.len();
        result.push_str(&input[cursor..value_start]);
        let value_end = input[value_start..]
            .find(|character: char| {
                character.is_whitespace() || matches!(character, ',' | '"' | '\'' | '}' | ']')
            })
            .map(|offset| value_start + offset)
            .unwrap_or(input.len());
        result.push_str(REDACTED);
        cursor = value_end;
    }
    result.push_str(&input[cursor..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_query_keys_and_bearer_tokens() {
        let value = "request https://example.test/path?key=secret-123&alt=sse Authorization: Bearer abc.def";
        let redacted = redact_sensitive(value);
        assert!(!redacted.contains("secret-123"));
        assert!(!redacted.contains("abc.def"));
        assert!(redacted.contains("key=[REDACTED]&alt=sse"));
        assert!(redacted.contains("Bearer [REDACTED]"));
    }

    #[test]
    fn leaves_normal_diagnostics_readable() {
        assert_eq!(
            redact_sensitive("connection timed out after 15s"),
            "connection timed out after 15s"
        );
    }
}
