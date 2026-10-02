// SPDX-License-Identifier: AGPL-3.0-only
//! The "Extract compressed folder" dialog (ARC-009, ARC-010).
//!
//! Laid out like Windows Explorer's "Extract Compressed (Zipped) Folders",
//! and as short: the title names the archive ("Extract tidewater.zip"),
//! then one field, "Files will be extracted to this folder", filled in
//! with the archive's folder and its name (`Downloads/tidewater`), with
//! Browse…, the "Show extracted files when finished" check box, and
//! Cancel and Extract. A folder that does not exist yet is created;
//! deleting the last part extracts straight into an existing folder,
//! where the usual name-conflict question decides about files already
//! there.
//!
//! What Explorer does not show waits behind the round (i) button at the
//! left of the footer: the result of the check of every member ("Checking
//! archive contents…", then the counts and size), what happens to the ZIP
//! and to files already there, the notes on shares and passwords, and
//! Open in archive manager. A problem never hides there: a check that
//! refuses the archive (a password, say) shows its reason under the field
//! at once and brings Open in archive manager into the footer. Extract
//! waits for the check, validates the folder, and keeps the dialog open
//! with the reason when it refuses. Closing the dialog cancels the
//! check.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::archive::{suggested_folder_name, ArchiveError, ExtractionSummary, ZipExtractor};
use ox_core::format;
use ox_core::location::normalise_location;
use ox_core::transfer::Cancellation;

use super::ArchiveTarget;
use crate::dialog_layer::{check_row, quiet_text, DialogFrame, DialogWidth};
use crate::icons::{self, Icon};
use crate::window::ButtonStyle;

/// What the dialog promises, in the information bubble.
const EXTRACT_MESSAGE: &str = "The ZIP is kept unchanged. A folder that does not exist yet is created; \
                               in an existing folder you are asked before any file is replaced.";
/// The field's label, as Explorer words it.
const TARGET_LABEL: &str = "Files will be extracted to this folder";
/// Extract with an empty field.
const NO_FOLDER: &str = "Enter the folder to extract to.";
/// Shown while the members are checked.
const CHECKING: &str = "Checking archive contents…";
/// For shares and encrypted archives, in the information bubble.
const EXTRACT_HINT: &str = "For SMB, open and sign in to the source and destination shares first. \
                            Password-protected ZIPs need an external archive manager.";
/// Extract before the check finished.
const WAIT_FOR_CHECK: &str = "Still checking the ZIP. Try again in a moment.";
/// A ZIP the check refused because a member is encrypted.
const HAS_PASSWORD: &str = "This ZIP has a password, so OpenXplorer cannot extract it. \
                            Open it in the archive manager instead.";
/// The information button's name for screen readers and its tooltip.
const INFO_LABEL: &str = "More about extracting";
/// How many characters wide the information bubble's notes wrap.
const INFO_CHARS: i32 = 44;
/// A destination that is not a writable folder.
const NOT_WRITABLE: &str = "Choose a writable folder outside Previous versions, not a server listing.";
/// The size of the information button's glyph.
const INFO_GLYPH: i32 = 16;

/// Where the user chose to extract to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractionChoice {
    /// The canonical folder the files go into: created when it does not
    /// exist, filled when it does.
    pub target_uri: String,
    /// Show the extracted files when the extraction finishes.
    pub show_result: bool,
}

/// What the dialog needs from the window.
pub(crate) struct ExtractDialogSetup {
    /// The folder suggested, and the base of a relative destination.
    pub default_destination: String,
    /// The suggested folder as the field shows it.
    pub shown_destination: String,
    /// Checks the members; its result shows before Extract is allowed.
    pub inspector: ZipExtractor,
    /// True for a folder the user may extract into: not a page, a server
    /// listing or a previous version (`writableLocation`).
    pub is_writable: Box<dyn Fn(&str) -> bool>,
}

/// The dialog for `archive`. `extract` runs with the user's choice;
/// `open_externally` runs for Open in archive manager.
pub(crate) fn extract_dialog(
    archive: &ArchiveTarget,
    setup: ExtractDialogSetup,
    extract: impl Fn(ExtractionChoice) + 'static,
    open_externally: impl Fn() + 'static,
) -> DialogFrame {
    let frame = DialogFrame::new(&format!("Extract {}", archive.name), DialogWidth::Standard);
    let body = frame.body();
    let suggested = suggested_folder_name(&archive.name).unwrap_or_default();
    let target = target_field(&body, &suggested_target(&setup.shown_destination, &suggested));
    let show = check_row("Show extracted files when finished", true);
    body.append(&show);
    let open_externally: Rc<dyn Fn()> = Rc::new(open_externally);
    let (info, summary) = info_button(&frame, &open_externally);
    frame.add_footer_start(&info);
    let fallback = frame.add_closing_button("Open in archive manager", ButtonStyle::Bordered, {
        let open_externally = Rc::clone(&open_externally);
        move || open_externally()
    });
    // Only when the check refuses the archive: then it is the way on.
    fallback.set_visible(false);
    let check = Rc::new(ArchiveCheck::default());
    check.start(
        archive.uri.clone(),
        setup.inspector,
        CheckShows {
            frame: frame.downgrade(),
            summary,
            fallback,
        },
    );
    frame.connect_closed({
        let check = Rc::clone(&check);
        move |_| check.cancel.cancel()
    });
    let form = ExtractForm {
        target,
        show,
        check,
        default_destination: setup.default_destination,
        is_writable: setup.is_writable,
    };
    add_buttons(&frame, form, extract);
    frame
}

/// The round (i) button and its bubble: the check's result (returned, to
/// be filled in), what happens to the ZIP and to files already there, the
/// notes on shares and passwords, and Open in archive manager.
fn info_button(frame: &DialogFrame, open_externally: &Rc<dyn Fn()>) -> (gtk::MenuButton, gtk::Label) {
    let summary = quiet_text(CHECKING);
    summary.set_accessible_role(gtk::AccessibleRole::Status);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["extract-info-text"])
        .build();
    for text in [
        summary.clone(),
        quiet_text(EXTRACT_MESSAGE),
        quiet_text(EXTRACT_HINT),
    ] {
        text.set_max_width_chars(INFO_CHARS);
        text.set_width_chars(INFO_CHARS);
        content.append(&text);
    }
    let manager = gtk::Button::with_label("Open in archive manager");
    manager.add_css_class(ButtonStyle::Bordered.css_class());
    manager.set_halign(gtk::Align::Start);
    content.append(&manager);
    let popover = gtk::Popover::builder().child(&content).build();
    manager.connect_clicked(glib::clone!(
        #[weak]
        frame,
        #[weak]
        popover,
        #[strong]
        open_externally,
        move |_| {
            popover.popdown();
            frame.close();
            open_externally();
        }
    ));
    let button = gtk::MenuButton::builder()
        .child(&icons::image(Icon::Info, INFO_GLYPH))
        .popover(&popover)
        .tooltip_text(INFO_LABEL)
        .valign(gtk::Align::Center)
        .css_classes(["extract-info", ButtonStyle::Bordered.css_class()])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(INFO_LABEL)]);
    (button, summary)
}

/// The suggested folder: `destination` and the archive's `name`, joined
/// with `\` after a UNC destination and `/` otherwise, as the field shows
/// locations.
fn suggested_target(destination: &str, name: &str) -> String {
    let trimmed = destination.trim_end_matches(['/', '\\']);
    let separator = if destination.starts_with('\\') { '\\' } else { '/' };
    format!("{trimmed}{separator}{name}")
}

/// "Files will be extracted to this folder" with its Browse… button.
fn target_field(body: &gtk::Box, text: &str) -> gtk::Entry {
    let caption = gtk::Label::builder()
        .label(TARGET_LABEL)
        .xalign(0.0)
        .css_classes(["field-label"])
        .build();
    let entry = gtk::Entry::builder().text(text).hexpand(true).build();
    entry.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
    caption.set_mnemonic_widget(Some(&entry));
    let browse = gtk::Button::with_label("Browse…");
    browse.add_css_class(ButtonStyle::Bordered.css_class());
    browse.add_css_class("extract-browse");
    browse.set_valign(gtk::Align::Center);
    browse.connect_clicked(glib::clone!(
        #[weak]
        entry,
        move |button| pick_folder_into(button, &entry)
    ));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&entry);
    row.append(&browse);
    body.append(&caption);
    body.append(&row);
    entry
}

/// Lets the user choose a folder, starting at the one the field names
/// when it exists, and writes its path into `entry`.
fn pick_folder_into(button: &gtk::Button, entry: &gtk::Entry) {
    let picker = gtk::FileDialog::builder()
        .title("Select a destination")
        .modal(true)
        .build();
    let typed = entry.text();
    let home = glib::home_dir();
    let start = normalise_location(&typed, None, Path::new(&home))
        .ok()
        // Only a local folder is checked here, on the main thread; a share
        // could take long to answer.
        .filter(|uri| uri.starts_with("file://"))
        .map(|uri| gio::File::for_uri(&uri))
        .filter(|folder| {
            folder.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
                == gio::FileType::Directory
        });
    if let Some(start) = start {
        picker.set_initial_folder(Some(&start));
    }
    let window = button.root().and_downcast::<gtk::Window>();
    picker.select_folder(
        window.as_ref(),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            entry,
            move |chosen| {
                if let Ok(folder) = chosen {
                    let text = folder.path().map_or_else(
                        || folder.uri().to_string(),
                        |path| path.to_string_lossy().into_owned(),
                    );
                    entry.set_text(&text);
                }
            }
        ),
    );
}

/// The check of every member, which Extract waits for.
#[derive(Debug, Default)]
struct ArchiveCheck {
    /// Set once the check passed.
    is_ready: Cell<bool>,
    /// Why the check refused the archive, once it did.
    refusal: RefCell<Option<String>>,
    /// Stops the check when the dialog closes.
    cancel: Cancellation,
}

/// Where the check's answer shows.
struct CheckShows {
    /// The dialog, whose error line shows a refusal at once.
    frame: glib::WeakRef<DialogFrame>,
    /// The counts in the information bubble.
    summary: gtk::Label,
    /// Open in archive manager in the footer, shown on a refusal.
    fallback: gtk::Button,
}

impl ArchiveCheck {
    /// Checks the archive at `uri` with `inspector` and shows the summary
    /// in the bubble, or the refusal under the field. An answer after the
    /// dialog closed is dropped (SAFE-013).
    fn start(self: &Rc<Self>, uri: String, inspector: ZipExtractor, shows: CheckShows) {
        let check = Rc::clone(self);
        let inspection = inspector.inspect_in_background(uri, self.cancel.clone());
        glib::spawn_future_local(async move {
            let inspected = inspection.await;
            if check.cancel.is_cancelled() {
                return;
            }
            match inspected {
                Ok(counts) => {
                    check.is_ready.set(true);
                    shows.summary.set_text(&summary_text(&counts));
                }
                Err(error) => {
                    let reason = refusal_text(&error);
                    shows.summary.set_text(&reason);
                    shows.fallback.set_visible(true);
                    if let Some(frame) = shows.frame.upgrade() {
                        frame.show_error(&reason);
                    }
                    check.refusal.replace(Some(reason));
                }
            }
        });
    }
}

/// Why the check refused the archive, in plain words.
fn refusal_text(error: &ArchiveError) -> String {
    match error {
        ArchiveError::PasswordProtected => HAS_PASSWORD.to_owned(),
        other => other.to_string(),
    }
}

/// `3 files · 1 folder · 1.2 MB unpacked`.
pub(super) fn summary_text(summary: &ExtractionSummary) -> String {
    let files = summary.file_count;
    let folders = summary.folder_count;
    let file_word = if files == 1 { "file" } else { "files" };
    let folder_word = if folders == 1 { "folder" } else { "folders" };
    let bytes = format::pretty_bytes(summary.unpacked_bytes);
    format!("{files} {file_word} · {folders} {folder_word} · {bytes} unpacked")
}

/// The fields Extract reads.
struct ExtractForm {
    target: gtk::Entry,
    show: gtk::CheckButton,
    check: Rc<ArchiveCheck>,
    default_destination: String,
    is_writable: Box<dyn Fn(&str) -> bool>,
}

impl ExtractForm {
    /// The user's choice, or why Extract refuses it.
    fn choice(&self) -> Result<ExtractionChoice, String> {
        if let Some(reason) = self.check.refusal.borrow().as_ref() {
            return Err(reason.clone());
        }
        if !self.check.is_ready.get() {
            return Err(WAIT_FOR_CHECK.to_owned());
        }
        let typed = self.target.text();
        if typed.trim().is_empty() {
            return Err(NO_FOLDER.to_owned());
        }
        let home = glib::home_dir();
        let target = normalise_location(typed.trim(), Some(&self.default_destination), Path::new(&home))
            .map_err(|error| error.to_string())?;
        if !(self.is_writable)(&target) {
            return Err(NOT_WRITABLE.to_owned());
        }
        Ok(ExtractionChoice {
            target_uri: target,
            show_result: self.show.is_active(),
        })
    }
}

/// Cancel and Extract.
fn add_buttons(frame: &DialogFrame, form: ExtractForm, extract: impl Fn(ExtractionChoice) + 'static) {
    frame.add_closing_button("Cancel", ButtonStyle::Bordered, || {});
    let confirm = frame.add_button("Extract", ButtonStyle::Accent);
    confirm.connect_clicked(glib::clone!(
        #[weak]
        frame,
        move |_| match form.choice() {
            Ok(choice) => {
                frame.close();
                extract(choice);
            }
            Err(message) => frame.show_error(&message),
        }
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ARC-009
    #[test]
    fn the_suggested_folder_joins_the_destination_and_the_archive_name() {
        assert_eq!(
            suggested_target("/home/demo/Downloads/", "Assets"),
            "/home/demo/Downloads/Assets"
        );
        assert_eq!(
            suggested_target("\\\\nas\\share\\", "Assets"),
            "\\\\nas\\share\\Assets"
        );
    }

    /// parity: ARC-008, ARC-009
    #[test]
    fn the_summary_counts_files_and_folders_in_the_singular_and_plural() {
        let one_each = ExtractionSummary {
            file_count: 1,
            folder_count: 1,
            unpacked_bytes: 912,
            entry_count: 2,
        };
        let several = ExtractionSummary {
            file_count: 3,
            folder_count: 0,
            unpacked_bytes: 1280,
            entry_count: 3,
        };

        assert_eq!(summary_text(&one_each), "1 file · 1 folder · 912 bytes unpacked");
        assert_eq!(summary_text(&several), "3 files · 0 folders · 1.3 KB unpacked");
    }
}
