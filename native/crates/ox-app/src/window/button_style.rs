// SPDX-License-Identifier: AGPL-3.0-only
//! The filled button looks of the skin, beside the flat buttons of the
//! bars.
//!
//! Ports `.primary`, `.danger` and the bordered secondary buttons of
//! `desktop/ui/style.css`, as `native/docs/ui-spec.md` §3.6 (E06, E07) and
//! §4.14 draw them; `resources/skin/base.css` and `dialogs.css` style the
//! classes.

/// How a button that stands on its own is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonStyle {
    /// The one main action of a surface, on the accent colour, such as
    /// "Save"; in a dialog, the button Enter presses.
    Accent,
    /// A secondary action with a thin border, such as Open or Try again.
    Bordered,
    /// The red button of a destructive question, such as "Move to Trash".
    Danger,
}

impl ButtonStyle {
    /// The CSS class that gives a button this look.
    pub(crate) const fn css_class(self) -> &'static str {
        match self {
            ButtonStyle::Accent => "accent",
            ButtonStyle::Bordered => "bordered",
            ButtonStyle::Danger => "danger",
        }
    }
}
