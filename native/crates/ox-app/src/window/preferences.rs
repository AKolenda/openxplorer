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
use ox_core::settings::{Column, PreferencesUpdate};

use crate::folder_view::details;
use crate::theme::ThemePreference;

use super::content::FolderView;
use super::BrowserWindow;

/// Sidebar width nobody changed (`resetLayout` in app.js).
const DEFAULT_SIDEBAR_WIDTH: i32 = 210;
/// The narrowest and widest sidebar the Python app saves.
const SIDEBAR_WIDTHS: (i32, i32) = (140, 560);

/// One preference the user changed in this window.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Preference {
    View(FolderView),
    ShowHidden(bool),
    DetailsPane(bool),
    Theme(ThemePreference),
    TextSize(u32),
    SidebarWidth(i32),
    ColumnWidths(Vec<(Column, f64)>),
}

impl Preference {
    /// The settings change that saves it.
    fn to_update(&self) -> PreferencesUpdate {
        let mut update = PreferencesUpdate::default();
        match self {
            Preference::View(view) => update.view = Some(view.setting().to_owned()),
            Preference::ShowHidden(show) => update.show_hidden = Some(*show),
            Preference::DetailsPane(show) => update.details = Some(*show),
            Preference::Theme(theme) => update.theme = Some(theme.key().to_owned()),
            Preference::TextSize(size) => update.text_size = Some(*size),
            Preference::SidebarWidth(width) => update.sidebar_width = Some(f64::from(*width)),
            Preference::ColumnWidths(widths) => update.column_widths = Some(widths.clone()),
        }
        update
    }
}

/// The sidebar width to start with: the saved one within the Python app's
/// limits, else 210.
fn start_sidebar_width(saved: Option<u32>) -> i32 {
    let (narrowest, widest) = SIDEBAR_WIDTHS;
    saved
        .and_then(|width| i32::try_from(width).ok())
        .map_or(DEFAULT_SIDEBAR_WIDTH, |width| width.clamp(narrowest, widest))
}

impl BrowserWindow {
    /// Applies the shared preferences to a new window and starts saving
    /// the ones the user changes.
    pub(super) fn apply_preferences(&self) {
        let preferences = self.context().settings_data().preferences;
        self.content().model.set_show_hidden(preferences.show_hidden);
        self.details_pane().root.set_visible(preferences.details);
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
        let update = preference.to_update();
        let reply = glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |result: Result<(), ox_core::settings::SettingsError>| {
                if let Err(error) = result {
                    let message = format!("Changed for this window, but could not be saved: {error}");
                    window.chrome().show_message(&message);
                }
            }
        );
        let change = Box::new(move |settings: &mut ox_core::settings::Settings| {
            settings.update_preferences(&update).map(|_| ())
        });
        self.context().change_settings(change, reply);
    }
}

#[cfg(test)]
mod tests {
    use crate::folder_view::grid::IconSize;

    use super::*;

    #[test]
    fn the_sidebar_starts_at_210_within_the_python_limits() {
        assert_eq!(start_sidebar_width(None), 210);
        assert_eq!(start_sidebar_width(Some(300)), 300);
        assert_eq!(start_sidebar_width(Some(90)), 140);
        assert_eq!(start_sidebar_width(Some(9000)), 560);
    }

    #[test]
    fn every_icon_size_is_saved_as_the_python_grid_view() {
        let update = Preference::View(FolderView::Icons(IconSize::Small)).to_update();
        assert_eq!(update.view.as_deref(), Some("grid"));
        let update = Preference::View(FolderView::Details).to_update();
        assert_eq!(update.view.as_deref(), Some("details"));
    }
}
