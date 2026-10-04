// SPDX-License-Identifier: AGPL-3.0-only
//! A window that chooses files for another application: its Open or Save
//! dialog, shown as the explorer itself.
//!
//! New in the native app (INT-032). The desktop portal sends an
//! application's file dialog to the app (see
//! [`ox_core::integration::FileChooserBus`]); the app opens a window in
//! picker mode for it. Everything the user browses with is the normal
//! window: the navigation pane, address bar, search, views and file
//! commands. Picker mode adds the bar at the bottom, as Windows' common
//! file dialog has: the file name (for Open and Save), the type list, the
//! caller's extra choices, and the accept and Cancel buttons. It narrows the
//! listing to the chosen type, or to folders when a folder is chosen, and
//! keeps the window to one tab with no Settings.
//!
//! The rules:
//!
//! - **One answer.** The call is answered exactly once: by the accept
//!   button (or activating a file), by Cancel, Escape or closing the
//!   window, or with "other" when the portal closes the dialog.
//! - **Local files only.** The portal accepts `file://` locations, so a
//!   choice is accepted only where GIO has a path for it: local folders,
//!   and shares and devices that `GVfs` mounts.
//! - **Never overwrite silently.** Saving over an existing file asks
//!   first, as every desktop's dialog does.
//! - **As Windows' dialog.** The File name box takes a name, a path from
//!   the folder shown, `~/…` or a full path: a folder opens, a file is
//!   the choice. Save adds the chosen type's extension to a name without
//!   one. A file typed in the address bar is the choice too, a dialog
//!   for one file keeps one selected, Escape and Ctrl+Q cancel, and
//!   nothing opens another window, tab or pane from the dialog.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::integration::{
    checked_name, ChooserAnswer, ChooserCall, ChooserMode, ChooserReply, ChooserRequest, FilterPattern,
};

use super::actions::QUIT_ACCELERATORS;
use super::{BrowserWindow, ButtonStyle, WindowAction};
use crate::dialog::Dialog;
use crate::folder_view::filter::ChooserListing;
use crate::locations::Page;

/// The size a picker window opens at; it never saves its size over the
/// explorer's.
const DEFAULT_SIZE: (i32, i32) = (980, 640);

/// The window commands that make no sense while choosing a file: more tabs,
/// windows or panes, and Settings.
const DISABLED_ACTIONS: [WindowAction; 14] = [
    WindowAction::NewTab,
    WindowAction::OpenTab,
    WindowAction::OpenTabBackground,
    WindowAction::OpenWindow,
    WindowAction::OpenSelectionInTabs,
    WindowAction::ReopenClosedTab,
    WindowAction::RestoreClosedTab,
    WindowAction::MoveTabToNewWindow,
    WindowAction::MoveTabToWindow,
    WindowAction::Settings,
    WindowAction::DefaultFileExplorer,
    WindowAction::OpenFileLocationInTab,
    WindowAction::OpenFileLocationInWindow,
    WindowAction::SplitView,
];

/// Shown when the File name box names nothing that exists.
const NOT_FOUND: &str = crate::i18n::message_id("“{name}” was not found. Check the file name and try again.");

/// Shown when a quoted list in File name names a folder.
const NOT_A_FILE: &str = crate::i18n::message_id("“{name}” is a folder, not a file.");

/// Shown when several files are chosen in a dialog for one.
const CHOOSE_ONE: &str = crate::i18n::message_id("Choose one file.");

/// One of the caller's extra choices and its control.
#[derive(Debug)]
enum ChoiceControl {
    /// A check box; its value is `"true"` or `"false"`.
    Check(String, gtk::CheckButton),
    /// A list; its value is the chosen option's ID.
    List(String, Vec<String>, gtk::DropDown),
}

impl ChoiceControl {
    /// The `(id, value)` pair the answer carries.
    fn value(&self) -> (String, String) {
        match self {
            Self::Check(id, check) => (id.clone(), check.is_active().to_string()),
            Self::List(id, options, list) => {
                let chosen = options.get(list.selected() as usize).cloned().unwrap_or_default();
                (id.clone(), chosen)
            }
        }
    }
}

/// A window's file dialog: the call and the bar's controls.
#[derive(Debug)]
pub(crate) struct Picker {
    request: ChooserRequest,
    reply: ChooserReply,
    /// The File name box of a Save dialog or a dialog that opens files.
    name: Option<gtk::Entry>,
    /// The type list, when the caller gave filters.
    types: Option<gtk::DropDown>,
    choices: Vec<ChoiceControl>,
    accept: gtk::Button,
    /// Set while the replace question is open, so the answer is not given
    /// twice.
    asking: Cell<bool>,
    /// The one item selected, in a dialog that chooses one, so a second
    /// item clicked with Ctrl or Shift takes its place.
    single: Cell<Option<u32>>,
}

impl Picker {
    /// Whether the dialog chooses one item: a file or a folder, not
    /// several.
    fn chooses_one(&self) -> bool {
        match &self.request.mode {
            ChooserMode::Open { multiple, .. } => !multiple,
            ChooserMode::Save { .. } | ChooserMode::SaveFiles { .. } => true,
        }
    }

    /// The extension of the type chosen in the list: that of its first
    /// pattern that is a plain extension, as Windows adds it to a name
    /// saved without one.
    fn chosen_extension(&self) -> Option<String> {
        let filter = self.request.filters.get(self.chosen_filter()?)?;
        filter.patterns.iter().find_map(|pattern| match pattern {
            FilterPattern::Glob(glob) => glob_extension(glob),
            FilterPattern::MimeType(_) => None,
        })
    }

    /// The type chosen in the list, an index into the request's filters.
    fn chosen_filter(&self) -> Option<usize> {
        self.types.as_ref().map(|types| types.selected() as usize)
    }

    /// The listing the window shows for the chosen type.
    fn listing(&self) -> ChooserListing {
        ChooserListing {
            folders_only: self.request.chooses_folder(),
            filter: self
                .chosen_filter()
                .and_then(|index| self.request.filters.get(index).cloned()),
        }
    }

    /// The `(id, value)` pairs of the extra choices.
    fn choice_values(&self) -> Vec<(String, String)> {
        self.choices.iter().map(ChoiceControl::value).collect()
    }
}

/// The local path of `uri`, if GIO has one: a local folder, or a share or
/// device mounted by `GVfs`.
fn local_path(uri: &str) -> Option<PathBuf> {
    if Page::from_uri(uri).is_some() {
        return None;
    }
    gio::File::for_uri(uri).path()
}

/// The folder a dialog opens in: the caller's, if it exists, else the
/// home folder.
fn start_folder(request: &ChooserRequest) -> PathBuf {
    request
        .current_folder
        .clone()
        .filter(|folder| folder.is_dir())
        .unwrap_or_else(glib::home_dir)
}

/// The path the File name box's `typed` text names, from `folder`: a
/// full path or `file:` URI, `~` or `~/…` from the home folder, else a
/// path from the folder shown. `None` for a location of another kind.
fn typed_path(typed: &str, folder: &Path) -> Option<PathBuf> {
    // Joining an absolute path replaces the folder.
    ox_core::location::typed_local_path(typed).map(|path| folder.join(path))
}

/// The extension a type's `glob` stands for: what follows `*.`, letters
/// and digits in parts between dots (`*.txt`, `*.tar.gz`), where a class
/// of one letter in both cases, as Chrome writes them (`*.[tT][xX][tT]`),
/// is that letter. `None` for any other pattern.
fn glob_extension(glob: &str) -> Option<String> {
    let mut extension = String::new();
    let mut rest = glob.strip_prefix("*.")?.chars();
    while let Some(c) = rest.next() {
        if c == '[' {
            let (first, second) = (rest.next()?, rest.next()?);
            let same_letter = first != second && first.to_lowercase().eq(second.to_lowercase());
            if rest.next()? != ']' || !first.is_alphabetic() || !same_letter {
                return None;
            }
            extension.extend(first.to_lowercase());
        } else if c.is_alphanumeric() || c == '.' {
            extension.push(c);
        } else {
            return None;
        }
    }
    let plain = extension.split('.').all(|part| !part.is_empty());
    plain.then_some(extension)
}

/// The name a Save dialog saves `written` as. As in Windows, a name
/// without a dot gets the chosen type's `extension`, and a name that ends
/// in a dot is saved without that dot and without an extension. A hidden
/// file's name (`.bashrc`) is kept as written.
fn saved_name(written: &str, extension: Option<String>) -> String {
    if let Some(name) = written.strip_suffix('.') {
        return name.to_owned();
    }
    match extension {
        Some(extension) if !written.contains('.') => format!("{written}.{extension}"),
        _ => written.to_owned(),
    }
}

impl BrowserWindow {
    /// Turns this new window into the dialog for `call` and shows it.
    pub(crate) fn begin_picking(&self, call: ChooserCall) {
        let ChooserCall { request, reply, .. } = call;
        let start = gio::File::for_path(start_folder(&request)).uri();
        let picker = Rc::new(self.build_picker_bar(request, reply.clone()));
        self.imp().picker.replace(Some(Rc::clone(&picker)));
        reply.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.close()
        ));
        let imp = self.imp();
        // The caller's title takes the tabs' place, as a dialog's caption.
        if let Some(bar) = imp.tab_strip.parent().and_downcast::<gtk::Box>() {
            let title = gtk::Label::builder()
                .label(picker.request.window_title())
                .valign(gtk::Align::Center)
                .css_classes(["picker-title"])
                .build();
            bar.insert_child_after(&title, Some(&*imp.tab_strip));
        }
        imp.tab_strip.set_visible(false);
        imp.new_tab_button.set_visible(false);
        imp.open_windows_button.set_visible(false);
        for action in DISABLED_ACTIONS {
            self.set_action_enabled(action, false);
        }
        // Not modal: with no parent of its own, a modal window would block
        // every other OpenXplorer window while the caller waits.
        self.set_default_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        self.folder_pane().model().set_chooser_listing(picker.listing());
        if self.add_tab(&start).is_err() {
            // The home folder always opens.
            let _ = self.add_tab(&gio::File::for_path(glib::home_dir()).uri());
        }
        self.listen_for_escape();
        self.cancel_on_quit_key();
        self.keep_one_selected();
        self.present();
        match &picker.name {
            Some(name) => {
                // A new window gives its file list the keyboard once the
                // first folder is listed, which ends after this: the name
                // box keeps it instead, so typing replaces the name.
                self.imp().file_list_awaits_focus.set(false);
                name.grab_focus();
                select_stem(name);
            }
            None => self.focus_new_file_list(),
        }
        self.update_picker();
    }

    /// Whether the keyboard is in the File name box, from which Alt+Left,
    /// Alt+Right and Alt+Up move through folders as in Windows' dialog.
    pub(super) fn focus_is_in_picker_name(&self) -> bool {
        let Some(name) = self.picker().and_then(|picker| picker.name.clone()) else {
            return false;
        };
        GtkWindowExt::focus(self)
            .is_some_and(|focus| focus == *name.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&name))
    }

    /// Whether this window is choosing files for another application.
    pub(crate) fn is_picking(&self) -> bool {
        self.imp().picker.borrow().is_some()
    }

    /// The dialog's title while picking.
    pub(super) fn picker_title(&self) -> Option<String> {
        let picker = self.imp().picker.borrow();
        picker.as_ref().map(|picker| picker.request.window_title())
    }

    /// Follows a new location or selection: the accept button, and in
    /// File name the selected files' names. A folder selected in an Open
    /// dialog empties File name, so Open opens the folder rather than the
    /// name typed or selected before.
    pub(super) fn picker_selection_changed(&self) {
        let Some(picker) = self.picker() else {
            return;
        };
        if let Some(name) = &picker.name {
            let selected = self.selected_entries();
            let files: Vec<&Entry> = selected.iter().filter(|entry| !entry.is_dir).collect();
            let opens = matches!(picker.request.mode, ChooserMode::Open { .. });
            if !self.imp().changing_model.get() {
                match (selected.as_slice(), files.as_slice()) {
                    ([entry], [_]) => name.set_text(&entry.name),
                    // Several files: every one, in quotes, as Windows'
                    // dialog lists them, so the box never holds only the
                    // first and wins over the others.
                    (_, [_, _, ..]) => name.set_text(&quoted_names(&files)),
                    ([entry], []) if entry.is_dir && opens => name.set_text(""),
                    _ => {}
                }
            }
        }
        self.update_picker();
    }

    /// A file was activated (double-click, Enter or typed in the address
    /// bar): it is the choice in an Open dialog, and the file to replace,
    /// after asking, in a Save dialog.
    pub(super) fn pick_activated(&self, entry: &Entry) {
        let Some(picker) = self.picker() else {
            return;
        };
        match &picker.request.mode {
            ChooserMode::Open { directory: false, .. } => {
                // The activated file is the choice, selected or not.
                match local_path(&entry.uri) {
                    Some(path) if !picker.asking.get() && !picker.reply.is_answered() => {
                        self.finish_picking(&picker, vec![path]);
                    }
                    Some(_) => {}
                    None => self.show_message(&not_local()),
                }
            }
            ChooserMode::Save { .. } => {
                let Some(path) = local_path(&entry.uri) else {
                    return self.show_message(&not_local());
                };
                if let Some(name) = &picker.name {
                    name.set_text(&entry.name);
                }
                if !picker.asking.get() && !picker.reply.is_answered() {
                    self.confirm_replace(&picker, vec![path], std::slice::from_ref(&entry.name));
                }
            }
            ChooserMode::Open { directory: true, .. } | ChooserMode::SaveFiles { .. } => {}
        }
    }

    /// Open with several items selected accepts them in an Open dialog.
    /// Returns false when the window should open them as usual.
    pub(super) fn pick_selection(&self) -> bool {
        if !self.is_picking() {
            return false;
        }
        self.accept_choice();
        true
    }

    /// Answers Cancelled when the window closes without a choice.
    pub(super) fn end_picking_on_close(&self) {
        if let Some(picker) = self.imp().picker.borrow().as_ref() {
            picker.reply.send(&ChooserAnswer::Cancelled);
        }
    }

    /// The window's picker, if it is one.
    fn picker(&self) -> Option<Rc<Picker>> {
        self.imp().picker.borrow().clone()
    }

    /// The entries selected in the folder.
    fn selected_entries(&self) -> Vec<Entry> {
        self.folder_pane()
            .model()
            .selected_items()
            .iter()
            .map(|item| item.entry().clone())
            .collect()
    }

    /// The current folder's local path, if it has one.
    fn picking_folder(&self) -> Option<PathBuf> {
        local_path(&self.current_uri()?)
    }

    /// Enables the accept button when there is something to accept.
    fn update_picker(&self) {
        let Some(picker) = self.picker() else {
            return;
        };
        let selected = self.selected_entries();
        let folder = self.picking_folder();
        let typed = picker
            .name
            .as_ref()
            .is_some_and(|name| !name.text().trim().is_empty());
        let ready = match &picker.request.mode {
            ChooserMode::Open { directory: false, .. } => {
                typed
                    || selected.iter().any(|entry| !entry.is_dir)
                    || matches!(selected.as_slice(), [entry] if entry.is_dir)
            }
            ChooserMode::Open { directory: true, .. } | ChooserMode::SaveFiles { .. } => {
                folder.is_some()
                    || matches!(selected.as_slice(), [entry] if entry.is_dir && local_path(entry.navigation_uri()).is_some())
            }
            ChooserMode::Save { .. } => {
                folder.is_some()
                    && picker
                        .name
                        .as_ref()
                        .is_some_and(|name| !name.text().trim().is_empty())
            }
        };
        picker.accept.set_sensitive(ready && !picker.asking.get());
    }

    /// The accept button: works out the choice and answers, asking first
    /// before replacing files.
    fn accept_choice(&self) {
        let Some(picker) = self.picker() else {
            return;
        };
        if picker.asking.get() || picker.reply.is_answered() {
            return;
        }
        let selected = self.selected_entries();
        let outcome = match &picker.request.mode {
            ChooserMode::Open {
                directory: false,
                multiple,
            } => match self.typed_choice(&picker, &selected) {
                Some(typed) => typed,
                None => self.chosen_files(&selected, *multiple),
            },
            ChooserMode::Open { directory: true, .. } => {
                self.chosen_folder(&selected).map(|folder| vec![folder])
            }
            ChooserMode::Save { .. } => self.chosen_save(&picker),
            ChooserMode::SaveFiles { names } => self.chosen_folder(&selected).map(|folder| {
                let existing: Vec<String> = names
                    .iter()
                    .filter(|name| folder.join(name).exists())
                    .cloned()
                    .collect();
                if existing.is_empty() {
                    vec![folder]
                } else {
                    self.confirm_replace(&picker, vec![folder], &existing);
                    Vec::new()
                }
            }),
        };
        match outcome {
            Ok(locations) if !locations.is_empty() => self.finish_picking(&picker, locations),
            Err(message) if !message.is_empty() => self.show_message(&message),
            // Nothing to answer yet: a folder opened, or a question is open.
            Ok(_) | Err(_) => {}
        }
    }

    /// The files of an Open dialog. A single selected folder opens instead.
    fn chosen_files(&self, selected: &[Entry], multiple: bool) -> Result<Vec<PathBuf>, String> {
        if let [entry] = selected {
            if entry.is_dir {
                self.navigate_or_report(entry.navigation_uri());
                return Ok(Vec::new());
            }
        }
        let files: Vec<&Entry> = selected.iter().filter(|entry| !entry.is_dir).collect();
        if files.is_empty() {
            return Ok(Vec::new());
        }
        if files.len() > 1 && !multiple {
            return Err(ox_core::i18n::gettext(CHOOSE_ONE));
        }
        files
            .iter()
            .map(|entry| local_path(&entry.uri).ok_or_else(not_local))
            .collect()
    }

    /// The folder of a folder dialog or `SaveFiles`: the one selected
    /// folder, else the current folder.
    fn chosen_folder(&self, selected: &[Entry]) -> Result<PathBuf, String> {
        if let [entry] = selected {
            if entry.is_dir {
                return local_path(entry.navigation_uri()).ok_or_else(not_local);
            }
        }
        self.picking_folder().ok_or_else(not_local)
    }

    /// The file of a Save dialog: the name in the current folder. A name
    /// of a folder there opens that folder; an existing file is replaced
    /// only after asking.
    fn chosen_save(&self, picker: &Rc<Picker>) -> Result<Vec<PathBuf>, String> {
        let Some(name_box) = &picker.name else {
            return Ok(Vec::new());
        };
        let typed = name_box.text();
        // Enter with no name does what the Save button, off then, does.
        if typed.trim().is_empty() {
            return Ok(Vec::new());
        }
        let shown = self.picking_folder().ok_or_else(not_local)?;
        let path = typed_path(&typed, &shown).ok_or_else(not_local)?;
        if path.is_dir() {
            name_box.set_text("");
            self.navigate_or_report(&gio::File::for_path(&path).uri());
            return Ok(Vec::new());
        }
        let missing = |folder: &Path| {
            ox_core::i18n::format_message(
                "The folder “{folder}” does not exist.",
                &[("folder", &folder.display().to_string())],
            )
        };
        // A path that ends in a slash names a folder, never a file.
        if typed.ends_with('/') {
            return Err(missing(&path));
        }
        let bad_name =
            || ox_core::i18n::format_message("“{name}” is not a valid file name.", &[("name", &typed)]);
        let folder = path.parent().map(Path::to_path_buf).ok_or_else(bad_name)?;
        if !folder.is_dir() {
            return Err(missing(&folder));
        }
        let written = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = checked_name(&saved_name(&written, picker.chosen_extension())).map_err(|_| bad_name())?;
        let target = folder.join(&name);
        if target.is_dir() {
            name_box.set_text("");
            self.navigate_or_report(&gio::File::for_path(&target).uri());
            return Ok(Vec::new());
        }
        if target.exists() {
            self.confirm_replace(picker, vec![target], &[name]);
            return Ok(Vec::new());
        }
        Ok(vec![target])
    }

    /// Asks whether to replace `names`, then answers with `locations`.
    fn confirm_replace(&self, picker: &Rc<Picker>, locations: Vec<PathBuf>, names: &[String]) {
        picker.asking.set(true);
        self.update_picker();
        let (title, question) = match names {
            [name] => (
                "Confirm Save As",
                format!("“{name}” already exists. Do you want to replace it?"),
            ),
            _ => (
                "Confirm Save",
                format!(
                    "{} files already exist in this folder. Replace them?",
                    names.len()
                ),
            ),
        };
        let picker = Rc::clone(picker);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let dialog = Dialog::new(&window, title, &question);
                let replace = dialog.add_button("Replace", ButtonStyle::Accent);
                dialog.add_cancel_button();
                dialog.open();
                let answer = dialog.next_response().await;
                dialog.finish();
                picker.asking.set(false);
                if answer == Some(replace) {
                    window.finish_picking(&picker, locations);
                } else {
                    window.update_picker();
                }
            }
        ));
    }

    /// What the File name box of an Open dialog chooses, when it names
    /// something other than the one file selected: a folder opens, a file
    /// is the choice, and a name that names nothing says so. `None` leaves
    /// the choice to the selection.
    fn typed_choice(&self, picker: &Picker, selected: &[Entry]) -> Option<Result<Vec<PathBuf>, String>> {
        let name_box = picker.name.as_ref()?;
        let typed = name_box.text();
        if typed.trim().is_empty() {
            return None;
        }
        // The selected file's own name: the selection, which a search
        // result's folder belongs to.
        if let [entry] = selected {
            if !entry.is_dir && entry.name == typed.as_str() {
                return None;
            }
        }
        // The selected files' names, as the box lists them: the selection.
        let files: Vec<&Entry> = selected.iter().filter(|entry| !entry.is_dir).collect();
        if files.len() > 1 && quoted_names(&files) == typed.as_str() {
            return None;
        }
        let Some(shown) = self.picking_folder() else {
            return Some(Err(not_local()));
        };
        if let Some(names) = parse_quoted_names(&typed) {
            return Some(typed_files(picker, &shown, &names));
        }
        let Some(path) = typed_path(&typed, &shown) else {
            return Some(Err(not_local()));
        };
        if path.is_dir() {
            name_box.set_text("");
            self.navigate_or_report(&gio::File::for_path(&path).uri());
            return Some(Ok(Vec::new()));
        }
        if path.is_file() {
            return Some(Ok(vec![path]));
        }
        Some(Err(ox_core::i18n::format_message(NOT_FOUND, &[("name", &typed)])))
    }

    /// In a dialog that chooses one item, a second item selected with
    /// Ctrl or Shift takes the first one's place, as Windows' dialogs
    /// select one item.
    fn keep_one_selected(&self) {
        let selection = self.folder_pane().model().selection().clone();
        selection.connect_selection_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |selection, position, count| {
                let Some(picker) = window.picker() else {
                    return;
                };
                if !picker.chooses_one() {
                    return;
                }
                let selected = selection.selection();
                if selected.size() <= 1 {
                    picker
                        .single
                        .set((selected.size() == 1).then(|| selected.minimum()));
                    return;
                }
                // The item just selected: the end of the changed range away
                // from the one kept so far, as Shift+click selects up or
                // down to the item clicked.
                let kept = picker.single.get();
                let last = position + count.saturating_sub(1);
                let newest = if kept.is_some_and(|kept| kept >= last) {
                    position
                } else {
                    last
                };
                let item = Some(newest).filter(|item| selected.contains(*item)).or(kept);
                if let Some(item) = item {
                    picker.single.set(Some(item));
                    selection.select_item(item, true);
                }
            }
        ));
    }

    /// Answers with `locations` and closes the window.
    fn finish_picking(&self, picker: &Picker, locations: Vec<PathBuf>) {
        picker.reply.send(&ChooserAnswer::Chosen {
            locations,
            filter: picker.chosen_filter(),
            choices: picker.choice_values(),
        });
        self.close();
    }

    /// Cancel, Escape, and Ctrl+Q in a dialog.
    pub(super) fn cancel_picking(&self) {
        if let Some(picker) = self.picker() {
            picker.reply.send(&ChooserAnswer::Cancelled);
        }
        self.close();
    }

    /// Escape cancels when nothing in the window used it first (the
    /// address bar, the search box and menus do).
    fn listen_for_escape(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Bubble);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if key == gtk::gdk::Key::Escape && modifiers.is_empty() {
                    window.cancel_picking();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        self.add_controller(keys);
    }

    /// Ctrl+Q cancels the dialog alone, never the other windows. The
    /// window's own key runs before the application's Quit accelerator,
    /// which still quits everything when `openxplorer --quit` asks.
    fn cancel_on_quit_key(&self) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        let trigger = gtk::ShortcutTrigger::parse_string(&QUIT_ACCELERATORS.join("|"));
        let cancel = gtk::CallbackAction::new(|widget, _| {
            if let Some(window) = widget.downcast_ref::<BrowserWindow>() {
                window.cancel_picking();
            }
            glib::Propagation::Stop
        });
        shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(cancel)));
        self.add_controller(shortcuts);
    }

    /// Builds the bar for `request` and puts it under the status bar.
    fn build_picker_bar(&self, request: ChooserRequest, reply: ChooserReply) -> Picker {
        let bar = &self.imp().picker_bar;
        bar.set_visible(true);
        let fields = gtk::Grid::builder()
            .column_spacing(12)
            .row_spacing(8)
            .hexpand(true)
            .build();
        fields.add_css_class("picker-fields");
        let mut row = 0;
        let suggested = match &request.mode {
            ChooserMode::Save { name } => Some(name.clone()),
            ChooserMode::Open { directory: false, .. } => Some(String::new()),
            ChooserMode::Open { directory: true, .. } | ChooserMode::SaveFiles { .. } => None,
        };
        let name = suggested.map(|suggested| {
            let entry = gtk::Entry::builder().text(suggested).hexpand(true).build();
            attach_field(&fields, row, "File name:", &entry);
            row += 1;
            entry
        });
        let types = (!request.filters.is_empty()).then(|| {
            let labels: Vec<&str> = request
                .filters
                .iter()
                .map(|filter| filter.name.as_str())
                .collect();
            let list = gtk::DropDown::from_strings(&labels);
            list.set_selected(u32::try_from(request.current_filter.unwrap_or(0)).unwrap_or(0));
            let caption = if name.is_some() {
                "Save as type:"
            } else {
                "File type:"
            };
            attach_field(&fields, row, caption, &list);
            row += 1;
            list
        });
        let choices = choice_controls(&fields, &mut row, &request);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.add_css_class("picker-buttons");
        buttons.set_valign(gtk::Align::End);
        let accept = gtk::Button::builder()
            .label(request.accept_label())
            .css_classes(["picker-button", ButtonStyle::Accent.css_class()])
            .build();
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .css_classes(["picker-button", ButtonStyle::Bordered.css_class()])
            .build();
        buttons.append(&accept);
        buttons.append(&cancel);
        if row > 0 {
            bar.append(&fields);
        } else {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            bar.append(&spacer);
        }
        bar.append(&buttons);
        self.connect_picker_controls(&accept, &cancel, name.as_ref(), types.as_ref());
        Picker {
            request,
            reply,
            name,
            types,
            choices,
            accept,
            asking: Cell::new(false),
            single: Cell::new(None),
        }
    }

    /// Connects the bar's buttons, name box and type list.
    fn connect_picker_controls(
        &self,
        accept: &gtk::Button,
        cancel: &gtk::Button,
        name: Option<&gtk::Entry>,
        types: Option<&gtk::DropDown>,
    ) {
        accept.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.accept_choice()
        ));
        cancel.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.cancel_picking()
        ));
        if let Some(name) = name {
            name.connect_changed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_| window.update_picker()
            ));
            name.connect_activate(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_| window.accept_choice()
            ));
        }
        if let Some(types) = types {
            types.connect_selected_notify(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_| {
                    if let Some(picker) = window.picker() {
                        window.folder_pane().model().set_chooser_listing(picker.listing());
                        window.update_status();
                    }
                }
            ));
        }
    }
}

/// The controls of the caller's extra choices, from row `row` of
/// `fields` on; `row` moves past them.
fn choice_controls(fields: &gtk::Grid, row: &mut i32, request: &ChooserRequest) -> Vec<ChoiceControl> {
    let mut choices = Vec::new();
    for choice in &request.choices {
        if choice.is_check_box() {
            let check = gtk::CheckButton::with_label(&choice.label);
            check.set_active(choice.initial == "true");
            fields.attach(&check, 1, *row, 1, 1);
            choices.push(ChoiceControl::Check(choice.id.clone(), check));
        } else {
            let labels: Vec<&str> = choice.options.iter().map(|(_, label)| label.as_str()).collect();
            let list = gtk::DropDown::from_strings(&labels);
            let ids: Vec<String> = choice.options.iter().map(|(id, _)| id.clone()).collect();
            let initial = ids.iter().position(|id| *id == choice.initial).unwrap_or(0);
            list.set_selected(u32::try_from(initial).unwrap_or(0));
            attach_field(fields, *row, &format!("{}:", choice.label), &list);
            choices.push(ChoiceControl::List(choice.id.clone(), ids, list));
        }
        *row += 1;
    }
    choices
}

/// Puts `caption` and `control` in row `row` of `fields`, labelled for
/// screen readers.
fn attach_field(
    fields: &gtk::Grid,
    row: i32,
    caption: &str,
    control: &(impl IsA<gtk::Widget> + IsA<gtk::Accessible>),
) {
    let label = gtk::Label::builder()
        .label(caption)
        .xalign(1.0)
        .css_classes(["picker-label"])
        .build();
    control.update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
    fields.attach(&label, 0, row, 1, 1);
    fields.attach(control, 1, row, 1, 1);
}

/// The files of a quoted list typed in an Open dialog's File name, each
/// from the folder shown or a path of its own; the first that is not a
/// file is named.
fn typed_files(picker: &Picker, shown: &Path, names: &[String]) -> Result<Vec<PathBuf>, String> {
    if names.len() > 1 && picker.chooses_one() {
        return Err(ox_core::i18n::gettext(CHOOSE_ONE));
    }
    names
        .iter()
        .map(|name| {
            let path = typed_path(name, shown).ok_or_else(not_local)?;
            if path.is_file() {
                return Ok(path);
            }
            let message = if path.is_dir() { NOT_A_FILE } else { NOT_FOUND };
            Err(ox_core::i18n::format_message(message, &[("name", name)]))
        })
        .collect()
}

/// `files`' names in quotes, separated by spaces, as Windows' File name
/// box lists several selected files: `"a.txt" "b.txt"`.
fn quoted_names(files: &[&Entry]) -> String {
    let quoted: Vec<String> = files.iter().map(|entry| format!("\"{}\"", entry.name)).collect();
    quoted.join(" ")
}

/// The names of a quoted list such as `"a.txt" "b.txt"`, `None` for text
/// that is not one (a plain name or path). A file name cannot hold a
/// quote here, as in Windows.
fn parse_quoted_names(text: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let inside = rest.strip_prefix('"')?;
        let end = inside.find('"')?;
        let name = inside[..end].trim();
        if !name.is_empty() {
            names.push(name.to_owned());
        }
        rest = inside[end + 1..].trim_start();
    }
    (!names.is_empty()).then_some(names)
}

/// Selects the name without its extension, as Windows does, so typing
/// replaces the name and keeps the type.
fn select_stem(entry: &gtk::Entry) {
    let text = entry.text();
    let end = Path::new(text.as_str())
        .file_stem()
        .map_or(text.chars().count(), |stem| {
            stem.to_string_lossy().chars().count()
        });
    entry.select_region(0, i32::try_from(end).unwrap_or(-1));
}

/// The message for a choice outside the local file system.
fn not_local() -> String {
    "Choose a folder on this computer or a connected drive.".to_owned()
}

/// Keeps the cancelled-or-chosen bookkeeping in one type for the window's
/// private state.
pub(super) type PickerSlot = RefCell<Option<Rc<Picker>>>;

#[cfg(test)]
mod tests {
    //! The picker as the portal drives it: a real backend on a
    //! `dbus-daemon` of the test's own, a second connection that owns
    //! `org.freedesktop.portal.Desktop`, and a test window that receives
    //! the call. The bus is not check.py's session bus, where GTK may
    //! already have started a real portal that owns that name.

    use std::cell::RefCell;
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::rc::Rc;

    use gtk::glib::translate::IntoGlib;
    use gtk::prelude::*;
    use gtk::{gdk, gio, glib};
    use ox_core::integration::{
        options_from_entries, path_variant, FileChooserBus, FILE_CHOOSER_INTERFACE, PORTAL_BACKEND_PATH,
        RESPONSE_CANCELLED, RESPONSE_SUCCESS,
    };

    use crate::test_support::harness::{capture, settle, wait_until, Fixture, TestWindow};
    use crate::window::tests::file_ops_support::{
        is_triggered_by, open_dialog, wait_for_no_dialog, window_shortcuts,
    };

    /// A `dbus-daemon` of the test's own, stopped when dropped.
    struct PrivateBus {
        daemon: Child,
        address: String,
        _directory: tempfile::TempDir,
    }

    impl PrivateBus {
        fn start() -> Self {
            let directory = tempfile::tempdir().expect("a folder for the bus");
            let config = directory.path().join("bus.conf");
            let listen = format!("unix:dir={}", directory.path().display());
            fs::write(
                &config,
                format!(
                    "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n \
                     \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n<busconfig><type>session</type>\
                     <listen>{listen}</listen><auth>EXTERNAL</auth><policy context=\"default\">\
                     <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/><allow own=\"*\"/>\
                     </policy></busconfig>\n"
                ),
            )
            .expect("the bus configuration is written");
            let mut daemon = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .args(["--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
                .expect("dbus-daemon is installed with dbus-run-session");
            let stdout = daemon.stdout.take().expect("standard output is piped");
            let mut address = String::new();
            BufReader::new(stdout)
                .read_line(&mut address)
                .expect("the daemon prints its address");
            Self {
                daemon,
                address: address.trim().to_owned(),
                _directory: directory,
            }
        }

        /// A fresh connection to this bus.
        fn connect(&self) -> gio::DBusConnection {
            let flags = gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
            gio::DBusConnection::for_address_sync(&self.address, flags, None, gio::Cancellable::NONE)
                .expect("connect to the private bus")
        }
    }

    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    /// Runs `future` on the main loop until it finishes.
    fn wait_for<T: 'static>(what: &str, future: impl std::future::Future<Output = T> + 'static) -> T {
        let result: Rc<RefCell<Option<T>>> = Rc::default();
        let slot = Rc::clone(&result);
        glib::spawn_future_local(async move {
            slot.replace(Some(future.await));
        });
        wait_until(what, || result.borrow().is_some());
        result.take().expect("the future finished")
    }

    /// The backend serving a test window, and a portal stand-in to call
    /// it.
    struct Portal {
        test: TestWindow,
        _backend: FileChooserBus,
        backend_name: String,
        frontend: gio::DBusConnection,
        _bus: PrivateBus,
    }

    impl Portal {
        fn new() -> Self {
            let bus = PrivateBus::start();
            let test = TestWindow::without_tabs();
            let connection = bus.connect();
            let backend_name = connection.unique_name().expect("a bus name").to_string();
            let window = test.window.downgrade();
            let mut backend = FileChooserBus::new(connection, move |call| {
                let window = window.upgrade().ok_or(ox_core::integration::ChooserNotShown)?;
                window.begin_picking(call);
                Ok(())
            });
            backend.export().expect("export the backend");
            let frontend = bus.connect();
            let owning = frontend.clone();
            let reply = wait_for("the portal's name", async move {
                owning
                    .call_future(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "RequestName",
                        Some(&("org.freedesktop.portal.Desktop", 4_u32).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        5000,
                    )
                    .await
            });
            assert_eq!(
                reply.expect("RequestName").get::<(u32,)>(),
                Some((1,)),
                "the stand-in owns the name"
            );
            Self {
                test,
                _backend: backend,
                backend_name,
                frontend,
                _bus: bus,
            }
        }

        /// Starts `method` with `entries`; the reply is filled in when the
        /// window answers.
        fn call(
            &self,
            method: &str,
            entries: &[(&str, glib::Variant)],
        ) -> Rc<RefCell<Option<(u32, glib::VariantDict)>>> {
            let handle =
                glib::variant::ObjectPath::try_from("/org/freedesktop/portal/desktop/request/1_1/picker")
                    .expect("a path");
            let parameters = glib::Variant::tuple_from_iter([
                handle.to_variant(),
                "org.example.Editor".to_variant(),
                "".to_variant(),
                "".to_variant(),
                options_from_entries(entries).end(),
            ]);
            let call = self.frontend.call_future(
                Some(&self.backend_name),
                PORTAL_BACKEND_PATH,
                FILE_CHOOSER_INTERFACE,
                method,
                Some(&parameters),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                60_000,
            );
            let answer: Rc<RefCell<Option<(u32, glib::VariantDict)>>> = Rc::default();
            let slot = Rc::clone(&answer);
            glib::spawn_future_local(async move {
                let reply = call.await.expect("the backend answers");
                slot.replace(reply.get::<(u32, glib::VariantDict)>());
            });
            let window = self.test.window.clone();
            wait_until("the picker", move || window.is_picking() && window.is_mapped());
            self.test.wait_for_listing("the picker's folder");
            answer
        }

        /// Waits for `answer` and returns its response and URIs.
        fn finish(answer: &Rc<RefCell<Option<(u32, glib::VariantDict)>>>) -> (u32, Vec<String>) {
            wait_until("the answer", || answer.borrow().is_some());
            let (response, results) = answer.borrow_mut().take().expect("answered");
            let uris: Vec<String> = results.lookup("uris").ok().flatten().unwrap_or_default();
            (response, uris)
        }
    }

    /// Save: the caller's folder and name, the type list narrows the
    /// listing, and the accept button answers with the new file.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn saving_answers_the_named_file_in_the_folder() {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        fixture.write("photo.png");
        std::fs::create_dir(fixture.path("Drafts")).expect("a folder");
        let portal = Portal::new();
        let text = ("Text".to_owned(), vec![(0_u32, "*.txt".to_owned())]).to_variant();
        let answer = portal.call(
            "SaveFile",
            &[
                (
                    "current_folder",
                    path_variant(&fixture.root().display().to_string()),
                ),
                ("current_name", "report.txt".to_variant()),
                (
                    "filters",
                    glib::Variant::array_from_iter_with_type(text.type_(), [text.clone()]),
                ),
            ],
        );
        let window = &portal.test.window;
        assert_eq!(window.title().as_deref(), Some("Save as"));
        let mut names = portal.test.names();
        names.sort();
        assert_eq!(
            names,
            ["Drafts", "notes.txt"],
            "the type list hides other files, never folders"
        );
        let picker = window.picker().expect("a picker");
        let name = picker.name.as_ref().expect("a name box");
        assert_eq!(name.text(), "report.txt");
        assert_eq!(picker.accept.label().as_deref(), Some("Save"));
        assert!(
            !window.lookup_action("new-tab").expect("the action").is_enabled(),
            "no new tabs"
        );
        assert!(
            !window.is_modal(),
            "the picker has no parent, so modal would block every other window"
        );
        capture(window, "picker-save.png");
        name.set_text("summary.txt");
        settle();
        picker.accept.emit_clicked();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("summary.txt")]);
        wait_until("the picker to close", || !window.is_visible());
    }

    /// Open: activating a file chooses it.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn activating_a_file_opens_it() {
        let fixture = Fixture::empty();
        fixture.write("letter.odt");
        let portal = Portal::new();
        let answer = portal.call(
            "OpenFile",
            &[(
                "current_folder",
                path_variant(&fixture.root().display().to_string()),
            )],
        );
        let window = &portal.test.window;
        assert_eq!(window.title().as_deref(), Some("Open"));
        window.activate_item(portal.test.position_of("letter.odt"));
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("letter.odt")]);
    }

    /// A folder dialog lists only folders and answers the selected one.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_folder_dialog_lists_and_answers_folders() {
        let fixture = Fixture::empty();
        fixture.write("loose.txt");
        std::fs::create_dir(fixture.path("Exports")).expect("a folder");
        let portal = Portal::new();
        let answer = portal.call(
            "OpenFile",
            &[
                ("directory", true.to_variant()),
                (
                    "current_folder",
                    path_variant(&fixture.root().display().to_string()),
                ),
            ],
        );
        assert_eq!(portal.test.names(), ["Exports"]);
        let window = &portal.test.window;
        window
            .folder_model()
            .selection()
            .select_item(portal.test.position_of("Exports"), true);
        settle();
        let picker = window.picker().expect("a picker");
        assert_eq!(picker.accept.label().as_deref(), Some("Select folder"));
        capture(window, "picker-folder.png");
        picker.accept.emit_clicked();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("Exports")]);
    }

    /// Escape in the file list cancels the dialog, as in Windows, even
    /// with a file selected (where it would otherwise clear the
    /// selection).
    ///
    /// parity: INT-032
    #[gtk::test]
    fn escape_in_the_file_list_cancels() {
        let fixture = Fixture::empty();
        fixture.write("letter.odt");
        let portal = Portal::new();
        let answer = portal.call(
            "OpenFile",
            &[(
                "current_folder",
                path_variant(&fixture.root().display().to_string()),
            )],
        );
        let window = &portal.test.window;
        wait_until("the folder's listing", || window.is_listed());
        portal.test.select_named("letter.odt");
        // Choosing the file in the list takes the keyboard there, from
        // File name where the dialog starts.
        window.folder_pane().focus_view();
        // The details view's own key handling, as a key press there runs.
        let view = window.folder_pane().details().column_view();
        let keys = view
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
            .find(|controller| controller.propagation_phase() == gtk::PropagationPhase::Capture)
            .expect("the details view handles keys");
        let handled = keys.emit_by_name::<bool>(
            "key-pressed",
            &[
                &gtk::gdk::Key::Escape.into_glib(),
                &0_u32,
                &gtk::gdk::ModifierType::empty(),
            ],
        );
        assert!(handled);
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_CANCELLED);
        assert!(uris.is_empty());
        wait_until("the picker to close", || !window.is_visible());
    }

    /// Opens a dialog of `method` with `entries` on `fixture`'s folder.
    fn dialog_on(
        portal: &Portal,
        fixture: &Fixture,
        method: &str,
        mut entries: Vec<(&str, glib::Variant)>,
    ) -> Rc<RefCell<Option<(u32, glib::VariantDict)>>> {
        entries.push((
            "current_folder",
            path_variant(&fixture.root().display().to_string()),
        ));
        portal.call(method, &entries)
    }

    /// A file typed in the address bar is the choice, not opened in
    /// another application.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_file_typed_in_the_address_bar_is_the_choice() {
        let fixture = Fixture::empty();
        fixture.write("letter.odt");
        let portal = Portal::new();
        let answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let window = &portal.test.window;

        window.submit_address(&fixture.path("letter.odt").display().to_string());

        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("letter.odt")]);
        assert!(
            portal.test.context.recorded_launches().is_empty(),
            "nothing opened"
        );
    }

    /// The Open dialog's File name box: a selected file fills it in, a
    /// folder typed there opens, a name that names nothing says so, and a
    /// full path is the choice.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn the_open_dialog_takes_a_file_name_or_a_path() {
        let fixture = Fixture::empty();
        fixture.write("letter.odt");
        fs::create_dir(fixture.path("Drafts")).expect("a folder");
        fs::write(fixture.path("Drafts/plan.txt"), b"x").expect("a file");
        let portal = Portal::new();
        let answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let window = &portal.test.window;
        let picker = window.picker().expect("a picker");
        let name = picker.name.clone().expect("Open has a File name box");
        assert!(!picker.accept.is_sensitive(), "nothing chosen yet");

        portal.test.select_named("letter.odt");
        settle();
        assert_eq!(name.text(), "letter.odt", "a selected file fills it in");

        name.set_text("Drafts");
        name.emit_activate();
        wait_until("the folder typed", || {
            portal.test.names().contains(&"plan.txt".to_owned())
        });
        assert_eq!(name.text(), "", "the box is cleared for the next name");

        name.set_text("missing.txt");
        name.emit_activate();
        assert!(window.shown_message().contains("was not found"));
        assert!(answer.borrow().is_none(), "no answer yet");

        name.set_text(&fixture.path("letter.odt").display().to_string());
        name.emit_activate();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("letter.odt")]);
    }

    /// Several files selected in an Open dialog for several files are all
    /// the choice, as in Windows: File name lists them in quotes, which the
    /// accept button sends, instead of keeping the first file's name. A
    /// quoted list typed in the box is the choice too.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn several_selected_files_are_all_sent() {
        let fixture = Fixture::empty();
        for name in ["letter.odt", "notes.md", "plan.txt"] {
            fixture.write(name);
        }
        let portal = Portal::new();
        let answer = dialog_on(
            &portal,
            &fixture,
            "OpenFile",
            vec![("multiple", true.to_variant())],
        );
        let window = &portal.test.window;
        wait_until("the folder's listing", || window.is_listed());
        let picker = window.picker().expect("a picker");
        let name = picker.name.clone().expect("Open has a File name box");

        portal.test.select_named("letter.odt");
        settle();
        assert_eq!(name.text(), "letter.odt");
        let model = window.folder_model();
        let notes = (0..model.n_items())
            .find(|position| model.name_at(*position).as_deref() == Some("notes.md"))
            .expect("notes.md is listed");
        model.selection().select_item(notes, false);
        settle();
        assert_eq!(
            name.text(),
            "\"letter.odt\" \"notes.md\"",
            "the box lists every selected file"
        );

        picker.accept.emit_clicked();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        let mut uris = uris;
        uris.sort();
        assert_eq!(
            uris,
            [fixture.uri_of("letter.odt"), fixture.uri_of("notes.md")],
            "both files are sent, not only the first"
        );
    }

    /// A quoted list reads as its names; anything else is no list.
    #[test]
    fn quoted_lists_read_as_their_names() {
        assert_eq!(
            super::parse_quoted_names("\"a.txt\" \"my notes.md\""),
            Some(vec!["a.txt".to_owned(), "my notes.md".to_owned()])
        );
        assert_eq!(
            super::parse_quoted_names(" \"one\" "),
            Some(vec!["one".to_owned()])
        );
        assert_eq!(super::parse_quoted_names("plain.txt"), None, "a plain name");
        assert_eq!(super::parse_quoted_names("\"open"), None, "an unclosed quote");
        assert_eq!(
            super::parse_quoted_names("\"a\" b"),
            None,
            "a name outside quotes"
        );
        assert_eq!(super::parse_quoted_names("\"\""), None, "no name");
    }

    /// A quoted list typed in File name opens every file in it; one that
    /// is not there, or is a folder, is named.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_typed_quoted_list_opens_every_file_in_it() {
        let fixture = Fixture::empty();
        for name in ["letter.odt", "notes.md"] {
            fixture.write(name);
        }
        fs::create_dir(fixture.path("Drafts")).expect("a folder");
        let portal = Portal::new();
        let answer = dialog_on(
            &portal,
            &fixture,
            "OpenFile",
            vec![("multiple", true.to_variant())],
        );
        let window = &portal.test.window;
        wait_until("the folder's listing", || window.is_listed());
        let name = window
            .picker()
            .expect("a picker")
            .name
            .clone()
            .expect("Open has a File name box");

        name.set_text("\"letter.odt\" \"gone.txt\"");
        name.emit_activate();
        assert!(
            window.shown_message().contains("gone.txt"),
            "the missing file is named"
        );
        assert!(answer.borrow().is_none(), "no answer yet");

        name.set_text("\"letter.odt\" \"Drafts\"");
        name.emit_activate();
        assert_eq!(
            window.shown_message(),
            "“Drafts” is a folder, not a file.",
            "a folder is not reported missing"
        );
        assert!(answer.borrow().is_none(), "no answer yet");

        name.set_text("\"notes.md\" \"letter.odt\"");
        name.emit_activate();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("notes.md"), fixture.uri_of("letter.odt")]);
    }

    /// Save takes a path from the folder shown and adds the chosen type's
    /// extension to a name without one; a missing folder is refused.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn saving_takes_a_path_and_adds_the_types_extension() {
        let fixture = Fixture::empty();
        fs::create_dir(fixture.path("Drafts")).expect("a folder");
        let portal = Portal::new();
        let text = ("Text".to_owned(), vec![(0_u32, "*.txt".to_owned())]).to_variant();
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![
                ("current_name", "notes.txt".to_variant()),
                (
                    "filters",
                    glib::Variant::array_from_iter_with_type(text.type_(), [text.clone()]),
                ),
            ],
        );
        let window = &portal.test.window;
        let picker = window.picker().expect("a picker");
        let name = picker.name.clone().expect("a name box");

        name.set_text("Missing/report");
        name.emit_activate();
        assert!(window.shown_message().contains("does not exist"));
        assert!(answer.borrow().is_none());

        name.set_text("Drafts/report");
        name.emit_activate();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(
            uris,
            [fixture.uri_of("Drafts/report.txt")],
            "the type's extension added"
        );
    }

    /// A name that has an extension is saved as typed, whatever the type.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_name_with_an_extension_is_saved_as_typed() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let text = ("Text".to_owned(), vec![(0_u32, "*.txt".to_owned())]).to_variant();
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![(
                "filters",
                glib::Variant::array_from_iter_with_type(text.type_(), [text.clone()]),
            )],
        );
        let name = portal
            .test
            .window
            .picker()
            .expect("a picker")
            .name
            .clone()
            .expect("a name box");
        name.set_text("notes.md");
        name.emit_activate();
        let (_, uris) = Portal::finish(&answer);
        assert_eq!(uris, [fixture.uri_of("notes.md")]);
    }

    /// A Save dialog starts with the keyboard in File name and the name
    /// selected without its extension, as Windows' does, so typing
    /// replaces the name; the folder's listing, which ends after the
    /// dialog shows, does not take the keyboard to the file list.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn the_save_dialog_starts_in_the_name_box_with_the_name_selected() {
        let fixture = Fixture::empty();
        for name in ["alpha.txt", "beta.txt", "report.md"] {
            fixture.write(name);
        }
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "report.txt".to_variant())],
        );
        let window = &portal.test.window;
        wait_until("the folder's listing", || window.is_listed());
        settle();
        assert!(
            window.focus_is_in_picker_name(),
            "the keyboard is in File name, not on {:?}",
            GtkWindowExt::focus(window).map(|focus| focus.type_())
        );
        let name = window
            .picker()
            .expect("a picker")
            .name
            .clone()
            .expect("a name box");
        assert_eq!(name.text(), "report.txt");
        assert_eq!(
            name.selection_bounds(),
            Some((0, 6)),
            "the name is selected without its extension"
        );
    }

    /// The filter list of one type, "Text", with `patterns`.
    fn text_type(patterns: &[&str]) -> glib::Variant {
        let patterns: Vec<(u32, String)> = patterns.iter().map(|glob| (0, (*glob).to_owned())).collect();
        let text = ("Text".to_owned(), patterns).to_variant();
        glib::Variant::array_from_iter_with_type(text.type_(), [text.clone()])
    }

    /// File name in a Save dialog, as Windows reads it: Enter with no name
    /// does nothing, a path that ends in a slash names a folder, and a
    /// pasted `file:` URI is a path. The type's extension is added from a
    /// case-insensitive glob, as Chrome sends them.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn the_save_dialog_reads_file_name_as_windows_does() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("filters", text_type(&["*.[tT][xX][tT]"]))],
        );
        let window = &portal.test.window;
        let name = window
            .picker()
            .expect("a picker")
            .name
            .clone()
            .expect("a name box");

        name.set_text("");
        name.emit_activate();
        assert!(window.is_listed(), "the folder is not listed again");
        assert!(answer.borrow().is_none());

        name.set_text("New/");
        name.emit_activate();
        assert!(window.shown_message().contains("does not exist"));
        assert!(answer.borrow().is_none(), "no New.txt in the folder shown");

        name.set_text(&fixture.uri_of("draft"));
        name.emit_activate();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("draft.txt")]);
    }

    /// The extension a type's glob stands for, Chrome's case-insensitive
    /// classes and several parts included; anything else has none.
    #[test]
    fn a_types_extension_comes_from_a_plain_glob() {
        let extension = super::glob_extension;
        assert_eq!(extension("*.txt").as_deref(), Some("txt"));
        assert_eq!(extension("*.[tT][xX][tT]").as_deref(), Some("txt"));
        assert_eq!(extension("*.[Mm][Pp]4").as_deref(), Some("mp4"));
        assert_eq!(extension("*.tar.gz").as_deref(), Some("tar.gz"));
        for other in [
            "*",
            "*.*",
            "*.",
            "*.[ab]",
            "*.[tt]",
            "*.[tT",
            "*.t?t",
            "*.tar..gz",
            "notes.txt",
        ] {
            assert_eq!(extension(other), None, "{other}");
        }
    }

    /// A Save dialog adds the type's extension to a name without a dot;
    /// a trailing dot is dropped instead, and a hidden file's name kept.
    #[test]
    fn a_saved_name_gets_the_types_extension_as_in_windows() {
        let saved = |written: &str| super::saved_name(written, Some("txt".to_owned()));
        assert_eq!(saved("report"), "report.txt");
        assert_eq!(saved("report."), "report");
        assert_eq!(saved("notes.md"), "notes.md");
        assert_eq!(saved(".bashrc"), ".bashrc");
        assert_eq!(super::saved_name("report", None), "report");
    }

    /// A selected name with spaces at either end is chosen as it is.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_name_with_spaces_at_its_ends_is_chosen_as_it_is() {
        let fixture = Fixture::empty();
        fixture.write(" notes.txt ");
        let portal = Portal::new();
        let answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let test = &portal.test;
        test.select_named(" notes.txt ");
        settle();
        test.window.picker().expect("a picker").accept.emit_clicked();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of(" notes.txt ")]);
    }

    /// In a Save dialog, a file activated or typed in the address bar is
    /// the file to replace, after asking: its own path, never its name in
    /// the folder shown with the type's extension added.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_file_activated_in_a_save_dialog_is_replaced_after_asking() {
        let fixture = Fixture::empty();
        fixture.write("README");
        fs::create_dir(fixture.path("Drafts")).expect("a folder");
        fs::write(fixture.path("Drafts/plan.txt"), b"x").expect("a file");
        let portal = Portal::new();
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("filters", text_type(&["*.txt", "README"]))],
        );
        let test = &portal.test;
        let window = &test.window;
        let name = window
            .picker()
            .expect("a picker")
            .name
            .clone()
            .expect("a name box");

        window.activate_item(test.position_of("README"));
        let question = open_dialog(test);
        assert_eq!(
            question.message_text(),
            "“README” already exists. Do you want to replace it?",
            "README itself, not a new README.txt"
        );
        assert_eq!(name.text(), "README");
        question.press("Cancel");
        wait_for_no_dialog(test);
        assert!(answer.borrow().is_none(), "Cancel keeps the dialog open");

        window.submit_address(&fixture.path("Drafts/plan.txt").display().to_string());
        let question = open_dialog(test);
        assert_eq!(name.text(), "plan.txt");
        question.press("Replace");
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(
            uris,
            [fixture.uri_of("Drafts/plan.txt")],
            "the file typed, not plan.txt in the folder shown"
        );
    }

    /// A folder selected after a file in an Open dialog is what Open
    /// opens: File name empties, so the file selected before is not sent.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn open_opens_a_folder_selected_after_a_file() {
        let fixture = Fixture::empty();
        fixture.write("letter.odt");
        fs::create_dir(fixture.path("Drafts")).expect("a folder");
        fs::write(fixture.path("Drafts/plan.txt"), b"x").expect("a file");
        let portal = Portal::new();
        let answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let test = &portal.test;
        let picker = test.window.picker().expect("a picker");
        let name = picker.name.clone().expect("Open has a File name box");

        test.select_named("letter.odt");
        settle();
        assert_eq!(name.text(), "letter.odt");
        test.select_named("Drafts");
        settle();
        assert_eq!(name.text(), "", "a folder empties File name");

        picker.accept.emit_clicked();
        wait_until("the folder to open", || {
            test.names().contains(&"plan.txt".to_owned())
        });
        assert!(answer.borrow().is_none(), "letter.odt is not sent");
    }

    /// Nothing opens another tab, window or pane from a dialog: a search
    /// result's folder opens in the dialog, but not in a new tab or
    /// window, and Split is off.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_dialog_opens_no_other_tab_window_or_pane() {
        let fixture = Fixture::empty();
        fixture.write("letter.odt");
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let test = &portal.test;
        let window = &test.window;
        test.start_search_cache();
        window.search_box().entry().set_text("letter");
        wait_until("the search's result", || {
            window.is_searching() && test.names() == ["letter.odt"]
        });
        // Selecting a result offers the location actions again.
        window.folder_model().select_only(0);
        settle();

        let enabled = |name: &str| window.lookup_action(name).expect("the action").is_enabled();
        assert!(enabled("open-file-location"), "the result's folder opens here");
        assert!(!enabled("open-file-location-in-tab"), "no second tab");
        assert!(!enabled("open-file-location-in-window"), "no new window");
        assert!(!enabled("split-view"), "no second pane");
    }

    /// Presses `keyval` with exactly `modifiers` wherever the keyboard
    /// is, through the window's shortcut controllers in the order GTK runs
    /// them, the application's accelerators among them: whether one took
    /// the key.
    fn press(test: &TestWindow, keyval: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        window_shortcuts(test)
            .iter()
            .filter(|shortcut| {
                let trigger = shortcut.trigger();
                trigger.is_some_and(|trigger| is_triggered_by(&trigger, keyval, modifiers))
            })
            .any(|shortcut| {
                shortcut.action().is_some_and(|action| {
                    action.activate(gtk::ShortcutActionFlags::empty(), &test.window, None)
                })
            })
    }

    /// Alt+Up, Alt+Left and Alt+Right work from the File name box, where
    /// the keyboard starts; Ctrl+N opens no window, and Ctrl+Q cancels the
    /// dialog alone instead of quitting every window.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn the_dialog_keys_work_from_the_name_box() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let answer = dialog_on(&portal, &fixture, "SaveFile", Vec::new());
        let test = &portal.test;
        let window = &test.window;
        wait_until("the folder's listing", || window.is_listed());
        let name = window
            .picker()
            .expect("a picker")
            .name
            .clone()
            .expect("a name box");
        let folder = fixture.uri();
        let parent = gio::File::for_uri(&folder)
            .parent()
            .expect("the fixture has a parent")
            .uri()
            .to_string();
        let shows = |uri: &str| window.current_uri().as_deref() == Some(uri);
        let press_in_name = |key: gdk::Key| {
            name.grab_focus();
            settle();
            assert!(window.focus_is_in_picker_name());
            press(test, key, gdk::ModifierType::ALT_MASK)
        };

        assert!(press_in_name(gdk::Key::Up), "Alt+Up acts from File name");
        wait_until("the parent folder", || shows(&parent));
        assert!(press_in_name(gdk::Key::Left), "Alt+Left acts from File name");
        wait_until("the folder again", || shows(&folder));
        assert!(press_in_name(gdk::Key::Right), "Alt+Right acts from File name");
        wait_until("the parent folder again", || shows(&parent));

        window.folder_pane().focus_view();
        assert!(
            !press(test, gdk::Key::n, gdk::ModifierType::CONTROL_MASK),
            "Ctrl+N opens no window"
        );

        name.grab_focus();
        assert!(press(test, gdk::Key::q, gdk::ModifierType::CONTROL_MASK));
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_CANCELLED, "Ctrl+Q cancels the dialog");
        assert!(uris.is_empty());
        wait_until("the picker to close", || !window.is_visible());
    }

    /// A dialog for one file keeps one selected: a second item selected
    /// with Ctrl takes the first one's place.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_dialog_for_one_file_keeps_one_selected() {
        let fixture = Fixture::empty();
        fixture.write("a.txt");
        fixture.write("b.txt");
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let test = &portal.test;
        let selection = test.window.folder_model().selection().clone();
        selection.select_item(test.position_of("a.txt"), true);
        settle();
        selection.select_item(test.position_of("b.txt"), false);
        settle();
        assert_eq!(test.selected_names(), ["b.txt"], "the newer one stays");
    }

    /// Shift+click in a dialog for one file keeps the item clicked,
    /// upward and downward, not the one beside the item kept before.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn shift_click_in_a_dialog_for_one_file_keeps_the_item_clicked() {
        let fixture = Fixture::empty();
        for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"] {
            fixture.write(name);
        }
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let test = &portal.test;
        let selection = test.window.folder_model().selection().clone();
        // Shift+click selects from the item kept to the item clicked.
        let shift_click = |from: &str, to: &str| {
            let (from, to) = (test.position_of(from), test.position_of(to));
            selection.select_range(from.min(to), from.abs_diff(to) + 1, false);
            settle();
        };
        selection.select_item(test.position_of("e.txt"), true);
        settle();

        shift_click("e.txt", "b.txt");
        assert_eq!(test.selected_names(), ["b.txt"], "upward");
        shift_click("b.txt", "d.txt");
        assert_eq!(test.selected_names(), ["d.txt"], "downward");
    }

    /// Closing the window answers Cancelled, once.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn closing_the_window_cancels() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let answer = portal.call(
            "SaveFile",
            &[(
                "current_folder",
                path_variant(&fixture.root().display().to_string()),
            )],
        );
        let window = &portal.test.window;
        assert!(
            !window.picker().expect("a picker").accept.is_sensitive(),
            "no name, nothing to save"
        );
        window.close();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_CANCELLED);
        assert!(uris.is_empty());
    }
}
