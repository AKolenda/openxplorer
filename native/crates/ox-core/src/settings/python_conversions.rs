// SPDX-License-Identifier: AGPL-3.0-only
//! Python's `str()` and `int()` applied to the JSON values of a recent file.
//!
//! Ports the conversions `Settings.__init__` in `v2.0.0:desktop/core.py` makes on
//! each `recent` entry: `str(item['name'])` and
//! `max(0, int(item.get('size') or 0))`. Both applications must keep and
//! skip the same entries, so every JSON type converts the way it does in
//! Python, with one known exception: a count written as a string of
//! non-ASCII decimal digits, such as `"１２"`, which Python's `int()`
//! accepts and `parse_python_int` skips.

use serde_json::{Number, Value};

use crate::location::python_strip;

/// Python's `str(value)` for JSON scalars; lists, objects and fractional
/// numbers use their JSON text instead of Python's `repr`.
pub(super) fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        other => other.to_string(),
    }
}

/// Python's `max(0, int(value or 0))` for a field that may be missing;
/// `None` where `int()` would raise, which makes the reader skip the entry.
pub(super) fn python_count(value: Option<&Value>) -> Option<u64> {
    let Some(value) = value else {
        return Some(0);
    };
    match value {
        Value::Null | Value::Bool(false) => Some(0),
        Value::Bool(true) => Some(1),
        Value::Number(number) => Some(number_count(number)),
        Value::String(text) if text.is_empty() => Some(0),
        Value::String(text) => parse_python_int(text),
        Value::Array(items) => items.is_empty().then_some(0),
        Value::Object(fields) => fields.is_empty().then_some(0),
    }
}

/// Python's `max(0, int(number))`: a fraction is truncated toward zero, a
/// negative number becomes 0, and a huge one saturates at `u64::MAX`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "`as` truncates toward zero like int(), and max(0.0) plus saturation stand in for max(0, ...)"
)]
fn number_count(number: &Number) -> u64 {
    if let Some(count) = number.as_u64() {
        return count;
    }
    number.as_f64().map_or(0, |float| float.max(0.0) as u64)
}

/// Parses ASCII decimal digits like Python's `int(str)` (surrounding white
/// space, a sign, and single underscores between digits), clamped to
/// `0..=u64::MAX`.
///
/// Python also accepts every other Unicode decimal digit (`int("１２")` is
/// 12); such a count is skipped here. The Python app writes counts as JSON
/// numbers, so only a hand-edited file holds one, and the standard library
/// has no table of Unicode decimal digit values to read it with.
fn parse_python_int(text: &str) -> Option<u64> {
    let trimmed = python_strip(text);
    let (is_negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if !is_python_digit_string(digits) {
        return None;
    }
    if is_negative {
        return Some(0);
    }
    let value = digits
        .bytes()
        .filter(u8::is_ascii_digit)
        .fold(0_u64, |total, digit| {
            total.saturating_mul(10).saturating_add(u64::from(digit - b'0'))
        });
    Some(value)
}

/// Whether `digits` is ASCII digits in groups separated by single
/// underscores, as Python's `int()` accepts (`1_000`, not `1__0` or `_1`).
fn is_python_digit_string(digits: &str) -> bool {
    let is_digit_group = |group: &str| !group.is_empty() && group.bytes().all(|byte| byte.is_ascii_digit());
    !digits.is_empty() && digits.split('_').all(is_digit_group)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// parity: HOME-011
    #[test]
    fn text_counts_parse_like_python_int() {
        assert_eq!(parse_python_int(" +42 "), Some(42));
        assert_eq!(parse_python_int("-7"), Some(0));
        assert_eq!(parse_python_int("1__0"), None);
        assert_eq!(parse_python_int("_1"), None);
        assert_eq!(parse_python_int("99999999999999999999999"), Some(u64::MAX));
        assert_eq!(parse_python_int("abc"), None);
        // A known gap: Python's int() reads these as 12 and 123.
        assert_eq!(parse_python_int("１２"), None);
        assert_eq!(parse_python_int("١٢٣"), None);
    }

    /// parity: HOME-011
    #[test]
    fn json_values_count_like_python_int() {
        assert_eq!(python_count(None), Some(0));
        assert_eq!(python_count(Some(&json!(null))), Some(0));
        assert_eq!(python_count(Some(&json!(true))), Some(1));
        assert_eq!(python_count(Some(&json!(2.9))), Some(2));
        assert_eq!(python_count(Some(&json!(-5))), Some(0));
        assert_eq!(python_count(Some(&json!(""))), Some(0));
        assert_eq!(python_count(Some(&json!("12.5"))), None);
        assert_eq!(python_count(Some(&json!([]))), Some(0));
        assert_eq!(python_count(Some(&json!([1]))), None);
        assert_eq!(python_count(Some(&json!({"a": 1}))), None);
    }

    /// parity: HOME-011
    #[test]
    fn json_values_convert_like_python_str() {
        assert_eq!(python_str(&json!("a.txt")), "a.txt");
        assert_eq!(python_str(&json!(null)), "None");
        assert_eq!(python_str(&json!(true)), "True");
        assert_eq!(python_str(&json!(false)), "False");
        assert_eq!(python_str(&json!(12)), "12");
    }
}
