// SPDX-License-Identifier: AGPL-3.0-only
//! Text formatting shared by the list, status bar and dialogs. Matches
//! `prettyBytes` and `dateText` in `desktop/ui/app.js`.

/// `912 bytes`, `71.0 KB`, `130 KB`, `1.1 MB`: one decimal below 100.
pub fn pretty_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    value /= 1024.0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

/// Local date for the Date modified column; `—` when unknown.
pub fn date_text(unix_seconds: u64) -> String {
    if unix_seconds == 0 {
        return "—".into();
    }
    glib::DateTime::from_unix_local(unix_seconds as i64)
        .and_then(|d| d.format("%Y-%m-%d"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "—".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_the_web_interface() {
        assert_eq!(pretty_bytes(912), "912 bytes");
        assert_eq!(pretty_bytes(72_704), "71.0 KB");
        assert_eq!(pretty_bytes(133_120), "130 KB");
    }
}
