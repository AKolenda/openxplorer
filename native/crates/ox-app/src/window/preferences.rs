// SPDX-License-Identifier: AGPL-3.0-only
//! Applying and saving the window's view preferences.
//!
//! Ports `applyLayout`, `saveLayout`, `resetLayout` and the
//! `fire('preferences', …)` calls in `desktop/ui/app.js`. A window starts
//! from the shared settings; view, hidden files, details pane, theme, text
//! size, sidebar width and column widths are saved when the user changes
//! them, off the main thread, through the Python app's own settings file
//! and lock. Resetting the layout returns every window's sidebar and
//! columns to their default widths.

use std::cell::Cell;
use std::ops::RangeInclusive;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::{ColumnWidth, PreferencesUpdate, SettingsError, Theme, SIDEBAR_WIDTHS};

use super::folder_pane::FolderView;
use super::BrowserWindow;
use crate::text_size::TextSize;

/// Sidebar width nobody changed (`resetLayout` in app.js).
const DEFAULT_SIDEBAR_WIDTH: i32 = 210;

/// Room the folder pane keeps beside the sidebar (`sidebarLimit` in app.js).
const FOLDER_PANE_ROOM: i32 = 300;

/// How far from the pane handle a double-click still resets the sidebar.
const HANDLE_REACH: f64 = 6.0;

/// One preference the user changed in this window.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Preference {
    /// Details or an icon size (saved as the Python app's `grid`).
    View(FolderView),
    /// Show hidden files.
    ShowHidden(bool),
    /// Show the details pane.
    DetailsPane(bool),
    /// System, Light or Dark.
    Theme(Theme),
    /// Text size.
    TextSize(TextSize),
    /// Sidebar width in pixels.
    SidebarWidth(i32),
    /// The details columns the user sized, in pixels.
    ColumnWidths(Vec<ColumnWidth>),
    /// The default sidebar width and column widths (`resetLayout`).
    DefaultLayout,
}

impl Preference {
    /// The settings change that saves it.
    fn into_update(self) -> PreferencesUpdate {
        let mut update = PreferencesUpdate::default();
        match self {
            Preference::View(view) => update.view = Some(view.setting()),
            Preference::ShowHidden(show) => update.show_hidden = Some(show),
            Preference::DetailsPane(show) => update.show_details_pane = Some(show),
            Preference::Theme(theme) => update.theme = Some(theme),
            Preference::TextSize(size) => update.text_size = Some(size.percent()),
            Preference::SidebarWidth(width) => update.sidebar_width = Some(f64::from(width)),
            Preference::ColumnWidths(widths) => update.column_widths = Some(widths),
            Preference::DefaultLayout => {
                update.sidebar_width = Some(f64::from(DEFAULT_SIDEBAR_WIDTH));
                // An empty list clears every saved column width.
                update.column_widths = Some(Vec::new());
            }
        }
        update
    }
}

/// The sidebar widths the settings save (ox-core's [`SIDEBAR_WIDTHS`]),
/// in GTK's pixels. The window never shows a width the settings would
/// refuse to save, nor saves one the Python app would ignore.
///
/// # Panics
///
/// Never: the settings' limits are a few hundred pixels.
pub(super) fn sidebar_widths() -> RangeInclusive<i32> {
    let pixels = |width: u32| i32::try_from(width).expect("sidebar widths are a few hundred pixels");
    pixels(*SIDEBAR_WIDTHS.start())..=pixels(*SIDEBAR_WIDTHS.end())
}

/// `width` limited to [`sidebar_widths`].
fn clamp_sidebar_width(width: i32) -> i32 {
    let limits = sidebar_widths();
    width.clamp(*limits.start(), *limits.end())
}

/// The sidebar width to start with: the saved one within the Python app's
/// limits, else 210.
fn start_sidebar_width(saved: Option<u32>) -> i32 {
    saved
        .and_then(|width| i32::try_from(width).ok())
        .map_or(DEFAULT_SIDEBAR_WIDTH, clamp_sidebar_width)
}

/// The widest the sidebar may be in a workspace `workspace_width` pixels
/// wide beside a details pane `details_width` wide, so the folder pane
/// keeps its room (`sidebarLimit` in app.js).
fn widest_sidebar(workspace_width: i32, details_width: i32) -> i32 {
    let room_left = workspace_width - details_width - FOLDER_PANE_ROOM;
    clamp_sidebar_width(room_left)
}

impl BrowserWindow {
    /// Applies the shared preferences to a new window and starts saving
    /// the ones the user changes.
    pub(super) fn apply_preferences(&self) {
        let preferences = self.context().settings_data().preferences;
        self.folder_pane()
            .model()
            .set_show_hidden(preferences.show_hidden);
        // `win.details-pane` starts from the same preferences.
        self.fit_details_pane();
        self.show_view(FolderView::from_setting(preferences.view));
        let workspace = self.workspace();
        workspace.set_position(start_sidebar_width(preferences.sidebar_width));
        let details_view = self.folder_pane().details();
        details_view.apply_column_widths(preferences.column_widths.as_ref());
        details_view.connect_columns_resized(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |widths| window.save_preference(Preference::ColumnWidths(widths))
        ));
        self.save_sidebar_width_after_drags();
        self.keep_sidebar_within_limit();
        self.reset_sidebar_on_double_click();
        self.follow_layout_reset();
    }

    /// Settings > "Reset sidebar and column widths": saves the default
    /// widths and has every window show them.
    pub(super) fn reset_layout(&self) {
        self.save_preference(Preference::DefaultLayout);
        self.context().announce_layout_reset();
    }

    /// Returns the sidebar and the columns to their default widths whenever
    /// any window resets the layout.
    fn follow_layout_reset(&self) {
        let handler = self.context().connect_layout_reset(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.show_default_layout()
        ));
        self.imp().handlers.borrow_mut().layout = Some(handler);
    }

    /// Shows the 210-pixel sidebar and the columns' default widths, without
    /// saving them: the window that reset the layout saves it once.
    fn show_default_layout(&self) {
        self.workspace().set_position(DEFAULT_SIDEBAR_WIDTH);
        self.folder_pane().details().apply_column_widths(None);
    }

    /// The widest the sidebar may be now, or `None` before the workspace
    /// is laid out.
    fn sidebar_limit(&self) -> Option<i32> {
        let workspace_width = self.workspace().width();
        if workspace_width == 0 {
            return None;
        }
        let pane = self.details_pane();
        let details_width = if pane.is_visible() { pane.width() } else { 0 };
        Some(widest_sidebar(workspace_width, details_width))
    }

    /// Stops a dragged sidebar where the folder pane would lose its room.
    fn keep_sidebar_within_limit(&self) {
        self.workspace().connect_position_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |workspace| {
                let limit = window.sidebar_limit();
                if let Some(limit) = limit.filter(|limit| workspace.position() > *limit) {
                    workspace.set_position(limit);
                }
            }
        ));
    }

    /// A double-click on the pane handle returns the sidebar to 210 pixels
    /// and saves that, as the Python app's resizer does.
    fn reset_sidebar_on_double_click(&self) {
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, presses, x, _| {
                let workspace = window.workspace();
                let on_handle = (x - f64::from(workspace.position())).abs() <= HANDLE_REACH;
                if presses == 2 && on_handle {
                    workspace.set_position(DEFAULT_SIDEBAR_WIDTH);
                    window.save_preference(Preference::SidebarWidth(DEFAULT_SIDEBAR_WIDTH));
                }
            }
        ));
        self.workspace().add_controller(click);
    }

    /// Saves the sidebar width when the user finishes dragging it, never
    /// for the window's own layout changes.
    fn save_sidebar_width_after_drags(&self) {
        let workspace = self.workspace();
        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        // Where the drag started, shared by its begin and end handlers.
        let start = Rc::new(Cell::new(0));
        drag.connect_drag_begin(glib::clone!(
            #[strong]
            start,
            #[weak(rename_to = paned)]
            workspace,
            move |_, _, _| start.set(paned.position())
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak(rename_to = paned)]
            workspace,
            move |_, _, _| {
                if paned.position() != start.get() {
                    window.save_preference(Preference::SidebarWidth(paned.position()));
                }
            }
        ));
        workspace.add_controller(drag);
    }

    /// Saves one preference. A failure leaves the change in this window and
    /// says so, as the Python app's text-size toast does.
    pub(super) fn save_preference(&self, preference: Preference) {
        let update = preference.into_update();
        let reply = glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |result: Result<(), SettingsError>| {
                if let Err(error) = result {
                    let message = format!("Changed for this window, but could not be saved: {error}");
                    window.show_message(&message);
                }
            }
        );
        self.context().update_preferences(update, reply);
    }
}

#[cfg(test)]
mod tests {
    use ox_core::settings::View;

    use crate::folder_view::grid::IconSize;

    use super::*;

    /// parity: SIDE-023
    #[test]
    fn the_sidebar_starts_at_210_within_the_python_limits() {
        assert_eq!(start_sidebar_width(None), 210);
        assert_eq!(start_sidebar_width(Some(300)), 300);
        assert_eq!(start_sidebar_width(Some(90)), 140);
        assert_eq!(start_sidebar_width(Some(9000)), 560);
    }

    /// parity: SIDE-023
    #[test]
    fn the_sidebar_leaves_the_folder_pane_300_pixels() {
        assert_eq!(widest_sidebar(1320, 262), 560);
        assert_eq!(widest_sidebar(900, 262), 338);
        assert_eq!(widest_sidebar(600, 262), 140, "never narrower than 140");
    }

    /// `resetLayout` in app.js saves `{sidebarWidth: 210, columnWidths: {}}`.
    #[test]
    fn resetting_the_layout_saves_the_210_pixel_sidebar_and_clears_the_columns() {
        let update = Preference::DefaultLayout.into_update();
        assert_eq!(update.sidebar_width, Some(210.0));
        assert_eq!(update.column_widths, Some(Vec::new()));
        assert_eq!(update.theme, None, "nothing else changes");
    }

    #[test]
    fn every_icon_size_is_saved_as_the_python_grid_view() {
        let update = Preference::View(FolderView::Icons(IconSize::Small)).into_update();
        assert_eq!(update.view, Some(View::Grid));
        let update = Preference::View(FolderView::Details).into_update();
        assert_eq!(update.view, Some(View::Details));
    }
}
