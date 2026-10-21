// SPDX-License-Identifier: AGPL-3.0-only
//! Folder Options, as "…" > Options opens it in Windows 11 File Explorer:
//! its View tab's Folder views and the advanced settings the app has too
//! (VIEW-020; it replaces Dolphin's display style dialog, VIEW-021).
//!
//! Folder views gives every folder the view of the folder Folder Options
//! was opened from (Apply to Folders: layout, icon size, columns, sort and
//! grouping) or the default view again (Reset Folders). Each asks first,
//! acts at once in every window, and leaves Folder Options open, as in
//! Windows. The app has no folder types, so Windows' "all folders of this
//! type" is every folder. Apply to Folders needs a folder: from This PC,
//! Network, Recent files, search results or the Recycle Bin it cannot be
//! pressed. The advanced settings take effect with OK or Apply, and
//! Restore Defaults sets their boxes back. Built on the app's one
//! [`Dialog`].

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::i18n::{gettext, gettext_static};
use ox_core::settings::{Preferences, PreferencesUpdate};

use super::{BrowserWindow, ButtonStyle};
use crate::dialog::{check_row, Dialog, DialogButton};
use crate::i18n::message_id;
use crate::locations::Page;

/// The question Apply to Folders asks, Windows' without "of this type".
const APPLY_QUESTION: &str = message_id("Do you want all folders to match this folder's view settings?");

/// The question Reset Folders asks.
const RESET_QUESTION: &str = message_id("Do you want to reset all folders to their default view settings?");

/// One of Windows' advanced settings that the app has too.
struct AdvancedSetting {
    /// Windows' wording.
    label: &'static str,
    read: fn(&Preferences) -> bool,
    write: fn(&mut PreferencesUpdate, bool),
}

/// The advanced settings, in Windows' order.
fn advanced_settings() -> [AdvancedSetting; 3] {
    [
        AdvancedSetting {
            label: message_id("Display the full path in the title bar"),
            read: |preferences| preferences.full_path_in_title,
            write: |update, on| update.full_path_in_title = Some(on),
        },
        AdvancedSetting {
            label: message_id("Restore previous folder windows at logon"),
            read: |preferences| preferences.restore_session,
            write: |update, on| update.restore_session = Some(on),
        },
        AdvancedSetting {
            label: message_id("Use check boxes to select items"),
            read: |preferences| preferences.selection_marker,
            write: |update, on| update.selection_marker = Some(on),
        },
    ]
}

/// Whether Apply to Folders can take the view of `uri`: a folder, local,
/// on a share or in a ZIP, not a page such as This PC, Recent files,
/// search results or the Recycle Bin. The folder's location is not saved.
fn is_view_source(uri: &str, searching: bool) -> bool {
    let is_list_of_places = ["trash:", "recent:"].iter().any(|scheme| uri.starts_with(scheme));
    !searching && !uri.is_empty() && Page::from_uri(uri).is_none() && !is_list_of_places
}

/// The advanced settings' boxes and what was last saved of them.
struct AdvancedChecks {
    checks: Vec<(AdvancedSetting, gtk::CheckButton)>,
    saved: RefCell<Vec<bool>>,
}

impl AdvancedChecks {
    /// The boxes, showing `preferences`, in a list added to `options`.
    fn add_to(options: &Dialog, preferences: &Preferences) -> Rc<Self> {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let checks: Vec<_> = advanced_settings()
            .into_iter()
            .map(|setting| {
                let check = check_row(gettext_static(setting.label), (setting.read)(preferences));
                list.append(&check);
                (setting, check)
            })
            .collect();
        let restore = gtk::Button::with_label(&gettext("Restore Defaults"));
        restore.set_halign(gtk::Align::End);
        list.append(&restore);
        options.add_labelled(&gettext("Advanced settings"), &list);
        let saved = checks.iter().map(|(_, check)| check.is_active()).collect();
        let advanced = Rc::new(Self {
            checks,
            saved: RefCell::new(saved),
        });
        restore.connect_clicked(glib::clone!(
            #[strong]
            advanced,
            move |_| advanced.restore_defaults()
        ));
        advanced
    }

    /// Sets every box to the app's default, to be saved with OK or Apply.
    fn restore_defaults(&self) {
        let defaults = Preferences::default();
        for (setting, check) in &self.checks {
            check.set_active((setting.read)(&defaults));
        }
    }

    /// The change the boxes make to what was saved, if any; it counts as
    /// saved from now on.
    fn take_change(&self) -> Option<PreferencesUpdate> {
        let mut saved = self.saved.borrow_mut();
        let mut update = PreferencesUpdate::default();
        let mut changed = false;
        for ((setting, check), was) in self.checks.iter().zip(saved.iter_mut()) {
            let now = check.is_active();
            if now != *was {
                (setting.write)(&mut update, now);
                *was = now;
                changed = true;
            }
        }
        changed.then_some(update)
    }

    /// Whether a box differs from what was saved.
    fn is_changed(&self) -> bool {
        let saved = self.saved.borrow();
        self.checks
            .iter()
            .zip(saved.iter())
            .any(|((_, check), was)| check.is_active() != *was)
    }

    /// Greys out `apply` of `options` while nothing is changed, as Windows
    /// does.
    fn follow_with(self: &Rc<Self>, options: &Dialog, apply: DialogButton) {
        options.set_button_sensitive(apply, false);
        for (_, check) in &self.checks {
            check.connect_toggled(glib::clone!(
                #[weak]
                options,
                #[weak(rename_to = advanced)]
                self,
                move |_| options.set_button_sensitive(apply, advanced.is_changed())
            ));
        }
    }
}

impl BrowserWindow {
    /// Opens Folder Options for the folder shown.
    pub(super) fn show_folder_options(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move { window.run_folder_options().await }
        ));
    }

    /// Shows Folder Options until OK, Cancel or Escape.
    async fn run_folder_options(&self) {
        let options = Dialog::new(self, &gettext("Folder Options"), "");
        let source = self
            .current_uri()
            .is_some_and(|uri| is_view_source(&uri, self.is_searching()));
        self.add_folder_views(&options, source);
        let advanced = AdvancedChecks::add_to(&options, &self.context().settings_data().preferences);
        let ok = options.add_button(&gettext("OK"), ButtonStyle::Accent);
        options.add_cancel_button();
        let apply = options.add_button(&gettext("Apply"), ButtonStyle::Bordered);
        advanced.follow_with(&options, apply);
        options.open();
        while let Some(answer) = options.next_response().await {
            if let Some(update) = advanced.take_change() {
                self.context()
                    .update_preferences(update, self.preference_failure_reply());
            }
            if answer == ok {
                options.finish();
                return;
            }
            options.set_button_sensitive(apply, false);
        }
    }

    /// Adds the Folder views box, whose Apply to Folders can be pressed
    /// when Folder Options was opened from a folder (`source`).
    fn add_folder_views(&self, options: &Dialog, source: bool) {
        let folder_views = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let text = gtk::Label::builder()
            .label(gettext(
                "You can apply this view (such as Details or Icons) to all folders.",
            ))
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(48)
            .build();
        folder_views.append(&text);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let apply = gtk::Button::with_label(&gettext("Apply to Folders"));
        apply.set_sensitive(source);
        let reset = gtk::Button::with_label(&gettext("Reset Folders"));
        buttons.append(&apply);
        buttons.append(&reset);
        folder_views.append(&buttons);
        options.add_labelled(&gettext("Folder views"), &folder_views);
        apply.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            options,
            move |_| {
                glib::spawn_future_local(async move { window.apply_to_folders(&options).await });
            }
        ));
        reset.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            options,
            move |_| {
                glib::spawn_future_local(async move { window.reset_folders(&options).await });
            }
        ));
    }

    /// Apply to Folders: after Yes, every folder shows the view of the
    /// folder shown and keeps none of its own.
    async fn apply_to_folders(&self, options: &Dialog) {
        if ask_folder_views(options, APPLY_QUESTION).await {
            self.context()
                .apply_view_to_all_folders(self.current_style(), self.preference_failure_reply());
        }
    }

    /// Reset Folders: after Yes, every folder forgets its view and shows
    /// the default one.
    async fn reset_folders(&self, options: &Dialog) {
        if ask_folder_views(options, RESET_QUESTION).await {
            self.context().reset_folder_views(self.preference_failure_reply());
        }
    }
}

/// Asks `question` over `options` as Windows' Folder Views message box
/// does; true for Yes.
async fn ask_folder_views(options: &Dialog, question: &str) -> bool {
    let ask = Dialog::new(options, &gettext("Folder Views"), &gettext(question));
    let yes = ask.add_button(&gettext("Yes"), ButtonStyle::Accent);
    ask.add_button(&gettext("No"), ButtonStyle::Bordered);
    ask.open_on_first_button();
    let answer = ask.next_response().await;
    ask.finish();
    answer == Some(yes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply to Folders takes the view of a folder, local or on a share,
    /// never of a page, Recent files, search results or the Recycle Bin.
    ///
    /// parity: VIEW-020
    #[test]
    fn only_a_folder_gives_its_view_to_every_folder() {
        assert!(is_view_source("file:///home/someone/Documents", false));
        assert!(is_view_source("smb://server/share/Reports", false));
        assert!(!is_view_source("file:///home/someone/Documents", true));
        assert!(!is_view_source(Page::ThisPc.uri(), false));
        assert!(!is_view_source(Page::Network.uri(), false));
        assert!(!is_view_source("trash:///", false));
        assert!(is_view_source("sftp://someone@server/folder", false));
        assert!(!is_view_source("recent:///", false));
    }

    /// Each advanced setting writes the preference it reads.
    #[test]
    fn the_advanced_settings_read_and_write_their_preference() {
        for setting in advanced_settings() {
            let mut update = PreferencesUpdate::default();
            (setting.write)(&mut update, true);
            let preferences = Preferences {
                full_path_in_title: update.full_path_in_title.unwrap_or(false),
                restore_session: update.restore_session.unwrap_or(false),
                selection_marker: update.selection_marker.unwrap_or(false),
                ..Preferences::default()
            };
            assert!((setting.read)(&preferences), "{}", setting.label);
        }
    }
}
