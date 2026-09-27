// SPDX-License-Identifier: AGPL-3.0-only
//! Text formatting shared by the list, status bar and dialogs.
//!
//! Ports `prettyBytes`, `dateText` and the Properties dialog's `timestamp`
//! from `desktop/ui/app.js`. Dates follow the user's `LC_TIME` locale the
//! way the web UI followed the browser locale: the order and separators of
//! the locale's numeric date, with a four-digit year and two-digit month
//! and day. The `locale_pattern` submodule explains how the pattern is found.

mod locale_pattern;

use glib::DateTime;

/// Shown in the Date modified column when a time is unknown.
const UNKNOWN_DATE: &str = "—";

/// Shown in the Properties dialog when a time is unknown.
const UNKNOWN_TIMESTAMP: &str = "Not provided";

/// A size unit and the number of bytes in one of it.
struct SizeUnit {
    name: &'static str,
    bytes: u128,
}

/// The units after bytes, in powers of 1024, smallest first. Sizes past
/// 1024 TB stay in TB, as in `prettyBytes`.
const SIZE_UNITS: [SizeUnit; 4] = [
    SizeUnit {
        name: "KB",
        bytes: 1 << 10,
    },
    SizeUnit {
        name: "MB",
        bytes: 1 << 20,
    },
    SizeUnit {
        name: "GB",
        bytes: 1 << 30,
    },
    SizeUnit {
        name: "TB",
        bytes: 1 << 40,
    },
];

/// `912 bytes`, `71.0 KB`, `130 KB`, `1.1 MB`: one decimal below 100 and
/// none from 100 up, in powers of 1024.
///
/// Halves round up, like JavaScript's `toFixed` (`1280` bytes is `1.3 KB`),
/// and the arithmetic is exact, so every size gets the same text as in the
/// web interface.
pub fn pretty_bytes(bytes: u64) -> String {
    // u128 keeps `bytes * 10` and the rounding exact for every u64 size.
    let bytes = u128::from(bytes);
    let Some(unit) = SIZE_UNITS.iter().rev().find(|unit| bytes >= unit.bytes) else {
        return format!("{bytes} bytes");
    };
    if bytes >= 100 * unit.bytes {
        let whole = round_half_up(bytes, unit.bytes);
        format!("{whole} {}", unit.name)
    } else {
        let tenths = round_half_up(bytes * 10, unit.bytes);
        format!("{}.{} {}", tenths / 10, tenths % 10, unit.name)
    }
}

/// `numerator / denominator` rounded to the nearest integer, halves up.
fn round_half_up(numerator: u128, denominator: u128) -> u128 {
    (2 * numerator + denominator) / (2 * denominator)
}

/// Local date for the Date modified column, for example `09/26/2026` in
/// the US, `26.09.2026` in Germany or `2026/09/26` in Japan; `—` when the
/// time is unknown (`None`) or out of range.
pub fn date_text(unix_seconds: Option<u64>) -> String {
    unix_seconds
        .and_then(local_time)
        .and_then(|time| format_date(&time))
        .unwrap_or_else(|| UNKNOWN_DATE.to_owned())
}

/// Local date and time for the Properties dialog's Created, Modified and
/// Accessed rows, for example `09/26/2026, 7:35:35 PM` in the US or
/// `26.09.2026, 19:35:35` in Germany; `Not provided` when the time is
/// unknown (`None`) or out of range.
pub fn date_time_text(unix_seconds: Option<u64>) -> String {
    unix_seconds
        .and_then(local_time)
        .and_then(|time| format_date_time(&time))
        .unwrap_or_else(|| UNKNOWN_TIMESTAMP.to_owned())
}

/// [`date_text`] for a time GIO already returned as a [`DateTime`], in the
/// time zone it carries. `None` if [`DateTime::format`] fails.
pub fn format_date(time: &DateTime) -> Option<String> {
    let pattern = locale_pattern::date_pattern();
    time.format(&pattern).ok().map(String::from)
}

/// [`date_time_text`] for a time GIO already returned as a [`DateTime`], in
/// the time zone it carries. `None` if [`DateTime::format`] fails.
pub fn format_date_time(time: &DateTime) -> Option<String> {
    let date = format_date(time)?;
    let pattern = locale_pattern::time_pattern();
    let clock = time.format(&pattern).ok()?;
    Some(format!("{date}, {clock}"))
}

/// The local time for a Unix timestamp; `None` for times a [`DateTime`]
/// cannot hold.
fn local_time(unix_seconds: u64) -> Option<DateTime> {
    let seconds = i64::try_from(unix_seconds).ok()?;
    DateTime::from_unix_local(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from the size examples in `desktop/ui/app.js::prettyBytes`.
    ///
    /// parity: VIEW-003
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
    ///
    /// parity: VIEW-003
    #[test]
    fn halves_round_up_like_to_fixed() {
        assert_eq!(pretty_bytes(1280), "1.3 KB");
        assert_eq!(pretty_bytes(1536), "1.5 KB");
        assert_eq!(pretty_bytes(102_912), "101 KB");
        assert_eq!(pretty_bytes(10_291), "10.0 KB");
        assert_eq!(pretty_bytes(10_292), "10.1 KB");
    }

    /// `prettyBytes` stops dividing at TB.
    ///
    /// parity: VIEW-003
    #[test]
    fn sizes_past_a_petabyte_stay_in_terabytes() {
        assert_eq!(pretty_bytes(1 << 50), "1024 TB");
        assert_eq!(pretty_bytes(1 << 53), "8192 TB");
        assert_eq!(pretty_bytes(u64::MAX), "16777216 TB");
    }

    /// Ported from `desktop/ui/app.js::dateText` (`n ? … : '—'`) and
    /// `timestamp` (`value ? … : 'Not provided'`). The web interface got 0
    /// for an unknown time; here it is `None`, and the entry module reads a
    /// reported 0 as `None` too.
    ///
    /// parity: VIEW-001
    #[test]
    fn unknown_times_use_the_web_placeholders() {
        assert_eq!(date_text(None), "—");
        assert_eq!(date_time_text(None), "Not provided");
        assert_eq!(date_text(Some(u64::MAX)), "—");
        assert_eq!(date_time_text(Some(u64::MAX)), "Not provided");
    }

    /// Without `setlocale` the process uses the C locale, whose `%x` is
    /// `%m/%d/%y`: the result is the US order with a four-digit year, as
    /// `toLocaleDateString` gives for `en-US`.
    ///
    /// parity: LOOK-026
    #[test]
    fn dates_follow_the_c_locale_with_a_full_year() {
        let time = DateTime::from_utc(2026, 9, 6, 19, 5, 7.0).expect("valid date");
        assert_eq!(format_date(&time).as_deref(), Some("09/06/2026"));
        assert_eq!(format_date_time(&time).as_deref(), Some("09/06/2026, 19:05:07"));
    }

    /// A known Unix time, in the process's own time zone: the Date modified
    /// text is a ten-character date, and the Properties timestamp starts
    /// with that same date.
    ///
    /// parity: LOOK-026
    #[test]
    fn date_time_text_starts_with_the_ten_character_local_date() {
        // 2026-09-21 14:13:20 UTC, still in 2026 in every time zone.
        let modified = Some(1_790_000_000);
        let text = date_text(modified);
        assert_eq!(text.len(), 10, "{text}");
        assert!(text.contains("2026"), "{text}");
        assert!(date_time_text(modified).starts_with(&text));
    }
}
