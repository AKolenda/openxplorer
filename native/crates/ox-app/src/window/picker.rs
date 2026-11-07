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
//!   the choice. Save adds the chosen type's extension to a name that
//!   has neither it nor a known type's, and choosing another type swaps
//!   the old type's extension for the new one's. A file typed in the
//!   address bar is the choice too, a dialog for one file keeps one
//!   selected, Escape and Ctrl+Q cancel, and nothing opens another
//!   window, tab or pane from the dialog.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::integration::{
    checked_name, remembered_location, ChooserAnswer, ChooserCall, ChooserMode, ChooserReply, ChooserRequest,
    DialogFolders, FileFilter, FilterPattern, StartFolder,
};

use super::actions::QUIT_ACCELERATORS;
use super::{BrowserWindow, ButtonStyle, WindowAction};
use crate::dialog::{DialogFrame, DialogWidth};
use crate::folder_view::filter::ChooserListing;
use local_paths::{ask_for_path, path_at_once, Known, LocalPaths};
use probe::{all_writable, probe, test_folder, test_replace, test_write, Probe, WriteTest};

mod local_paths;
mod probe;

/// The size a picker window opens at; it never saves its size over the
/// explorer's.
const DEFAULT_SIZE: (i32, i32) = (980, 640);

/// The window commands that make no sense while choosing a file: more tabs,
/// windows or panes, Settings, Format…, as a dialog closes on its own
/// once answered, leaving no window to report the format in, Full
/// screen, as a Windows Open or Save dialog keeps its size, and Share,
/// which Windows' file dialogs do not have.
const DISABLED_ACTIONS: [WindowAction; 17] = [
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
    WindowAction::FormatDrive,
    WindowAction::FullScreen,
    WindowAction::Share,
];

/// The width, in characters, of an Open dialog's type list, about the
/// width of Windows' type box beside File name.
const OPEN_TYPE_LIST_CHARS: i32 = 24;

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

/// How a dialog's choice is being made ([`BrowserWindow::run_choice`]).
#[derive(Debug)]
enum Choosing {
    /// The accept button, Enter in File name, or Open with several items.
    Accept,
    /// A file activated by double-click, Enter or the address bar.
    Activated(Box<Entry>),
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
    /// The type chosen before the list last changed, an index into the
    /// request's filters, so a Save dialog can swap its extension.
    last_type: Cell<usize>,
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
    /// item clicked with Ctrl or Shift takes its place. The item itself,
    /// not its position: files another application creates or removes
    /// move it in the list.
    single: RefCell<Option<glib::Object>>,
    /// Set once the dialog is a modal child of the caller's window
    /// ([`super::caller_window`]).
    attached_to_caller: Rc<Cell<bool>>,
    /// The calling application, whose last folder the dialog remembers;
    /// `None` when it could not be named, and nothing is remembered.
    caller: Option<String>,
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

    /// The type chosen in the list.
    fn chosen_type(&self) -> Option<&FileFilter> {
        self.request.filters.get(self.chosen_filter()?)
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

/// How long the dialog checks each folder it may start in. Shorter than
/// [`probe::PROBE_TIMEOUT`] where that is long, so a dialog whose last
/// folder is on a share that stopped answering opens at once elsewhere.
#[cfg(not(test))]
const START_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);
/// Shorter than [`probe::PROBE_TIMEOUT`] in tests too, so they tell the
/// two apart.
#[cfg(test)]
const START_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// The location to open for `folder` when it is a folder that answers in
/// time; `None` otherwise. A network location is used only while `GVfs`
/// has it mounted: the dialog never mounts a share or asks for a password
/// on its own.
async fn start_location(folder: StartFolder) -> Option<String> {
    // One deadline for the whole check, asking GVfs for a share's path
    // included, which waits longer on its own.
    glib::future_with_timeout(START_TIMEOUT, start_location_now(folder))
        .await
        .ok()
        .flatten()
}

/// [`start_location`] without its deadline.
async fn start_location_now(folder: StartFolder) -> Option<String> {
    let (uri, path) = match folder {
        StartFolder::Path(path) => (openable_uri(&gio::File::for_path(&path).uri(), &path), path),
        StartFolder::Location(uri) => match path_at_once(&uri) {
            Known::Path(path) => (uri, path),
            Known::NoPath => return None,
            Known::Unknown => match ask_for_path(&uri).await.path() {
                Some(path) => (uri, path),
                None => on_a_mount_with_its_user(&uri).await?,
            },
        },
    };
    (probe(&path).await == Probe::Folder).then_some(uri)
}

/// A remembered `location` that has no path of its own, found on a share
/// `GVfs` mounted with a user: the address the dialog opens and its local
/// path. It is looked up by the mount's address, so nothing is mounted.
async fn on_a_mount_with_its_user(location: &str) -> Option<(String, PathBuf)> {
    let roots: Vec<String> = gio::VolumeMonitor::get()
        .mounts()
        .iter()
        .map(|mount| mount.root().uri().to_string())
        .collect();
    let mounted = address_on_mount(location, &roots)?;
    let path = ask_for_path(&mounted).await.path()?;
    Some(opened_on_mount(path))
}

/// What the dialog opens for `path`, a folder's local path on a share
/// mounted with a user: that path, as no tab takes the mount's address,
/// which holds the user (SAFE-010).
fn opened_on_mount(path: PathBuf) -> (String, PathBuf) {
    (openable_uri(&gio::File::for_path(&path).uri(), &path), path)
}

/// The address a tab opens for the local `path`, whose GIO address is
/// `uri`: that address, which names a share by its own address for a path
/// in `GVfs`'s FUSE export, unless it holds a user (`smb://sam@nas/…`,
/// as for a share mounted with one), which no tab takes (SAFE-010); then
/// the path's literal `file://` address.
fn openable_uri(uri: &str, path: &Path) -> String {
    let has_user = glib::Uri::parse(uri, glib::UriFlags::NONE)
        .ok()
        .and_then(|parsed| parsed.userinfo())
        .is_some();
    if !has_user {
        return uri.to_owned();
    }
    glib::filename_to_uri(path, None).map_or_else(|_| String::new(), |literal| literal.to_string())
}

/// The address of the remembered `location` on one of the mounts whose
/// root addresses are `roots`: the memory keeps a location without its
/// user ([`remembered_location`]), while `GVfs` names a share mounted with
/// a user by an address that holds it (`sftp://anna@build.example/`), and
/// gives the address without it no path. `None` when no mount holds it,
/// or when mounts with different users do: the memory cannot tell which
/// account the folder was used through.
fn address_on_mount(location: &str, roots: &[String]) -> Option<String> {
    let mut found = roots.iter().filter_map(|root| {
        // A mount without a user has the location's own path already.
        glib::Uri::parse(root, glib::UriFlags::NONE).ok()?.userinfo()?;
        let kept = remembered_location(root)?;
        let rest = location.strip_prefix(kept.trim_end_matches('/'))?;
        (rest.is_empty() || rest.starts_with('/')).then(|| format!("{}{rest}", root.trim_end_matches('/')))
    });
    let first = found.next()?;
    found.all(|other| other == first).then_some(first)
}

/// The dialog's message when the caller could not write `name` in
/// `folder`, in Windows' words; `Ok` when it can.
fn write_refusal(test: WriteTest, folder: &Path, name: &str) -> Result<(), String> {
    let folder_text = folder.display().to_string();
    match test {
        // The caller looks again and asks before replacing it.
        WriteTest::Writable | WriteTest::Appeared => Ok(()),
        WriteTest::Denied => Err(ox_core::i18n::gettext(
            "You don't have permission to save in this location. Contact the administrator to obtain \
             permission.",
        )),
        WriteTest::ReadOnlyFile => Err(ox_core::i18n::format_message(
            "“{name}” is set to read-only. Try again with a different file name.",
            &[("name", name)],
        )),
        WriteTest::NoSpace => Err(ox_core::i18n::format_message(
            "There is not enough space in “{folder}” to save the file.",
            &[("folder", &folder_text)],
        )),
        WriteTest::Unavailable => Err(ox_core::i18n::format_message(
            "“{folder}” is not available. If it is on a network, make sure the share is still connected.",
            &[("folder", &folder_text)],
        )),
        WriteTest::NoAnswer => Err(not_answering(folder)),
    }
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

/// The extension of a type: that of its first pattern that is a plain
/// extension ([`glob_extension`]), as Windows adds it to a name saved
/// without one. All Files (`*`, `*.*`) has none.
fn type_extension(filter: &FileFilter) -> Option<String> {
    filter.patterns.iter().find_map(|pattern| match pattern {
        FilterPattern::Glob(glob) => glob_extension(glob),
        FilterPattern::MimeType(_) => None,
    })
}

/// The name a Save dialog saves `written` as, with `chosen` the type
/// chosen. As in Windows, a name that ends in a dot is saved without that
/// dot and without an extension, and any other name gets the type's
/// extension unless it fits the type already or ends in the extension
/// of a file type the desktop knows ([`has_known_extension`]): in a PDF
/// dialog "Notes v2.1" is saved as "Notes v2.1.pdf", while "notes.md" in
/// a text one stays as it is. A hidden file's name (`.bashrc`) is kept
/// as written, as Linux users expect.
fn saved_name(written: &str, chosen: Option<&FileFilter>) -> String {
    if let Some(name) = written.strip_suffix('.') {
        return name.to_owned();
    }
    let Some((chosen, extension)) = chosen.and_then(|chosen| Some((chosen, type_extension(chosen)?))) else {
        return written.to_owned();
    };
    if written.starts_with('.') || chosen.matches(written, None) || has_known_extension(written) {
        written.to_owned()
    } else {
        format!("{written}.{extension}")
    }
}

/// Whether `name` ends in the extension of a file type the desktop knows,
/// as Windows keeps a name typed with a registered extension: GIO's guess
/// from the name alone is not `application/octet-stream`, its answer for
/// an extension it does not know. GIO also calls a guess uncertain when
/// several types share an extension (`.png`, `.html`, `.json`), so that
/// alone does not make it unknown. An extension of digits only ("v2.1",
/// "Q3.2025") is taken for part of the name, as people number versions
/// that way, though the desktop knows `.1` as a manual page.
fn has_known_extension(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty() || extension.is_empty() || extension.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let (guessed, _) = gio::content_type_guess(Some(name), None);
    !gio::content_type_is_unknown(&guessed)
}

/// The name File name takes when a Save dialog's type changes from
/// `from` to `to`, as GTK's and Windows' dialogs do: a name that ends in
/// one of the old type's extensions gets the new type's instead
/// ("Title.html" becomes "Title.mhtml"). `None` leaves the name alone: it
/// has none of those extensions, fits the new type already, or the new
/// type has no extension of its own (All Files).
fn renamed_for_type(name: &str, from: &FileFilter, to: &FileFilter) -> Option<String> {
    let extension = type_extension(to)?;
    if to.matches(name, None) {
        return None;
    }
    let old = from
        .patterns
        .iter()
        .filter_map(|pattern| match pattern {
            FilterPattern::Glob(glob) => glob_extension(glob),
            FilterPattern::MimeType(_) => None,
        })
        .map(|old| format!(".{old}"))
        .filter(|old| {
            let start = name.len().saturating_sub(old.len());
            start > 0 && name.is_char_boundary(start) && name[start..].to_lowercase() == old.to_lowercase()
        })
        .max_by_key(String::len)?;
    Some(format!("{}.{extension}", &name[..name.len() - old.len()]))
}

impl BrowserWindow {
    /// Turns this new window into the dialog for `call` and shows it.
    pub(crate) fn begin_picking(&self, call: ChooserCall) {
        let ChooserCall {
            request,
            reply,
            parent_window,
            caller,
            ..
        } = call;
        let start_folders = self.start_folders(&request, caller.as_deref());
        let mut picker = self.build_picker_bar(request, reply.clone());
        picker.caller = caller;
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
        for action in DISABLED_ACTIONS {
            self.set_action_enabled(action, false);
        }
        self.update_share_command();
        self.set_default_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        self.apply_chooser_listing();
        self.open_start_folder(start_folders);
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

    /// The folders the dialog for `request` may start in, in order, by
    /// Windows' rule ([`ox_core::integration::start_folders`]): the
    /// caller's file or folder, its last folder, then Documents, the
    /// Desktop and the home folder.
    fn start_folders(&self, request: &ChooserRequest, caller: Option<&str>) -> Vec<StartFolder> {
        let memory = caller.and_then(|app| {
            DialogFolders::open(&self.context().settings_directory())
                .of(app)
                .cloned()
        });
        let file_folder = request
            .folder_of_file
            .then_some(request.current_folder.as_deref())
            .flatten();
        let initial = request.suggested_folder.as_deref();
        let known = ox_core::places::FolderLocations::from_environment().read_paths();
        let defaults = [
            known.path(ox_core::places::KnownFolder::Documents).to_path_buf(),
            known.path(ox_core::places::KnownFolder::Desktop).to_path_buf(),
            glib::home_dir(),
        ];
        ox_core::integration::start_folders(file_folder, initial, memory.as_ref(), &defaults)
    }

    /// Opens the first of `folders` that is there, else the home folder.
    /// Each is checked off the main thread and for a short time only
    /// ([`START_TIMEOUT`]): a folder that is gone, or on a share that is
    /// not connected or stopped answering, is passed over without an
    /// error, as Windows' dialog falls back to the next folder.
    fn open_start_folder(&self, folders: Vec<StartFolder>) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                for folder in folders {
                    let found = start_location(folder).await;
                    if window.picker().is_none_or(|picker| picker.reply.is_answered()) {
                        return;
                    }
                    if let Some(uri) = found {
                        if window.add_tab(&uri).is_ok() {
                            return;
                        }
                    }
                }
                window.open_first_tab(&glib::home_dir());
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
    /// bar): it is the choice in an Open dialog, and the file to save in
    /// a Save dialog, checked as the Save button checks it and replaced
    /// only after asking ([`Self::run_choice`]).
    pub(super) fn pick_activated(&self, entry: &Entry) {
        self.run_choice(Choosing::Activated(Box::new(entry.clone())));
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
    ///
    /// This is the one place a dialog sends its answer (the portal's own
    /// Close aside, which ox-core answers), and so the one place its folder
    /// is remembered: only for a choice really sent, never for a close
    /// that was refused or a dialog the portal closed first.
    pub(super) fn end_picking_on_close(&self) {
        let Some(picker) = self.picker() else {
            return;
        };
        if picker.reply.is_answered() {
            return;
        }
        let answer = picker.pending.take().unwrap_or(ChooserAnswer::Cancelled);
        if let ChooserAnswer::Chosen { locations, .. } = &answer {
            self.remember_folder(&picker, locations);
        }
        picker.reply.send(&answer);
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

    /// Whether this window is a file dialog for another application
    /// (INT-032).
    pub(super) fn is_file_dialog(&self) -> bool {
        self.picker().is_some()
    }

    /// The window's picker, if it is one.
    fn picker(&self) -> Option<Rc<Picker>> {
        self.imp().picker.borrow().clone()
    }

    /// Gives the name in a Save dialog's File name the extension of the
    /// type just chosen when it had the previous type's
    /// ([`renamed_for_type`]), and selects the name without it again.
    fn rename_for_chosen_type(&self) {
        let Some(picker) = self.picker() else {
            return;
        };
        let Some(chosen) = picker.chosen_filter() else {
            return;
        };
        let previous = picker.last_type.replace(chosen);
        let (ChooserMode::Save { .. }, Some(name)) = (&picker.request.mode, &picker.name) else {
            return;
        };
        let filters = &picker.request.filters;
        let (Some(from), Some(to)) = (filters.get(previous), filters.get(chosen)) else {
            return;
        };
        if let Some(renamed) = renamed_for_type(&name.text(), from, to) {
            name.set_text(&renamed);
            select_stem(name);
        }
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

    /// The accept button, Enter in File name, and Open with several
    /// items selected: works out the choice and answers, asking first
    /// before replacing files ([`Self::run_choice`]).
    fn accept_choice(&self) {
        self.run_choice(Choosing::Accept);
    }

    /// Every way a dialog's choice is made goes through here: the accept
    /// button, Enter in File name, Open with several items, and a file
    /// activated by double-click, Enter or the address bar. The choice is
    /// worked out off the main thread with the same checks, a Save's
    /// write test included, while the accept button waits, so a share
    /// that stopped answering cannot freeze the app; then the dialog
    /// answers ([`Self::finish_picking`]), asks before replacing, or says
    /// why it cannot.
    fn run_choice(&self, choosing: Choosing) {
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
                let outcome = match &choosing {
                    Choosing::Accept => window.work_out_choice(&picker).await,
                    Choosing::Activated(entry) => window.activated_choice(&picker, entry).await,
                };
                picker.checking.set(false);
                window.update_picker();
                if picker.reply.is_answered() {
                    return;
                }
                match outcome {
                    Ok(locations) if !locations.is_empty() => window.finish_picking(&picker, locations),
                    Err(message) if !message.is_empty() => window.show_message(&message),
                    // Nothing to answer yet: a folder opened, or a question is open.
                    Ok(_) | Err(_) => {}
                }
            }
        ));
    }

    /// The locations an activated file chooses: the file itself in an
    /// Open dialog; in a Save dialog the file to replace, checked as
    /// [`Self::checked_save_target`] checks a typed name.
    async fn activated_choice(&self, picker: &Rc<Picker>, entry: &Entry) -> Result<Vec<PathBuf>, String> {
        match &picker.request.mode {
            ChooserMode::Open { directory: false, .. } => {
                // The activated file is the choice, selected or not.
                let path = self.local_path_now(&entry.uri).await.ok_or_else(not_local)?;
                Ok(vec![path])
            }
            ChooserMode::Save { .. } => {
                let path = self.local_path_now(&entry.uri).await.ok_or_else(not_local)?;
                if let Some(name) = &picker.name {
                    name.set_text(&entry.name);
                }
                // The file's real name, bytes and all; the name shown
                // (Entry::name, a display name) is only for what the
                // dialog says.
                let folder = path.parent().map(Path::to_path_buf).ok_or_else(not_local)?;
                let real = path
                    .file_name()
                    .map(std::ffi::OsStr::to_os_string)
                    .ok_or_else(not_local)?;
                self.checked_save_target(picker, folder, real, entry.name.clone())
                    .await
            }
            ChooserMode::Open { directory: true, .. } | ChooserMode::SaveFiles { .. } => Ok(Vec::new()),
        }
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
            ChooserMode::Open {
                directory: true,
                multiple,
            } => self.chosen_folders(&selected, *multiple).await,
            ChooserMode::Save { .. } => self.chosen_save(picker).await,
            ChooserMode::SaveFiles { names } => {
                let folder = self.chosen_folder(&selected).await?;
                // The names as the caller gave them, bytes and all; their
                // text is only for what the dialog says.
                let mut existing = Vec::new();
                for name in names {
                    match probe(&folder.join(name)).await {
                        Probe::NoAnswer => return Err(not_answering(&folder)),
                        found if found.exists() => existing.push(name),
                        _ => {}
                    }
                }
                // The folder takes new files (a file of the dialog's
                // own), and every file to be replaced may be written.
                let first = names.first().map(|name| name.to_string_lossy().into_owned());
                write_refusal(test_folder(&folder).await, &folder, &first.unwrap_or_default())?;
                // Another program may have created one of the other files
                // meanwhile: it is replaced only after asking too.
                for name in names {
                    if existing.contains(&name) {
                        continue;
                    }
                    match probe(&folder.join(name)).await {
                        Probe::NoAnswer => return Err(not_answering(&folder)),
                        found if found.exists() => existing.push(name),
                        _ => {}
                    }
                }
                for name in &existing {
                    let test = test_replace(&folder.join(name)).await;
                    write_refusal(test, &folder, &name.to_string_lossy())?;
                }
                let existing: Vec<String> = existing
                    .iter()
                    .map(|name| name.to_string_lossy().into_owned())
                    .collect();
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

    /// All selected folders for an Open dialog; the current folder only
    /// when no items were selected.
    async fn chosen_folders(&self, selected: &[Entry], multiple: bool) -> Result<Vec<PathBuf>, String> {
        if selected.is_empty() {
            return self.chosen_folder(selected).await.map(|folder| vec![folder]);
        }
        if selected.len() > 1 && !multiple {
            return Err(ox_core::i18n::gettext(CHOOSE_ONE));
        }
        let mut paths = Vec::with_capacity(selected.len());
        for entry in selected {
            if !entry.is_dir {
                return Ok(Vec::new());
            }
            paths.push(
                self.local_path_now(entry.navigation_uri())
                    .await
                    .ok_or_else(not_local)?,
            );
        }
        Ok(paths)
    }

    /// The folder of `SaveFiles`: the one selected
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
        let name = checked_name(&saved_name(&written, picker.chosen_type())).map_err(|_| bad_name())?;
        self.checked_save_target(picker, folder, name.clone().into(), name)
            .await
    }

    /// The file `name` in `folder` a Save dialog answers with, by the same
    /// checks whether it was typed or activated: a folder of that name
    /// opens; the caller must be able to write there ([`test_write`],
    /// which also finds a file another program creates meanwhile); an
    /// existing file is replaced only after asking. `name` is the real
    /// file name; `label` is how the dialog names it to the user.
    async fn checked_save_target(
        &self,
        picker: &Rc<Picker>,
        folder: PathBuf,
        name: std::ffi::OsString,
        label: String,
    ) -> Result<Vec<PathBuf>, String> {
        let Some(name_box) = &picker.name else {
            return Ok(Vec::new());
        };
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
        let mut exists = found.exists();
        let mut test = test_write(&folder, name.as_ref(), exists).await;
        if test == WriteTest::Appeared {
            // Another program created the file while the folder was tested:
            // look again, and never answer it as a new file.
            match probe(&target).await {
                Probe::NoAnswer => return Err(not_answering(&folder)),
                Probe::Folder => {
                    name_box.set_text("");
                    self.navigate_or_report(&gio::File::for_path(&target).uri());
                    return Ok(Vec::new());
                }
                Probe::File | Probe::Missing => {
                    exists = true;
                    test = test_write(&folder, name.as_ref(), true).await;
                }
            }
        }
        write_refusal(test, &folder, &label)?;
        if exists {
            self.confirm_replace(picker, vec![target], &[label]);
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
                    let only = (selected.size() == 1)
                        .then(|| selection.item(selected.minimum()))
                        .flatten();
                    picker.single.replace(only);
                    return;
                }
                // The item just selected: the end of the changed range away
                // from the one kept so far, as Shift+click selects up or
                // down to the item clicked.
                let kept = picker.single.borrow().as_ref().and_then(|kept| {
                    let (rest, first) = gtk::BitsetIter::init_first(&selected)?;
                    std::iter::once(first)
                        .chain(rest)
                        .find(|position| selection.item(*position).as_ref() == Some(kept))
                });
                let last = position + count.saturating_sub(1);
                let newest = if kept.is_some_and(|kept| kept >= last) {
                    position
                } else {
                    last
                };
                let item = Some(newest).filter(|item| selected.contains(*item)).or(kept);
                if let Some(item) = item {
                    picker.single.replace(selection.item(item));
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
                // The portal may have closed the dialog meanwhile.
                if picker.reply.is_answered() {
                    return;
                }
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

    /// Remembers where the user chose `locations` as the caller's last
    /// folder, shared by its Open and Save dialogs, as Windows does when a
    /// dialog is answered (never when it is cancelled; called only as the
    /// choice is sent, [`Self::end_picking_on_close`]): the folder of the
    /// file chosen, the folder chosen, or for a folder dialog the folder
    /// shown. A folder is kept by its own location, so a share is
    /// remembered as `smb://…` rather than its FUSE path, and without any
    /// user name or password. A memory that cannot be written only loses
    /// the memory, never the answer.
    fn remember_folder(&self, picker: &Picker, locations: &[PathBuf]) {
        let Some(caller) = picker.caller.as_deref() else {
            return;
        };
        let Some(first) = locations.first() else {
            return;
        };
        let shown = self.current_uri().and_then(|uri| {
            let path = picker.local_paths.known(&uri).path()?;
            Some((uri, path))
        });
        let folder = match &picker.request.mode {
            ChooserMode::SaveFiles { .. } => first.clone(),
            ChooserMode::Open { directory: true, .. } => match &shown {
                Some((_, path)) => path.clone(),
                None => first.parent().map_or_else(|| first.clone(), Path::to_path_buf),
            },
            ChooserMode::Open { .. } | ChooserMode::Save { .. } => match first.parent() {
                Some(parent) => parent.to_path_buf(),
                None => return,
            },
        };
        // A share's FUSE path, typed in File name, is kept by the share's
        // own address too: GVfs gives it as the path's location.
        let location = match shown {
            Some((uri, path)) if path == folder => uri,
            _ => gio::File::for_path(&folder).uri().to_string(),
        };
        let initial = picker.request.suggested_folder.as_deref();
        let _ =
            DialogFolders::open(&self.context().settings_directory()).remember(caller, initial, &location);
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
        // Windows' Open dialog has no caption for its type list: the list
        // sits right of File name, above Open and Cancel. "Save as type:"
        // is the Save dialog's own row.
        let opens_files = matches!(request.mode, ChooserMode::Open { directory: false, .. });
        let types = (!request.filters.is_empty()).then(|| {
            let labels: Vec<&str> = request
                .filters
                .iter()
                .map(|filter| filter.name.as_str())
                .collect();
            let list = gtk::DropDown::from_strings(&labels);
            list.set_selected(u32::try_from(request.current_filter.unwrap_or(0)).unwrap_or(0));
            if opens_files {
                place_open_type_list(&fields, &list);
            } else {
                let caption = if name.is_some() {
                    "Save as type:"
                } else {
                    "File type:"
                };
                attach_field(&fields, row, caption, &list);
                row += 1;
            }
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
            // The buttons are on a row of their own under the fields, at
            // the right, as in Windows' Open and Save dialogs.
            bar.set_orientation(gtk::Orientation::Vertical);
            bar.set_spacing(12);
            buttons.set_halign(gtk::Align::End);
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
            last_type: Cell::new(types.as_ref().map_or(0, |types| types.selected() as usize)),
            types,
            local_paths: LocalPaths::default(),
            choices,
            accept,
            asking: Cell::new(false),
            pending: RefCell::new(None),
            checking: Cell::new(false),
            single: RefCell::new(None),
            attached_to_caller: Rc::default(),
            caller: None,
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
                        window.rename_for_chosen_type();
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

/// Puts an Open dialog's type list right of File name, on row 0 of
/// `fields`, without a caption, as Windows' Open dialog has it. A long
/// type name is shortened in the closed list and shown whole in its
/// popup, so the list keeps the width of Windows' type box.
fn place_open_type_list(fields: &gtk::Grid, list: &gtk::DropDown) {
    list.set_factory(Some(&type_name_factory(true)));
    list.set_list_factory(Some(&type_name_factory(false)));
    list.set_hexpand(false);
    list.add_css_class("picker-type-list");
    // Unlabeled on screen, still named for screen readers.
    list.update_property(&[gtk::accessible::Property::Label("File type")]);
    fields.attach(list, 2, 0, 1, 1);
}

/// Rows that show a type's name: `shortened` to the type list's width,
/// with the whole name as a tooltip, or whole.
fn type_name_factory(shortened: bool) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let label = gtk::Label::builder().xalign(0.0).build();
        if shortened {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_width_chars(OPEN_TYPE_LIST_CHARS);
            label.set_max_width_chars(OPEN_TYPE_LIST_CHARS);
        }
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&label));
        }
    });
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let name = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .map(|name| name.string());
        if let (Some(label), Some(name)) = (item.child().and_downcast::<gtk::Label>(), name) {
            label.set_label(&name);
            if shortened {
                label.set_tooltip_text(Some(&name));
            }
        }
    });
    factory
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
    use gtk::subclass::prelude::*;
    use gtk::{gdk, gio, glib};
    use ox_core::integration::{
        options_from_entries, path_variant, FileChooserBus, FileFilter, FilterPattern,
        FILE_CHOOSER_INTERFACE, PORTAL_BACKEND_PATH, RESPONSE_CANCELLED, RESPONSE_SUCCESS,
    };

    use crate::test_support::harness::{
        capture, settle, wait_for as wait_a_while, wait_until, Fixture, TestWindow,
    };
    use crate::test_support::pointer::{screen_point, Button, Pointer};
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

        /// Starts `method` with `entries` for the application the portal
        /// names `app_id`.
        fn call_for(
            &self,
            app_id: &str,
            method: &str,
            entries: &[(&str, glib::Variant)],
        ) -> Rc<RefCell<Option<(u32, glib::VariantDict)>>> {
            self.call_as(app_id, "", method, entries)
        }

        /// Starts `method` with `entries` for the caller's window
        /// `parent`, as the portal names it (`wayland:<handle>`).
        fn call_from(
            &self,
            parent: &str,
            method: &str,
            entries: &[(&str, glib::Variant)],
        ) -> Rc<RefCell<Option<(u32, glib::VariantDict)>>> {
            self.call_as("org.example.Editor", parent, method, entries)
        }

        /// Starts `method` with `entries` for `app_id`'s window `parent`.
        fn call_as(
            &self,
            app_id: &str,
            parent: &str,
            method: &str,
            entries: &[(&str, glib::Variant)],
        ) -> Rc<RefCell<Option<(u32, glib::VariantDict)>>> {
            let handle =
                glib::variant::ObjectPath::try_from("/org/freedesktop/portal/desktop/request/1_1/picker")
                    .expect("a path");
            let parameters = glib::Variant::tuple_from_iter([
                handle.to_variant(),
                app_id.to_variant(),
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
    /// freeze the app: the dialog opens in the next folder Windows would
    /// use (here the home folder, as the test home has no Documents or
    /// Desktop), without an error.
    ///
    /// parity: INT-032, INT-035
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
            window.shown_message().is_empty(),
            "Windows falls back without an error: {}",
            window.shown_message()
        );
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// An Open or Save dialog has no Share, even where the desktop can
    /// share, as Windows' file dialogs have none.
    ///
    /// parity: INT-032, CMD-037
    #[gtk::test]
    fn the_dialog_has_no_share() {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        let email = crate::test_support::portal::EmailPortal::answering(0);
        let portal = Portal::new();
        let answer = portal.call(
            "OpenFile",
            &[(
                "current_folder",
                path_variant(&fixture.root().display().to_string()),
            )],
        );
        let window = &portal.test.window;
        window.follow_share_service(&email.name());
        wait_until("the window to ask the portal", || {
            window.share_service_checked(&email.name())
        });
        window.folder_model().select_only(0);
        settle();

        let share = crate::test_support::harness::descendants::<gtk::Button>(window)
            .into_iter()
            .find(|button| button.action_name().as_deref() == Some("win.share"))
            .expect("the command bar has Share");
        assert!(!share.is_visible(), "hidden in a dialog");
        assert!(
            !window.lookup_action("share").expect("the action").is_enabled(),
            "and disabled"
        );
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// Saving into a folder whose share stopped answering says so and
    /// answers nothing; the dialog stays usable.
    ///
    /// Answers the open dialog of `portal` with the file `name` in
    /// `folder`, typed as a full path in File name, and waits for the
    /// answer.
    fn save_as(
        portal: &Portal,
        answer: &Rc<RefCell<Option<(u32, glib::VariantDict)>>>,
        path: &std::path::Path,
    ) {
        let window = &portal.test.window;
        let picker = window.picker().expect("a picker");
        let name = picker.name.as_ref().expect("a name box");
        name.set_text(&path.display().to_string());
        enter(name);
        let (response, uris) = Portal::finish(answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [gio::File::for_path(path).uri().to_string()]);
    }

    /// The folder `window` shows, as a local path.
    fn shown_folder(window: &crate::window::BrowserWindow) -> Option<std::path::PathBuf> {
        window
            .current_uri()
            .and_then(|uri| gio::File::for_uri(&uri).path())
    }

    /// Copies the dialog memory of `from` into `to`'s settings folder, as
    /// the file is still there when the app starts again.
    fn restart_with_memory_of(from: &Portal, to: &Portal) {
        let file = from
            .test
            .settings_directory()
            .join(ox_core::integration::DIALOG_FOLDERS_FILE);
        fs::copy(
            &file,
            to.test
                .settings_directory()
                .join(ox_core::integration::DIALOG_FOLDERS_FILE),
        )
        .expect("the memory was written");
    }

    /// Each application's dialogs start where the user last chose a file
    /// in them, Open and Save alike, also after the app starts again; another
    /// application keeps its own, and a new folder passed by the
    /// application is honoured, as in Windows.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn each_apps_dialogs_start_in_its_last_folder() {
        let fixture = Fixture::empty();
        let downloads = fixture.path("Downloads");
        let reports = fixture.path("Reports");
        let pictures = fixture.path("Pictures");
        for folder in [&downloads, &reports, &pictures] {
            fs::create_dir(folder).expect("a folder");
        }
        let suggested = |folder: &std::path::Path| {
            vec![
                ("current_folder", path_variant(&folder.display().to_string())),
                ("current_name", "Quarterly notes.html".to_variant()),
            ]
        };

        let first = Portal::new();
        let answer = first.call_for("org.example.Browser", "SaveFile", &suggested(&downloads));
        assert_eq!(
            shown_folder(&first.test.window).as_deref(),
            Some(downloads.as_path())
        );
        save_as(&first, &answer, &reports.join("Quarterly notes.html"));

        // The app starts again: its Open dialog suggests Downloads again.
        let open = Portal::new();
        restart_with_memory_of(&first, &open);
        let answer = open.call_for(
            "org.example.Browser",
            "OpenFile",
            &[("current_folder", path_variant(&downloads.display().to_string()))],
        );
        assert_eq!(
            shown_folder(&open.test.window).as_deref(),
            Some(reports.as_path()),
            "Open starts where Save was last used"
        );
        open.test.window.cancel_picking();
        Portal::finish(&answer);

        let other = Portal::new();
        restart_with_memory_of(&first, &other);
        let answer = other.call_for("org.example.Mail", "SaveFile", &suggested(&downloads));
        assert_eq!(
            shown_folder(&other.test.window).as_deref(),
            Some(downloads.as_path()),
            "another app has its own memory"
        );
        other.test.window.cancel_picking();
        Portal::finish(&answer);

        let moved = Portal::new();
        restart_with_memory_of(&first, &moved);
        let answer = moved.call_for("org.example.Browser", "SaveFile", &suggested(&pictures));
        assert_eq!(
            shown_folder(&moved.test.window).as_deref(),
            Some(pictures.as_path()),
            "a folder other than the first one passed is the app's choice"
        );
        moved.test.window.cancel_picking();
        Portal::finish(&answer);
    }

    /// A Save that names both a file and a folder opens the file's folder
    /// and still remembers the last folder under the folder the app
    /// suggested, so the app's next Save from that folder starts there.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn a_save_naming_a_file_and_a_folder_remembers_by_the_folder() {
        let fixture = Fixture::empty();
        let downloads = fixture.path("Downloads");
        let letters = fixture.path("Letters");
        let reports = fixture.path("Reports");
        for folder in [&downloads, &letters, &reports] {
            fs::create_dir(folder).expect("a folder");
        }

        let first = Portal::new();
        let answer = first.call_for(
            "org.example.Editor",
            "SaveFile",
            &[
                ("current_folder", path_variant(&downloads.display().to_string())),
                (
                    "current_file",
                    path_variant(&letters.join("draft.odt").display().to_string()),
                ),
            ],
        );
        assert_eq!(
            shown_folder(&first.test.window).as_deref(),
            Some(letters.as_path()),
            "the file's own folder opens"
        );
        save_as(&first, &answer, &reports.join("draft.odt"));

        let next = Portal::new();
        restart_with_memory_of(&first, &next);
        let answer = next.call_for(
            "org.example.Editor",
            "SaveFile",
            &[
                ("current_folder", path_variant(&downloads.display().to_string())),
                ("current_name", "notes.odt".to_variant()),
            ],
        );
        assert_eq!(
            shown_folder(&next.test.window).as_deref(),
            Some(reports.as_path()),
            "the same suggested folder reopens the last one"
        );
        next.test.window.cancel_picking();
        Portal::finish(&answer);
    }

    /// A file the application names with its full path opens its own
    /// folder, before the remembered one; a remembered folder that is gone
    /// or on a share that stopped answering is passed over quickly and
    /// without an error.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn a_full_path_wins_and_a_lost_last_folder_is_passed_over() {
        let fixture = Fixture::empty();
        let letters = fixture.path("Letters");
        let downloads = fixture.path("Downloads");
        let share = fixture.path("Share");
        for folder in [&letters, &downloads, &share] {
            fs::create_dir(folder).expect("a folder");
        }
        let remember = |portal: &Portal, last: &std::path::Path| {
            ox_core::integration::DialogFolders::open(portal.test.settings_directory())
                .remember(
                    "org.example.Editor",
                    Some(&downloads),
                    &gio::File::for_path(last).uri(),
                )
                .expect("the memory is written");
        };

        // While the caller passes its first folder, the last one opens.
        let reopening = Portal::new();
        remember(&reopening, &share);
        let answer = reopening.call(
            "OpenFile",
            &[("current_folder", path_variant(&downloads.display().to_string()))],
        );
        assert_eq!(
            shown_folder(&reopening.test.window).as_deref(),
            Some(share.as_path())
        );
        reopening.test.window.cancel_picking();
        Portal::finish(&answer);

        let saving = Portal::new();
        remember(&saving, &share);
        let answer = saving.call(
            "SaveFile",
            &[(
                "current_file",
                path_variant(&letters.join("draft.odt").display().to_string()),
            )],
        );
        assert_eq!(
            shown_folder(&saving.test.window).as_deref(),
            Some(letters.as_path())
        );
        saving.test.window.cancel_picking();
        Portal::finish(&answer);

        let gone = Portal::new();
        remember(&gone, &fixture.path("Removed"));
        let started = std::time::Instant::now();
        let answer = gone.call(
            "OpenFile",
            &[("current_folder", path_variant(&downloads.display().to_string()))],
        );
        assert_eq!(
            shown_folder(&gone.test.window).as_deref(),
            Some(downloads.as_path())
        );
        assert!(
            gone.test.window.shown_message().is_empty(),
            "{}",
            gone.test.window.shown_message()
        );
        gone.test.window.cancel_picking();
        Portal::finish(&answer);

        let offline = Portal::new();
        remember(&offline, &share);
        super::probe::stop_answering(&share);
        let answer = offline.call(
            "OpenFile",
            &[("current_folder", path_variant(&downloads.display().to_string()))],
        );
        assert_eq!(
            shown_folder(&offline.test.window).as_deref(),
            Some(downloads.as_path())
        );
        assert!(offline.test.window.shown_message().is_empty());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "no dialog waited long for a folder"
        );
        offline.test.window.cancel_picking();
        Portal::finish(&answer);
    }

    /// A file chosen in a Save dialog by double-click, Enter or the
    /// address bar goes through the same check as the Save button: a
    /// read-only file is reported, not offered for replacing.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn activating_a_read_only_file_in_a_save_dialog_says_so() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::empty();
        fixture.write("signed.txt");
        let signed = fixture.path("signed.txt");
        fs::set_permissions(&signed, fs::Permissions::from_mode(0o444)).expect("read-only");
        let portal = Portal::new();
        let answer = dialog_on(&portal, &fixture, "SaveFile", Vec::new());
        let test = &portal.test;
        let window = &test.window;

        window.activate_item(test.position_of("signed.txt"));
        wait_until("the dialog to say so", || {
            window
                .shown_message()
                .contains("“signed.txt” is set to read-only")
        });
        assert!(window.dialog_layer().shown().is_none(), "no replace question");
        window.submit_address(&signed.display().to_string());
        wait_until("the check of the typed file", || {
            window.picker().is_some_and(|picker| !picker.checking.get())
        });
        settle();
        assert!(window.dialog_layer().shown().is_none(), "nor for the address bar");
        assert!(answer.borrow().is_none(), "nothing is answered");

        fs::set_permissions(&signed, fs::Permissions::from_mode(0o644)).expect("writable again");
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// Save files checks the file it would replace by its own name, also
    /// one that is not UTF-8.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn saving_files_checks_a_name_that_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::empty();
        let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
        let existing = fixture.root().join(name);
        fs::write(&existing, "signed").expect("a file");
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o444)).expect("read-only");
        let portal = Portal::new();
        let names: Vec<Vec<u8>> = vec![b"caf\xe9.txt\0".to_vec()];
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFiles",
            vec![("files", names.to_variant())],
        );
        let window = &portal.test.window;

        window.picker().expect("a picker").accept.emit_clicked();
        wait_until("the dialog to say so", || {
            window.shown_message().contains("is set to read-only")
        });
        assert!(window.dialog_layer().shown().is_none(), "no replace question");
        assert!(answer.borrow().is_none(), "nothing is answered");

        fs::set_permissions(&existing, fs::Permissions::from_mode(0o644)).expect("writable again");
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// The folder is remembered only when the choice is really sent: a
    /// dialog the portal closes while it still checks the file chosen
    /// remembers nothing.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn a_dialog_closed_before_it_answers_remembers_nothing() {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        let portal = Portal::new();
        let answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let test = &portal.test;
        // The check of whether the caller may write the file waits.
        super::probe::stop_answering(&fixture.path("notes.txt"));

        test.window.activate_item(test.position_of("notes.txt"));
        let closing = portal.frontend.call_future(
            Some(&portal.backend_name),
            "/org/freedesktop/portal/desktop/request/1_1/picker",
            "org.freedesktop.impl.portal.Request",
            "Close",
            None,
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            5000,
        );
        wait_for("the portal's Close", closing).expect("the portal may close it");
        assert_eq!(Portal::finish(&answer).0, ox_core::integration::RESPONSE_OTHER);
        wait_for(
            "the write check to end",
            glib::timeout_future(super::probe::PROBE_TIMEOUT * 3),
        );

        let memory = test
            .settings_directory()
            .join(ox_core::integration::DIALOG_FOLDERS_FILE);
        assert!(!memory.exists(), "a dialog that did not answer remembers nothing");
    }

    /// A file activated in a Save dialog is checked and answered by its
    /// real name, also one that is not UTF-8, whose shown name differs.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn an_activated_file_is_saved_by_its_real_name() {
        use std::os::unix::ffi::OsStrExt;
        let fixture = Fixture::empty();
        let real = fixture.root().join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
        fs::write(&real, "old").expect("a file");
        let portal = Portal::new();
        let answer = dialog_on(&portal, &fixture, "SaveFile", Vec::new());
        let test = &portal.test;
        let shown = test
            .names()
            .into_iter()
            .find(|name| name.starts_with("caf"))
            .expect("the file is listed");

        test.window.activate_item(test.position_of(&shown));
        let question = replace_question(test);
        press_in(&question, "Replace");

        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [gio::File::for_path(&real).uri().to_string()]);
    }

    /// A file another program creates while Save files checks the folder
    /// is replaced only after asking, as one that was there already.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn a_file_that_appears_while_saving_files_is_replaced_only_after_asking() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let names: Vec<Vec<u8>> = vec![b"one.txt\0".to_vec(), b"two.txt\0".to_vec()];
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFiles",
            vec![("files", names.to_variant())],
        );
        super::probe::appear_when_tested(&fixture.path("two.txt"));
        let window = &portal.test.window;

        window.picker().expect("a picker").accept.emit_clicked();

        let question = replace_question(&portal.test);
        assert_eq!(
            question.message().as_str(),
            "“two.txt” already exists. Do you want to replace it?"
        );
        assert!(
            answer.borrow().is_none(),
            "nothing is answered before the user says so"
        );
        press_in(&question, "Cancel");
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// A share that stops answering while Save files checks it is
    /// reported, never answered as a folder the files can be saved in.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn saving_files_where_the_share_stops_answering_says_so() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let names: Vec<Vec<u8>> = vec![b"one.txt\0".to_vec()];
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFiles",
            vec![("files", names.to_variant())],
        );
        super::probe::stop_answering_after_test(&fixture.path("one.txt"));
        let window = &portal.test.window;

        window.picker().expect("a picker").accept.emit_clicked();
        wait_until("the dialog to say so", || {
            window.shown_message().contains("is not answering") || answer.borrow().is_some()
        });

        assert!(answer.borrow().is_none(), "nothing is answered");
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// A file another program creates while Save checks the folder is
    /// replaced only after asking, as any file that was there already.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn a_file_that_appears_while_saving_is_replaced_only_after_asking() {
        let fixture = Fixture::empty();
        let target = fixture.path("report.txt");
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
        super::probe::appear_when_tested(&target);
        let window = &portal.test.window;
        window.picker().expect("a picker").accept.emit_clicked();

        let question = replace_question(&portal.test);
        assert!(
            answer.borrow().is_none(),
            "nothing is answered before the user says so"
        );
        assert_eq!(
            fs::read_to_string(&target).expect("the other program's file"),
            "written by another program"
        );
        press_in(&question, "Cancel");
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// Save files checks every file it would replace before asking, so a
    /// read-only one among them is reported, not found out by the caller
    /// halfway through saving.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn saving_files_checks_every_file_it_would_replace() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::empty();
        let second = fixture.path("two.txt");
        fs::write(&second, "signed").expect("a file");
        fs::set_permissions(&second, fs::Permissions::from_mode(0o444)).expect("read-only");
        let portal = Portal::new();
        let names: Vec<Vec<u8>> = vec![b"one.txt\0".to_vec(), b"two.txt\0".to_vec()];
        let answer = portal.call(
            "SaveFiles",
            &[
                (
                    "current_folder",
                    path_variant(&fixture.root().display().to_string()),
                ),
                ("files", names.to_variant()),
            ],
        );
        let window = &portal.test.window;
        window.picker().expect("a picker").accept.emit_clicked();
        wait_until("the dialog to say so", || {
            window.shown_message().contains("“two.txt” is set to read-only")
        });
        assert!(answer.borrow().is_none(), "nothing is answered");
        assert!(window.dialog_layer().shown().is_none(), "no replace question");
        fs::set_permissions(&second, fs::Permissions::from_mode(0o644)).expect("writable again");
        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
    }

    /// A remembered share whose local path `GVfs` is slow to give is
    /// passed over within the start timeout, not after `GVfs`'s own wait.
    ///
    /// parity: INT-035
    #[gtk::test]
    fn a_slow_share_lookup_is_bounded_by_the_start_timeout() {
        let uri = format!("{}projects/Reports", super::local_paths::NEVER_ANSWERS);
        let started = std::time::Instant::now();

        let found = glib::MainContext::default().block_on(super::start_location(
            ox_core::integration::StartFolder::Location(uri),
        ));

        assert_eq!(found, None);
        let waited = started.elapsed();
        assert!(
            waited < super::START_TIMEOUT + std::time::Duration::from_millis(150),
            "waited {waited:?}, more than the start timeout {:?}",
            super::START_TIMEOUT
        );
    }

    /// A remembered folder, kept without its user, is found on the share
    /// `GVfs` mounted with that user, by the mount's own address; on no
    /// other share, nor on a mount that only starts the same way. No tab
    /// takes that address, so the dialog opens the folder's local path.
    ///
    /// parity: INT-035, SAFE-010
    #[gtk::test]
    fn a_remembered_folder_is_found_on_a_share_mounted_with_a_user() {
        let roots = [
            "sftp://build.example/".to_owned(),
            "sftp://anna@build.example.org/".to_owned(),
            "sftp://anna@build.example/".to_owned(),
            "smb://sam@studio-nas/projects/".to_owned(),
        ];

        assert_eq!(
            super::address_on_mount("sftp://build.example/srv/My%20Out", &roots[1..]).as_deref(),
            Some("sftp://anna@build.example/srv/My%20Out")
        );
        assert_eq!(
            super::address_on_mount("smb://studio-nas/projects", &roots).as_deref(),
            Some("smb://sam@studio-nas/projects")
        );
        assert_eq!(
            super::address_on_mount("smb://studio-nas/projects-old/Film", &roots),
            None,
            "another share that starts the same way"
        );
        assert_eq!(
            super::address_on_mount("sftp://build.example/srv", &roots[..1]),
            None,
            "a mount without a user has the address's own path"
        );
        assert_eq!(
            super::address_on_mount("sftp://elsewhere.example/srv", &roots),
            None
        );
        let two_accounts = [
            "sftp://anna@build.example/".to_owned(),
            "sftp://ben@build.example/".to_owned(),
        ];
        assert_eq!(
            super::address_on_mount("sftp://build.example/srv", &two_accounts),
            None,
            "two accounts on one server: which one was used is unknown"
        );

        // GIO names a path on a share mounted with a user by an address
        // that holds the user: the tab opens the path itself instead.
        let fuse = std::path::Path::new(
            "/run/user/1000/gvfs/smb-share:server=studio-nas,share=projects,user=sam/Film",
        );
        assert_eq!(
            super::openable_uri("smb://sam@studio-nas/projects/Film", fuse),
            "file:///run/user/1000/gvfs/smb-share:server=studio-nas,share=projects,user=sam/Film"
        );
        assert_eq!(
            super::openable_uri("smb://studio-nas/projects/Film", fuse),
            "smb://studio-nas/projects/Film",
            "a share mounted without a user keeps its own address"
        );

        let fixture = Fixture::empty();
        let test = TestWindow::without_tabs();
        assert!(
            test.window.add_tab("smb://sam@studio-nas/projects").is_err(),
            "no tab takes an address with a user"
        );
        let (opened, path) = super::opened_on_mount(fixture.root().to_path_buf());
        assert_eq!(path, fixture.root());
        test.window.add_tab(&opened).expect("the folder opens");
        assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    }

    /// Saving where the caller cannot create the file, as on a share the
    /// server shares read-only, says so in Windows' words instead of
    /// answering a file the caller will fail to write.
    ///
    /// parity: INT-032, INT-035
    #[gtk::test]
    fn saving_where_the_file_cannot_be_written_says_so() {
        use std::os::unix::fs::PermissionsExt;
        // root may write anything, so the refusal holds for users only.
        let is_root = fs::metadata("/proc/self").is_ok_and(|metadata| {
            use std::os::unix::fs::MetadataExt;
            metadata.uid() == 0
        });
        if is_root {
            return;
        }
        let fixture = Fixture::empty();
        let archive = fixture.path("Archive");
        fs::create_dir(&archive).expect("a folder");
        fs::set_permissions(&archive, fs::Permissions::from_mode(0o555)).expect("read-only");
        let portal = Portal::new();
        let answer = portal.call(
            "SaveFile",
            &[
                ("current_folder", path_variant(&archive.display().to_string())),
                ("current_name", "report.txt".to_variant()),
            ],
        );
        let window = &portal.test.window;
        let picker = window.picker().expect("a picker");
        picker.accept.emit_clicked();
        wait_until("the dialog to say so", || {
            window
                .shown_message()
                .contains("You don't have permission to save in this location")
        });
        assert!(answer.borrow().is_none(), "nothing is answered");
        assert!(!archive.join("report.txt").exists(), "nothing is left behind");
        fs::set_permissions(&archive, fs::Permissions::from_mode(0o755)).expect("writable again");
        picker.accept.emit_clicked();
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(
            uris,
            [gio::File::for_path(archive.join("report.txt")).uri().to_string()]
        );
        assert!(!archive.join("report.txt").exists(), "the test file was removed");
    }

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
            FolderView::Compact | FolderView::Icons(_) | FolderView::Tiles | FolderView::Content => {
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

    /// Multiple directory selection grants exactly the selected folders;
    /// selecting nothing still chooses the currently displayed folder.
    #[gtk::test]
    fn a_multiple_folder_dialog_never_substitutes_the_parent_for_selected_children() {
        let fixture = Fixture::empty();
        for name in ["First", "Second", "Unselected"] {
            std::fs::create_dir(fixture.path(name)).unwrap();
        }
        for choose_children in [true, false] {
            let portal = Portal::new();
            let answer = portal.call(
                "OpenFile",
                &[
                    ("directory", true.to_variant()),
                    ("multiple", true.to_variant()),
                    (
                        "current_folder",
                        path_variant(&fixture.root().display().to_string()),
                    ),
                ],
            );
            let window = &portal.test.window;
            if choose_children {
                for name in ["First", "Second"] {
                    window
                        .folder_model()
                        .selection()
                        .select_item(portal.test.position_of(name), false);
                }
                settle();
                assert_eq!(window.selected_entries().len(), 2);
            }
            window.picker().unwrap().accept.emit_clicked();
            let (response, mut uris) = Portal::finish(&answer);
            assert_eq!(response, RESPONSE_SUCCESS);
            uris.sort();
            let expected = if choose_children {
                vec![fixture.uri_of("First"), fixture.uri_of("Second")]
            } else {
                vec![gio::File::for_path(fixture.root()).uri().to_string()]
            };
            assert_eq!(uris, expected);
        }
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

    /// The Save dialog's File name box.
    fn name_box(window: &crate::window::BrowserWindow) -> gtk::Entry {
        window
            .picker()
            .expect("a picker")
            .name
            .clone()
            .expect("a name box")
    }

    /// Clicks the item called `name` in the dialog's list, as GTK's click
    /// on it ends, with Ctrl (`toggle`) and Shift (`range`), and gives the
    /// list the keyboard as the click does.
    fn click_item(test: &TestWindow, name: &str, toggle: bool, range: bool) {
        let position = test.position_of(name);
        let pane = test.window.folder_pane();
        pane.focus_view();
        // The list binds its rows on a layout frame, which settle() does
        // not wait for.
        wait_until("the item on screen", || {
            pane.owners().widget_at(position).is_some()
        });
        let cell = pane.owners().widget_at(position).expect("the item is on screen");
        cell.activate_action("list.select-item", Some(&(position, toggle, range).to_variant()))
            .expect("an item can be selected");
        settle();
    }

    /// Creates `name` in the fixture as another application would, and
    /// waits until the dialog's list shows it and has settled.
    fn create_elsewhere(test: &TestWindow, fixture: &Fixture, name: &str) {
        fixture.write(name);
        wait_until("the new file in the list", || {
            test.names().iter().any(|listed| listed == name) && test.window.is_listed()
        });
        // Longer than the watch's 350 ms quiet time, so a second refresh
        // the change may cause has run too.
        wait_a_while(std::time::Duration::from_millis(800));
    }

    /// How many pixels of `entry`, as it paints itself, are the blue of
    /// selected text.
    fn selection_blue(entry: &gtk::Entry) -> usize {
        crate::test_support::harness::wait_for_frames(&entry.root().expect("on a window"), 3);
        let paintable = gtk::WidgetPaintable::new(Some(entry));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(entry.width()), f64::from(entry.height()));
        let Some(node) = snapshot.to_node() else {
            return 0;
        };
        let renderer = entry
            .native()
            .and_then(|native| native.renderer())
            .expect("a drawn window");
        let texture = renderer.render_texture(&node, None::<&gtk::graphene::Rect>);
        let mut downloader = gdk::TextureDownloader::new(&texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, _) = downloader.download_bytes();
        // C31's #0078d4.
        bytes
            .chunks_exact(4)
            .filter(|pixel| {
                pixel[3] > 200
                    && pixel[0] < 30
                    && (100..=140).contains(&pixel[1])
                    && (190..=230).contains(&pixel[2])
            })
            .count()
    }

    /// As in Windows' Save dialog, the selected name is drawn selected
    /// only while File name has the keyboard: with the keyboard in the
    /// file list, the box shows no blue selection a key would not
    /// replace, and the selection comes back with the keyboard.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn the_name_is_drawn_selected_only_while_the_name_box_has_the_keyboard() {
        let fixture = Fixture::empty();
        fixture.write("alpha.txt");
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "scan of the week.pdf".to_variant())],
        );
        let window = &portal.test.window;
        wait_until("the folder's listing", || window.is_listed());
        settle();
        let name = name_box(window);
        assert!(window.focus_is_in_picker_name());
        let focused = selection_blue(&name);
        assert!(
            focused > 100,
            "the selected name is drawn selected ({focused} blue pixels)"
        );

        window.folder_pane().focus_view();
        settle();
        assert!(!window.focus_is_in_picker_name());
        assert_eq!(
            selection_blue(&name),
            0,
            "no selection is drawn without the keyboard"
        );

        name.grab_focus();
        settle();
        assert!(
            selection_blue(&name) > 100,
            "the selection shows again with the keyboard"
        );
        // The blue is the selection's, not the focused box's underline.
        name.select_region(0, 0);
        settle();
        let underline = selection_blue(&name);
        assert!(
            focused > underline + 100,
            "the selection draws more than the underline ({focused} against {underline})"
        );
    }

    /// Resizing the dialog across every layout width, as a window manager
    /// does when it places the dialog, leaves the keyboard in File name
    /// and the name selected, with files appearing meanwhile.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn resizing_the_save_dialog_leaves_the_keyboard_in_the_name_box() {
        let fixture = Fixture::empty();
        for name in ["alpha.txt", "beta.txt"] {
            fixture.write(name);
        }
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "Scan.pdf".to_variant())],
        );
        let test = &portal.test;
        let window = &test.window;
        wait_until("the folder's listing", || window.is_listed());
        settle();
        let name = name_box(window);
        let mut bands = Vec::new();
        for (step, width) in [1300, 1100, 1000, 900, 640, 1300, 980].into_iter().enumerate() {
            window.set_default_size(width, 640);
            wait_a_while(std::time::Duration::from_millis(200));
            let band = window.window_width();
            if !bands.contains(&band) {
                bands.push(band);
            }
            if step == 3 {
                create_elsewhere(test, &fixture, "gamma.txt");
            }
            assert!(
                window.focus_is_in_picker_name(),
                "the keyboard stays in File name at {width} pixels, not on {:?}",
                GtkWindowExt::focus(window).map(|focus| focus.type_())
            );
            assert_eq!(name.selection_bounds(), Some((0, 4)), "at {width} pixels");
        }
        assert_eq!(bands.len(), 5, "the dialog went through every width: {bands:?}");
    }

    /// A file another application creates in the folder lists it, as
    /// Windows' dialog does, without taking the keyboard out of File name
    /// or changing what was typed and selected there, and without
    /// selecting anything in the list.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_file_created_elsewhere_leaves_the_keyboard_in_the_name_box() {
        let fixture = Fixture::empty();
        for name in ["alpha.txt", "beta.txt", "report.md"] {
            fixture.write(name);
        }
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "Scan.pdf".to_variant())],
        );
        let test = &portal.test;
        let window = &test.window;
        wait_until("the folder's listing", || window.is_listed());
        settle();
        let name = name_box(window);
        name.set_text("receipt 4.pdf");
        name.select_region(0, 9);

        for created in ["0 first.pdf", "scan 2.pdf", "zz last.pdf"] {
            create_elsewhere(test, &fixture, created);
            assert!(
                window.focus_is_in_picker_name(),
                "the keyboard stays in File name after {created} appears, not on {:?}",
                GtkWindowExt::focus(window).map(|focus| focus.type_())
            );
            assert_eq!(name.text(), "receipt 4.pdf", "what was typed stays");
            assert_eq!(name.selection_bounds(), Some((0, 9)), "and so does its selection");
            assert!(
                test.selected_names().is_empty(),
                "nothing is selected in the list"
            );
        }
        fs::remove_file(fixture.path("beta.txt")).expect("remove a file");
        wait_until("the removed file to leave the list", || {
            !test.names().iter().any(|listed| listed == "beta.txt")
        });
        wait_a_while(std::time::Duration::from_millis(800));
        assert!(window.focus_is_in_picker_name(), "also after a file is removed");
        assert_eq!(name.selection_bounds(), Some((0, 9)));
    }

    /// A file clicked in the list stays the one selected, and its name the
    /// one in File name, while other applications create, rename and
    /// remove files around it; the keyboard stays in the list.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_file_clicked_in_a_save_dialog_stays_selected_while_files_change() {
        let fixture = Fixture::empty();
        for name in ["alpha.txt", "beta.txt", "report.md", "zeta.txt"] {
            fixture.write(name);
        }
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "Scan.pdf".to_variant())],
        );
        let test = &portal.test;
        let window = &test.window;
        wait_until("the folder's listing", || window.is_listed());
        settle();
        click_item(test, "report.md", false, false);
        let name = name_box(window);
        assert_eq!(
            name.text(),
            "report.md",
            "the clicked file's name is in File name"
        );
        assert_eq!(test.selected_names(), ["report.md"]);

        create_elsewhere(test, &fixture, "0 first.pdf");
        fs::rename(fixture.path("zeta.txt"), fixture.path("omega.txt")).expect("rename a file");
        wait_until("the renamed file", || {
            test.names().iter().any(|listed| listed == "omega.txt")
        });
        fs::remove_file(fixture.path("alpha.txt")).expect("remove a file");
        wait_until("the removed file to leave the list", || {
            !test.names().iter().any(|listed| listed == "alpha.txt")
        });
        wait_a_while(std::time::Duration::from_millis(800));
        assert_eq!(
            test.selected_names(),
            ["report.md"],
            "the clicked file stays selected"
        );
        assert_eq!(name.text(), "report.md");
        assert!(
            window.folder_pane().view_has_focus(),
            "the keyboard stays in the list, not on {:?}",
            GtkWindowExt::focus(window).map(|focus| focus.type_())
        );
    }

    /// After files appear above the file kept selected, Shift+click in a
    /// dialog for one file still keeps the item clicked: the item kept is
    /// followed, not the place in the list it had.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn shift_click_keeps_the_item_clicked_after_files_appear() {
        let fixture = Fixture::empty();
        for name in ["b.txt", "c.txt", "d.txt", "e.txt"] {
            fixture.write(name);
        }
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let test = &portal.test;
        wait_until("the folder's listing", || test.window.is_listed());
        settle();
        click_item(test, "e.txt", false, false);
        create_elsewhere(test, &fixture, "a.txt");
        create_elsewhere(test, &fixture, "a2.txt");
        assert_eq!(test.selected_names(), ["e.txt"]);
        click_item(test, "b.txt", false, true);
        assert_eq!(
            test.selected_names(),
            ["b.txt"],
            "the item clicked, above the one kept"
        );
    }

    /// Clicks at (`x`, `y`) in `widget` with the private display's real
    /// pointer, as a hand clicks.
    fn click_at(pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>, (x, y): (f64, f64)) {
        let point = screen_point(pointer, widget, (x, y), (720, 500));
        pointer.move_to(point.0, point.1);
        pointer.press(Button::Primary);
        pointer.release(Button::Primary);
        wait_a_while(std::time::Duration::from_millis(300));
    }

    /// Types `text`, made of letters and spaces, on the private display's
    /// real keyboard.
    fn type_keys(pointer: &mut Pointer, text: &str) {
        for character in text.chars() {
            let key = if character == ' ' {
                gdk::Key::space
            } else {
                gdk::Key::from_name(character.to_string()).expect("a letter key")
            };
            pointer.tap_key(key);
        }
        wait_a_while(std::time::Duration::from_millis(300));
    }

    /// The user's case, with the real keyboard and pointer: typing as the
    /// dialog opens replaces the name without its extension, also after
    /// other applications created files in the folder; a file clicked in
    /// the list puts its name in File name; a click on the list's empty
    /// space leaves the name in the box without drawing it selected, and
    /// typing then goes to the list (type-to-select), never into a box
    /// that looked as if it would take it.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn typing_in_a_save_dialog_replaces_the_name_while_files_appear() {
        let mut pointer = Pointer::connect();
        let fixture = Fixture::empty();
        for name in ["invoice.pdf", "notes.txt", "scan 01.pdf"] {
            fixture.write(name);
        }
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "Scan.pdf".to_variant())],
        );
        let test = &portal.test;
        let window = &test.window;
        wait_until("the folder's listing", || window.is_listed());
        settle();
        let name = name_box(window);
        // The pointer rests over the folder, as it would after a click in
        // the calling application, and does not click.
        let over = screen_point(
            &mut pointer,
            &window.folder_pane().input_widget(),
            (40.0, 40.0),
            (720, 500),
        );
        pointer.move_to(over.0, over.1);
        create_elsewhere(test, &fixture, "scan 02.pdf");
        create_elsewhere(test, &fixture, "0 cover.pdf");
        type_keys(&mut pointer, "receipt");
        assert_eq!(name.text(), "receipt.pdf", "typing replaced the selected name");
        assert!(test.selected_names().is_empty());

        let row = window
            .folder_pane()
            .owners()
            .widget_at(test.position_of("notes.txt"))
            .expect("the row is on screen");
        click_at(&mut pointer, &row, (8.0, f64::from(row.height()) / 2.0));
        assert_eq!(test.selected_names(), ["notes.txt"]);
        assert_eq!(
            name.text(),
            "notes.txt",
            "a clicked file's name goes into File name"
        );
        create_elsewhere(test, &fixture, "scan 03.pdf");
        assert_eq!(
            test.selected_names(),
            ["notes.txt"],
            "the clicked file stays selected"
        );
        assert_eq!(name.text(), "notes.txt");

        name.set_text("receipt.pdf");
        name.grab_focus();
        super::select_stem(&name);
        let list = window.folder_pane().input_widget();
        click_at(&mut pointer, &list, (40.0, f64::from(list.height()) - 20.0));
        assert!(!window.focus_is_in_picker_name(), "the keyboard went to the list");
        assert_eq!(
            selection_blue(&name),
            0,
            "the box draws no selection it would not replace"
        );
        type_keys(&mut pointer, "inv");
        assert_eq!(name.text(), "invoice.pdf", "typing selected a file in the list");
        assert_eq!(test.selected_names(), ["invoice.pdf"]);
        // Back to the screen's middle, where the display starts it, so the
        // next tests' windows and menus do not open under a pointer this
        // test left there: GTK 4.14 crashes in gdk_surface_request_motion
        // when address_bar's « menu closes over it.
        pointer.move_to(720, 500);
    }

    /// The Save dialog as it opens, with the keyboard in the file list
    /// (as after a click on its empty space, which
    /// [`typing_in_a_save_dialog_replaces_the_name_while_files_appear`]
    /// makes with the real pointer) and after another application created
    /// a file in the folder, light and dark, for review with
    /// `$OX_NATIVE_CAPTURE_DIR` (and `GDK_SCALE=2`, where the test
    /// pointer's screen positions would not hold).
    #[gtk::test]
    fn the_save_dialog_name_box_is_captured_light_and_dark() {
        let _theme = crate::test_support::harness::ThemeGuard::keep();
        for theme in ["light", "dark"] {
            let fixture = Fixture::empty();
            for name in [
                "Invoice 1031.pdf",
                "Meeting notes.txt",
                "Scan 01.pdf",
                "Scan 02.pdf",
            ] {
                fixture.write(name);
            }
            let portal = Portal::new();
            let _answer = dialog_on(
                &portal,
                &fixture,
                "SaveFile",
                vec![("current_name", "Scan 03.pdf".to_variant())],
            );
            let test = &portal.test;
            let window = &test.window;
            test.activate("theme", Some(theme));
            wait_until("the folder's listing", || window.is_listed());
            settle();
            let name = name_box(window);
            assert!(window.focus_is_in_picker_name());
            capture(window, &format!("picker-save-name-opened-{theme}.png"));

            window.folder_pane().focus_view();
            settle();
            assert!(!window.focus_is_in_picker_name());
            assert_eq!(name.text(), "Scan 03.pdf");
            capture(window, &format!("picker-save-name-list-clicked-{theme}.png"));

            name.grab_focus();
            super::select_stem(&name);
            create_elsewhere(test, &fixture, "Scan 02 (back).pdf");
            assert!(
                window.focus_is_in_picker_name(),
                "the keyboard is still in File name"
            );
            assert_eq!(name.selection_bounds(), Some((0, 7)));
            capture(window, &format!("picker-save-name-file-appeared-{theme}.png"));
        }
    }

    /// Two types, as an image viewer's Open and Save dialogs offer them.
    fn image_types() -> glib::Variant {
        let types = [
            (
                "Images (*.png *.jpg *.jpeg *.webp *.gif *.bmp *.tiff)".to_owned(),
                vec![(0_u32, "*.png".to_owned()), (0_u32, "*.jpg".to_owned())],
            )
                .to_variant(),
            ("All files".to_owned(), vec![(0_u32, "*".to_owned())]).to_variant(),
        ];
        glib::Variant::array_from_iter_with_type(types[0].type_(), types.clone())
    }

    /// The captions of the dialog's bar.
    fn bar_captions(window: &crate::window::BrowserWindow) -> Vec<String> {
        crate::test_support::harness::descendants::<gtk::Label>(&*window.imp().picker_bar)
            .iter()
            .filter(|label| label.has_css_class("picker-label"))
            .map(|label| label.label().to_string())
            .collect()
    }

    /// Where `widget` is in `window`: its left, top, right and bottom.
    fn bounds_in(
        widget: &impl IsA<gtk::Widget>,
        window: &crate::window::BrowserWindow,
    ) -> (f32, f32, f32, f32) {
        let bounds = widget.compute_bounds(window).expect("laid out in the window");
        (
            bounds.x(),
            bounds.y(),
            bounds.x() + bounds.width(),
            bounds.y() + bounds.height(),
        )
    }

    /// Windows' Open dialog (Properties > Customize > Choose File..., an
    /// image viewer's Open): the type list sits right of File name without
    /// a caption, "Save as type:" nowhere, and Open and Cancel on a row
    /// below them at the right. A long type name keeps the list to the
    /// width of Windows' type box.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn an_open_dialog_has_its_type_list_beside_file_name_and_no_caption() {
        let fixture = Fixture::empty();
        fixture.write("photo.png");
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", vec![("filters", image_types())]);
        let window = &portal.test.window;
        wait_until("the folder's listing", || window.is_listed());
        crate::test_support::harness::wait_for_frames(window, 3);
        let picker = window.picker().expect("a picker");
        let name = picker.name.clone().expect("a name box");
        let types = picker.types.clone().expect("a type list");
        assert_eq!(
            bar_captions(window),
            ["File name:"],
            "the type list has no caption"
        );

        let (name_left, name_top, name_right, name_bottom) = bounds_in(&name, window);
        let (types_left, types_top, types_right, types_bottom) = bounds_in(&types, window);
        let (accept_left, accept_top, _, _) = bounds_in(&picker.accept, window);
        assert!(types_left > name_right, "the type list is right of File name");
        let middle = |top: f32, bottom: f32| f32::midpoint(top, bottom);
        assert!(
            (middle(types_top, types_bottom) - middle(name_top, name_bottom)).abs() < 3.0,
            "on File name's row"
        );
        assert!(
            accept_top >= name_bottom.max(types_bottom),
            "Open is on a row below"
        );
        assert!(
            accept_left > name_left + (name_right - name_left) / 2.0,
            "at the right"
        );
        assert!(
            types_right - types_left < (name_right - name_left),
            "a long type name does not widen the list past File name"
        );
    }

    /// A Save dialog keeps "Save as type:" on its own row under File name,
    /// with Save and Cancel on a row below them at the right, as Windows'
    /// Save dialog does.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn a_save_dialog_keeps_save_as_type_under_file_name() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![
                ("current_name", "photo.png".to_variant()),
                ("filters", image_types()),
            ],
        );
        let window = &portal.test.window;
        wait_until("the folder's listing", || window.is_listed());
        crate::test_support::harness::wait_for_frames(window, 3);
        let picker = window.picker().expect("a picker");
        let name = picker.name.clone().expect("a name box");
        let types = picker.types.clone().expect("a type list");
        assert_eq!(bar_captions(window), ["File name:", "Save as type:"]);
        let (name_left, _, name_right, name_bottom) = bounds_in(&name, window);
        let (types_left, types_top, types_right, types_bottom) = bounds_in(&types, window);
        let (accept_left, accept_top, _, _) = bounds_in(&picker.accept, window);
        assert!(types_top >= name_bottom, "Save as type is under File name");
        assert!((types_left - name_left).abs() < 1.0 && (types_right - name_right).abs() < 1.0);
        assert!(accept_top >= types_bottom, "Save is on a row below the fields");
        assert!(
            accept_left > name_left + (name_right - name_left) / 2.0,
            "at the right"
        );
    }

    /// Open and Save dialogs with a type list, light and dark, for review
    /// with `$OX_NATIVE_CAPTURE_DIR` (and `GDK_SCALE=2`).
    #[gtk::test]
    fn open_and_save_type_lists_are_captured_light_and_dark() {
        let _theme = crate::test_support::harness::ThemeGuard::keep();
        for theme in ["light", "dark"] {
            for (method, extra) in [
                ("OpenFile", None),
                ("SaveFile", Some(("current_name", "Scan 03.png".to_variant()))),
            ] {
                let fixture = Fixture::empty();
                for name in ["Beach.jpg", "Garden.png", "Scan 01.png", "Scan 02.png"] {
                    fixture.write(name);
                }
                let portal = Portal::new();
                let mut entries = vec![("filters", image_types())];
                entries.extend(extra);
                let _answer = dialog_on(&portal, &fixture, method, entries);
                let test = &portal.test;
                test.activate("theme", Some(theme));
                wait_until("the folder's listing", || test.window.is_listed());
                settle();
                let mode = if method == "OpenFile" { "open" } else { "save" };
                capture(&test.window, &format!("picker-type-list-{mode}-{theme}.png"));
            }
        }
    }

    /// Chromium's Save page as: choosing "Webpage, Single File" turns
    /// "Title.html" into "Title.mhtml", so Chromium does not write MHTML
    /// into a .html file, and the name stays selected without its
    /// extension; All Files keeps the name, and so does a name typed
    /// without the type's extension. The saved file has the name shown.
    ///
    /// parity: INT-032
    #[gtk::test]
    fn choosing_another_type_in_a_save_dialog_changes_the_extension() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let types: Vec<glib::Variant> = [
            (
                "Webpage, HTML Only",
                &["*.[hH][tT][mM][lL]", "*.[hH][tT][mM]"][..],
            ),
            ("Webpage, Single File", &["*.[mM][hH][tT][mM][lL]"][..]),
            ("All Files", &["*.*"][..]),
        ]
        .iter()
        .map(|(name, globs)| {
            let patterns: Vec<(u32, String)> = globs.iter().map(|glob| (0, (*glob).to_owned())).collect();
            ((*name).to_owned(), patterns).to_variant()
        })
        .collect();
        let answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![
                ("current_name", "Title.html".to_variant()),
                (
                    "filters",
                    glib::Variant::array_from_iter_with_type(types[0].type_(), types.clone()),
                ),
            ],
        );
        let picker = portal.test.window.picker().expect("a picker");
        let name = picker.name.clone().expect("a name box");
        let list = picker.types.clone().expect("a type list");

        list.set_selected(1);
        assert_eq!(name.text(), "Title.mhtml");
        assert_eq!(
            name.selection_bounds(),
            Some((0, 5)),
            "the name without its extension"
        );
        list.set_selected(2);
        assert_eq!(name.text(), "Title.mhtml", "All Files keeps the name");
        list.set_selected(0);
        assert_eq!(name.text(), "Title.mhtml", "not the extension of All Files");
        name.set_text("Title.mhtml");
        list.set_selected(1);
        list.set_selected(0);
        assert_eq!(name.text(), "Title.html");
        name.set_text("Title");
        list.set_selected(1);
        assert_eq!(name.text(), "Title");

        enter(&name);
        let (response, uris) = Portal::finish(&answer);
        assert_eq!(response, RESPONSE_SUCCESS);
        assert_eq!(uris, [fixture.uri_of("Title.mhtml")]);
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

    /// A type of `globs` named `name`.
    fn file_type(name: &str, globs: &[&str]) -> FileFilter {
        FileFilter {
            name: name.to_owned(),
            patterns: globs
                .iter()
                .map(|glob| FilterPattern::Glob((*glob).to_owned()))
                .collect(),
        }
    }

    /// A Save dialog adds the type's extension to a name without one; a
    /// trailing dot is dropped instead, and a hidden file's name kept.
    /// As in Windows, a name with a dot gets it too unless it fits the type
    /// or ends in a known type's extension: Chromium's Print > Save as PDF
    /// leaves the name to the dialog, so "Notes v2.1" must not be saved
    /// without ".pdf". Before, only a name without a dot got an extension.
    ///
    /// parity: INT-032
    #[test]
    fn a_saved_name_gets_the_types_extension_as_in_windows() {
        let text = file_type("Text", &["*.txt"]);
        let saved = |written: &str| super::saved_name(written, Some(&text));
        assert_eq!(saved("report"), "report.txt");
        assert_eq!(saved("report."), "report");
        assert_eq!(saved("notes.md"), "notes.md");
        assert_eq!(saved("Notes.TXT"), "Notes.TXT");
        assert_eq!(
            saved("photo.png"),
            "photo.png",
            "GIO calls .png uncertain, yet knows it"
        );
        assert_eq!(saved(".bashrc"), ".bashrc");
        assert_eq!(saved("Mr. Smith"), "Mr. Smith.txt");
        assert_eq!(saved("draft.qqqzzz"), "draft.qqqzzz.txt");
        assert_eq!(super::saved_name("report", None), "report");
        let pdf = file_type("PDF", &["*.[pP][dD][fF]"]);
        let as_pdf = |written: &str| super::saved_name(written, Some(&pdf));
        assert_eq!(
            as_pdf("Notes v2.1"),
            "Notes v2.1.pdf",
            "a version, not a manual page"
        );
        assert_eq!(as_pdf("Q3.2025"), "Q3.2025.pdf");
        assert_eq!(as_pdf("Notes.PDF"), "Notes.PDF");
        let all_files = file_type("All Files", &["*.*"]);
        assert_eq!(super::saved_name("Notes v2.1", Some(&all_files)), "Notes v2.1");
        assert_eq!(super::saved_name("report", Some(&all_files)), "report");
    }

    /// Changing a Save dialog's type swaps the extension of the old type
    /// for the new one's, as GTK's and Windows' dialogs do, with Chromium's
    /// Save page as types; a name with no extension of the old type, one
    /// that fits the new type, and All Files leave the name alone.
    ///
    /// parity: INT-032
    #[test]
    fn a_new_type_swaps_the_extension_of_the_old_one() {
        let html = file_type("Webpage, HTML Only", &["*.[hH][tT][mM][lL]", "*.[hH][tT][mM]"]);
        let single = file_type("Webpage, Single File", &["*.[mM][hH][tT][mM][lL]"]);
        let complete = file_type("Webpage, Complete", &["*.[hH][tT][mM]", "*.[hH][tT][mM][lL]"]);
        let all_files = file_type("All Files", &["*.*"]);
        let renamed = super::renamed_for_type;
        assert_eq!(
            renamed("Title.html", &html, &single).as_deref(),
            Some("Title.mhtml")
        );
        assert_eq!(
            renamed("Title.HTM", &html, &single).as_deref(),
            Some("Title.mhtml")
        );
        assert_eq!(
            renamed("Title.mhtml", &single, &complete).as_deref(),
            Some("Title.htm")
        );
        assert_eq!(renamed("Title.html", &html, &complete), None, "it fits already");
        assert_eq!(renamed("Title.html", &html, &all_files), None);
        assert_eq!(renamed("Title", &html, &single), None);
        assert_eq!(renamed("notes.txt", &html, &single), None);
        assert_eq!(renamed(".html", &html, &single), None, "a hidden file's name");
        let archives = file_type("Archive", &["*.gz", "*.tar.gz"]);
        let zip = file_type("ZIP", &["*.zip"]);
        assert_eq!(
            renamed("backup.tar.gz", &archives, &zip).as_deref(),
            Some("backup.zip")
        );
        let upper = file_type("HTML", &["*.HTML"]);
        let lower = file_type("MHTML", &["*.mhtml"]);
        assert_eq!(
            renamed("Title.HTML", &upper, &lower).as_deref(),
            Some("Title.mhtml"),
            "a plain glob in capitals"
        );
        assert_eq!(
            renamed("Title.html", &upper, &lower).as_deref(),
            Some("Title.mhtml")
        );
        let pages = file_type("Seiten", &["*.[äÄ]"]);
        assert_eq!(
            renamed("Brief.Ä", &pages, &single).as_deref(),
            Some("Brief.mhtml")
        );
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

    /// F11 leaves a dialog at its size, as a Windows Open or Save dialog
    /// does, while Ctrl+E still moves to the search box.
    ///
    /// parity: INT-032, CMD-036
    #[gtk::test]
    fn a_dialog_does_not_go_full_screen() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let test = &portal.test;
        let window = &test.window;
        wait_until("the folder's listing", || window.is_listed());
        window.folder_pane().focus_view();

        let full_screen = window.lookup_action("full-screen").expect("the action");
        assert!(!full_screen.is_enabled(), "Full screen is off in a dialog");
        assert!(
            !press(test, gdk::Key::F11, gdk::ModifierType::empty()),
            "F11 does nothing"
        );
        assert!(!window.imp().wants_full_screen.get());
        assert!(press(test, gdk::Key::e, gdk::ModifierType::CONTROL_MASK));
        assert!(window.focus_is_in_text_field(), "Ctrl+E moves to the search box");
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

    /// A dialog does not stand in for the last browsing window while a
    /// drive is formatted: that window still waits, as the dialog closes on
    /// its own once answered, which would end the application mid-format.
    /// For the same reason a dialog formats no drive itself.
    ///
    /// parity: DEV-012, INT-032
    #[gtk::test]
    fn a_dialog_does_not_let_the_last_window_close_during_a_format() {
        let fixture = Fixture::empty();
        let portal = Portal::new();
        let answer = portal.call(
            "OpenFile",
            &[(
                "current_folder",
                path_variant(&fixture.root().display().to_string()),
            )],
        );
        let browsing = portal.test.open_beside(&fixture.uri());
        let dialog_formats = portal.test.window.is_action_enabled("format-drive");
        let formatting = portal.test.context.hold_writes_for_formatting();

        browsing.window.close();
        settle();
        let stayed = browsing.window.is_visible();
        let refusal = browsing.window.shown_message_text();
        portal.test.window.close();
        let (response, _) = Portal::finish(&answer);
        drop(formatting);
        browsing.window.close();
        settle();

        assert!(!dialog_formats, "a dialog offers no Format…");
        assert!(browsing.window.is_action_enabled("format-drive"));
        assert!(stayed, "the last browsing window stays open");
        assert_eq!(
            refusal,
            "Wait until the drive is formatted before closing OpenXplorer."
        );
        assert_eq!(response, RESPONSE_CANCELLED, "the dialog still closes");
        assert!(
            !browsing.window.is_visible(),
            "it closes once the drive is formatted"
        );
    }

    /// A pane split off in a dialog, where Split view is off, lists only
    /// the type chosen, as the first one does.
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

        window.split_tab(None).expect("the folder splits");
        let split = window.pane_on(PaneSide::End);
        wait_until("the split pane's listing", || {
            !window.is_loading() && split.model().n_items() == 1
        });
        settle();
        assert_eq!(split.model().n_items(), 1, "only notes.txt");
    }

    /// An Open or Save dialog shows folders as every window does after
    /// Folder Options' Apply to Folders and Reset Folders in another
    /// window.
    ///
    /// parity: INT-032, VIEW-020
    #[gtk::test]
    fn a_dialog_follows_apply_to_folders_and_reset_folders() {
        use crate::window::tests::view_styles::{
            ask_from_folder_options, close_folder_options, open_folder_options,
        };
        let fixture = Fixture::standard();
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, "OpenFile", Vec::new());
        let picker = &portal.test.window;
        let sort = || crate::window::BrowserWindow::style_of_pane(picker.folder_pane()).sort;
        assert_eq!(sort(), "name");
        let explorer = portal.test.open_beside(&fixture.uri());
        explorer.activate("sort", Some("size"));

        let options = open_folder_options(&explorer);
        ask_from_folder_options(&options, "Apply to Folders").press("Yes");
        wait_until("the dialog sorted by size", || sort() == "size");
        ask_from_folder_options(&options, "Reset Folders").press("Yes");
        wait_until("the dialog sorted by name again", || sort() == "name");
        close_folder_options(&explorer, &options, "OK");
        assert!(picker.is_picking(), "the dialog is still open");
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

    /// The groups' clock pinned to noon today, so that files dated a few
    /// minutes before it are today's whatever the real time: just after
    /// midnight, files dated minutes before the real time fell in
    /// Yesterday. The real clock is back once this is dropped.
    struct NoonClock(glib::DateTime);

    impl NoonClock {
        fn pin() -> Self {
            let now = glib::DateTime::now_local().expect("the clock is readable");
            let noon = glib::DateTime::from_local(now.year(), now.month(), now.day_of_month(), 12, 0, 0.0)
                .expect("noon today");
            crate::folder_view::groups::set_clock_for_tests(Some(noon.clone()));
            Self(noon)
        }

        /// Sets the modification time of `path` to `seconds` before noon.
        fn date(&self, path: &std::path::Path, seconds: u64) {
            let noon = u64::try_from(self.0.to_unix()).expect("noon is after 1970");
            let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(noon - seconds);
            fs::File::options()
                .write(true)
                .open(path)
                .and_then(|file| file.set_modified(time))
                .expect("the date is set");
        }
    }

    impl Drop for NoonClock {
        fn drop(&mut self) {
            crate::folder_view::groups::set_clock_for_tests(None);
        }
    }

    /// The first group heading on screen and how far the list is
    /// scrolled, once the window has drawn and laid the list out again.
    fn top_of_the_list(window: &crate::window::BrowserWindow) -> (Option<String>, f64) {
        crate::test_support::harness::wait_for_frames(window, 4);
        let column_view = window.folder_pane().details().column_view().clone();
        // GTK lays the list out again later for reasons of its own, such as
        // KWin sizing the window: the list must stay where it was.
        for list in crate::test_support::harness::descendants::<gtk::ListView>(&column_view) {
            list.queue_allocate();
        }
        crate::test_support::harness::wait_for_frames(window, 4);
        let mut headings: Vec<(f32, String)> =
            crate::test_support::harness::descendants::<gtk::Label>(&column_view)
                .into_iter()
                .filter(|label| label.has_css_class("group-title") && label.is_mapped())
                .filter_map(|label| {
                    let point = label.compute_point(&column_view, &gtk::graphene::Point::new(0.0, 0.0))?;
                    Some((point.y(), label.text().to_string()))
                })
                .collect();
        headings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let first = headings.into_iter().next().map(|(_, text)| text);
        (first, window.folder_pane().details().vadjustment().value())
    }

    /// A Save dialog on a folder grouped and sorted by date modified
    /// shows the list from its first heading, also after another file
    /// type shows more files: the files the type hid come before the one
    /// that was at the top, which GTK kept at the top edge, scrolling
    /// "Today" out of sight (Chrome's Save dialog in Downloads, `*.svg`
    /// then All files).
    ///
    /// parity: INT-032, VIEW-022
    #[gtk::test]
    fn a_grouped_dialog_list_stays_at_its_top_when_the_type_changes() {
        let fixture = Fixture::empty();
        let clock = NoonClock::pin();
        // Newest first: today's files, the drawing among them, then
        // older periods, a little more than the dialog shows.
        let files = [
            ("notes.txt", 60),
            ("photo.png", 120),
            ("drawing.svg", 180),
            ("setup.deb", 240),
            ("logo.svg", 2 * 86_400),
            ("report.pdf", 3 * 86_400),
            ("music.ogg", 40 * 86_400),
            ("map.svg", 40 * 86_400 + 60),
            ("letter.odt", 400 * 86_400),
            ("old.zip", 1_200 * 86_400),
        ];
        for (name, age) in files {
            fixture.write(name);
            clock.date(&fixture.path(name), age);
        }
        for number in 0..24 {
            let name = format!("archive {number:02}.tar");
            fixture.write(&name);
            clock.date(&fixture.path(&name), 1_300 * 86_400);
        }
        // Enough old drawings that the SVG files alone fill more than the
        // view, as in a real Downloads folder.
        for number in 0..14 {
            let name = format!("sketch {number:02}.svg");
            fixture.write(&name);
            clock.date(&fixture.path(&name), 1_300 * 86_400);
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
        test.activate("view", Some("details"));
        test.activate("sort", Some("modified"));
        test.activate("direction", Some("descending"));
        test.activate("group-by", Some("modified"));
        assert_eq!(test.names().first().map(String::as_str), Some("drawing.svg"));

        let (heading, scrolled) = top_of_the_list(window);
        assert!(scrolled < 0.5, "the SVG files from their top: {scrolled}");
        assert!(heading.is_some_and(|heading| heading.starts_with("Today")));

        let types = window
            .picker()
            .expect("a picker")
            .types
            .clone()
            .expect("a type list");
        types.set_selected(1);
        wait_until("every file", || test.names().len() == files.len() + 24 + 14);
        let (heading, scrolled) = top_of_the_list(window);
        assert!(scrolled < 0.5, "All files from their top, not {scrolled} down");
        assert_eq!(heading.as_deref(), Some("Today (4)"), "the first heading shows");

        // Down the list and back up to its top, by the scroll bar, then
        // the SVG files and All files again.
        let adjustment = window.folder_pane().details().vadjustment();
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
        top_of_the_list(window);
        adjustment.set_value(0.0);
        let (heading, scrolled) = top_of_the_list(window);
        assert!(scrolled < 0.5 && heading.as_deref() == Some("Today (4)"));
        for (shown, files, first) in [(0, 3 + 14, "Today (1)"), (1, files.len() + 24 + 14, "Today (4)")] {
            types.set_selected(shown);
            wait_until("the type's files", || test.names().len() == files);
            let (heading, scrolled) = top_of_the_list(window);
            assert!(scrolled < 0.5, "type {shown} from its top, not {scrolled} down");
            assert_eq!(
                heading.as_deref(),
                Some(first),
                "type {shown} shows its first heading"
            );
        }
    }

    /// The command bar's Sort menu in a Save dialog, as Chrome opens one
    /// through the portal: Group by and More open beside the menu, and the
    /// compositor only ever sees one popup holding a grab, the Sort menu,
    /// as for a menu without submenus. Each submenu is a popup of the Sort
    /// menu's surface without a grab of its own (`gdk_popup_get_autohide`,
    /// what GDK sends `xdg_popup.grab` for), one opens only once the other
    /// has left the screen, and a submenu leaves the screen before the
    /// menu. Side menus that each took a second, nested grab once froze
    /// the KDE Plasma Wayland session in this dialog.
    ///
    /// parity: VIEW-022, INT-032
    #[gtk::test]
    fn the_sort_menu_of_a_save_dialog_keeps_one_grabbing_popup() {
        sort_menu_keeps_one_grab("SaveFile", vec![("current_name", "page.html".to_variant())]);
    }

    /// The same in an Open dialog.
    ///
    /// parity: VIEW-022, INT-032
    #[gtk::test]
    fn the_sort_menu_of_an_open_dialog_keeps_one_grabbing_popup() {
        sort_menu_keeps_one_grab("OpenFile", Vec::new());
    }

    /// A row the pointer rests on in a Save dialog's Sort menu shows the
    /// hover's gray, as in a browsing window, even right after a key: the
    /// release of the Ctrl+S that opened the dialog, or a Tab to its name,
    /// turns the window's focus-visible on, and the row the pointer moves
    /// the keyboard to drew in the keyboard's blue.
    ///
    /// parity: VIEW-022, INT-032
    #[gtk::test]
    fn a_menu_row_the_pointer_rests_on_shows_the_hover_after_a_key() {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "page.html".to_variant())],
        );
        let window = portal.test.window.clone();
        let sort = crate::window::widget_tree::descendants::<gtk::MenuButton>(&window)
            .into_iter()
            .find(|button| button.has_css_class("sort-command"))
            .expect("the dialog's command bar has Sort");
        let menu = sort
            .popover()
            .and_downcast::<crate::window::menu_popover::MenuPopover>()
            .expect("an app menu");
        sort.popup();
        wait_until("the Sort menu", || menu.is_mapped());
        // As a key released after the focus moved leaves it.
        window.set_focus_visible(true);

        menu.hover_row(Some("Date modified"));

        let row = menu.row("Date modified");
        assert!(
            row.has_focus(),
            "the keyboard goes on from the row the pointer is on"
        );
        assert!(!window.gets_focus_visible());
        assert!(
            !row.state_flags().contains(gtk::StateFlags::FOCUS_VISIBLE),
            "the row draws as hovered, not as keyboard-focused"
        );
        menu.popdown();
    }

    /// The dividers of a menu that opens from the command bar keep the
    /// menu's own side margins in a narrow window, such as a dialog
    /// narrower than 1190 pixels, as in a wide one: the command bar's
    /// narrow-window rule for its own dividers once reached them too.
    ///
    /// parity: VIEW-022, INT-032
    #[gtk::test]
    fn menu_dividers_in_a_narrow_dialog_keep_the_menu_s_margins() {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        let portal = Portal::new();
        let _answer = dialog_on(
            &portal,
            &fixture,
            "SaveFile",
            vec![("current_name", "page.html".to_variant())],
        );
        let window = portal.test.window.clone();
        window.set_default_size(900, 600);
        wait_until("a narrow dialog", || window.has_css_class("max-1190"));
        let sort = crate::window::widget_tree::descendants::<gtk::MenuButton>(&window)
            .into_iter()
            .find(|button| button.has_css_class("sort-command"))
            .expect("the dialog's command bar has Sort");
        let menu = sort
            .popover()
            .and_downcast::<crate::window::menu_popover::MenuPopover>()
            .expect("an app menu");
        sort.popup();
        wait_until("the Sort menu", || menu.is_mapped());
        let list = menu.row_list();
        let dividers: Vec<gtk::Separator> = crate::window::widget_tree::descendants::<gtk::Separator>(&list);
        wait_until("the menu laid out", || {
            dividers.iter().all(|divider| divider.width() > 0)
        });
        assert!(!dividers.is_empty(), "the Sort menu has dividers");
        for divider in &dividers {
            assert_eq!(
                list.width() - divider.width(),
                10,
                "5 pixels at either side (menus.css), not the command bar's 6"
            );
        }
        // And 4 pixels above and below, not none: a divider asks for its
        // 1-pixel line and its margins. This reads what it asks for rather
        // than where the list placed it.
        for divider in &dividers {
            let (height, _, _, _) = divider.measure(gtk::Orientation::Vertical, -1);
            assert_eq!(height, 1 + 4 + 4, "4 pixels above and below a divider");
        }
        menu.popdown();
    }

    /// The popups of `window`'s menus on screen, each with whether it
    /// holds a pointer grab.
    fn shown_popups(window: &gtk::Widget) -> Vec<(crate::window::menu_popover::MenuPopover, bool)> {
        crate::window::widget_tree::descendants::<crate::window::menu_popover::MenuPopover>(window)
            .into_iter()
            .filter(WidgetExt::is_mapped)
            .map(|menu| {
                let popup = gtk::prelude::NativeExt::surface(&menu).and_downcast::<gdk::Popup>();
                let grabs = popup.is_some_and(|popup| popup.is_autohide());
                (menu, grabs)
            })
            .collect()
    }

    /// Opens a `method` dialog with `entries` and goes through its Sort
    /// menu's Group by and More, checking only the Sort menu grabs.
    fn sort_menu_keeps_one_grab(method: &str, entries: Vec<(&str, glib::Variant)>) {
        let fixture = Fixture::empty();
        fixture.write("notes.txt");
        let portal = Portal::new();
        let _answer = dialog_on(&portal, &fixture, method, entries);
        let test = &portal.test;
        let window: &gtk::Widget = test.window.upcast_ref();
        let sort = crate::window::widget_tree::descendants::<gtk::MenuButton>(window)
            .into_iter()
            .find(|button| button.has_css_class("sort-command"))
            .expect("the dialog's command bar has Sort");
        let menu = sort
            .popover()
            .and_downcast::<crate::window::menu_popover::MenuPopover>()
            .expect("an app menu");
        let grabbing = || -> Vec<crate::window::menu_popover::MenuPopover> {
            shown_popups(window)
                .into_iter()
                .filter_map(|(menu, grabs)| grabs.then_some(menu))
                .collect()
        };
        // Every submenu that comes on screen: whether the menu was on screen
        // as it went, and whether another submenu was.
        let unmapped: Rc<RefCell<Vec<&'static str>>> = Rc::default();

        sort.popup();
        wait_until("the Sort menu", || menu.is_mapped());
        assert_eq!(grabbing(), std::slice::from_ref(&menu), "the Sort menu grabs");
        let menu_surface = gtk::prelude::NativeExt::surface(&menu).expect("on screen");

        // The pointer rests on More, then on Group by.
        menu.hover_row(Some("More"));
        wait_until("More", || {
            menu.open_submenu_menu().is_some_and(|more| more.is_mapped())
        });
        let more = menu.open_submenu_menu().expect("More");
        let more_surface = gtk::prelude::NativeExt::surface(&more).expect("on screen");
        let order = Rc::clone(&unmapped);
        more_surface.connect_mapped_notify(move |surface| {
            if !surface.is_mapped() {
                order.borrow_mut().push("More");
            }
        });
        menu.hover_row(Some("Group by"));
        wait_until("Group by", || {
            menu.open_submenu_menu()
                .is_some_and(|group_by| group_by != more && group_by.is_mapped())
        });
        let group_by = menu.open_submenu_menu().expect("Group by");
        assert_eq!(
            *unmapped.borrow(),
            ["More"],
            "More left the screen before Group by came"
        );
        settle();
        let popup = gtk::prelude::NativeExt::surface(&group_by)
            .and_downcast::<gdk::Popup>()
            .expect("a popup");
        assert!(!popup.is_autohide(), "a submenu takes no grab of its own");
        assert_eq!(
            popup.parent().as_ref(),
            Some(&menu_surface),
            "a popup of the Sort menu's"
        );
        assert_eq!(
            grabbing(),
            std::slice::from_ref(&menu),
            "still only the Sort menu grabs"
        );
        assert_eq!(shown_popups(window).len(), 2, "the Sort menu and Group by");

        // Choosing Type groups the list and closes both, Group by first.
        let group_by_surface = gtk::prelude::NativeExt::surface(&group_by).expect("on screen");
        for (surface, name) in [(&group_by_surface, "Group by"), (&menu_surface, "Sort")] {
            let order = Rc::clone(&unmapped);
            surface.connect_mapped_notify(move |surface| {
                if !surface.is_mapped() {
                    order.borrow_mut().push(name);
                }
            });
        }
        group_by.row("Type").emit_activate();
        assert_eq!(test.action_state("group-by").as_deref(), Some("type"));
        wait_until("the menus to close", || !menu.is_visible());
        settle();
        assert_eq!(*unmapped.borrow(), ["More", "Group by", "Sort"]);
        assert!(shown_popups(window).is_empty(), "no menu is left on screen");

        // The keyboard: Right on More opens it with no further grab, and a
        // choice there sorts.
        sort.popup();
        wait_until("the Sort menu again", || menu.is_mapped());
        assert!(menu.open_submenu_menu().is_none(), "it opens without a submenu");
        menu.row("More").grab_focus();
        assert!(menu.press_in_list(gdk::Key::Right));
        let more = menu.open_submenu_menu().expect("Right opens More");
        wait_until("More", || more.is_mapped());
        assert_eq!(grabbing(), std::slice::from_ref(&menu));
        more.row("Size").emit_activate();
        assert_eq!(test.action_state("sort").as_deref(), Some("size"));
        wait_until("the menu to close", || !menu.is_visible());
    }

    /// An Open dialog for PNG pictures, which a website's upload button
    /// asks the portal for, leaves every bundled icon it showed loaded
    /// once it closes, however many other icons pass through the icon
    /// theme's cache of recent ones: so GTK's loading thread never drops
    /// the last reference to a cached icon while the next dialog looks it
    /// up, which `GLib` reports as `object_ref` on a finalized object and
    /// which crashed the service in `begin_picking`.
    #[gtk::test]
    fn a_closed_dialog_leaves_the_icons_it_showed_loaded() {
        let fixture = Fixture::empty();
        for index in 0..12 {
            fixture.write(&format!("picture-{index:02}.png"));
        }
        fixture.write("notes.txt");
        std::fs::create_dir(fixture.path("Drafts")).expect("a folder");
        let portal = Portal::new();
        let png = (
            "PNG image".to_owned(),
            vec![(0_u32, "*.png".to_owned()), (1, "image/png".to_owned())],
        )
            .to_variant();
        let answer = portal.call(
            "OpenFile",
            &[
                (
                    "current_folder",
                    path_variant(&fixture.root().display().to_string()),
                ),
                (
                    "filters",
                    glib::Variant::array_from_iter_with_type(png.type_(), [png.clone()]),
                ),
                ("current_filter", png.clone()),
            ],
        );
        let window = &portal.test.window;
        settle();
        let theme = gtk::IconTheme::for_display(&WidgetExt::display(window));
        // The icon each image shows, as the theme caches it for the image.
        let shown: Vec<(String, i32, i32, gtk::TextDirection)> =
            crate::test_support::harness::descendants::<gtk::Image>(window)
                .into_iter()
                .filter(|image| image.storage_type() == gtk::ImageType::IconName)
                .filter_map(|image| {
                    let name = image.icon_name()?.to_string();
                    let is_bundled = name.starts_with("ox-");
                    is_bundled.then(|| (name, image.pixel_size(), image.scale_factor(), image.direction()))
                })
                .collect();
        assert!(shown.len() > 10, "the dialog shows its icons: {shown:?}");
        let looked_up = |(name, size, scale, direction): &(String, i32, i32, gtk::TextDirection)| {
            theme.lookup_by_gicon(
                &gio::ThemedIcon::new(name),
                *size,
                *scale,
                *direction,
                gtk::IconLookupFlags::PRELOAD,
            )
        };
        let cached: Vec<glib::WeakRef<gtk::IconPaintable>> =
            shown.iter().map(|key| looked_up(key).downgrade()).collect();
        let dialog = window.downgrade();

        window.cancel_picking();
        assert_eq!(Portal::finish(&answer).0, RESPONSE_CANCELLED);
        drop(portal);
        wait_until("the dialog to go", || dialog.upgrade().is_none());
        // Far more distinct icons than the cache of recent ones holds, and
        // time for GTK's loading threads to finish and let go.
        for (name, ..) in &shown {
            for size in 60..72 {
                drop(theme.lookup_icon(
                    name,
                    &[],
                    size,
                    1,
                    gtk::TextDirection::Ltr,
                    gtk::IconLookupFlags::empty(),
                ));
            }
        }
        for _ in 0..20 {
            crate::test_support::harness::wait_for(std::time::Duration::from_millis(10));
        }

        for (key, cached) in shown.iter().zip(&cached) {
            let kept = cached.upgrade();
            assert!(kept.is_some(), "{key:?} is still loaded");
            assert_eq!(Some(looked_up(key)), kept, "the theme hands out the kept {key:?}");
        }
    }
}
