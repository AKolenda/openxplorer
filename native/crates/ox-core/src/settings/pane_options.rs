// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane's own options (PROP-010, PROP-011), as the context
//! menu of Dolphin's Information panel offers them: whether the pane
//! describes the item under the pointer, which fields it shows, whether
//! dates are condensed, and whether audio and video start by themselves.

use serde::Serialize;
use serde_json::Value;

/// How the details pane behaves. Stored as `detailsPaneOptions`, which
/// the Python app does not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailsPaneOptions {
    /// The pane describes the item under the pointer, and the selection
    /// again when the pointer leaves the items.
    pub follow_hover: bool,
    /// The names of the fields the user turned off, such as `Size`.
    pub hidden_fields: Vec<String>,
    /// Dates show the day only; off, they add the time.
    pub condensed_dates: bool,
    /// Audio and video start playing when previewed.
    pub auto_play: bool,
}

impl Default for DetailsPaneOptions {
    fn default() -> Self {
        Self {
            follow_hover: false,
            hidden_fields: Vec::new(),
            condensed_dates: true,
            auto_play: false,
        }
    }
}

impl DetailsPaneOptions {
    /// Whether these are the options of a new installation.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Whether the field `name` is shown.
    pub fn shows(&self, name: &str) -> bool {
        !self.hidden_fields.iter().any(|hidden| hidden == name)
    }

    /// The options stored in `value`; a missing or mistyped option keeps
    /// its default, and `None` when `value` is not an object.
    pub fn from_json(value: &Value) -> Option<Self> {
        let values = value.as_object()?;
        let defaults = Self::default();
        let flag = |key: &str, default: bool| values.get(key).and_then(Value::as_bool).unwrap_or(default);
        let hidden_fields = values
            .get("hiddenFields")
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            follow_hover: flag("followHover", defaults.follow_hover),
            hidden_fields,
            condensed_dates: flag("condensedDates", defaults.condensed_dates),
            auto_play: flag("autoPlay", defaults.auto_play),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// parity: PROP-010
    #[test]
    fn pane_options_round_trip_and_ignore_mistyped_values() {
        let options = DetailsPaneOptions {
            follow_hover: true,
            hidden_fields: vec!["Size".to_owned()],
            condensed_dates: false,
            auto_play: true,
        };
        let stored = serde_json::to_value(&options).expect("serialises");

        assert_eq!(DetailsPaneOptions::from_json(&stored), Some(options));
        let odd = DetailsPaneOptions::from_json(&json!({"followHover": "yes", "hiddenFields": [1, "Type"]}))
            .expect("an object");
        assert!(!odd.follow_hover && odd.condensed_dates);
        assert!(!odd.shows("Type") && odd.shows("Size"));
        assert_eq!(DetailsPaneOptions::from_json(&json!([])), None);
    }
}
