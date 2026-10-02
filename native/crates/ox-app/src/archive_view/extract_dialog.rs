// SPDX-License-Identifier: AGPL-3.0-only
//! The "Extract compressed folder" dialog (ARC-009, ARC-010).
//!
//! Ports `extractDialog` in `v2.0.0:desktop/ui/app.js`: the archive, the
//! destination folder and the new folder's name with a live "Extract
//! into:" line, the check of every member before anything is written
//! ("Checking archive contents…", then the counts and size), "Show
//! extracted files when finished", and Open in archive manager, Cancel
//! and Extract. Extract waits for the check, validates the name and the
//! destination, and keeps the dialog open with the reason when it
//! refuses. Closing the dialog cancels the check.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::archive::{suggested_folder_name, ExtractionSummary, ZipExtractor};
use ox_core::format;
use ox_core::location::{normalise_location, validate_name};
use ox_core::transfer::Cancellation;

use super::ArchiveTarget;
use crate::dialog::{check_row, labelled_entry, quiet_text, DialogFrame, DialogWidth};
use crate::icons::{Art, ArtImage};
use crate::window::ButtonStyle;

/// What the dialog promises.
const EXTRACT_MESSAGE: &str =
    "The ZIP is kept unchanged. Files are unpacked into a new folder; existing files are never replaced.";
/// Shown while the members are checked.
const CHECKING: &str = "Checking archive contents…";
/// For shares and encrypted archives.
const EXTRACT_HINT: &str = "For SMB, open and sign in to the source and destination shares first. \
                            Password-protected ZIPs need an external archive manager.";
/// Extract before the check finished.
const WAIT_FOR_CHECK: &str =
    "Wait for the ZIP check to finish. Unsupported archives need an external archive manager.";
/// A destination that is not a writable folder.
const NOT_WRITABLE: &str = "Choose a writable folder outside Previous versions, not a server listing.";
/// The size of the archive's picture (`zipFolderIcon(40)`).
const SOURCE_ART_SIZE: i32 = 40;

/// Where the user chose to extract to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractionChoice {
    /// The canonical folder the new folder goes in.
    pub destination_uri: String,
    /// The new folder's name.
    pub folder_name: String,
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
    let frame = DialogFrame::new("Extract compressed folder", DialogWidth::Standard);
    frame.set_message(EXTRACT_MESSAGE);
    let body = frame.body();
    body.append(&source_heading(&archive.name));
    let destination = labelled_entry(&body, "Destination folder", &setup.shown_destination);
    let suggested = suggested_folder_name(&archive.name).unwrap_or_default();
    let name = labelled_entry(&body, "New folder name", &suggested);
    body.append(&target_line(&destination, &name));
    let summary = quiet_text(CHECKING);
    summary.add_css_class("extract-summary");
    summary.set_accessible_role(gtk::AccessibleRole::Status);
    body.append(&summary);
    let show = check_row("Show extracted files when finished", true);
    body.append(&show);
    body.append(&quiet_text(EXTRACT_HINT));
    let check = Rc::new(ArchiveCheck::default());
    check.start(archive.uri.clone(), setup.inspector, &summary);
    frame.connect_closed({
        let check = Rc::clone(&check);
        move |_| check.cancel.cancel()
    });
    let form = ExtractForm {
        destination,
        name,
        show,
        check,
        default_destination: setup.default_destination,
        is_writable: setup.is_writable,
    };
    add_buttons(&frame, form, extract, open_externally);
    frame
}

/// The archive's picture and name (`.extract-source`).
fn source_heading(name: &str) -> gtk::Box {
    let heading = gtk::Box::builder().css_classes(["extract-source"]).build();
    heading.append(&ArtImage::new(Art::ZipFolder, SOURCE_ART_SIZE));
    let label = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(["extract-source-name"])
        .build();
    heading.append(&label);
    heading
}

/// "Extract into: <destination>/<name>", following both fields.
fn target_line(destination: &gtk::Entry, name: &gtk::Entry) -> gtk::Label {
    let line = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .css_classes(["extract-target"])
        .build();
    line.set_accessible_role(gtk::AccessibleRole::Status);
    let update = glib::clone!(
        #[weak]
        line,
        #[weak]
        destination,
        #[weak]
        name,
        move || line.set_text(&target_text(&destination.text(), &name.text()))
    );
    update();
    let update = Rc::new(update);
    for field in [destination, name] {
        let update = Rc::clone(&update);
        field.connect_changed(move |_| update());
    }
    line
}

/// The "Extract into:" text: `\` after a UNC destination, `/` otherwise.
fn target_text(destination: &str, name: &str) -> String {
    let trimmed = destination.trim_end_matches(['/', '\\']);
    let separator = if destination.starts_with('\\') { '\\' } else { '/' };
    format!("Extract into: {trimmed}{separator}{name}")
}

/// The check of every member, which Extract waits for.
#[derive(Debug, Default)]
struct ArchiveCheck {
    /// Set once the check passed.
    is_ready: Cell<bool>,
    /// Stops the check when the dialog closes.
    cancel: Cancellation,
}

impl ArchiveCheck {
    /// Checks the archive at `uri` with `inspector` and shows the summary
    /// or the refusal in `summary`. An answer after the dialog closed is
    /// dropped (SAFE-013).
    fn start(self: &Rc<Self>, uri: String, inspector: ZipExtractor, summary: &gtk::Label) {
        let check = Rc::clone(self);
        let inspection = inspector.inspect_in_background(uri, self.cancel.clone());
        glib::spawn_future_local(glib::clone!(
            #[weak]
            summary,
            async move {
                let inspected = inspection.await;
                if check.cancel.is_cancelled() {
                    return;
                }
                match inspected {
                    Ok(counts) => {
                        check.is_ready.set(true);
                        summary.set_text(&summary_text(&counts));
                    }
                    Err(error) => {
                        summary.set_text(&error.to_string());
                        summary.add_css_class("error");
                    }
                }
            }
        ));
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
    destination: gtk::Entry,
    name: gtk::Entry,
    show: gtk::CheckButton,
    check: Rc<ArchiveCheck>,
    default_destination: String,
    is_writable: Box<dyn Fn(&str) -> bool>,
}

impl ExtractForm {
    /// The user's choice, or why Extract refuses it.
    fn choice(&self) -> Result<ExtractionChoice, String> {
        if !self.check.is_ready.get() {
            return Err(WAIT_FOR_CHECK.to_owned());
        }
        let name = self.name.text();
        let folder_name = validate_name(&name).map_err(|error| error.to_string())?;
        let typed = self.destination.text();
        let home = glib::home_dir();
        let destination = normalise_location(&typed, Some(&self.default_destination), Path::new(&home))
            .map_err(|error| error.to_string())?;
        if !(self.is_writable)(&destination) {
            return Err(NOT_WRITABLE.to_owned());
        }
        Ok(ExtractionChoice {
            destination_uri: destination,
            folder_name: folder_name.to_owned(),
            show_result: self.show.is_active(),
        })
    }
}

/// Open in archive manager, Cancel and Extract.
fn add_buttons(
    frame: &DialogFrame,
    form: ExtractForm,
    extract: impl Fn(ExtractionChoice) + 'static,
    open_externally: impl Fn() + 'static,
) {
    frame.add_closing_button("Open in archive manager", ButtonStyle::Bordered, open_externally);
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
    fn the_target_line_joins_the_destination_and_the_new_name() {
        assert_eq!(
            target_text("/home/demo/Downloads/", "Assets"),
            "Extract into: /home/demo/Downloads/Assets"
        );
        assert_eq!(
            target_text("\\\\nas\\share\\", "Assets"),
            "Extract into: \\\\nas\\share\\Assets"
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
