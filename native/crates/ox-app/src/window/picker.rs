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
//! file dialog has: the file name for Save, the type list, the caller's
//! extra choices, and the accept and Cancel buttons. It narrows the
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

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::integration::{
    checked_name, ChooserAnswer, ChooserCall, ChooserMode, ChooserReply, ChooserRequest,
};

use super::{BrowserWindow, ButtonStyle, WindowAction};
use crate::dialog::Dialog;
use crate::folder_view::filter::ChooserListing;
use crate::locations::Page;

/// The size a picker window opens at; it never saves its size over the
/// explorer's.
const DEFAULT_SIZE: (i32, i32) = (980, 640);

/// The window commands that make no sense while choosing a file: more tabs
/// or windows, and Settings.
const DISABLED_ACTIONS: [WindowAction; 11] = [
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
];

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
    /// The name box of a Save dialog.
    name: Option<gtk::Entry>,
    /// The type list, when the caller gave filters.
    types: Option<gtk::DropDown>,
    choices: Vec<ChoiceControl>,
    accept: gtk::Button,
    /// Set while the replace question is open, so the answer is not given
    /// twice.
    asking: Cell<bool>,
}

impl Picker {
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
        self.present();
        match &picker.name {
            Some(name) => {
                name.grab_focus();
                select_stem(name);
            }
            None => self.focus_new_file_list(),
        }
        self.update_picker();
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

    /// Follows a new location or selection: the accept button, and in a
    /// Save dialog the name of a selected file.
    pub(super) fn picker_selection_changed(&self) {
        let Some(picker) = self.picker() else {
            return;
        };
        if let (Some(name), ChooserMode::Save { .. }) = (&picker.name, &picker.request.mode) {
            if let [entry] = self.selected_entries().as_slice() {
                if !entry.is_dir && !self.imp().changing_model.get() {
                    name.set_text(&entry.name);
                }
            }
        }
        self.update_picker();
    }

    /// A file was activated (double-click or Enter): it is the choice in
    /// an Open dialog, and the name to save over in a Save dialog.
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
                if let Some(name) = &picker.name {
                    name.set_text(&entry.name);
                }
                self.accept_choice();
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
        let ready = match &picker.request.mode {
            ChooserMode::Open { directory: false, .. } => !selected.is_empty(),
            ChooserMode::Open { directory: true, .. } | ChooserMode::SaveFiles { .. } => {
                folder.is_some() || matches!(selected.as_slice(), [entry] if entry.is_dir)
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
            } => self.chosen_files(&selected, *multiple),
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
            return Err("Choose one file.".to_owned());
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
        let typed = name_box.text().trim().to_owned();
        let folder = self.picking_folder().ok_or_else(not_local)?;
        let name = checked_name(&typed).map_err(|_| format!("“{typed}” is not a valid file name."))?;
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

    /// Answers with `locations` and closes the window.
    fn finish_picking(&self, picker: &Picker, locations: Vec<PathBuf>) {
        picker.reply.send(&ChooserAnswer::Chosen {
            locations,
            filter: picker.chosen_filter(),
            choices: picker.choice_values(),
        });
        self.close();
    }

    /// Cancel and Escape.
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
        let name = matches!(request.mode, ChooserMode::Save { .. }).then(|| {
            let ChooserMode::Save { name: suggested } = &request.mode else {
                unreachable!("only a Save dialog has a name box");
            };
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
    use gtk::{gio, glib};
    use ox_core::integration::{
        options_from_entries, path_variant, FileChooserBus, FILE_CHOOSER_INTERFACE, PORTAL_BACKEND_PATH,
        RESPONSE_CANCELLED, RESPONSE_SUCCESS,
    };

    use crate::test_support::harness::{capture, settle, wait_until, Fixture, TestWindow};

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
        portal.test.select_named("letter.odt");
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
