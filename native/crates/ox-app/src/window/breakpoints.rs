// SPDX-License-Identifier: AGPL-3.0-only
//! The narrow-window layouts.
//!
//! Ports the `@media(max-width: 1190px / 1050px / 960px / 680px)` rules of
//! `desktop/ui/style.css`. [`WindowWidth`] names the band the window's
//! width falls in; the window carries a CSS class for every limit it is
//! within (`max-1190` and so on), so `resources/skin/breakpoints.css`
//! narrows paddings and widths, and [`BrowserWindow::fit_to_width`] hides
//! what CSS cannot:
//! the details pane below 961 pixels, and the search box, some commands,
//! two columns and the build text below 681. Hiding the details pane this
//! way leaves the saved preference alone, so it comes back when the window
//! is wider again. The 1190-pixel column and sidebar widths of the web
//! stylesheet are overridden there by later `!important` rules, so they
//! are not ported.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::details::DetailsColumns;

use super::details_pane::PANE_WIDTH;
use super::tab_layout::TAB_WIDTH;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The details pane's width from 1190 pixels down (`.details{width:235px}`).
const NARROW_PANE_WIDTH: i32 = 235;
/// A tab's width from 960 pixels down (`.tab{width:180px}`).
const NARROW_TAB_WIDTH: i32 = 180;
/// A tab's width from 680 pixels down (`.tab{width:150px}`).
const COMPACT_TAB_WIDTH: i32 = 150;

/// The band of widths the window is in, narrowest last.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WindowWidth {
    /// Wider than 1190 pixels: the full layout.
    #[default]
    Wide,
    /// 1051 to 1190 pixels: a narrower details pane and search box.
    Medium,
    /// 961 to 1050 pixels: the appearance button also loses its label.
    Reduced,
    /// 681 to 960 pixels: no details pane, narrower tabs.
    Narrow,
    /// 680 pixels or less: no search box, fewer commands and columns.
    Compact,
}

/// One `@media(max-width)` rule of the web stylesheet.
#[derive(Debug, Clone, Copy)]
struct WidthLimit {
    /// The widest window the rule applies to, in pixels.
    max_width: i32,
    /// The band that starts at this limit.
    band: WindowWidth,
    /// The CSS class of a window within the limit.
    css_class: &'static str,
}

/// Every width limit, widest first.
const LIMITS: [WidthLimit; 4] = [
    WidthLimit {
        max_width: 1190,
        band: WindowWidth::Medium,
        css_class: "max-1190",
    },
    WidthLimit {
        max_width: 1050,
        band: WindowWidth::Reduced,
        css_class: "max-1050",
    },
    WidthLimit {
        max_width: 960,
        band: WindowWidth::Narrow,
        css_class: "max-960",
    },
    WidthLimit {
        max_width: 680,
        band: WindowWidth::Compact,
        css_class: "max-680",
    },
];

impl WindowWidth {
    /// The band `width` pixels fall in.
    pub(super) fn for_width(width: i32) -> Self {
        // The narrowest limit the width is within decides.
        let narrowest_within = LIMITS.iter().rev().find(|limit| width <= limit.max_width);
        narrowest_within.map_or(WindowWidth::Wide, |limit| limit.band)
    }

    /// The CSS classes of a window in this band: one per limit it is in.
    pub(super) fn css_classes(self) -> Vec<&'static str> {
        LIMITS
            .iter()
            .filter(|limit| self >= limit.band)
            .map(|limit| limit.css_class)
            .collect()
    }

    /// Whether there is room for the details pane (`.details{display:none}`
    /// at 960 pixels).
    pub(super) fn has_room_for_details(self) -> bool {
        self < WindowWidth::Narrow
    }

    /// Whether the appearance button shows its "Light" or "Dark" label
    /// (`.appearance-label{display:none}` at 1050 pixels).
    pub(super) fn shows_appearance_label(self) -> bool {
        self < WindowWidth::Reduced
    }

    /// Whether the window is at its most compact (the 680-pixel rules).
    pub(super) fn is_compact(self) -> bool {
        self == WindowWidth::Compact
    }

    /// The details pane's width: [`PANE_WIDTH`], narrower from 1190 pixels.
    pub(super) fn details_pane_width(self) -> i32 {
        if self >= WindowWidth::Medium {
            NARROW_PANE_WIDTH
        } else {
            PANE_WIDTH
        }
    }

    /// A tab's width when there is room: [`TAB_WIDTH`], narrower from 960
    /// and again from 680 pixels.
    pub(super) fn tab_width(self) -> i32 {
        match self {
            WindowWidth::Wide | WindowWidth::Medium | WindowWidth::Reduced => TAB_WIDTH,
            WindowWidth::Narrow => NARROW_TAB_WIDTH,
            WindowWidth::Compact => COMPACT_TAB_WIDTH,
        }
    }
}

impl BrowserWindow {
    /// Follows the window's width as GTK allocates it. The layout changes
    /// after the allocation, because changing styles during one would
    /// start it again.
    pub(super) fn follow_width(&self, width: i32) {
        let band = WindowWidth::for_width(width);
        if self.imp().window_width.replace(band) == band {
            return;
        }
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.fit_to_width(band)
        ));
    }

    /// Lays the window out for `band`.
    fn fit_to_width(&self, band: WindowWidth) {
        for limit in LIMITS {
            self.remove_css_class(limit.css_class);
        }
        for class in band.css_classes() {
            self.add_css_class(class);
        }
        self.fit_details_pane();
        self.details_pane().set_width(band.details_pane_width());
        let compact = band.is_compact();
        self.tab_strip().set_tab_width(band.tab_width());
        self.search_box().set_visible(!compact);
        self.command_bar().fit_to_width(band);
        self.status_bar().set_build_visible(!compact);
        let details_columns = if compact {
            DetailsColumns::NameAndSize
        } else {
            DetailsColumns::All
        };
        self.folder_pane().details().show_columns(details_columns);
        self.render_landing();
    }

    /// The width band the window is in now.
    pub(super) fn window_width(&self) -> WindowWidth {
        self.imp().window_width.get()
    }

    /// Shows the details pane while `win.details-pane` is on and the window
    /// has room for it. Hiding it for lack of room leaves the action, and
    /// so the saved preference, as it is.
    pub(super) fn fit_details_pane(&self) {
        let switched_on = self
            .window_action_state(WindowAction::DetailsPane)
            .and_then(|state| state.get::<bool>())
            .unwrap_or(true);
        let room = self.window_width().has_room_for_details();
        self.details_pane().set_visible(switched_on && room);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window width and the layout it gets.
    struct WidthCase {
        width: i32,
        band: WindowWidth,
        classes: &'static [&'static str],
    }

    #[test]
    fn each_width_limit_of_the_web_stylesheet_starts_a_band() {
        let cases = [
            WidthCase {
                width: 1440,
                band: WindowWidth::Wide,
                classes: &[],
            },
            WidthCase {
                width: 1190,
                band: WindowWidth::Medium,
                classes: &["max-1190"],
            },
            WidthCase {
                width: 1000,
                band: WindowWidth::Reduced,
                classes: &["max-1190", "max-1050"],
            },
            WidthCase {
                width: 960,
                band: WindowWidth::Narrow,
                classes: &["max-1190", "max-1050", "max-960"],
            },
            WidthCase {
                width: 600,
                band: WindowWidth::Compact,
                classes: &["max-1190", "max-1050", "max-960", "max-680"],
            },
        ];
        for case in cases {
            let band = WindowWidth::for_width(case.width);
            assert_eq!(band, case.band, "{} pixels", case.width);
            assert_eq!(band.css_classes(), case.classes, "{} pixels", case.width);
        }
    }

    #[test]
    fn narrow_windows_drop_the_details_pane_and_narrow_the_tabs() {
        assert!(WindowWidth::Reduced.has_room_for_details());
        assert!(!WindowWidth::Narrow.has_room_for_details());
        assert_eq!(WindowWidth::Wide.details_pane_width(), 262);
        assert_eq!(WindowWidth::Medium.details_pane_width(), 235);
        assert_eq!(WindowWidth::Narrow.tab_width(), 180);
        assert_eq!(WindowWidth::Compact.tab_width(), 150);
        assert!(!WindowWidth::Reduced.shows_appearance_label());
    }
}
