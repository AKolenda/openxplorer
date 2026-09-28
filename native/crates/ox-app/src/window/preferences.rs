// SPDX-License-Identifier: AGPL-3.0-only
//! Applying and saving the window's view preferences.
//!
//! Ports `applyLayout`, `saveLayout` and the `fire('preferences', …)` calls
//! in `desktop/ui/app.js`. A window starts from the shared settings; view,
//! hidden files, details pane, theme, text size, sidebar width and column
//! widths are saved when the user changes them, off the main thread,
//! through the Python app's own settings file and lock.

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::{Column, PreferencesUpdate, Settings, SettingsError};

use crate::folder_view::details;
use crate::theme::ThemePreference;

use super::content::FolderView;
use super::BrowserWindow;

/// Sidebar width nobody changed (`resetLayout` in app.js).
const DEFAULT_SIDEBAR_WIDTH: i32 = 210;
/// The narrowest sidebar the Python app saves.
const NARROWEST_SIDEBAR: i32 = 140;
/// The widest sidebar the Python app saves.
const WIDEST_SIDEBAR: i32 = 560;

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
    Theme(ThemePreference),
    /// Text size in percent.
    TextSize(u32),
    /// Sidebar width in pixels.
    SidebarWidth(i32),
    /// The details columns the user sized, in pixels.
    ColumnWidths(Vec<(Column, f64)>),
}

impl Preference {
    /// The settings change that saves it.
    fn into_update(self) -> PreferencesUpdate {
        let mut update = PreferencesUpdate::default();
        match self {
            Preference::View(view) => update.view = Some(view.setting().to_owned()),
            Preference::ShowHidden(show) => update.show_hidden = Some(show),
            Preference::DetailsPane(show) => update.details = Some(show),
            Preference::Theme(theme) => update.theme = Some(theme.key().to_owned()),
            Preference::TextSize(size) => update.text_size = Some(size),
            Preference::SidebarWidth(width) => update.sidebar_width = Some(f64::from(width)),
            Preference::ColumnWidths(widths) => update.column_widths = Some(widths),
        }
        update
    }
}

/// The sidebar width to start with: the saved one within the Python app's
/// limits, else 210.
fn start_sidebar_width(saved: Option<u32>) -> i32 {
    saved
        .and_then(|width| i32::try_from(width).ok())
        .map_or(DEFAULT_SIDEBAR_WIDTH, |width| {
            width.clamp(NARROWEST_SIDEBAR, WIDEST_SIDEBAR)
        })
}

/// The widest the sidebar may be in a workspace `workspace_width` pixels
/// wide beside a details pane `details_width` wide, so the folder pane
/// keeps its room (`sidebarLimit` in app.js).
fn widest_sidebar(workspace_width: i32, details_width: i32) -> i32 {
    let room_left = workspace_width - details_width - FOLDER_PANE_ROOM;
    room_left.clamp(NARROWEST_SIDEBAR, WIDEST_SIDEBAR)
}

impl BrowserWindow {
    /// Applies the shared preferences to a new window and starts saving
    /// the ones the user changes.
    pub(super) fn apply_preferences(&self) {
        let preferences = self.context().settings_data().preferences;
        self.content().model.set_show_hidden(preferences.show_hidden);
        // `win.details-pane` starts from the same preferences.
        self.fit_details_pane();
        self.show_view(FolderView::from_setting(&preferences.view));
        let workspace = &self.chrome().workspace;
        workspace.set_position(start_sidebar_width(preferences.sidebar_width));
        let details_view = &self.content().details;
        details::apply_column_widths(details_view, preferences.column_widths.as_ref());
        details::connect_columns_resized(
            details_view,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |widths| window.save_preference(Preference::ColumnWidths(widths))
            ),
        );
        self.save_sidebar_width_after_drags();
        self.keep_sidebar_within_limit();
        self.reset_sidebar_on_double_click();
    }

    /// The widest the sidebar may be now, or `None` before the workspace
    /// is laid out.
    fn sidebar_limit(&self) -> Option<i32> {
        let workspace_width = self.chrome().workspace.width();
        if workspace_width == 0 {
            return None;
        }
        let pane = &self.details_pane().root;
        let details_width = if pane.is_visible() { pane.width() } else { 0 };
        Some(widest_sidebar(workspace_width, details_width))
    }

    /// Stops a dragged sidebar where the folder pane would lose its room.
    fn keep_sidebar_within_limit(&self) {
        self.chrome().workspace.connect_position_notify(glib::clone!(
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
                let workspace = &window.chrome().workspace;
                let on_handle = (x - f64::from(workspace.position())).abs() <= HANDLE_REACH;
                if presses == 2 && on_handle {
                    workspace.set_position(DEFAULT_SIDEBAR_WIDTH);
                    window.save_preference(Preference::SidebarWidth(DEFAULT_SIDEBAR_WIDTH));
                }
            }
        ));
        self.chrome().workspace.add_controller(click);
    }

    /// Saves the sidebar width when the user finishes dragging it, never
    /// for the window's own layout changes.
    fn save_sidebar_width_after_drags(&self) {
        let workspace = &self.chrome().workspace;
        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let start = Rc::new(Cell::new(0));
        let paned = workspace.downgrade();
        drag.connect_drag_begin(glib::clone!(
            #[strong]
            start,
            #[strong]
            paned,
            move |_, _, _| {
                if let Some(paned) = paned.upgrade() {
                    start.set(paned.position());
                }
            }
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _| {
                let Some(paned) = paned.upgrade() else { return };
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
                    window.chrome().show_message(&message);
                }
            }
        );
        let change =
            Box::new(move |settings: &mut Settings| settings.update_preferences(&update).map(|_| ()));
        self.context().change_settings(change, reply);
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn every_icon_size_is_saved_as_the_python_grid_view() {
        let update = Preference::View(FolderView::Icons(IconSize::Small)).into_update();
        assert_eq!(update.view.as_deref(), Some("grid"));
        let update = Preference::View(FolderView::Details).into_update();
        assert_eq!(update.view.as_deref(), Some("details"));
    }
}
