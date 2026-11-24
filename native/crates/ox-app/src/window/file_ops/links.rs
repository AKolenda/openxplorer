// SPDX-License-Identifier: AGPL-3.0-only
//! Create links, as a drop with Ctrl+Shift or the drop menu's "Create
//! links here" asks (DND-019), and New ▸ Shortcut, Windows' Create
//! Shortcut wizard (OPS-004, OPS-051).
//!
//! A drop's links are made by ox-core's [`create_links`] as the window's
//! one operation. New ▸ Shortcut asks, on two pages as Windows' wizard
//! does, for the location (an item of this computer, a network share or
//! an Internet address) and then for the name, and a refusal, such as a
//! path where nothing is or a taken name, stays on its page. The new links are
//! selected afterwards, as other operations' items are, and Undo moves
//! them to the Trash (OPS-029).

use glib::clone;
use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::ops::{
    create_link, create_links, create_named_shortcuts, create_shortcuts, shortcut_location, starting_label,
    suggested_name, LinkRequest, NamedShortcuts, NewLink, ShortcutLocation,
};
use ox_core::transfer::TransferMode;

use super::running::FinishedOperation;
use super::FileCommand;
use crate::dialog::Dialog;
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// The Create Shortcut wizard's title, as Windows titles it.
const WIZARD_TITLE: &str = crate::i18n::message_id("Create Shortcut");

/// The question of its first page.
const LOCATION_QUESTION: &str = crate::i18n::message_id("What item would you like to create a shortcut for?");

/// What its first page says the wizard does, in Windows' words.
const WIZARD_MESSAGE: &str = crate::i18n::message_id(
    "This wizard helps you to create shortcuts to local or network programs, files, folders, computers, or \
     Internet addresses.",
);

/// The question of its second page.
const NAME_QUESTION: &str = crate::i18n::message_id("What would you like to name the shortcut?");

/// Why a location could not be read.
const NOT_READ: &str = crate::i18n::message_id("The location could not be read. Try again.");

impl BrowserWindow {
    /// Creates the links `request` asks for once the drop handler has
    /// returned, so the drag has finished first.
    pub(in crate::window) fn spawn_links(&self, request: LinkRequest) {
        glib::spawn_future_local(clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                window.run_links(&request).await;
            }
        ));
    }

    /// Runs `request` as the window's one operation and reports it; does
    /// nothing while another operation runs.
    async fn run_links(&self, request: &LinkRequest) {
        let Some(context) = self.begin_operation(starting_label(TransferMode::Copy)) else {
            return;
        };
        let outcome = create_links(request, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }

    /// Makes the shortcuts `request` asks for, named as Windows names them
    /// (Send to ▸ Desktop (create shortcut), CMD-038), as the window's one
    /// operation and reports them; Undo moves them to the Trash. Does
    /// nothing while another operation runs. The shortcuts are elsewhere,
    /// so the rows at `selected`, the items' own rows, stay selected, as in
    /// Windows Explorer.
    pub(in crate::window) async fn run_shortcuts(&self, request: &LinkRequest, selected: Vec<String>) {
        let Some(context) = self.begin_operation(starting_label(TransferMode::Copy)) else {
            return;
        };
        let outcome = create_shortcuts(request, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            undo: outcome.undo,
            select_after: selected,
        });
        self.conclude_operation(finished).await;
    }

    /// Makes the shortcuts `request` names, such as a drive's pasted or
    /// dropped into a folder ("Local Disk - Shortcut"), as the window's
    /// one operation, asking about a failure in the progress window
    /// (OPS-047), then selects them, as Windows
    /// Explorer selects what a paste made; Undo moves them to the Trash.
    /// False when it could not start, after saying why, as while another
    /// operation runs: a paste or a drop then leaves its other items too.
    pub(in crate::window) async fn run_named_shortcuts(&self, request: &NamedShortcuts) -> bool {
        let Some(context) = self.begin_operation(starting_label(TransferMode::Copy)) else {
            if self.is_writing_files() {
                self.show_message(ox_core::i18n::gettext_static(
                    "Wait for the running file operation to finish.",
                ));
            }
            return false;
        };
        let outcome = create_named_shortcuts(request, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
        true
    }

    /// New ▸ Shortcut: Windows' Create Shortcut wizard (OPS-051). It asks
    /// for the location, which must exist, then for the name, which it
    /// proposes as Windows does; Back returns to the location, keeping it;
    /// Finish makes the shortcut, then selects it, and a refusal, such as a
    /// taken name, stays on its page.
    pub(crate) async fn create_link(&self) {
        if !self.allows(FileCommand::New) || self.refuses_writes() {
            return;
        }
        let Some(folder) = self.current_uri() else {
            return;
        };
        let mut typed = String::new();
        loop {
            let Some((location, chosen)) = self.ask_shortcut_location(&folder, &typed).await else {
                return;
            };
            typed = chosen;
            match self.ask_shortcut_name(&folder, &typed, &location).await {
                WizardEnd::Back => {}
                WizardEnd::Done => return,
            }
        }
    }

    /// The wizard's first page, showing `typed`: the location, read once
    /// Next is pressed; `None` when cancelled.
    async fn ask_shortcut_location(&self, folder: &str, typed: &str) -> Option<(ShortcutLocation, String)> {
        let message = format!(
            "{}\n{}",
            ox_core::i18n::gettext_static(LOCATION_QUESTION),
            ox_core::i18n::gettext_static(WIZARD_MESSAGE)
        );
        let dialog = Dialog::with_title_bar(self, ox_core::i18n::gettext_static(WIZARD_TITLE), &message);
        let field = gtk::Entry::builder()
            .text(typed)
            .activates_default(true)
            .hexpand(true)
            .build();
        let browse = gtk::Button::with_label(ox_core::i18n::gettext_static("Browse…"));
        browse.add_css_class(ButtonStyle::Bordered.css_class());
        browse.connect_clicked(clone!(
            #[weak]
            dialog,
            #[weak]
            field,
            move |_| browse_for_item(&dialog, &field)
        ));
        let row = gtk::Box::builder().spacing(8).build();
        row.append(&field);
        row.append(&browse);
        dialog.add_labelled(
            ox_core::i18n::gettext_static("Type the location of the item:"),
            &row,
        );
        field.update_property(&[gtk::accessible::Property::Label(ox_core::i18n::gettext_static(
            "Type the location of the item:",
        ))]);
        dialog.add_note(ox_core::i18n::gettext_static("Click Next to continue."));
        dialog.add_cancel_button();
        dialog.add_button(ox_core::i18n::gettext_static("Next"), ButtonStyle::Accent);
        dialog.open_focusing(&field);
        loop {
            dialog.next_response().await?;
            let (text, here) = (field.text().to_string(), folder.to_owned());
            let read = gio::spawn_blocking({
                let text = text.clone();
                move || shortcut_location(&text, &here)
            })
            .await;
            match read {
                Ok(Ok(location)) => {
                    dialog.finish();
                    return Some((location, text));
                }
                Ok(Err(error)) => dialog.show_error(&error.to_string()),
                Err(_) => dialog.show_error(ox_core::i18n::gettext_static(NOT_READ)),
            }
        }
    }

    /// The wizard's second page: the name of the shortcut to `location`,
    /// typed as `typed`, then the shortcut itself.
    async fn ask_shortcut_name(&self, folder: &str, typed: &str, location: &ShortcutLocation) -> WizardEnd {
        let dialog = Dialog::with_title_bar(
            self,
            ox_core::i18n::gettext_static(WIZARD_TITLE),
            ox_core::i18n::gettext_static(NAME_QUESTION),
        );
        let name = dialog.add_text_field(
            ox_core::i18n::gettext_static("Type a name for this shortcut:"),
            &suggested_name(location),
        );
        dialog.add_note(ox_core::i18n::gettext_static(
            "Click Finish to create the shortcut.",
        ));
        let back = dialog.add_button(ox_core::i18n::gettext_static("Back"), ButtonStyle::Bordered);
        dialog.add_cancel_button();
        dialog.add_button(ox_core::i18n::gettext_static("Finish"), ButtonStyle::Accent);
        dialog.open_focusing(&name);
        name.select_region(0, -1);
        loop {
            let Some(answer) = dialog.next_response().await else {
                return WizardEnd::Done;
            };
            if answer == back {
                dialog.finish();
                return WizardEnd::Back;
            }
            let request = NewLink {
                folder: folder.to_owned(),
                name: name.text().to_string(),
                target: typed.to_owned(),
            };
            // A drive may have started formatting while the dialog was open.
            let (context, _writing) = match self.admit_write() {
                Ok(admitted) => admitted,
                Err(refusal) => {
                    dialog.show_error(&refusal);
                    continue;
                }
            };
            dialog.set_busy(Some(&context.cancel));
            let outcome = create_link(&request, &context).await;
            dialog.set_busy(None);
            match outcome {
                Ok(link) => {
                    dialog.finish();
                    self.context().record_operation(link.undo_record());
                    self.reload_selecting(vec![link.uri]);
                    return WizardEnd::Done;
                }
                Err(error) => dialog.show_error(&error.to_string()),
            }
        }
    }
}

/// How the wizard's second page ended.
enum WizardEnd {
    /// Back to the location.
    Back,
    /// The shortcut was made, or the wizard cancelled.
    Done,
}

/// Browse…: the desktop's file chooser over `dialog`; the item chosen
/// goes into `field` between double quotes, as Windows' Browse puts it.
fn browse_for_item(dialog: &Dialog, field: &gtk::Entry) {
    let chooser = gtk::FileDialog::builder()
        .title(ox_core::i18n::gettext_static("Browse for Files or Folders"))
        .modal(true)
        .build();
    chooser.open(
        Some(dialog),
        gio::Cancellable::NONE,
        clone!(
            #[weak]
            field,
            move |chosen| {
                if let Some(path) = chosen.ok().and_then(|file| file.path()) {
                    field.set_text(&format!("\"{}\"", path.display()));
                }
            }
        ),
    );
}
