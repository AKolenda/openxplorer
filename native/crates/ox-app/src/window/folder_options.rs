// SPDX-License-Identifier: AGPL-3.0-only
//! Folder Options, as "…" > Options opens it in Windows 11 File Explorer,
//! with its General, View and Search tabs (VIEW-020; it replaces Dolphin's
//! display style dialog, VIEW-021). Each option is on the tab Windows puts
//! it on, in Windows' words, and only the options the app can honour are
//! there.
//!
//! General: "Open File Explorer to", the start places the app has (Home,
//! This PC, or a folder chosen in Settings); Browse folders, the same
//! window or a window of each folder's own; Click items as follows, where
//! only Double-click can be chosen, as the app has no single-click mode
//! (SEL-018); Privacy, whether the navigation pane shows Recent files
//! ("Show recently used files") and Recent locations ("Show frequently
//! used folders"), and Clear File Explorer history, which forgets them and
//! the typed addresses at once; and "More settings…", which keeps the
//! choices made, as OK does, and opens Settings: Settings left the "…"
//! menu for it (CMD-007), as Windows 11 keeps File Explorer's options here
//! and the rest in Settings.
//!
//! View: Folder views gives every folder the view of the folder Folder
//! Options was opened from (Apply to Folders: layout, icon size, columns,
//! sort and grouping) or the default view again (Reset Folders). Each asks
//! first, acts at once in every window, and leaves Folder Options open, as
//! in Windows. The app has no folder types, so Windows' "all folders of
//! this type" is every folder. Apply to Folders needs a folder: from This
//! PC, Network, Recent, search results or the Recycle Bin it cannot be
//! pressed. Then the advanced settings, in Windows' order and tree.
//!
//! Search: "Always search file names and contents" starts every search
//! with the search strip's "Names and contents".
//!
//! The options take effect with OK or Apply, and each tab's Restore
//! Defaults sets its choices back. Built on the app's one [`Dialog`], as
//! a window of its own with a title bar, modal to the window it was opened
//! from ([`Dialog::with_title_bar`]). [`super::explorer_options`] does what the options that only Folder
//! Options sets mean in a window.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::i18n::{gettext, gettext_static};
use ox_core::location::{is_virtual_location, same_location, RECENT_LOCATIONS_URI, RECENT_URI};
use ox_core::settings::{Preferences, PreferencesUpdate};

use super::{BrowserWindow, ButtonStyle};
use crate::dialog::{Dialog, DialogButton};
use crate::i18n::message_id;
use crate::locations::Page;

/// The question Apply to Folders asks, Windows' without "of this type".
const APPLY_QUESTION: &str = message_id("Do you want all folders to match this folder's view settings?");

/// The question Reset Folders asks.
const RESET_QUESTION: &str = message_id("Do you want to reset all folders to their default view settings?");

/// Why Click items as follows offers only Double-click.
const NO_SINGLE_CLICK: &str = message_id("OpenXplorer opens items with a double-click.");

/// One of Windows' options that the app has too: a check box, or a choice
/// of two.
struct Setting {
    /// Windows' wording: the check box, or the heading over the choices.
    label: &'static str,
    /// The two choices in Windows' order, each with the value it means;
    /// `None` for a check box.
    choices: Option<[(&'static str, bool); 2]>,
    read: fn(&Preferences) -> bool,
    /// Writes the choice into an update of `Preferences`; an option that
    /// shares a preference with another reads what the update already
    /// holds.
    write: fn(&mut PreferencesUpdate, &Preferences, bool),
}

/// A check box.
const fn check(
    label: &'static str,
    read: fn(&Preferences) -> bool,
    write: fn(&mut PreferencesUpdate, &Preferences, bool),
) -> Setting {
    Setting {
        label,
        choices: None,
        read,
        write,
    }
}

/// The tab an option is on; each tab's Restore Defaults sets its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    View,
    Search,
}

/// Browse folders, on the General tab.
fn browse_folders() -> Setting {
    Setting {
        label: message_id("Browse folders"),
        choices: Some([
            (message_id("Open each folder in the same window"), false),
            (message_id("Open each folder in its own window"), true),
        ]),
        read: |preferences| preferences.open_folders_in_own_window,
        write: |update, _, on| update.open_folders_in_own_window = Some(on),
    }
}

/// Privacy's check boxes, on the General tab: whether the navigation pane
/// shows Recent files and Recent locations.
fn privacy_settings() -> [Setting; 2] {
    [
        check(
            message_id("Show recently used files"),
            |preferences| shows_place(preferences, RECENT_URI),
            |update, preferences, on| show_place(update, preferences, RECENT_URI, on),
        ),
        check(
            message_id("Show frequently used folders"),
            |preferences| shows_place(preferences, RECENT_LOCATIONS_URI),
            |update, preferences, on| show_place(update, preferences, RECENT_LOCATIONS_URI, on),
        ),
    ]
}

/// Whether the navigation pane shows the place `uri`.
fn shows_place(preferences: &Preferences, uri: &str) -> bool {
    !preferences
        .hidden_sidebar_places
        .iter()
        .any(|place| same_location(place, uri))
}

/// Shows or hides the place `uri` in the navigation pane, keeping the
/// other places `update` or `preferences` hide.
fn show_place(update: &mut PreferencesUpdate, preferences: &Preferences, uri: &str, shown: bool) {
    let mut places = update
        .hidden_sidebar_places
        .clone()
        .unwrap_or_else(|| preferences.hidden_sidebar_places.clone());
    places.retain(|place| !same_location(place, uri));
    if !shown {
        places.push(uri.to_owned());
    }
    update.hidden_sidebar_places = Some(places);
}

/// The advanced settings, in Windows' order, under its "Files and
/// Folders".
fn advanced_settings() -> [Setting; 11] {
    [
        // The opposite of Settings' "Show previews".
        check(
            message_id("Always show icons, never thumbnails"),
            |preferences| !preferences.view_options.show_previews,
            |update, preferences, on| {
                let mut options = preferences.view_options.clone();
                options.show_previews = !on;
                update.view_options = Some(options);
            },
        ),
        check(
            message_id("Display the full path in the title bar"),
            |preferences| preferences.full_path_in_title,
            |update, _, on| update.full_path_in_title = Some(on),
        ),
        // View > Show > Hidden items and Ctrl+H change it too (VIEW-023).
        Setting {
            label: message_id("Hidden files and folders"),
            choices: Some([
                (message_id("Don't show hidden files, folders, or drives"), false),
                (message_id("Show hidden files, folders, and drives"), true),
            ]),
            read: |preferences| preferences.show_hidden,
            write: |update, _, on| update.show_hidden = Some(on),
        },
        // The opposite of View > Show > File name extensions (VIEW-069),
        // checked by default as in Windows.
        check(
            message_id("Hide extensions for known file types"),
            |preferences| !preferences.show_extensions,
            |update, _, on| update.show_extensions = Some(!on),
        ),
        check(
            message_id("Remember each folder's view settings"),
            |preferences| preferences.per_folder_views,
            |update, _, on| update.per_folder_views = Some(on),
        ),
        check(
            message_id("Restore previous folder windows at logon"),
            |preferences| preferences.restore_session,
            |update, _, on| update.restore_session = Some(on),
        ),
        // The items' infotips.
        check(
            message_id("Show pop-up description for folder and desktop items"),
            |preferences| !preferences.hide_infotips,
            |update, _, on| update.hide_infotips = Some(!on),
        ),
        check(
            message_id("Show preview handlers in preview pane"),
            |preferences| !preferences.hide_preview_handlers,
            |update, _, on| update.hide_preview_handlers = Some(!on),
        ),
        check(
            message_id("Show status bar"),
            |preferences| !preferences.hide_status_bar,
            |update, _, on| update.hide_status_bar = Some(!on),
        ),
        // View > Show > Item check boxes changes it too (SEL-014).
        check(
            message_id("Use check boxes to select items"),
            |preferences| preferences.selection_marker,
            |update, _, on| update.selection_marker = Some(on),
        ),
        Setting {
            label: message_id("When typing into list view"),
            choices: Some([
                (message_id("Automatically type into the Search Box"), true),
                (message_id("Select the typed item in the view"), false),
            ]),
            read: |preferences| preferences.type_into_search_box,
            write: |update, _, on| update.type_into_search_box = Some(on),
        },
    ]
}

/// The Search tab's option, under "When searching non-indexed locations".
fn search_settings() -> [Setting; 1] {
    [check(
        message_id("Always search file names and contents (this might take several minutes)"),
        |preferences| preferences.search_contents,
        |update, _, on| update.search_contents = Some(on),
    )]
}

/// An option as Folder Options shows it.
struct ShownSetting {
    setting: Setting,
    tab: Tab,
    /// The check box, or the choice that means on.
    on: gtk::CheckButton,
    /// The choice that means off, for a choice of two.
    off: Option<gtk::CheckButton>,
    /// The value last saved.
    saved: Cell<bool>,
}

impl ShownSetting {
    /// Adds `setting` of `tab` to `parent`, showing `preferences`; the
    /// choices of a choice of two stand under its heading, indented, as
    /// in Windows.
    fn add(setting: Setting, tab: Tab, parent: &gtk::Box, preferences: &Preferences) -> Self {
        let value = (setting.read)(preferences);
        let (on, off) = match setting.choices {
            None => {
                let check = option_check(gettext_static(setting.label));
                parent.append(&check);
                (check, None)
            }
            Some(choices) => {
                // Windows titles a group box on the General tab with the
                // heading, and a node of the View tab's tree.
                let heading = match tab {
                    Tab::General => group_caption(gettext_static(setting.label)),
                    Tab::View | Tab::Search => option_heading(gettext_static(setting.label)),
                };
                parent.append(&heading);
                let buttons = choices.map(|(label, _)| {
                    let button = option_check(gettext_static(label));
                    button.set_margin_start(CHOICE_INDENT);
                    button
                        .update_relation(&[gtk::accessible::Relation::DescribedBy(&[heading.upcast_ref()])]);
                    parent.append(&button);
                    button
                });
                buttons[1].set_group(Some(&buttons[0]));
                let [first, second] = buttons;
                let (on, off) = if choices[0].1 {
                    (first, second)
                } else {
                    (second, first)
                };
                (on, Some(off))
            }
        };
        let shown = Self {
            setting,
            tab,
            on,
            off,
            saved: Cell::new(value),
        };
        shown.show(value);
        shown
    }

    /// The value chosen.
    fn value(&self) -> bool {
        self.on.is_active()
    }

    /// Shows `value`: checks the box, or chooses the choice that means it.
    fn show(&self, value: bool) {
        match (&self.off, value) {
            (Some(off), false) => off.set_active(true),
            _ => self.on.set_active(value),
        }
    }
}

/// How far a choice stands in from its heading, as in Windows' tree.
const CHOICE_INDENT: i32 = 22;

/// A check box or a choice of Folder Options, closer together than a
/// dialog's check boxes, as Windows lists them.
fn option_check(label: &str) -> gtk::CheckButton {
    gtk::CheckButton::builder()
        .label(label)
        .css_classes(["option-row"])
        .build()
}

/// The heading over a choice of two.
fn option_heading(label: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .css_classes(["option-heading"])
        .build()
}

/// A group's caption on the General and Search tabs, as Windows' group
/// boxes are titled.
fn group_caption(label: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .css_classes(["field-label"])
        .build()
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
    settings: Vec<ShownSetting>,
    start: gtk::DropDown,
    places: Vec<Option<String>>,
    saved_start: Cell<u32>,
    /// "More settings…" on the General tab, which opens Settings.
    more_settings: gtk::Button,
    /// Privacy's Clear, which clears File Explorer's history at once.
    clear_history: gtk::Button,
}

impl OptionsForm {
    /// The choices, showing `preferences`, on the tabs `general`, `view`
    /// and `search`.
    fn add_to(general: &gtk::Box, view: &gtk::Box, search: &gtk::Box, preferences: &Preferences) -> Rc<Self> {
        let mut settings = Vec::new();
        let (start, places) = open_file_explorer_to(general, preferences);
        let browse = gtk::Box::new(gtk::Orientation::Vertical, 0);
        settings.push(ShownSetting::add(
            browse_folders(),
            Tab::General,
            &browse,
            preferences,
        ));
        general.append(&browse);
        general.append(&click_items_group());
        let privacy = gtk::Box::new(gtk::Orientation::Vertical, 0);
        privacy.append(&group_caption(&gettext("Privacy")));
        for setting in privacy_settings() {
            settings.push(ShownSetting::add(setting, Tab::General, &privacy, preferences));
        }
        let clear_history = tab_button(&gettext("Clear"));
        privacy.append(&clear_row(&clear_history));
        general.append(&privacy);
        let restore_general = restore_defaults_button();
        let more_settings = tab_button(&gettext("More settings…"));
        general.append(&general_footer(&more_settings, &restore_general));

        let caption = gtk::Label::builder()
            .label(gettext("Advanced settings:"))
            .xalign(0.0)
            .build();
        view.append(&caption);
        let tree = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tree.set_margin_top(6);
        tree.set_margin_bottom(8);
        tree.set_margin_start(8);
        tree.set_margin_end(8);
        tree.append(&option_heading(&gettext("Files and Folders")));
        let files_and_folders = gtk::Box::new(gtk::Orientation::Vertical, 0);
        files_and_folders.set_margin_start(CHOICE_INDENT);
        for setting in advanced_settings() {
            settings.push(ShownSetting::add(
                setting,
                Tab::View,
                &files_and_folders,
                preferences,
            ));
        }
        tree.append(&files_and_folders);
        tree.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
        view.append(&advanced_list(&tree));
        let restore_view = restore_defaults_button();
        view.append(&restore_view);

        search.append(&group_caption(&gettext("When searching non-indexed locations")));
        for setting in search_settings() {
            settings.push(ShownSetting::add(setting, Tab::Search, search, preferences));
        }
        let restore_search = restore_defaults_button();
        search.append(&restore_search);

        let form = Rc::new(Self {
            settings,
            saved_start: Cell::new(start.selected()),
            start,
            places,
            more_settings,
            clear_history,
        });
        for (button, tab) in [
            (restore_general, Tab::General),
            (restore_view, Tab::View),
            (restore_search, Tab::Search),
        ] {
            button.connect_clicked(glib::clone!(
                #[weak]
                form,
                move |_| form.restore_defaults(tab)
            ));
        }
        form
    }

    /// Sets every choice of `tab` to the app's default, to be saved with
    /// OK or Apply.
    fn restore_defaults(&self, tab: Tab) {
        let defaults = Preferences::default();
        for shown in self.settings.iter().filter(|shown| shown.tab == tab) {
            shown.show((shown.setting.read)(&defaults));
        }
        if tab == Tab::General {
            self.start.set_selected(0);
        }
    }

    /// The change the choices make to `preferences`, if any; it counts as
    /// saved from now on.
    fn take_change(&self, preferences: &Preferences) -> Option<PreferencesUpdate> {
        let mut update = PreferencesUpdate::default();
        let mut changed = false;
        for shown in &self.settings {
            let now = shown.value();
            if now != shown.saved.get() {
                (shown.setting.write)(&mut update, preferences, now);
                shown.saved.set(now);
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
        self.settings
            .iter()
            .any(|shown| shown.value() != shown.saved.get())
            || self.start.selected() != self.saved_start.get()
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
        for shown in &self.settings {
            let update = Rc::clone(&update);
            shown.on.connect_toggled(move |_| update());
        }
        self.start.connect_selected_notify(move |_| update());
    }
}

/// "Open File Explorer to:" and its list of start places on `general`,
/// showing the one `preferences` choose.
fn open_file_explorer_to(
    general: &gtk::Box,
    preferences: &Preferences,
) -> (gtk::DropDown, Vec<Option<String>>) {
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
    (start, places)
}

/// Click items as follows, as Windows lists it, where only Double-click
/// can be chosen: the file list has no single-click mode (SEL-018), so
/// Single-click and its underline choices are greyed out and say why.
fn click_items_group() -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    group.append(&group_caption(&gettext("Click items as follows")));
    let single = option_check(&gettext("Single-click to open an item (point to select)"));
    let consistent = option_check(&gettext("Underline icon titles consistent with my browser"));
    let on_point = option_check(&gettext("Underline icon titles only when I point at them"));
    let double = option_check(&gettext("Double-click to open an item (single-click to select)"));
    on_point.set_group(Some(&consistent));
    on_point.set_active(true);
    double.set_group(Some(&single));
    double.set_active(true);
    let reason = gettext_static(NO_SINGLE_CLICK);
    for underline in [&consistent, &on_point] {
        underline.set_margin_start(CHOICE_INDENT);
    }
    for unavailable in [&single, &consistent, &on_point] {
        unavailable.set_sensitive(false);
        unavailable.set_tooltip_text(Some(reason));
    }
    for button in [&single, &consistent, &on_point, &double] {
        group.append(button);
    }
    group
}

/// "Clear File Explorer history" and its Clear button, on one line.
fn clear_row(clear: &gtk::Button) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_top(8);
    let label = gtk::Label::builder()
        .label(gettext("Clear File Explorer history"))
        .xalign(0.0)
        .hexpand(true)
        .mnemonic_widget(clear)
        .build();
    clear.update_relation(&[gtk::accessible::Relation::DescribedBy(&[label.upcast_ref()])]);
    row.append(&label);
    row.append(clear);
    row
}

/// The Advanced settings list, scrolled in a border as Windows draws its
/// tree.
fn advanced_list(tree: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(tree)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(ADVANCED_LIST_HEIGHT)
        .vexpand(true)
        .css_classes(["advanced-settings"])
        .build()
}

/// The Advanced settings list's height, about ten lines, as in Windows.
const ADVANCED_LIST_HEIGHT: i32 = 260;

/// A push button in a tab, drawn as the dialog's own bordered buttons
/// (Cancel, Apply), as Windows draws Apply to Folders and Reset Folders.
fn tab_button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class(ButtonStyle::Bordered.css_class());
    button.add_css_class("dialog-button");
    button
}

/// A tab's Restore Defaults, at its bottom right as in Windows.
fn restore_defaults_button() -> gtk::Button {
    let button = tab_button(&gettext("Restore Defaults"));
    button.set_halign(gtk::Align::End);
    button.set_valign(gtk::Align::End);
    button.set_vexpand(true);
    button
}

/// The foot of the General tab: "More settings…" at the left, which
/// opens the rest of the app's settings, and Restore Defaults at the
/// right, at the bottom of the tab.
fn general_footer(more_settings: &gtk::Button, restore: &gtk::Button) -> gtk::Box {
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.set_valign(gtk::Align::End);
    footer.set_vexpand(true);
    more_settings.set_halign(gtk::Align::Start);
    more_settings.set_hexpand(true);
    more_settings.set_tooltip_text(Some(&gettext("Open OpenXplorer's Settings")));
    restore.set_vexpand(false);
    footer.append(more_settings);
    footer.append(restore);
    footer
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
        let options = Dialog::with_title_bar(self, &gettext("Folder Options"), "");
        let source = self
            .current_uri()
            .is_some_and(|uri| is_view_source(&uri, self.is_searching()));
        let tabs = gtk::Notebook::new();
        let general = tab_page();
        let view = tab_page();
        let search = tab_page();
        self.add_folder_views(&options, &view, source);
        let preferences = self.context().settings_data().preferences;
        let form = OptionsForm::add_to(&general, &view, &search, &preferences);
        tabs.append_page(&general, Some(&gtk::Label::new(Some(&gettext("General")))));
        tabs.append_page(&view, Some(&gtk::Label::new(Some(&gettext("View")))));
        tabs.append_page(&search, Some(&gtk::Label::new(Some(&gettext("Search")))));
        options.add_field(&tabs);
        let ok = options.add_button(&gettext("OK"), ButtonStyle::Accent);
        options.add_cancel_button();
        let apply = options.add_button(&gettext("Apply"), ButtonStyle::Bordered);
        form.follow_with(&options, apply);
        self.open_settings_from(&options, &form);
        form.clear_history.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.clear_explorer_history()
        ));
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

    /// Makes `form`'s "More settings…" keep the choices made, as OK does,
    /// close Folder Options and open Settings. Settings left the "…" menu
    /// for this button (CMD-007); Ctrl+, opens it too.
    fn open_settings_from(&self, options: &Dialog, form: &Rc<OptionsForm>) {
        form.more_settings.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            options,
            #[weak]
            form,
            move |_| {
                let preferences = window.context().settings_data().preferences;
                if let Some(update) = form.take_change(&preferences) {
                    window
                        .context()
                        .update_preferences(update, window.preference_failure_reply());
                }
                options.close();
                window.open_settings(None);
            }
        ));
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
        let apply = tab_button(&gettext("Apply to Folders"));
        apply.set_sensitive(source);
        let reset = tab_button(&gettext("Reset Folders"));
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

    /// Every option of every tab writes the preference it reads, and the
    /// two Privacy check boxes, which share the navigation pane's hidden
    /// places, keep each other's choice and the places hidden before.
    ///
    /// parity: VIEW-020
    #[test]
    fn every_option_reads_and_writes_its_preference() {
        let settings = std::iter::once(browse_folders())
            .chain(privacy_settings())
            .chain(advanced_settings())
            .chain(search_settings());
        for setting in settings {
            for on in [true, false] {
                let mut update = PreferencesUpdate::default();
                (setting.write)(&mut update, &Preferences::default(), on);
                let mut preferences = Preferences::default();
                preferences.apply(&update);
                assert_eq!((setting.read)(&preferences), on, "{}", setting.label);
            }
        }

        let mut preferences = Preferences {
            hidden_sidebar_places: vec!["file:///media/Example%20disk".to_owned()],
            ..Preferences::default()
        };
        let mut update = PreferencesUpdate::default();
        let [files, folders] = privacy_settings();
        (files.write)(&mut update, &preferences, false);
        (folders.write)(&mut update, &preferences, false);
        preferences.apply(&update);
        assert!(!(files.read)(&preferences) && !(folders.read)(&preferences));
        assert!(preferences
            .hidden_sidebar_places
            .contains(&"file:///media/Example%20disk".to_owned()));
        let mut update = PreferencesUpdate::default();
        (files.write)(&mut update, &preferences, true);
        preferences.apply(&update);
        assert!((files.read)(&preferences) && !(folders.read)(&preferences));
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
