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
use crate::dialog::{DialogFrame, DialogWidth};
use crate::folder_view::filter::ChooserListing;
use local_paths::{ask_for_path, Known, LocalPaths};
use probe::{all_writable, probe, Probe};

mod local_paths;
mod probe;

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
    /// The local paths found so far, asked off the main thread.
    local_paths: LocalPaths,
    choices: Vec<ChoiceControl>,
    accept: gtk::Button,
    /// Set while the replace question is open, so the answer is not given
    /// twice.
    asking: Cell<bool>,
    /// The choice to answer with once the window really closes; the
    /// window answers Cancelled without one.
    pending: RefCell<Option<ChooserAnswer>>,
    /// Set while the accept button's choice is checked on disk, off the
    /// main thread.
    checking: Cell<bool>,
    /// The one item selected, in a dialog that chooses one, so a second
    /// item clicked with Ctrl or Shift takes its place.
    single: Cell<Option<u32>>,
    /// Set once the dialog is a modal child of the caller's window
    /// ([`super::caller_window`]).
    attached_to_caller: Rc<Cell<bool>>,
}

impl Picker {
    /// Whether the dialog is answering or asking, so accepting again does
    /// nothing.
    fn is_busy(&self) -> bool {
        self.asking.get() || self.checking.get()
    }

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

/// The message for a folder or file on a share that stopped answering.
fn not_answering(path: &Path) -> String {
    ox_core::i18n::format_message(
        "“{path}” is not answering. Its network share may have stopped responding.",
        &[("path", &path.display().to_string())],
    )
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
        let ChooserCall {
            request,
            reply,
            parent_window,
            ..
        } = call;
        let wanted_folder = request.current_folder.clone();
        let mut picker = self.build_picker_bar(request, reply.clone());
        picker.attached_to_caller = super::caller_window::attach_to_caller(self.upcast_ref(), &parent_window);
        let picker = Rc::new(picker);
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
        self.set_default_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        self.apply_chooser_listing();
        self.open_start_folder(wanted_folder);
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

    /// Opens the folder the dialog starts in: the caller's `wanted` folder
    /// if it is one, else the home folder. The caller's folder is checked
    /// off the main thread: on a share that stopped answering, the check
    /// would freeze every window. The dialog then opens in the home folder
    /// and says so.
    fn open_start_folder(&self, wanted: Option<PathBuf>) {
        let Some(wanted) = wanted else {
            self.open_first_tab(&glib::home_dir());
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let found = probe(&wanted).await;
                if window.picker().is_none_or(|picker| picker.reply.is_answered()) {
                    return;
                }
                if found == Probe::Folder {
                    window.open_first_tab(&wanted);
                    return;
                }
                window.open_first_tab(&glib::home_dir());
                if found == Probe::NoAnswer {
                    window.show_message(&not_answering(&wanted));
                }
            }
        ));
    }

    /// Opens the dialog's first tab at `folder`, else the home folder.
    fn open_first_tab(&self, folder: &Path) {
        if self.add_tab(&gio::File::for_path(folder).uri()).is_err() {
            // The home folder always opens.
            let _ = self.add_tab(&gio::File::for_path(glib::home_dir()).uri());
        }
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
                match self.local_path(&entry.uri) {
                    Some(path) if !picker.is_busy() && !picker.reply.is_answered() => {
                        self.finish_picking(&picker, vec![path]);
                    }
                    Some(_) => {}
                    None => self.show_message(&not_local()),
                }
            }
            ChooserMode::Save { .. } => {
                let Some(path) = self.local_path(&entry.uri) else {
                    return self.show_message(&not_local());
                };
                if let Some(name) = &picker.name {
                    name.set_text(&entry.name);
                }
                if !picker.is_busy() && !picker.reply.is_answered() {
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

    /// Answers with the choice made when the window closes, or Cancelled
    /// without one. The answer waits for the close: answered first, a
    /// window that then refused to close (an update installing, files
    /// being written) kept a dead dialog whose Save, Cancel and Escape
    /// did nothing.
    pub(super) fn end_picking_on_close(&self) {
        if let Some(picker) = self.imp().picker.borrow().as_ref() {
            let answer = picker.pending.take().unwrap_or(ChooserAnswer::Cancelled);
            picker.reply.send(&answer);
        }
    }

    /// The window refused to close: the dialog stays usable, its choice
    /// not sent.
    pub(super) fn picking_close_refused(&self) {
        if let Some(picker) = self.picker() {
            picker.pending.take();
            picker.asking.set(false);
            self.update_picker();
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

    /// Narrows both folder panes to what the dialog may choose and the type
    /// chosen, so a pane split off in the dialog lists what the other does.
    pub(super) fn apply_chooser_listing(&self) {
        let Some(picker) = self.picker() else {
            return;
        };
        for pane in self.folder_panes() {
            pane.model().set_chooser_listing(picker.listing());
        }
    }

    /// The current folder's local path, if it is known to have one.
    fn picking_folder(&self) -> Option<PathBuf> {
        self.local_path(&self.current_uri()?)
    }

    /// The current folder's local path, waiting for `GVfs` if needed.
    async fn picking_folder_now(&self) -> Option<PathBuf> {
        self.local_path_now(&self.current_uri()?).await
    }

    /// The local path of `uri`, if it is known to have one: a local file,
    /// or a share or device mounted by `GVfs` that already answered. A
    /// location not asked about yet is asked off the main thread, and the
    /// dialog updates itself with the answer.
    fn local_path(&self, uri: &str) -> Option<PathBuf> {
        let picker = self.picker()?;
        let known = picker.local_paths.known(uri);
        if known != Known::Unknown {
            return known.path();
        }
        if picker.local_paths.start_asking(uri) {
            let uri = uri.to_owned();
            glib::spawn_future_local(glib::clone!(
                #[weak(rename_to = window)]
                self,
                async move {
                    picker.local_paths.remember(&uri, ask_for_path(&uri).await);
                    window.update_picker();
                }
            ));
        }
        None
    }

    /// The local path of `uri`, asking `GVfs` off the main thread and
    /// waiting for its answer when it is not known yet.
    async fn local_path_now(&self, uri: &str) -> Option<PathBuf> {
        let picker = self.picker()?;
        let known = picker.local_paths.known(uri);
        if known != Known::Unknown {
            return known.path();
        }
        let answer = ask_for_path(uri).await;
        picker.local_paths.remember(uri, answer.clone());
        answer.path()
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
                    || matches!(selected.as_slice(), [entry] if entry.is_dir && self.local_path(entry.navigation_uri()).is_some())
            }
            // Save stays on with a name, also where nothing can be saved:
            // pressing it says why, rather than doing nothing.
            ChooserMode::Save { .. } => typed,
        };
        picker
            .accept
            .set_sensitive(ready && !picker.is_busy() && !picker.reply.is_answered());
        // Where the folder shown has no local path, the button says why.
        let has_no_path = self
            .current_uri()
            .is_some_and(|uri| picker.local_paths.known(&uri) == Known::NoPath);
        picker
            .accept
            .set_tooltip_text(has_no_path.then(not_local).as_deref());
    }

    /// The accept button: works out the choice and answers, asking first
    /// before replacing files.
    ///
    /// What is on disk is checked off the main thread ([`probe`]), and
    /// the accept button waits meanwhile, so a share that stopped
    /// answering cannot freeze the app.
    fn accept_choice(&self) {
        let Some(picker) = self.picker() else {
            return;
        };
        if picker.is_busy() || picker.reply.is_answered() {
            return;
        }
        picker.checking.set(true);
        self.update_picker();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let outcome = window.work_out_choice(&picker).await;
                picker.checking.set(false);
                window.update_picker();
                match outcome {
                    Ok(locations) if !locations.is_empty() => window.finish_picking(&picker, locations),
                    Err(message) if !message.is_empty() => window.show_message(&message),
                    // Nothing to answer yet: a folder opened, or a question is open.
                    Ok(_) | Err(_) => {}
                }
            }
        ));
    }

    /// The locations the accept button chooses, as [`Self::accept_choice`]
    /// describes.
    async fn work_out_choice(&self, picker: &Rc<Picker>) -> Result<Vec<PathBuf>, String> {
        let selected = self.selected_entries();
        match &picker.request.mode {
            ChooserMode::Open {
                directory: false,
                multiple,
            } => match self.typed_choice(picker, &selected).await {
                Some(typed) => typed,
                None => self.chosen_files(&selected, *multiple).await,
            },
            ChooserMode::Open { directory: true, .. } => {
                self.chosen_folder(&selected).await.map(|folder| vec![folder])
            }
            ChooserMode::Save { .. } => self.chosen_save(picker).await,
            ChooserMode::SaveFiles { names } => {
                let folder = self.chosen_folder(&selected).await?;
                let mut existing = Vec::new();
                for name in names {
                    match probe(&folder.join(name)).await {
                        Probe::NoAnswer => return Err(not_answering(&folder)),
                        found if found.exists() => existing.push(name.clone()),
                        _ => {}
                    }
                }
                if existing.is_empty() {
                    return Ok(vec![folder]);
                }
                self.confirm_replace(picker, vec![folder], &existing);
                Ok(Vec::new())
            }
        }
    }

    /// The files of an Open dialog. A single selected folder opens instead.
    async fn chosen_files(&self, selected: &[Entry], multiple: bool) -> Result<Vec<PathBuf>, String> {
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
        let mut paths = Vec::with_capacity(files.len());
        for entry in files {
            paths.push(self.local_path_now(&entry.uri).await.ok_or_else(not_local)?);
        }
        Ok(paths)
    }

    /// The folder of a folder dialog or `SaveFiles`: the one selected
    /// folder, else the current folder.
    async fn chosen_folder(&self, selected: &[Entry]) -> Result<PathBuf, String> {
        if let [entry] = selected {
            if entry.is_dir {
                return self
                    .local_path_now(entry.navigation_uri())
                    .await
                    .ok_or_else(not_local);
            }
        }
        self.picking_folder_now().await.ok_or_else(not_local)
    }

    /// The file of a Save dialog: the name in the current folder. A name
    /// of a folder there opens that folder; an existing file is replaced
    /// only after asking.
    async fn chosen_save(&self, picker: &Rc<Picker>) -> Result<Vec<PathBuf>, String> {
        let Some(name_box) = &picker.name else {
            return Ok(Vec::new());
        };
        let typed = name_box.text();
        // Enter with no name does what the Save button, off then, does.
        if typed.trim().is_empty() {
            return Ok(Vec::new());
        }
        let shown = self.picking_folder_now().await.ok_or_else(not_local)?;
        let path = typed_path(&typed, &shown).ok_or_else(not_local)?;
        let found = probe(&path).await;
        if found == Probe::NoAnswer {
            return Err(not_answering(&path));
        }
        if found == Probe::Folder {
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
        match probe(&folder).await {
            Probe::Folder => {}
            Probe::NoAnswer => return Err(not_answering(&folder)),
            Probe::File | Probe::Missing => return Err(missing(&folder)),
        }
        let written = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = checked_name(&saved_name(&written, picker.chosen_extension())).map_err(|_| bad_name())?;
        let target = folder.join(&name);
        let found = probe(&target).await;
        if found == Probe::NoAnswer {
            return Err(not_answering(&folder));
        }
        if found == Probe::Folder {
            name_box.set_text("");
            self.navigate_or_report(&gio::File::for_path(&target).uri());
            return Ok(Vec::new());
        }
        if found.exists() {
            self.confirm_replace(picker, vec![target], &[name]);
            return Ok(Vec::new());
        }
        Ok(vec![target])
    }

    /// Asks whether to replace `names`, then answers with `locations`.
    ///
    /// The question is part of the dialog window, on its dialog layer, not
    /// a window of its own: a separate modal window took every click and
    /// key from the dialog, so when it opened behind the dialog or on
    /// another screen, Save, Cancel, Escape and the close button all did
    /// nothing, and the dialog could only be ended by stopping `OpenXplorer`.
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
        let frame = DialogFrame::new(title, DialogWidth::Standard);
        frame.set_message(&question);
        let locations = Rc::new(locations);
        let replace = frame.add_closing_button(
            "Replace",
            ButtonStyle::Accent,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                picker,
                move || window.finish_picking(&picker, locations.as_ref().clone())
            ),
        );
        frame.add_closing_button("Cancel", ButtonStyle::Bordered, || {});
        // However the question ends: a button, Escape, or the dialog
        // closing under it.
        frame.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            picker,
            move |_| {
                picker.asking.set(false);
                window.update_picker();
            }
        ));
        self.present_window_dialog(&frame);
        replace.grab_focus();
    }

    /// What the File name box of an Open dialog chooses, when it names
    /// something other than the one file selected: a folder opens, a file
    /// is the choice, and a name that names nothing says so. `None` leaves
    /// the choice to the selection.
    async fn typed_choice(
        &self,
        picker: &Picker,
        selected: &[Entry],
    ) -> Option<Result<Vec<PathBuf>, String>> {
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
        let Some(shown) = self.picking_folder_now().await else {
            return Some(Err(not_local()));
        };
        if let Some(names) = parse_quoted_names(&typed) {
            return Some(typed_files(picker, &shown, &names).await);
        }
        let Some(path) = typed_path(&typed, &shown) else {
            return Some(Err(not_local()));
        };
        match probe(&path).await {
            Probe::Folder => {
                name_box.set_text("");
                self.navigate_or_report(&gio::File::for_path(&path).uri());
                return Some(Ok(Vec::new()));
            }
            Probe::File => return Some(Ok(vec![path])),
            Probe::NoAnswer => return Some(Err(not_answering(&path))),
            Probe::Missing => {}
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

    /// Closes the window, which answers with `locations`. An Open dialog
    /// first finds out, off the main thread, whether the caller may write
    /// them; the dialog waits for that, never longer than [`probe`]
    /// allows.
    fn finish_picking(&self, picker: &Rc<Picker>, locations: Vec<PathBuf>) {
        let is_open = matches!(picker.request.mode, ChooserMode::Open { .. });
        picker.asking.set(true);
        self.update_picker();
        let picker = Rc::clone(picker);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let writable = is_open && all_writable(&locations).await;
                // The answer waits for the window to close
                // ([`BrowserWindow::end_picking_on_close`]).
                picker.pending.replace(Some(ChooserAnswer::Chosen {
                    locations,
                    filter: picker.chosen_filter(),
                    choices: picker.choice_values(),
                    writable,
                }));
                window.close();
            }
        ));
    }

    /// Cancel, Escape, and Ctrl+Q in a dialog: closes the window, which
    /// answers Cancelled.
    pub(super) fn cancel_picking(&self) {
        if let Some(picker) = self.picker() {
            picker.pending.take();
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
                // Caps Lock and Num Lock do not make it another key: with
                // Caps Lock on, Escape in File name did nothing.
                let held = gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::HYPER_MASK
                    | gtk::gdk::ModifierType::META_MASK;
                if key == gtk::gdk::Key::Escape && !modifiers.intersects(held) {
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
            local_paths: LocalPaths::default(),
            choices,
            accept,
            asking: Cell::new(false),
            pending: RefCell::new(None),
            checking: Cell::new(false),
            single: Cell::new(None),
            attached_to_caller: Rc::default(),
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
                    if window.is_picking() {
                        window.apply_chooser_listing();
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
async fn typed_files(picker: &Picker, shown: &Path, names: &[String]) -> Result<Vec<PathBuf>, String> {
    if names.len() > 1 && picker.chooses_one() {
        return Err(ox_core::i18n::gettext(CHOOSE_ONE));
    }
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let path = typed_path(name, shown).ok_or_else(not_local)?;
        let message = match probe(&path).await {
            Probe::File => {
                files.push(path);
                continue;
            }
            Probe::NoAnswer => return Err(not_answering(&path)),
            Probe::Folder => NOT_A_FILE,
            Probe::Missing => NOT_FOUND,
        };
        return Err(ox_core::i18n::format_message(message, &[("name", name)]));
    }
    Ok(files)
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
    use crate::window::tests::file_ops_support::{is_triggered_by, window_shortcuts};

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
            self.call_from("", method, entries)
        }

        /// Starts `method` with `entries` for the caller's window
        /// `parent`, as the portal names it (`wayland:<handle>`).
        fn call_from(
            &self,
            parent: &str,
            method: &str,
            entries: &[(&str, glib::Variant)],
        ) -> Rc<RefCell<Option<(u32, glib::VariantDict)>>> {
            let handle =
                glib::variant::ObjectPath::try_from("/org/freedesktop/portal/desktop/request/1_1/picker")
                    .expect("a path");
            let parameters = glib::Variant::tuple_from_iter([
                handle.to_variant(),
                "org.example.Editor".to_variant(),
                parent.to_variant(),
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
            // The start folder opens once it is checked, off the main thread.
            let window = self.test.window.clone();
            wait_until("the start folder", move || window.current_uri().is_some());
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

    /// Presses Enter in File name and waits until the dialog has checked
    /// what it names, which happens off the main thread.
    fn enter(name: &gtk::Entry) {
        name.emit_activate();
        let window = name
            .root()
            .and_downcast::<super::BrowserWindow>()
            .expect("the box is in a dialog");
        wait_until("the name to be checked", || {
            window.picker().is_none_or(|picker| !picker.checking.get())
        });
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

    /// A caller's folder on a share that stopped answering does not
    /// freeze the app: the dialog opens in the home folder and says so.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_folder_that_does_not_answer_opens_the_dialog_at_home() {
        let fixture = Fixture::empty();
        let share = fixture.path("Share");
        fs::create_dir(&share).expect("a folder");
        super::probe::stop_answering(&share);
        let portal = Portal::new();

        let answer = portal.call(
            "OpenFile",
            &[("current_folder", path_variant(&share.display().to_string()))],
        );

        let window = &portal.test.window;
        let home = gio::File::for_path(glib::home_dir()).uri().to_string();
        assert_eq!(window.current_uri().as_deref(), Some(home.as_str()));
        assert!(
            window.shown_message().contains("is not answering"),
            "{}",
            window.shown_message()
        );
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// Saving into a folder whose share stopped answering says so and
    /// answers nothing; the dialog stays usable.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn saving_where_the_share_stopped_answering_says_so() {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        let portal = Portal::new();
        let answer = portal.call(
            "SaveFile",
            &[
                (
                    "current_folder",
                    path_variant(&fixture.root().display().to_string()),
                ),
                ("current_name", "report.txt".to_variant()),
            ],
        );
        let window = &portal.test.window;
        let picker = window.picker().expect("a picker");
        super::probe::stop_answering(fixture.root());

        picker.accept.emit_clicked();
        let accept_waits = !picker.accept.is_sensitive();
        wait_until("the dialog to say so", || {
            window.shown_message().contains("is not answering")
        });

        assert!(accept_waits, "Save waits while the folder is checked");
        assert!(answer.borrow().is_none(), "nothing is answered");
        assert!(picker.accept.is_sensitive(), "Save can be pressed again");
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// A file opened from a share that stopped answering is answered as
    /// read-only: whether it may be written is not known.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_file_on_a_share_that_stopped_answering_opens_read_only() {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        let portal = Portal::new();
        let answer = portal.call(
            "OpenFile",
            &[(
                "current_folder",
                path_variant(&fixture.root().display().to_string()),
            )],
        );
        super::probe::stop_answering(fixture.root());

        portal
            .test
            .window
            .activate_item(portal.test.position_of("notes.txt"));
        wait_until("the answer", || answer.borrow().is_some());

        let (response, results) = answer.borrow_mut().take().expect("answered");
        let uris: Vec<String> = results.lookup("uris").ok().flatten().unwrap_or_default();
        let writable: Option<bool> = results.lookup("writable").ok().flatten();
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("notes.txt")]);
        assert_eq!(writable, Some(false));
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

    /// Sets the modification time of `name` in `fixture` to `seconds` ago.
    fn date_file(fixture: &Fixture, name: &str, seconds: u64) {
        fs::File::options()
            .write(true)
            .open(fixture.path(name))
            .and_then(|file| {
                file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(seconds))
            })
            .expect("the date is set");
    }

    /// How far the visible view is scrolled once the window has drawn it
    /// and GTK has laid the list out again, as it does later for reasons
    /// of its own (the compositor sizing the window, a row measured again).
    fn scrolled_after_a_relayout(window: &crate::window::BrowserWindow) -> f64 {
        use crate::test_support::harness::{descendants, wait_for_frames};
        use crate::window::folder_pane::FolderView;
        wait_for_frames(window, 4);
        let pane = window.folder_pane();
        let (lists, adjustment): (Vec<gtk::Widget>, gtk::Adjustment) = match pane.view() {
            FolderView::Details => {
                let details = pane.details();
                let lists = descendants::<gtk::ListView>(details.column_view())
                    .into_iter()
                    .map(Cast::upcast)
                    .collect();
                (lists, details.vadjustment())
            }
            FolderView::Compact | FolderView::Icons(_) => {
                let icons = pane.icon_view();
                (vec![icons.grid().clone().upcast()], icons.scroll_adjustment())
            }
        };
        for list in lists {
            list.queue_allocate();
        }
        wait_for_frames(window, 4);
        adjustment.value()
    }

    /// A dialog's list at its top stays there when another file type
    /// shows more files, in Details, the icons and Compact: the files the
    /// type had hidden come before the one at the top edge, and GTK kept
    /// that one there, so the list slid down (Chrome's Save dialog in
    /// Downloads, `*.svg` then All files, not grouped).
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_dialog_list_at_its_top_stays_there_when_the_type_changes() {
        use crate::folder_view::icon_size::IconSize;
        use crate::window::folder_pane::FolderView;
        let fixture = Fixture::empty();
        // Newest first: the drawings come after newer files of other
        // types, and alone they fill more than the view.
        let mut svgs = 0;
        for number in 0..240_u64 {
            let svg = number % 3 == 2;
            let name = if svg {
                svgs += 1;
                format!("drawing {number:03}.svg")
            } else {
                format!("file {number:03}.txt")
            };
            fixture.write(&name);
            date_file(&fixture, &name, 60 + number * 3_600);
        }
        let portal = Portal::new();
        let filters = [
            ("SVG".to_owned(), vec![(0_u32, "*.svg".to_owned())]).to_variant(),
            ("All files".to_owned(), vec![(0_u32, "*".to_owned())]).to_variant(),
        ];
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![
                ("current_name", "picture.svg".to_variant()),
                (
                    "filters",
                    glib::Variant::array_from_iter_with_type(filters[0].type_(), filters.clone()),
                ),
            ],
        );
        let test = &portal.test;
        let window = &test.window;
        let types = window
            .picker()
            .expect("a picker")
            .types
            .clone()
            .expect("a type list");
        test.activate("sort", Some("modified"));
        test.activate("direction", Some("descending"));
        for view in [
            FolderView::Details,
            FolderView::Icons(IconSize::MEDIUM),
            FolderView::Compact,
        ] {
            test.activate("view", Some(view.as_str()));
            types.set_selected(0);
            wait_until("the drawings", || test.names().len() == svgs);
            // Down the list and back to its top, as the user scrolls.
            let pane = window.folder_pane();
            let adjustment = match view {
                FolderView::Details => pane.details().vadjustment(),
                _ => pane.icon_view().scroll_adjustment(),
            };
            wait_until(&format!("a list longer than {}", view.as_str()), || {
                adjustment.upper() > adjustment.page_size() + 1.0
            });
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            scrolled_after_a_relayout(window);
            adjustment.set_value(0.0);
            let view_name = view.as_str();
            let scrolled = scrolled_after_a_relayout(window);
            assert!(
                scrolled < 0.5,
                "{view_name}: the drawings from their top: {scrolled}"
            );

            types.set_selected(1);
            wait_until("every file", || test.names().len() == 240);
            let scrolled = scrolled_after_a_relayout(window);
            assert!(
                scrolled < 0.5,
                "{view_name}: All files from their top, not {scrolled} along"
            );

            types.set_selected(0);
            wait_until("the drawings again", || test.names().len() == svgs);
            let scrolled = scrolled_after_a_relayout(window);
            assert!(
                scrolled < 0.5,
                "{view_name}: the drawings again from their top, not {scrolled}"
            );
        }
    }

    /// A dialog has a window group of its own, so that while it is modal
    /// the other `OpenXplorer` windows stay usable. A caller's window it
    /// cannot become a child of (an X11 window, or any window when the
    /// app runs on X11) leaves it a window of its own, not modal.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_dialog_it_cannot_attach_stays_a_window_of_its_own() {
        let fixture = Fixture::empty();
        let other = TestWindow::open(&fixture.uri());
        let portal = Portal::new();
        let on_wayland = WidgetExt::display(&portal.test.window).type_().name() == "GdkWaylandDisplay";
        let parent = if on_wayland {
            "x11:4c0000a"
        } else {
            "wayland:handle-from-another-session"
        };
        let _answer = portal.call_from(
            parent,
            "OpenFile",
            &[(
                "current_folder",
                path_variant(&fixture.root().display().to_string()),
            )],
        );
        let window = &portal.test.window;
        let picker = window.picker().expect("a picker");
        assert!(!picker.attached_to_caller.get());
        assert!(!window.is_modal());
        assert_ne!(window.group(), other.window.group(), "a window group of its own");
    }

    /// On Wayland a dialog becomes a modal child of the window the caller
    /// exported, as KDE's own dialog does: the compositor keeps it above
    /// that window and sends clicks on the window to it. The other
    /// `OpenXplorer` windows are in other window groups, so stay usable.
    /// Under X11, as check.py runs the tests, there is no exported window
    /// to attach to; the test then only checks the window group.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn on_wayland_a_dialog_is_a_modal_child_of_its_callers_window() {
        let fixture = Fixture::empty();
        let caller = gtk::Window::builder().title("Editor").build();
        caller.present();
        wait_until("the caller's window", || caller.is_mapped());
        let handle: Rc<RefCell<Option<String>>> = Rc::default();
        let exporting = caller
            .surface()
            .and_downcast::<gdk_wayland::WaylandToplevel>()
            .is_some_and(|toplevel| {
                let slot = Rc::clone(&handle);
                toplevel.export_handle(move |_, result| {
                    slot.replace(Some(result.map(ToString::to_string).unwrap_or_default()));
                })
            });
        if exporting {
            wait_until("the exported handle", || handle.borrow().is_some());
        }
        // A compositor without xdg-foreign exports nothing to attach to.
        let parent = handle
            .borrow()
            .clone()
            .filter(|handle| !handle.is_empty())
            .map(|handle| format!("wayland:{handle}"))
            .unwrap_or_default();
        let portal = Portal::new();
        let _answer = portal.call_from(
            &parent,
            "SaveFile",
            &[
                (
                    "current_folder",
                    path_variant(&fixture.root().display().to_string()),
                ),
                ("current_name", "notes.txt".to_variant()),
            ],
        );
        let window = &portal.test.window;
        let picker = window.picker().expect("a picker");
        assert_ne!(
            GtkWindowExt::group(window),
            GtkWindowExt::group(&caller),
            "a window group of its own"
        );
        if !parent.is_empty() {
            assert!(picker.attached_to_caller.get(), "a child of the caller's window");
            assert!(window.is_modal(), "modal while the caller waits");
        }
        caller.destroy();
    }

    /// The replace question, once the dialog shows it on its own layer.
    fn replace_question(test: &TestWindow) -> crate::dialog::DialogFrame {
        wait_until("the replace question", || {
            test.window.dialog_layer().shown().is_some()
        });
        test.window.dialog_layer().shown().expect("the question")
    }

    /// Clicks the button labelled `label` in `frame`.
    fn press_in(frame: &crate::dialog::DialogFrame, label: &str) {
        crate::test_support::harness::descendants::<gtk::Button>(frame)
            .into_iter()
            .find(|button| button.label().as_deref() == Some(label))
            .unwrap_or_else(|| panic!("the question has {label}"))
            .emit_clicked();
    }

    /// Escape cancels with Caps Lock or Num Lock on, from File name where
    /// the keyboard starts; before, the lock made it another key there.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn escape_cancels_with_caps_lock_on() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "notes.txt".to_variant())],
        );
        let window = &portal.test.window;
        let locks = gdk::ModifierType::LOCK_MASK | gdk::ModifierType::from_bits_truncate(1 << 4);
        let handled = window
            .observe_controllers()
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
            .filter(|controller| controller.propagation_phase() == gtk::PropagationPhase::Bubble)
            .any(|controller| {
                controller
                    .emit_by_name::<bool>("key-pressed", &[&gdk::Key::Escape.into_glib(), &0_u32, &locks])
            });
        assert!(handled, "Escape is taken");
        let (response, _) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_CANCELLED);
    }

    /// A dialog whose window refuses to close, here while it writes files,
    /// stays usable and unanswered, and says why, for Save and for Cancel:
    /// answered first, it used to stay open dead, with Save, Cancel and
    /// Escape doing nothing. Once the writing ends, Save answers.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_dialog_that_cannot_close_yet_stays_usable() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "notes.txt".to_variant())],
        );
        let window = &portal.test.window;
        let picker = window.picker().expect("a picker");
        assert!(window.begin_test_write());

        picker.accept.emit_clicked();
        wait_until("the reason", || {
            window
                .shown_message()
                .contains("Wait until the files are written")
        });
        assert!(window.is_visible() && answer.borrow().is_none(), "Save waits");
        assert!(picker.accept.is_sensitive(), "Save works again");
        window.cancel_picking();
        settle();
        assert!(
            window.is_visible() && answer.borrow().is_none(),
            "Cancel waits too"
        );

        window.end_test_write();
        picker.accept.emit_clicked();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("notes.txt")]);
        wait_until("the dialog to close", || !window.is_visible());
    }

    /// No tab moves into or out of a dialog: "Move tab to window" in
    /// another window does not offer it, and it takes no tab, which would
    /// make closing it ask about tabs.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn no_tab_moves_into_a_dialog() {
        let fixture = Fixture::empty();
        let other = TestWindow::open(&fixture.uri());
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let dialog = &portal.test.window;
        assert!(
            !other.window.tab_move_window_ids().contains(&dialog.id()),
            "the dialog is not offered"
        );
        assert!(dialog.is_busy_for_tab_moves(), "the dialog takes no tab");
    }

    /// The replace question is part of the dialog window, not a window of
    /// its own that could open out of sight and take every click and key
    /// from the dialog; while it is open, the dialog can still be ended.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn the_replace_question_is_part_of_the_dialog() {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        let portal = Portal::new();
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "notes.txt".to_variant())],
        );
        let window = &portal.test.window;
        let toplevels = gtk::Window::list_toplevels().len();
        window.picker().expect("a picker").accept.emit_clicked();
        let question = replace_question(&portal.test);
        assert!(question.is_ancestor(window), "inside the dialog window");
        assert_eq!(
            gtk::Window::list_toplevels().len(),
            toplevels,
            "no window of its own"
        );

        window.cancel_picking();
        let (response, _) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_CANCELLED);
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
        enter(&name);
        wait_until("the folder typed", || {
            portal.test.names().contains(&"plan.txt".to_owned())
        });
        assert_eq!(name.text(), "", "the box is cleared for the next name");

        name.set_text("missing.txt");
        enter(&name);
        assert!(window.shown_message().contains("was not found"));
        assert!(answer.borrow().is_none(), "no answer yet");

        name.set_text(&fixture.path("letter.odt").display().to_string());
        enter(&name);
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
        enter(&name);
        assert!(
            window.shown_message().contains("gone.txt"),
            "the missing file is named"
        );
        assert!(answer.borrow().is_none(), "no answer yet");

        name.set_text("\"letter.odt\" \"Drafts\"");
        enter(&name);
        assert_eq!(
            window.shown_message(),
            "“Drafts” is a folder, not a file.",
            "a folder is not reported missing"
        );
        assert!(answer.borrow().is_none(), "no answer yet");

        name.set_text("\"notes.md\" \"letter.odt\"");
        enter(&name);
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
        enter(&name);
        assert!(window.shown_message().contains("does not exist"));
        assert!(answer.borrow().is_none());

        name.set_text("Drafts/report");
        enter(&name);
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
        enter(&name);
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
        enter(&name);
        assert!(window.is_listed(), "the folder is not listed again");
        assert!(answer.borrow().is_none());

        name.set_text("New/");
        enter(&name);
        assert!(window.shown_message().contains("does not exist"));
        assert!(answer.borrow().is_none(), "no New.txt in the folder shown");

        name.set_text(&fixture.uri_of("draft"));
        enter(&name);
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
        let question = replace_question(test);
        assert_eq!(
            question.message().as_str(),
            "“README” already exists. Do you want to replace it?",
            "README itself, not a new README.txt"
        );
        assert_eq!(name.text(), "README");
        press_in(&question, "Cancel");
        wait_until("the question to close", || {
            window.dialog_layer().shown().is_none()
        });
        assert!(answer.borrow().is_none(), "Cancel keeps the dialog open");

        window.submit_address(&fixture.path("Drafts/plan.txt").display().to_string());
        let question = replace_question(test);
        assert_eq!(name.text(), "plan.txt");
        press_in(&question, "Replace");
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

    /// F3 opens no second pane in a dialog, and a pane split off anyway
    /// lists only the type chosen, as the first one does.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn every_pane_of_a_dialog_lists_only_the_type_chosen() {
        use crate::window::session::PaneSide;
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        fixture.write("photo.png");
        let portal = Portal::new();
        let filters = [("Text".to_owned(), vec![(0_u32, "*.txt".to_owned())]).to_variant()];
        let _answer = dialog_on(
            &portal,
            &fixture,
            "OpenFile",
            vec![(
                "filters",
                glib::Variant::array_from_iter_with_type(filters[0].type_(), filters.clone()),
            )],
        );
        let test = &portal.test;
        let window = &test.window;
        wait_until("the text file", || test.names() == ["notes.txt"]);

        press(test, gdk::Key::F3, gdk::ModifierType::empty());
        assert!(
            !gtk::subclass::prelude::ObjectSubclassIsExt::imp(window)
                .end_pane_column
                .is_visible(),
            "F3 splits nothing"
        );

        window.split_tab(None).expect("the folder splits");
        let split = window.pane_on(PaneSide::End);
        wait_until("the split pane's listing", || {
            !window.is_loading() && split.model().n_items() == 1
        });
        settle();
        assert_eq!(split.model().n_items(), 1, "only notes.txt");
    }

    /// Save stays on with a name where nothing can be saved, such as This
    /// PC, and pressing it says why instead of doing nothing; the button's
    /// tooltip says so too.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn saving_where_there_is_no_folder_says_why() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "notes.txt".to_variant())],
        );
        let test = &portal.test;
        let window = &test.window;
        test.wait_for_listing("the starting folder");
        window
            .navigate(crate::locations::Page::ThisPc.uri())
            .expect("This PC opens");
        test.wait_for_listing("This PC");
        let accept = window.picker().expect("a picker").accept.clone();

        assert!(accept.is_sensitive(), "Save is not greyed out silently");
        assert_eq!(
            accept.tooltip_text().as_deref(),
            Some(super::not_local().as_str())
        );
        accept.emit_clicked();
        wait_until("the reason", || window.shown_message() == super::not_local());
        assert!(window.picker().is_some_and(|picker| !picker.reply.is_answered()));
    }

    /// The local path of a share is asked off the main thread: the dialog
    /// does not wait for a share that stopped answering, and choosing
    /// there gives up after the probe's time limit.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_share_s_local_path_is_never_waited_for_on_the_main_thread() {
        use super::local_paths::NEVER_ANSWERS;
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let window = portal.test.window.clone();
        let share = format!("{NEVER_ANSWERS}Reports");

        let started = std::time::Instant::now();
        assert_eq!(window.local_path(&share), None, "not known yet");
        assert!(started.elapsed() < super::probe::PROBE_TIMEOUT, "nothing waited");
        let picker = window.picker().expect("a picker");
        assert!(
            !picker.local_paths.start_asking(&share),
            "being asked off the main thread"
        );
        wait_until("the question to give up", || {
            picker.local_paths.start_asking(&share)
        });
        picker.local_paths.stop_asking(&share);

        let path = wait_for("the path", async move { window.local_path_now(&share).await });
        assert_eq!(path, None, "a share that does not answer has no path");
    }
}
