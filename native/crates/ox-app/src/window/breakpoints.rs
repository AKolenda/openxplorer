// SPDX-License-Identifier: AGPL-3.0-only
//! The narrow-window layouts.
//!
//! Ports the `@media(max-width: 1190px / 1050px / 960px / 680px)` rules of
//! `desktop/ui/style.css`. [`WindowWidth`] names the band the window's
//! width falls in; the window carries a CSS class for every limit it is
//! within (`max-1190` and so on), so `resources/style.css` narrows paddings
//! and widths, and [`BrowserWindow::fit_to_width`] hides what CSS cannot:
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

use super::BrowserWindow;

/// The band of widths the window is in, narrowest last.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WindowWidth {
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

/// Every width limit, with the CSS class of windows within it.
const LIMITS: [(i32, WindowWidth, &str); 4] = [
    (1190, WindowWidth::Medium, "max-1190"),
    (1050, WindowWidth::Reduced, "max-1050"),
    (960, WindowWidth::Narrow, "max-960"),
    (680, WindowWidth::Compact, "max-680"),
];

impl WindowWidth {
    /// The band `width` pixels fall in.
    pub fn for_width(width: i32) -> Self {
        // The narrowest limit the width is within decides.
        let narrowest_first = LIMITS.iter().rev();
        narrowest_first
            .map(|(limit, band, _)| (*limit, *band))
            .find(|(limit, _)| width <= *limit)
            .map_or(WindowWidth::Wide, |(_, band)| band)
    }

    /// The CSS classes of a window in this band: one per limit it is in.
    pub fn css_classes(self) -> Vec<&'static str> {
        LIMITS
            .iter()
            .filter(|(_, band, _)| self >= *band)
            .map(|(_, _, class)| *class)
            .collect()
    }

    /// Whether there is room for the details pane (`.details{display:none}`
    /// at 960 pixels).
    pub fn has_room_for_details(self) -> bool {
        self < WindowWidth::Narrow
    }

    /// Whether the appearance button shows its "Light" or "Dark" label
    /// (`.appearance-label{display:none}` at 1050 pixels).
    pub fn shows_appearance_label(self) -> bool {
        self < WindowWidth::Reduced
    }

    /// Whether the window is at its most compact (the 680-pixel rules).
    pub fn is_compact(self) -> bool {
        self == WindowWidth::Compact
    }

    /// The details pane's width (`.details{width:235px}` at 1190 pixels).
    pub fn details_pane_width(self) -> i32 {
        if self >= WindowWidth::Medium {
            235
        } else {
            262
        }
    }

    /// A tab's width when there is room (`.tab{width:180px}` at 960
    /// pixels, 150 at 680).
    pub fn tab_width(self) -> i32 {
        match self {
            WindowWidth::Wide | WindowWidth::Medium | WindowWidth::Reduced => 215,
            WindowWidth::Narrow => 180,
            WindowWidth::Compact => 150,
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
        for (_, _, class) in LIMITS {
            self.remove_css_class(class);
        }
        for class in band.css_classes() {
            self.add_css_class(class);
        }
        self.show_details_pane_if_room();
        self.details_pane().set_width(band.details_pane_width());
        let compact = band.is_compact();
        let chrome = self.chrome();
        chrome.tabs.set_tab_width(band.tab_width());
        chrome.search.root.set_visible(!compact);
        chrome.commands.fit_to_width(band);
        chrome.status.show_build(!compact);
        let details_columns = if compact {
            DetailsColumns::NameAndSize
        } else {
            DetailsColumns::All
        };
        self.content().details.show_columns(details_columns);
        self.render_landing();
    }

    /// The width band the window is in now.
    pub(super) fn window_width(&self) -> WindowWidth {
        self.imp().window_width.get()
    }

    /// Shows the details pane when `switched_on` and the window has room.
    pub(super) fn place_details_pane(&self, switched_on: bool) {
        let room = self.window_width().has_room_for_details();
        self.details_pane().root.set_visible(switched_on && room);
    }

    /// Shows the details pane while `win.details-pane` is on and the window
    /// has room for it.
    fn show_details_pane_if_room(&self) {
        let switched_on = self
            .action_state("details-pane")
            .and_then(|state| state.get::<bool>())
            .unwrap_or(true);
        self.place_details_pane(switched_on);
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
