// SPDX-License-Identifier: AGPL-3.0-only
//! The window's integration commands: Open with…, Open in Terminal, the
//! "Open in <editor>" shortcuts and Check for updates; and what the window
//! shows of the application's updates.
//!
//! Ports `openWithDialog`, `terminalMenuItem`, the editor items of
//! `entryMenu` and the status bar's `check-updates` button in
//! `desktop/ui/app.js` (OPEN-011, OPEN-015, OPEN-017, UPD-001), and the
//! window's part of the update lock (`on_delete` in
//! `desktop/winspace.py`, UPD-005). The work itself is in
//! [`crate::integration`] and [`crate::update`].

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::update::Activity;

use super::actions::{plain_action, text_action};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::integration::{self, OpenWithDialog, OpenWithSubject};
use crate::locations::Page;
use crate::update::{UpdateDialog, UpdateState};

/// The actions that act on one item, or on the folder when nothing is
/// selected, and are off for several items (`entryMenu`).
const SINGLE_ITEM_ACTIONS: [WindowAction; 3] = [
    WindowAction::OpenWith,
    WindowAction::OpenInTerminal,
    WindowAction::OpenInEditor,
];

/// The item an integration command acts on: the one selected item, or the
/// folder shown when nothing is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandSubject {
    /// Where it is.
    uri: String,
    /// Its name.
    name: String,
    /// It opens as a folder.
    is_folder: bool,
}

impl BrowserWindow {
    /// Adds Open with…, Open in Terminal, Open in <editor> and Check for
    /// updates, and follows the application's updates.
    pub(super) fn install_integration_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::OpenWith, BrowserWindow::open_with),
            text_action(WindowAction::ChangeApp, BrowserWindow::change_app),
            plain_action(WindowAction::OpenInTerminal, BrowserWindow::open_in_terminal),
            text_action(WindowAction::OpenInTerminalOf, |window, uri| {
                window.open_terminal_at(uri.to_owned());
            }),
            text_action(WindowAction::OpenWithOf, BrowserWindow::open_folder_with),
            text_action(WindowAction::OpenInEditor, BrowserWindow::open_in_editor),
            plain_action(WindowAction::CheckUpdates, BrowserWindow::check_for_updates),
        ]);
        self.follow_selection_for_integration();
        self.follow_updates();
        self.load_editor_shortcuts();
    }

    /// Reads the installed code editors off the main thread, so the
    /// context menus offer "Open in <editor>" from then on.
    fn load_editor_shortcuts(&self) {
        let integration = self.context().desktop_integration().clone();
        glib::spawn_future_local(async move {
            integration.editor_shortcuts().await;
        });
    }

    /// The one selected item, or the folder shown when nothing is
    /// selected; `None` for several items or a landing page.
    fn command_subject(&self) -> Option<CommandSubject> {
        let items = self.folder_pane().model().selected_items();
        match items.as_slice() {
            [] => self.folder_subject(),
            [item] => {
                let entry = item.entry();
                Some(CommandSubject {
                    uri: entry.navigation_uri().to_owned(),
                    name: entry.name.clone(),
                    is_folder: entry.is_dir,
                })
            }
            _ => None,
        }
    }

    /// The folder shown, unless it is a landing page.
    fn folder_subject(&self) -> Option<CommandSubject> {
        let uri = self.current_uri()?;
        if Page::from_uri(&uri).is_some() {
            return None;
        }
        let name = self.imp().locations.borrow().title_for(&uri);
        Some(CommandSubject {
            uri,
            name,
            is_folder: true,
        })
    }

    /// Open with…: the Open with dialog for the item.
    fn open_with(&self) {
        let Some(subject) = self.command_subject() else {
            return;
        };
        let subject = OpenWithSubject {
            uri: subject.uri,
            name: subject.name,
            is_folder: subject.is_folder,
        };
        OpenWithDialog::present_for(self, subject, self.application_launcher(), self.reporter());
    }

    /// Open folder with…: the Open with dialog for the folder at `uri`,
    /// such as a Quick access pin (`sidebarMenu`).
    fn open_folder_with(&self, uri: &str) {
        let name = self.imp().locations.borrow().title_for(uri);
        let subject = OpenWithSubject {
            uri: uri.to_owned(),
            name,
            is_folder: true,
        };
        OpenWithDialog::present_for(self, subject, self.application_launcher(), self.reporter());
    }

    /// Change app… in Properties: closes Properties and opens the Open
    /// with dialog for the file at `uri` (`propertiesDialog` in app.js).
    fn change_app(&self, uri: &str) {
        if let Some(properties) = self.dialog_layer().shown() {
            properties.close();
        }
        let name = self.imp().locations.borrow().base_name(uri);
        let subject = OpenWithSubject {
            uri: uri.to_owned(),
            name,
            is_folder: false,
        };
        OpenWithDialog::present_for(self, subject, self.application_launcher(), self.reporter());
    }

    /// The launch context of this window's display: an application
    /// started with it gets startup notification and focus (INT-023).
    pub(crate) fn launch_context(&self) -> gtk::gdk::AppLaunchContext {
        WidgetExt::display(self).app_launch_context()
    }

    /// Starts applications with this window's display, so they get
    /// startup notification and focus (INT-023).
    fn application_launcher(&self) -> integration::Launcher {
        let launch_context = self.launch_context();
        Box::new(move |app_id, prepared, default| {
            integration::launch(app_id, prepared, default, launch_context.upcast_ref())
        })
    }

    /// Shows messages in this window's message line, while it is open.
    fn reporter(&self) -> impl Fn(&str) + 'static {
        glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |message: &str| window.show_message(message)
        )
    }

    /// Open in Terminal: the terminal in the folder, or in the folder of
    /// the selected file; says in the message line what opened or why not.
    fn open_in_terminal(&self) {
        if let Some(subject) = self.command_subject() {
            self.open_terminal_at(subject.uri);
        }
    }

    /// Opens the terminal in the folder at `uri`, or in the folder of the
    /// file there; says in the message line what opened or why not.
    fn open_terminal_at(&self, uri: String) {
        let integration = self.context().desktop_integration();
        let settings_directory = integration.settings_directory().to_owned();
        let sandbox = integration.sandbox();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let opened = integration::open_terminal(uri, &settings_directory, sandbox).await;
                let message = opened.unwrap_or_else(|error| error.to_string());
                window.show_message(&message);
            }
        ));
    }

    /// Open in <editor>: the item in the code editor whose desktop ID is
    /// `editor_id`, through Open with's checks ("Opened with <name>").
    fn open_in_editor(&self, editor_id: &str) {
        let Some(subject) = self.command_subject() else {
            return;
        };
        let launcher = self.application_launcher();
        let editor_id = editor_id.to_owned();
        let integration = self.context().desktop_integration().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let editors = integration.editor_shortcuts().await;
                let name = editors
                    .iter()
                    .find(|editor| editor.id == editor_id)
                    .map_or_else(|| editor_id.clone(), |editor| editor.name.clone());
                let prepared = integration::prepare_launch(subject.uri, editor_id.clone()).await;
                let launched = prepared
                    .and_then(|prepared| launcher(&editor_id, &prepared, integration::DefaultChoice::Keep));
                let message = match launched {
                    Ok(_) => format!("Opened with {name}"),
                    Err(error) => error.to_string(),
                };
                window.show_message(&message);
            }
        ));
    }

    /// Enables the single-item commands for at most one selected item.
    fn follow_selection_for_integration(&self) {
        let selection = self.folder_pane().model().selection().clone();
        selection.connect_selection_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _| {
                let selected = window.folder_pane().model().summary().count;
                for action in SINGLE_ITEM_ACTIONS {
                    window.set_action_enabled(action, selected <= 1);
                }
            }
        ));
    }

    /// Check for updates: the Software updates dialog, which checks at
    /// once (UPD-001).
    fn check_for_updates(&self) {
        let updates = self.context().updates();
        let app = self.application().map(|app| app.downgrade());
        UpdateDialog::present_for(self, updates, move || {
            let app = app.as_ref().and_then(glib::WeakRef::upgrade);
            work_in_windows(app.as_ref())
        });
    }

    /// Shows the application's updates in this window: the status bar's
    /// notice, and the lock while an update installs (UPD-005).
    fn follow_updates(&self) {
        let updates = self.context().updates();
        let handler = updates.connect_state_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |updates| window.show_update_state(&updates.state())
        ));
        self.imp().handlers.borrow_mut().updates = Some(handler);
        self.show_update_state(&updates.state());
    }

    fn show_update_state(&self, state: &UpdateState) {
        self.status_bar()
            .show_update_notice(state.status_bar_notice().as_deref());
        // Nothing in any window works while the package manager runs;
        // the Software updates dialog stays usable.
        self.set_sensitive(!state.is_installing());
    }

    /// Why the window may not close now, if it may not (UPD-005).
    pub(super) fn close_refusal(&self) -> Option<String> {
        self.context().updates().close_refusal()
    }

    /// The tooltip of the status bar's "Check for updates", for tests.
    #[cfg(test)]
    pub(crate) fn status_bar_update_tooltip(&self) -> String {
        self.status_bar().check_updates_tooltip()
    }

    /// The message line's text, for tests of other modules.
    #[cfg(test)]
    pub(crate) fn shown_message_text(&self) -> String {
        self.shown_message().to_string()
    }

    /// Whether any tab of this window is being listed.
    fn is_listing_any_tab(&self) -> bool {
        let session = self.imp().session.borrow();
        session.tabs().iter().any(|tab| tab.listing_state.is_listing())
    }
}

/// Whether any window of `app` has work an update must not interrupt:
/// folder listings and file writes (UPD-007, OPS-024).
fn work_in_windows(app: Option<&gtk::Application>) -> Activity {
    let windows = app.map(GtkApplicationExt::windows).unwrap_or_default();
    let busy = windows
        .into_iter()
        .filter_map(|window| window.downcast::<BrowserWindow>().ok())
        .any(|window| window.is_listing_any_tab() || window.is_writing_files());
    if busy {
        Activity::Busy
    } else {
        Activity::Idle
    }
}
