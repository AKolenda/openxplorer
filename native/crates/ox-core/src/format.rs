// SPDX-License-Identifier: AGPL-3.0-only
//! Text formatting shared by the list, status bar and dialogs.
//!
//! Ports `prettyBytes`, `dateText` and the Properties dialog's `timestamp`
//! from `desktop/ui/app.js`. Dates follow the user's `LC_TIME` locale the
//! way the web UI followed the browser locale: the order and separators of
//! the locale's numeric date, with a four-digit year and two-digit month
//! and day. See [`locale_pattern`] for how the pattern is found.

mod locale_pattern;

use glib::DateTime;

/// Shown in the Date modified column when a time is unknown.
const UNKNOWN_DATE: &str = "—";

/// Shown in the Properties dialog when a time is unknown.
const UNKNOWN_TIMESTAMP: &str = "Not provided";

/// Size units after bytes; values past 1024 TB stay in TB.
const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];

/// `912 bytes`, `71.0 KB`, `130 KB`, `1.1 MB`: one decimal below 100 and
/// none from 100 up, in powers of 1024.
///
/// Halves round up, like JavaScript's `toFixed` (`1280` bytes is `1.3 KB`),
/// and the arithmetic is exact, so every size gets the same text as in the
/// web interface.
pub fn pretty_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    // `unit` is the index into UNITS; the value is bytes / 1024^(unit + 1).
    let mut unit = 0;
    while unit + 1 < UNITS.len() && u128::from(bytes) >= 1024u128.pow(unit as u32 + 2) {
        unit += 1;
    }
    let divisor = 1024u128.pow(unit as u32 + 1);
    let bytes = u128::from(bytes);
    if bytes >= 100 * divisor {
        let whole = round_half_up(bytes, divisor);
        format!("{whole} {}", UNITS[unit])
    } else {
        let tenths = round_half_up(bytes * 10, divisor);
        format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[unit])
    }
}

/// `numerator / denominator` rounded to the nearest integer, halves up.
fn round_half_up(numerator: u128, denominator: u128) -> u128 {
    (2 * numerator + denominator) / (2 * denominator)
}

/// Local date for the Date modified column, for example `09/26/2026` in
/// the US, `26.09.2026` in Germany or `2026/09/26` in Japan; `—` when the
/// time is unknown (zero) or out of range.
pub fn date_text(unix_seconds: u64) -> String {
    local_time(unix_seconds)
        .and_then(|time| format_date(&time))
        .unwrap_or_else(|| UNKNOWN_DATE.to_string())
}

/// Local date and time for the Properties dialog's Created, Modified and
/// Accessed rows, for example `09/26/2026, 7:35:35 PM` in the US or
/// `26.09.2026, 19:35:35` in Germany; `Not provided` when the time is
/// unknown (zero) or out of range.
pub fn date_time_text(unix_seconds: u64) -> String {
    local_time(unix_seconds)
        .and_then(|time| format_date_time(&time))
        .unwrap_or_else(|| UNKNOWN_TIMESTAMP.to_string())
}

/// [`date_text`] for a time GIO already returned as a `DateTime`, in the
/// time zone it carries. `None` if GLib cannot format it.
pub fn format_date(time: &DateTime) -> Option<String> {
    let pattern = locale_pattern::date_pattern();
    time.format(&pattern).ok().map(String::from)
}

/// [`date_time_text`] for a time GIO already returned as a `DateTime`, in
/// the time zone it carries. `None` if GLib cannot format it.
pub fn format_date_time(time: &DateTime) -> Option<String> {
    let date = format_date(time)?;
    let pattern = locale_pattern::time_pattern();
    let clock = time.format(&pattern).ok()?;
    Some(format!("{date}, {clock}"))
}

/// The local time for a Unix timestamp; `None` for zero, which the file
/// listing uses for "unknown", and for times GLib cannot represent.
fn local_time(unix_seconds: u64) -> Option<DateTime> {
    if unix_seconds == 0 {
        return None;
    }
    let seconds = i64::try_from(unix_seconds).ok()?;
    DateTime::from_unix_local(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from the size examples in `desktop/ui/app.js::prettyBytes`.
    #[test]
    fn sizes_match_the_web_interface() {
        assert_eq!(pretty_bytes(0), "0 bytes");
        assert_eq!(pretty_bytes(1), "1 bytes");
        assert_eq!(pretty_bytes(912), "912 bytes");
        assert_eq!(pretty_bytes(1023), "1023 bytes");
        assert_eq!(pretty_bytes(1024), "1.0 KB");
        assert_eq!(pretty_bytes(72_704), "71.0 KB");
        assert_eq!(pretty_bytes(102_400), "100 KB");
        assert_eq!(pretty_bytes(133_120), "130 KB");
        assert_eq!(pretty_bytes(1_048_063), "1023 KB");
        assert_eq!(pretty_bytes(1_048_064), "1024 KB");
        assert_eq!(pretty_bytes(1_048_576), "1.0 MB");
        assert_eq!(pretty_bytes(104_805_376), "100.0 MB");
        assert_eq!(pretty_bytes(1 << 30), "1.0 GB");
        assert_eq!(pretty_bytes(1 << 40), "1.0 TB");
    }

    /// JavaScript's `toFixed` rounds exact halves up; Rust's formatter
    /// would round them to even. Values from running `prettyBytes` in Node.
    #[test]
    fn halves_round_up_like_to_fixed() {
        assert_eq!(pretty_bytes(1280), "1.3 KB");
        assert_eq!(pretty_bytes(1536), "1.5 KB");
        assert_eq!(pretty_bytes(102_912), "101 KB");
        assert_eq!(pretty_bytes(10_291), "10.0 KB");
        assert_eq!(pretty_bytes(10_292), "10.1 KB");
    }

    /// `prettyBytes` stops dividing at TB.
    #[test]
    fn sizes_past_a_petabyte_stay_in_terabytes() {
        assert_eq!(pretty_bytes(1 << 50), "1024 TB");
        assert_eq!(pretty_bytes(1 << 53), "8192 TB");
        assert_eq!(pretty_bytes(u64::MAX), "16777216 TB");
    }

    /// Ported from `desktop/ui/app.js::dateText` (`n ? … : '—'`) and
    /// `timestamp` (`value ? … : 'Not provided'`).
    #[test]
    fn unknown_times_use_the_web_placeholders() {
        assert_eq!(date_text(0), "—");
        assert_eq!(date_time_text(0), "Not provided");
        assert_eq!(date_text(u64::MAX), "—");
        assert_eq!(date_time_text(u64::MAX), "Not provided");
    }

    /// Without `setlocale` the process uses the C locale, whose `%x` is
    /// `%m/%d/%y`: the result is the US order with a four-digit year, as
    /// `toLocaleDateString` gives for `en-US`.
    #[test]
    fn dates_follow_the_c_locale_with_a_full_year() {
        let time = DateTime::from_utc(2026, 9, 6, 19, 5, 7.0).expect("valid date");
        assert_eq!(format_date(&time).as_deref(), Some("09/06/2026"));
        assert_eq!(format_date_time(&time).as_deref(), Some("09/06/2026, 19:05:07"));
    }

    #[test]
    fn local_dates_are_formatted() {
        let text = date_text(1_790_000_000);
        assert_eq!(text.len(), 10, "{text}");
        assert!(text.contains("2026"), "{text}");
        assert!(date_time_text(1_790_000_000).starts_with(&text));
    }
}
