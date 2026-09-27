// SPDX-License-Identifier: AGPL-3.0-only
//! The fixed choices of the `theme`, `view` and `contextMenu` preferences,
//! and the [`Appearance`] a theme resolves to.
//!
//! Ports the whitelists in `update_preferences` (`desktop/core.py`) and the
//! light-or-dark decision of `applyTheme` (`desktop/ui/app.js`). Each
//! choice serialises to the exact string both applications store in
//! `settings.json`; [`from_key`](Theme::from_key) is the case-sensitive
//! check Python's `value in (...)` makes, so anything else is ignored.

use serde::Serialize;

/// The colour theme (`preferences.theme`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the desktop's light or dark style.
    #[default]
    System,
    /// Always the light palette.
    Light,
    /// Always the dark palette.
    Dark,
}

impl Theme {
    /// Every theme, in the order the Appearance menu lists them.
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];

    /// The value stored in `settings.json` and used as the action target.
    pub const fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    /// The theme stored as `key`, or `None` for anything else.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.as_str() == key)
    }

    /// The appearance this theme draws on a desktop whose own colour scheme
    /// is `desktop`: [`Theme::System`] follows the desktop, the others
    /// ignore it (`applyTheme` in `desktop/ui/app.js`).
    pub const fn appearance(self, desktop: Appearance) -> Appearance {
        match self {
            Theme::System => desktop,
            Theme::Light => Appearance::Light,
            Theme::Dark => Appearance::Dark,
        }
    }
}

/// The colours actually drawn: `data-theme` in `desktop/ui/app.js`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Appearance {
    /// Light surfaces with dark text.
    Light,
    /// Dark surfaces with light text.
    Dark,
}

/// How folders are shown (`preferences.view`).
///
/// The file keeps these two values so the Python app can read it; the
/// native app may offer finer icon sizes that all store [`View::Grid`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    /// The Details list with columns.
    #[default]
    Details,
    /// Icons in a grid.
    Grid,
}

impl View {
    /// Every view.
    pub const ALL: [View; 2] = [View::Details, View::Grid];

    /// The value stored in `settings.json`.
    pub const fn as_str(self) -> &'static str {
        match self {
            View::Details => "details",
            View::Grid => "grid",
        }
    }

    /// The view stored as `key`, or `None` for anything else.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.as_str() == key)
    }
}

/// The context menu style (`preferences.contextMenu`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextMenu {
    /// The classic, full Windows 10 menu.
    #[default]
    Win10,
    /// The compact Windows 11 menu.
    Win11,
}

impl ContextMenu {
    /// Every style.
    pub const ALL: [ContextMenu; 2] = [ContextMenu::Win10, ContextMenu::Win11];

    /// The value stored in `settings.json`.
    pub const fn as_str(self) -> &'static str {
        match self {
            ContextMenu::Win10 => "win10",
            ContextMenu::Win11 => "win11",
        }
    }

    /// The style stored as `key`, or `None` for anything else.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.as_str() == key)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;

    fn stored<T: Serialize>(choice: T) -> Value {
        serde_json::to_value(choice).expect("a choice serialises to a string")
    }

    /// parity: SET-016
    #[test]
    fn stored_values_match_the_keys() {
        for theme in Theme::ALL {
            assert_eq!(stored(theme), json!(theme.as_str()));
            assert_eq!(Theme::from_key(theme.as_str()), Some(theme));
        }
        for view in View::ALL {
            assert_eq!(stored(view), json!(view.as_str()));
            assert_eq!(View::from_key(view.as_str()), Some(view));
        }
        for style in ContextMenu::ALL {
            assert_eq!(stored(style), json!(style.as_str()));
            assert_eq!(ContextMenu::from_key(style.as_str()), Some(style));
        }
    }

    /// parity: SET-016
    #[test]
    fn keys_are_the_python_whitelists_and_case_sensitive() {
        assert_eq!(Theme::ALL.map(Theme::as_str), ["system", "light", "dark"]);
        assert_eq!(View::ALL.map(View::as_str), ["details", "grid"]);
        assert_eq!(ContextMenu::ALL.map(ContextMenu::as_str), ["win10", "win11"]);
        assert_eq!(Theme::from_key("Dark"), None);
        assert_eq!(View::from_key("large"), None);
        assert_eq!(ContextMenu::from_key(""), None);
    }

    /// parity: LOOK-003
    #[test]
    fn system_theme_follows_the_desktop() {
        assert_eq!(Theme::System.appearance(Appearance::Dark), Appearance::Dark);
        assert_eq!(Theme::System.appearance(Appearance::Light), Appearance::Light);
        assert_eq!(Theme::Light.appearance(Appearance::Dark), Appearance::Light);
        assert_eq!(Theme::Dark.appearance(Appearance::Light), Appearance::Dark);
    }
}
