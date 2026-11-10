// SPDX-License-Identifier: AGPL-3.0-only
//! View > Show > File name extensions (VIEW-069): whether names show the
//! extension of a known file type, as Windows' "Hide extensions for known
//! file types" decides. Off by default, as in Windows. The choice is saved
//! and every window follows it, as Folder Options' check box does; the
//! file list, its tooltips and accessible names, the details pane and the
//! crumb of an opened archive show names by it
//! ([`crate::folder_view::shown_name`]). The real names never change: a
//! rename in place keeps the hidden extension ([`super::file_ops`]).

use gtk::prelude::*;

use crate::folder_view::shown_name::Extensions;

use super::actions::toggle_action;
use super::preferences::Preference;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What `shown` means for names.
const fn extensions(shown: bool) -> Extensions {
    if shown {
        Extensions::Shown
    } else {
        Extensions::Hidden
    }
}

impl BrowserWindow {
    /// Adds the File name extensions toggle, starting from the saved
    /// preference.
    pub(super) fn install_file_name_extensions_action(&self) {
        let shown = self.context().settings_data().preferences.show_extensions;
        self.add_action_entries([toggle_action(
            WindowAction::FileNameExtensions,
            shown,
            |window, shown| {
                window.show_file_name_extensions(shown);
                window.save_preference(Preference::FileNameExtensions(shown));
            },
        )]);
        self.show_file_name_extensions(shown);
    }

    /// Whether names show the extensions of known types now.
    pub(super) fn file_name_extensions(&self) -> Extensions {
        let state = self.window_action_state(WindowAction::FileNameExtensions);
        let shown = state.and_then(|state| state.get::<bool>());
        extensions(shown.unwrap_or_else(|| self.context().settings_data().preferences.show_extensions))
    }

    /// Shows names with or without the extensions of known types in both
    /// panes, the details pane and the address.
    fn show_file_name_extensions(&self, shown: bool) {
        for pane in self.folder_panes() {
            pane.owners().set_extensions(extensions(shown));
        }
        self.update_details_pane();
        if let Some(uri) = self.current_uri() {
            self.render_address_of(&uri);
        }
        // A Save dialog's File name shows its name the same way.
        self.show_picker_name_again();
    }

    /// Takes up a choice that Folder Options, Settings or another window
    /// saved, and gives a pane made since the window's choice.
    pub(super) fn follow_file_name_extensions_preference(&self) {
        let saved = self.context().settings_data().preferences.show_extensions;
        let state = self.window_action_state(WindowAction::FileNameExtensions);
        let Some(shown) = state.and_then(|state| state.get::<bool>()) else {
            return;
        };
        if shown != saved {
            self.set_action_state(WindowAction::FileNameExtensions, &saved.to_variant());
            self.show_file_name_extensions(saved);
            return;
        }
        for pane in self.folder_panes() {
            pane.owners().set_extensions(extensions(shown));
        }
    }
}
