// SPDX-License-Identifier: AGPL-3.0-only
//! Folder Options, as "…" > Options opens it in Windows 11 File Explorer,
//! with its General and View tabs (VIEW-020; it replaces Dolphin's display
//! style dialog, VIEW-021). Each option is on the tab Windows puts it on,
//! and only the options the app has are there.
//!
//! General: "Open File Explorer to", the start places the app has (Home,
//! This PC, or a folder chosen in Settings).
//!
//! View: Folder views gives every folder the view of the folder Folder
//! Options was opened from (Apply to Folders: layout, icon size, columns,
//! sort and grouping) or the default view again (Reset Folders). Each asks
//! first, acts at once in every window, and leaves Folder Options open, as
//! in Windows. The app has no folder types, so Windows' "all folders of
//! this type" is every folder. Apply to Folders needs a folder: from This
//! PC, Network, Recent, search results or the Recycle Bin it cannot be
//! pressed. Then the advanced settings, in Windows' order.
//!
//! The options take effect with OK or Apply, and each tab's Restore
//! Defaults sets its choices back. Built on the app's one [`Dialog`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::i18n::{gettext, gettext_static};
use ox_core::location::is_virtual_location;
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
    /// Writes the choice into an update of `Preferences`.
    write: fn(&mut PreferencesUpdate, &Preferences, bool),
}

/// The advanced settings, in Windows' order.
fn advanced_settings() -> [AdvancedSetting; 6] {
    [
        // The opposite of Settings' "Show previews".
        AdvancedSetting {
            label: message_id("Always show icons, never thumbnails"),
            read: |preferences| !preferences.view_options.show_previews,
            write: |update, preferences, on| {
                let mut options = preferences.view_options.clone();
                options.show_previews = !on;
                update.view_options = Some(options);
            },
        },
        AdvancedSetting {
            label: message_id("Display the full path in the title bar"),
            read: |preferences| preferences.full_path_in_title,
            write: |update, _, on| update.full_path_in_title = Some(on),
        },
        AdvancedSetting {
            label: message_id("Remember each folder's view settings"),
            read: |preferences| preferences.per_folder_views,
            write: |update, _, on| update.per_folder_views = Some(on),
        },
        AdvancedSetting {
            label: message_id("Restore previous folder windows at logon"),
            read: |preferences| preferences.restore_session,
            write: |update, _, on| update.restore_session = Some(on),
        },
        AdvancedSetting {
            label: message_id("Show hidden files, folders, and drives"),
            read: |preferences| preferences.show_hidden,
            write: |update, _, on| update.show_hidden = Some(on),
        },
        AdvancedSetting {
            label: message_id("Use check boxes to select items"),
            read: |preferences| preferences.selection_marker,
            write: |update, _, on| update.selection_marker = Some(on),
        },
    ]
}

/// Whether Apply to Folders can take the view of `uri`: a folder, local,
/// on a share or in a ZIP, not a page such as This PC, the Recycle Bin,
/// Recent or search results. The folder's location is not
/// saved.
fn is_view_source(uri: &str, searching: bool) -> bool {
    !searching && !uri.is_empty() && !is_virtual_location(uri)
}

/// The start places "Open File Explorer to" offers: Home (`None`), This
/// PC, and the place chosen in Settings when it is another one.
fn start_places(chosen: Option<&str>) -> Vec<Option<String>> {
    let mut places = vec![None, Some(Page::ThisPc.uri().to_owned())];
    if let Some(chosen) = chosen.filter(|chosen| !places.contains(&Some((*chosen).to_owned()))) {
        places.push(Some(chosen.to_owned()));
    }
    places
}

/// The name "Open File Explorer to" shows for `place`.
fn start_place_name(place: Option<&str>) -> String {
    match place {
        None => gettext("Home"),
        Some(uri) => match Page::from_uri(uri) {
            Some(page) => page.title().to_owned(),
            None => gio::File::for_uri(uri).parse_name().to_string(),
        },
    }
}

/// The choices of Folder Options and what was last saved of them.
struct OptionsForm {
    checks: Vec<(AdvancedSetting, gtk::CheckButton)>,
    saved: RefCell<Vec<bool>>,
    start: gtk::DropDown,
    places: Vec<Option<String>>,
    saved_start: Cell<u32>,
}

impl OptionsForm {
    /// The choices, showing `preferences`: "Open File Explorer to" on
    /// `general` and the advanced settings on `view`.
    fn add_to(general: &gtk::Box, view: &gtk::Box, preferences: &Preferences) -> Rc<Self> {
        let places = start_places(preferences.startup_folder.as_deref());
        let names: Vec<String> = places
            .iter()
            .map(|place| start_place_name(place.as_deref()))
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let start = gtk::DropDown::from_strings(&names);
        start.set_halign(gtk::Align::Start);
        let chosen = places
            .iter()
            .position(|place| *place == preferences.startup_folder)
            .unwrap_or(0);
        start.set_selected(u32::try_from(chosen).unwrap_or(0));
        let caption = gtk::Label::builder()
            .label(gettext("Open File Explorer to:"))
            .xalign(0.0)
            .mnemonic_widget(&start)
            .build();
        start.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
        let open_to = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        open_to.append(&caption);
        open_to.append(&start);
        general.append(&open_to);
        let restore_general = restore_defaults_button();
        general.append(&restore_general);

        let caption = gtk::Label::builder()
            .label(gettext("Advanced settings:"))
            .xalign(0.0)
            .build();
        view.append(&caption);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let checks: Vec<_> = advanced_settings()
            .into_iter()
            .map(|setting| {
                let check = check_row(gettext_static(setting.label), (setting.read)(preferences));
                list.append(&check);
                (setting, check)
            })
            .collect();
        list.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
        view.append(&list);
        let restore_view = restore_defaults_button();
        view.append(&restore_view);

        let saved = checks.iter().map(|(_, check)| check.is_active()).collect();
        let form = Rc::new(Self {
            checks,
            saved: RefCell::new(saved),
            saved_start: Cell::new(start.selected()),
            start,
            places,
        });
        restore_general.connect_clicked(glib::clone!(
            #[weak]
            form,
            move |_| form.start.set_selected(0)
        ));
        restore_view.connect_clicked(glib::clone!(
            #[weak]
            form,
            move |_| form.restore_view_defaults()
        ));
        form
    }

    /// Sets every advanced setting to the app's default, to be saved with
    /// OK or Apply.
    fn restore_view_defaults(&self) {
        let defaults = Preferences::default();
        for (setting, check) in &self.checks {
            check.set_active((setting.read)(&defaults));
        }
    }

    /// The change the choices make to `preferences`, if any; it counts as
    /// saved from now on.
    fn take_change(&self, preferences: &Preferences) -> Option<PreferencesUpdate> {
        let mut saved = self.saved.borrow_mut();
        let mut update = PreferencesUpdate::default();
        let mut changed = false;
        for ((setting, check), was) in self.checks.iter().zip(saved.iter_mut()) {
            let now = check.is_active();
            if now != *was {
                (setting.write)(&mut update, preferences, now);
                *was = now;
                changed = true;
            }
        }
        let start = self.start.selected();
        if start != self.saved_start.get() {
            let place = self.places.get(start as usize).cloned().flatten();
            // An empty folder is Home.
            update.startup_folder = Some(place.unwrap_or_default());
            self.saved_start.set(start);
            changed = true;
        }
        changed.then_some(update)
    }

    /// Whether a choice differs from what was saved.
    fn is_changed(&self) -> bool {
        let saved = self.saved.borrow();
        let checks = self
            .checks
            .iter()
            .zip(saved.iter())
            .any(|((_, check), was)| check.is_active() != *was);
        checks || self.start.selected() != self.saved_start.get()
    }

    /// Greys out `apply` of `options` while nothing is changed, as Windows
    /// does.
    fn follow_with(self: &Rc<Self>, options: &Dialog, apply: DialogButton) {
        options.set_button_sensitive(apply, false);
        let update = glib::clone!(
            #[weak]
            options,
            #[weak(rename_to = form)]
            self,
            move || options.set_button_sensitive(apply, form.is_changed())
        );
        let update = Rc::new(update);
        for (_, check) in &self.checks {
            let update = Rc::clone(&update);
            check.connect_toggled(move |_| update());
        }
        self.start.connect_selected_notify(move |_| update());
    }
}

/// A tab's Restore Defaults, at its end.
fn restore_defaults_button() -> gtk::Button {
    let button = gtk::Button::with_label(&gettext("Restore Defaults"));
    button.set_halign(gtk::Align::End);
    button
}

/// A tab's page.
fn tab_page() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page
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
        let tabs = gtk::Notebook::new();
        let general = tab_page();
        let view = tab_page();
        self.add_folder_views(&options, &view, source);
        let form = OptionsForm::add_to(&general, &view, &self.context().settings_data().preferences);
        tabs.append_page(&general, Some(&gtk::Label::new(Some(&gettext("General")))));
        tabs.append_page(&view, Some(&gtk::Label::new(Some(&gettext("View")))));
        options.add_field(&tabs);
        let ok = options.add_button(&gettext("OK"), ButtonStyle::Accent);
        options.add_cancel_button();
        let apply = options.add_button(&gettext("Apply"), ButtonStyle::Bordered);
        form.follow_with(&options, apply);
        options.open();
        while let Some(answer) = options.next_response().await {
            let preferences = self.context().settings_data().preferences;
            if let Some(update) = form.take_change(&preferences) {
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

    /// Adds the Folder views box to `view`, whose Apply to Folders can be
    /// pressed when Folder Options was opened from a folder (`source`).
    fn add_folder_views(&self, options: &Dialog, view: &gtk::Box, source: bool) {
        let folder_views = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let caption = gtk::Label::builder()
            .label(gettext("Folder views"))
            .xalign(0.0)
            .css_classes(["field-label"])
            .build();
        folder_views.append(&caption);
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
        folder_views.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
        view.append(&folder_views);
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
    use ox_core::location::{RECENT_LOCATIONS_URI, RECENT_URI, TRASH_URI};

    use super::*;

    /// Apply to Folders takes the view of a folder, local or on a share,
    /// never of a page, Recent, search results or the Recycle Bin.
    ///
    /// parity: VIEW-020
    #[test]
    fn only_a_folder_gives_its_view_to_every_folder() {
        assert!(is_view_source("file:///home/someone/Documents", false));
        assert!(is_view_source("smb://server/share/Reports", false));
        assert!(is_view_source("sftp://someone@server/folder", false));
        assert!(!is_view_source("file:///home/someone/Documents", true));
        for place in [
            Page::ThisPc.uri(),
            Page::Network.uri(),
            TRASH_URI,
            "trash:///old%20folder",
            RECENT_URI,
            RECENT_LOCATIONS_URI,
        ] {
            assert!(!is_view_source(place, false), "{place}");
        }
    }

    /// Each advanced setting writes the preference it reads.
    #[test]
    fn the_advanced_settings_read_and_write_their_preference() {
        for setting in advanced_settings() {
            for on in [true, false] {
                let mut update = PreferencesUpdate::default();
                (setting.write)(&mut update, &Preferences::default(), on);
                let mut preferences = Preferences::default();
                preferences.apply(&update);
                assert_eq!((setting.read)(&preferences), on, "{}", setting.label);
            }
        }
    }

    /// "Open File Explorer to" offers Home and This PC, and a place chosen
    /// in Settings too.
    ///
    /// parity: TAB-055
    #[test]
    fn open_file_explorer_to_offers_the_start_places() {
        assert_eq!(start_places(None), [None, Some(Page::ThisPc.uri().to_owned())]);
        assert_eq!(start_places(Some(Page::ThisPc.uri())).len(), 2);
        assert_eq!(
            start_places(Some("file:///srv/Example%20projects")).last(),
            Some(&Some("file:///srv/Example%20projects".to_owned()))
        );
    }
}
