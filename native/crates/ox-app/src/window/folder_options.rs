// SPDX-License-Identifier: AGPL-3.0-only
//! Folder Options, as "…" > Options opens it in Windows 11 File Explorer,
//! with its General, View and Search tabs (VIEW-020; it replaces Dolphin's
//! display style dialog, VIEW-021). Each option is on the tab Windows puts
//! it on, in Windows' words, and only the options the app can honour are
//! there.
//!
//! General: "Open `OpenXplorer` to" (Windows' "Open File Explorer to",
//! named for the app), the start places the app has (Home,
//! This PC, or a folder chosen in Settings); Browse folders, the same
//! window or a window of each folder's own; Click items as follows, where
//! only Double-click can be chosen, as the app has no single-click mode
//! (SEL-018); Privacy, whether the navigation pane shows Recent files
//! ("Show recently used files") and Recent locations ("Show frequently
//! used folders"), and Clear `OpenXplorer` history (Windows' "Clear File
//! Explorer history", named for the app), which forgets them and the
//! typed addresses at once. The rest of the app's options are in Settings,
//! which the command bar's gear and Ctrl+, open (CMD-007).
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
//! Defaults sets its choices back. It is a window of its own, modal to
//! the window it was opened from alone, laid out as Windows 11's: about
//! 380 by 483 pixels, the Properties window's classic tabs, group boxes
//! with a picture at their left, and OK, Cancel and Apply
//! ([`PropertiesWindow`]). [`super::explorer_options`] does what the options that only Folder
//! Options sets mean in a window.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::i18n::{gettext, gettext_static};
use ox_core::location::{is_virtual_location, same_location, RECENT_LOCATIONS_URI, RECENT_URI};
use ox_core::settings::{Preferences, PreferencesUpdate};

use super::{BrowserWindow, ButtonStyle};
use crate::dialog::Dialog;
use crate::i18n::message_id;
use crate::icons::Icon;
use crate::locations::Page;
use crate::properties::{ApplyFuture, Pending, PropertiesWindow};

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
    /// Adds `setting` of `tab` to `parent`, showing `preferences`. On the
    /// View tab a choice of two stands under its heading, indented, as a
    /// node of Windows' tree; on the General tab the group box it is in
    /// is its heading, as in Windows.
    fn add(setting: Setting, tab: Tab, parent: &gtk::Box, preferences: &Preferences) -> Self {
        let value = (setting.read)(preferences);
        let (on, off) = match setting.choices {
            None => {
                let check = option_check(gettext_static(setting.label));
                parent.append(&check);
                (check, None)
            }
            Some(choices) => {
                let heading = (tab != Tab::General).then(|| {
                    let heading = option_heading(gettext_static(setting.label));
                    parent.append(&heading);
                    heading
                });
                let buttons = choices.map(|(label, _)| {
                    let button = option_check(gettext_static(label));
                    if let Some(heading) = &heading {
                        button.set_margin_start(CHOICE_INDENT);
                        button.update_relation(&[gtk::accessible::Relation::DescribedBy(&[
                            heading.upcast_ref()
                        ])]);
                    }
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

/// The width of Folder Options, as Windows 11 draws it at 100 %: about
/// 380 by 483 pixels with its title bar, its pages the standard property
/// sheet's 227 by 215 dialog units.
const WINDOW_WIDTH: i32 = 380;

/// How far a choice stands in from its heading, as in Windows.
const CHOICE_INDENT: i32 = 16;

/// The size of a group box's picture, as Windows draws it.
const GROUP_ART: i32 = 32;

/// A check box or a choice of Folder Options, close together, as Windows
/// lists them.
fn option_check(label: &str) -> gtk::CheckButton {
    let check = gtk::CheckButton::builder()
        .label(label)
        .css_classes(["option-row"])
        .build();
    // A long option wraps, as in Windows, rather than widening the
    // window past Windows' width.
    if let Some(text) = check.last_child().and_downcast::<gtk::Label>() {
        wrap(&text);
    }
    check
}

/// Lets `label` wrap at the width it is given, asking for no more.
fn wrap(label: &gtk::Label) {
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(1);
    label.set_hexpand(true);
    label.set_xalign(0.0);
}

/// The heading over a choice of two.
fn option_heading(label: &str) -> gtk::Label {
    let heading = gtk::Label::builder()
        .label(label)
        .css_classes(["option-heading"])
        .build();
    wrap(&heading);
    heading
}

/// A group box titled `title` with `icon` at its left, as Windows draws
/// Folder Options' groups, and the column its controls go in.
fn icon_group(title: &str, icon: Icon) -> (gtk::Frame, gtk::Box) {
    let (frame, content) = options_group(title);
    content.set_orientation(gtk::Orientation::Horizontal);
    content.add_css_class("options-group-content");
    let art = crate::icons::image(icon, GROUP_ART);
    art.set_valign(gtk::Align::Start);
    art.add_css_class("options-group-art");
    content.append(&art);
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .css_classes(["options-group-column"])
        .build();
    content.append(&column);
    (frame, column)
}

/// A group box titled `title` without a picture, as the Search tab's.
fn plain_group(title: &str) -> (gtk::Frame, gtk::Box) {
    let (frame, content) = options_group(title);
    content.add_css_class("options-group-content");
    (frame, content)
}

/// A Properties group box whose title is drawn on its top line, as
/// Windows draws one: the title is lifted out of the box by the
/// stylesheet, which the frame then must not clip.
fn options_group(title: &str) -> (gtk::Frame, gtk::Box) {
    let (frame, content) = crate::properties::group(title);
    frame.set_overflow(gtk::Overflow::Visible);
    (frame, content)
}

/// Whether Apply to Folders can take the view of `uri`: a folder, local,
/// on a share or in a ZIP, not a page such as This PC, the Recycle Bin,
/// Recent or search results. The folder's location is not
/// saved.
fn is_view_source(uri: &str, searching: bool) -> bool {
    !searching && !uri.is_empty() && !is_virtual_location(uri)
}

/// The start places "Open `OpenXplorer` to" offers: Home (`None`), This
/// PC, and the place chosen in Settings when it is another one.
fn start_places(chosen: Option<&str>) -> Vec<Option<String>> {
    let mut places = vec![None, Some(Page::ThisPc.uri().to_owned())];
    if let Some(chosen) = chosen.filter(|chosen| !places.contains(&Some((*chosen).to_owned()))) {
        places.push(Some(chosen.to_owned()));
    }
    places
}

/// The name "Open `OpenXplorer` to" shows for `place`.
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
    /// Privacy's Clear, which clears File Explorer's history at once.
    clear_history: gtk::Button,
}

impl OptionsForm {
    /// The choices, showing `preferences`, on the tabs `general`, `view`
    /// and `search`, laid out as Windows 11 lays out Folder Options.
    fn add_to(general: &gtk::Box, view: &gtk::Box, search: &gtk::Box, preferences: &Preferences) -> Rc<Self> {
        let mut settings = Vec::new();
        let (start, places) = open_file_explorer_to(general, preferences);

        let (browse, column) = icon_group(&gettext("Browse folders"), Icon::WindowMultiple);
        settings.push(ShownSetting::add(
            browse_folders(),
            Tab::General,
            &column,
            preferences,
        ));
        general.append(&browse);
        general.append(&click_items_group());
        let (privacy, column) = icon_group(&gettext("Privacy"), Icon::ShieldLock);
        for setting in privacy_settings() {
            settings.push(ShownSetting::add(setting, Tab::General, &column, preferences));
        }
        let clear_history = push_button(&gettext("Clear"));
        column.append(&clear_row(&clear_history));
        general.append(&privacy);
        let restore_general = push_button(&gettext("Restore Defaults"));
        general.append(&tab_footer(&restore_general));

        let caption = gtk::Label::builder()
            .label(gettext("Advanced settings:"))
            .xalign(0.0)
            .css_classes(["options-caption"])
            .build();
        view.append(&caption);
        let tree = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tree.add_css_class("advanced-tree");
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
        let restore_view = push_button(&gettext("Restore Defaults"));
        // The list takes the room the tab has, as in Windows.
        let view_footer = tab_footer(&restore_view);
        view_footer.set_vexpand(false);
        view.append(&view_footer);

        let (non_indexed, column) = plain_group(&gettext("When searching non-indexed locations"));
        for setting in search_settings() {
            settings.push(ShownSetting::add(setting, Tab::Search, &column, preferences));
        }
        search.append(&non_indexed);
        let restore_search = push_button(&gettext("Restore Defaults"));
        search.append(&tab_footer(&restore_search));

        let form = Rc::new(Self {
            settings,
            saved_start: Cell::new(start.selected()),
            start,
            places,
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

    /// Lets `options` grey out Apply while nothing is changed, as Windows
    /// does.
    fn follow_with(&self, options: &PropertiesWindow) {
        let update = glib::clone!(
            #[weak]
            options,
            move || options.changed()
        );
        let update = Rc::new(update);
        for shown in &self.settings {
            let update = Rc::clone(&update);
            shown.on.connect_toggled(move |_| update());
        }
        self.start.connect_selected_notify(move |_| update());
    }
}

/// What OK and Apply save: the choices changed in Folder Options.
struct OptionsChange {
    window: glib::WeakRef<BrowserWindow>,
    form: Rc<OptionsForm>,
}

impl Pending for OptionsChange {
    fn has_changes(&self) -> bool {
        self.form.is_changed()
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move {
            if let Some(window) = self.window.upgrade() {
                window.save_options(&self.form);
            }
            Ok(())
        })
    }
}

/// "Open `OpenXplorer` to:" and its list of start places on `general`,
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
    start.add_css_class("properties-combo");
    let chosen = places
        .iter()
        .position(|place| *place == preferences.startup_folder)
        .unwrap_or(0);
    start.set_selected(u32::try_from(chosen).unwrap_or(0));
    let caption = gtk::Label::builder()
        .label(gettext("Open OpenXplorer to:"))
        .xalign(0.0)
        .mnemonic_widget(&start)
        .build();
    start.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
    let open_to = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    open_to.add_css_class("options-open-to");
    open_to.append(&caption);
    open_to.append(&start);
    general.append(&open_to);
    (start, places)
}

/// Click items as follows, as Windows lists it, where only Double-click
/// can be chosen: the file list has no single-click mode (SEL-018), so
/// Single-click and its underline choices are greyed out and say why.
fn click_items_group() -> gtk::Frame {
    let (group, column) = icon_group(&gettext("Click items as follows"), Icon::Open);
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
        column.append(button);
    }
    group
}

/// "Clear `OpenXplorer` history" and its Clear button, on one line.
fn clear_row(clear: &gtk::Button) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("options-clear-row");
    let label = gtk::Label::builder()
        .label(gettext("Clear OpenXplorer history"))
        .xalign(0.0)
        .hexpand(true)
        .mnemonic_widget(clear)
        .build();
    clear.update_relation(&[gtk::accessible::Relation::DescribedBy(&[label.upcast_ref()])]);
    row.append(&label);
    row.append(clear);
    row
}

/// The Advanced settings list, scrolled in a sunken border as Windows
/// draws its tree, taking the room the tab has.
fn advanced_list(tree: &gtk::Box) -> gtk::Frame {
    let scrolled = gtk::ScrolledWindow::builder()
        .child(tree)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(ADVANCED_LIST_HEIGHT)
        .vexpand(true)
        .build();
    gtk::Frame::builder()
        .child(&scrolled)
        .vexpand(true)
        .css_classes(["properties-list-frame", "advanced-settings"])
        .build()
}

/// The Advanced settings list's least height; it takes the rest of the
/// tab, as tall as the General tab makes it.
const ADVANCED_LIST_HEIGHT: i32 = 120;

/// A Windows push button in a tab, as Apply to Folders and Clear.
fn push_button(label: &str) -> gtk::Button {
    crate::properties::push_button(label)
}

/// The foot of a tab: Restore Defaults at its bottom right, as in
/// Windows.
fn tab_footer(restore: &gtk::Button) -> gtk::Box {
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.add_css_class("options-tab-footer");
    footer.set_valign(gtk::Align::End);
    footer.set_vexpand(true);
    restore.set_halign(gtk::Align::End);
    restore.set_hexpand(true);
    footer.append(restore);
    footer
}

impl BrowserWindow {
    /// Opens Folder Options for the folder shown: a window of its own with
    /// a title bar, modal to this window alone, as in Windows 11.
    pub(super) fn show_folder_options(&self) {
        let options = PropertiesWindow::new(&gettext("Folder Options"), None, Some(self));
        options.add_css_class("ox-folder-options");
        options.set_window_width(WINDOW_WIDTH);
        options.set_transient_for(Some(self));
        options.set_modal(true);
        options.set_destroy_with_parent(true);
        let source = self
            .current_uri()
            .is_some_and(|uri| is_view_source(&uri, self.is_searching()));
        let general = crate::properties::page();
        let view = crate::properties::page();
        let search = crate::properties::page();
        self.add_folder_views(&options, &view, source);
        let preferences = self.context().settings_data().preferences;
        let form = OptionsForm::add_to(&general, &view, &search, &preferences);
        options.add_tab("general", &gettext("General"), &general);
        options.add_tab("view", &gettext("View"), &view);
        options.add_tab("search", &gettext("Search"), &search);
        options.add_pending(Rc::new(OptionsChange {
            window: self.downgrade(),
            form: Rc::clone(&form),
        }));
        form.follow_with(&options);
        form.clear_history.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.clear_explorer_history()
        ));
        self.quiet_for_dialog();
        options.present();
    }

    /// Saves the choices changed in `form`, as OK and Apply do.
    fn save_options(&self, form: &OptionsForm) {
        let preferences = self.context().settings_data().preferences;
        if let Some(update) = form.take_change(&preferences) {
            self.context()
                .update_preferences(update, self.preference_failure_reply());
        }
    }

    /// Adds the Folder views group box to `view`, whose Apply to Folders
    /// can be pressed when Folder Options was opened from a folder
    /// (`source`).
    fn add_folder_views(&self, options: &PropertiesWindow, view: &gtk::Box, source: bool) {
        let (group, column) = icon_group(&gettext("Folder views"), Icon::Grid);
        let text = gtk::Label::builder()
            .label(gettext(
                "You can apply this view (such as Details or Icons) to all folders.",
            ))
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(1)
            .hexpand(true)
            .build();
        column.append(&text);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.add_css_class("options-folder-views-buttons");
        let apply = push_button(&gettext("Apply to Folders"));
        apply.set_sensitive(source);
        let reset = push_button(&gettext("Reset Folders"));
        buttons.append(&apply);
        buttons.append(&reset);
        column.append(&buttons);
        view.append(&group);
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
    async fn apply_to_folders(&self, options: &PropertiesWindow) {
        if ask_folder_views(options, APPLY_QUESTION).await {
            self.context()
                .apply_view_to_all_folders(self.current_style(), self.preference_failure_reply());
        }
    }

    /// Reset Folders: after Yes, every folder forgets its view and shows
    /// the default one.
    async fn reset_folders(&self, options: &PropertiesWindow) {
        if ask_folder_views(options, RESET_QUESTION).await {
            self.context().reset_folder_views(self.preference_failure_reply());
        }
    }
}

/// Asks `question` over `options` as Windows' Folder Views message box
/// does; true for Yes.
async fn ask_folder_views(options: &PropertiesWindow, question: &str) -> bool {
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

    /// "Open `OpenXplorer` to" offers Home and This PC, and a place chosen
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
