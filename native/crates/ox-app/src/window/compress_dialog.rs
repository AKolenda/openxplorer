// SPDX-License-Identifier: AGPL-3.0-only
//! Compress to › Additional options (ARC-023): the Create archive
//! dialog, laid out as Windows 11's. From the top: the heading, the
//! archive's whole path with Browse…, the archive format, its compression
//! method and level, the link check boxes, and Create and Cancel.
//! Choosing a format or a method changes the path's ending. Only what the
//! app writes is offered: ZIP (deflated) and TAR, plain or compressed with
//! gzip, bzip2 or XZ. Links are never kept, so the link check boxes show
//! grayed and clear. A taken name is refused in the dialog; nothing is
//! replaced, and Undo takes the new archive away again.

use std::path::Path;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::archive::{ArchiveError, CompressionRequest, CreatedArchive, ZipCompressor};
use ox_core::location::{normalise_location, parent_location, validate_name, ItemKind};
use ox_core::ops::CreatedItem;
use ox_core::transfer::Cancellation;

use crate::archive_view::{compressed_file_name, compression_success_text, ZIP_ENDING};
use crate::dialog::{check_row, DialogFrame, DialogWidth};

use super::transfer_panel::TransferKind;
use super::{BrowserWindow, ButtonStyle};

/// The heading.
const HEADING: &str = crate::i18n::message_id("Select a destination and create archive");
/// What the dialog is called by assistive technologies.
const ACCESSIBLE_NAME: &str = crate::i18n::message_id("Create archive");
/// The method of a TAR that is not compressed.
const NO_COMPRESSION: &str = crate::i18n::message_id("None");

/// An archive format the app writes, with its compression methods and
/// the ending each gives the archive.
struct Format {
    label: &'static str,
    methods: &'static [(&'static str, &'static str)],
}

/// The formats offered, in the drop-down's order.
const FORMATS: [Format; 2] = [
    Format {
        label: "zip",
        methods: &[("Deflate", ".zip")],
    },
    Format {
        label: "tar",
        methods: &[
            (NO_COMPRESSION, ".tar"),
            ("GZip", ".tar.gz"),
            ("BZip2", ".tar.bz2"),
            ("XZ", ".tar.xz"),
        ],
    },
];

/// The archive endings a path loses when the format or method changes,
/// longer ones first.
const KNOWN_ENDINGS: [&str; 8] = [
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".tar", ".zip",
];

/// The level the slider starts at, the formats' usual default.
const DEFAULT_LEVEL: f64 = 6.0;

/// The controls whose values make the request.
#[derive(Clone)]
struct CreateArchiveControls {
    path: gtk::Entry,
    format: gtk::DropDown,
    method: gtk::DropDown,
    level: gtk::Scale,
}

impl CreateArchiveControls {
    /// The method chosen, as (label, ending).
    fn method(&self) -> (&'static str, &'static str) {
        let format = FORMATS
            .get(self.format.selected() as usize)
            .unwrap_or(&FORMATS[0]);
        *format
            .methods
            .get(self.method.selected() as usize)
            .unwrap_or(&format.methods[0])
    }

    /// Offers the chosen format's methods, the first chosen.
    fn offer_methods(&self) {
        let format = FORMATS
            .get(self.format.selected() as usize)
            .unwrap_or(&FORMATS[0]);
        let labels: Vec<String> = format
            .methods
            .iter()
            .map(|(label, _)| ox_core::i18n::gettext(label))
            .collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.method.set_model(Some(&gtk::StringList::new(&labels)));
        self.method.set_selected(0);
        self.follow_method();
    }

    /// Gives the path the chosen method's ending, and grays out the level
    /// for a TAR that is not compressed.
    fn follow_method(&self) {
        let (label, ending) = self.method();
        self.path.set_text(&with_ending(&self.path.text(), ending));
        self.level.set_sensitive(label != NO_COMPRESSION);
    }
}

impl BrowserWindow {
    /// Asks where and how to archive the selection.
    pub(super) fn ask_compress_to(&self) {
        let selected = self.folder_pane().model().selected_items();
        let (Some(first), Some(folder)) = (selected.first(), self.current_uri()) else {
            return;
        };
        let uris: Vec<String> = selected.iter().map(|item| item.entry().uri.clone()).collect();
        self.ask_compress(uris, &folder, &first.entry().name);
    }

    /// Shows the Create archive dialog for `uris` in `folder`, suggesting
    /// an archive named after `first_name` beside them.
    pub(super) fn ask_compress(&self, uris: Vec<String>, folder: &str, first_name: &str) {
        let suggested = compressed_file_name(first_name, ZIP_ENDING)
            .next()
            .unwrap_or_default();
        let frame = DialogFrame::new(ox_core::i18n::gettext_static(HEADING), DialogWidth::Standard);
        frame.add_css_class("create-archive");
        frame.update_property(&[gtk::accessible::Property::Label(ox_core::i18n::gettext_static(
            ACCESSIBLE_NAME,
        ))]);
        let body = frame.body();
        let path = path_field(&body, &location_text(folder, &suggested));
        let format_labels: Vec<&str> = FORMATS.iter().map(|format| format.label).collect();
        let format = labelled_dropdown(&body, &ox_core::i18n::gettext("Archive format:"), &format_labels);
        let method = labelled_dropdown(&body, &ox_core::i18n::gettext("Compression method:"), &[]);
        let level = level_slider(&body);
        for label in [
            ox_core::i18n::gettext("Retain symbolic links"),
            ox_core::i18n::gettext("Retain hard links"),
        ] {
            let check = check_row(&label, false);
            check.set_sensitive(false);
            body.append(&check);
        }
        let controls = CreateArchiveControls {
            path: path.clone(),
            format: format.clone(),
            method: method.clone(),
            level,
        };
        controls.offer_methods();
        format.connect_selected_notify(glib::clone!(
            #[strong]
            controls,
            move |_| controls.offer_methods()
        ));
        method.connect_selected_notify(glib::clone!(
            #[strong]
            controls,
            move |_| controls.follow_method()
        ));
        let create = frame.add_button(&ox_core::i18n::gettext("Create"), ButtonStyle::Accent);
        frame.add_closing_button(&ox_core::i18n::gettext("Cancel"), ButtonStyle::Bordered, || {});
        create.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            frame,
            #[strong]
            controls,
            move |_| match archive_request(&controls, &uris) {
                Ok(request) => {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "the slider runs from 1 to 9 in whole steps"
                    )]
                    let level = controls.level.value().round() as u32;
                    window.compress_to(request, &frame, level);
                }
                Err(message) => frame.show_error(&message),
            }
        ));
        path.connect_activate(glib::clone!(
            #[weak]
            create,
            move |_| create.emit_clicked()
        ));
        self.present_window_dialog(&frame);
        path.grab_focus();
    }

    /// Compresses as `request` says; a taken name keeps `frame` open with
    /// the reason, anything else closes it and reports in the folder.
    fn compress_to(&self, request: CompressionRequest, frame: &DialogFrame, level: u32) {
        if !self.may_start_archive_operation() {
            return;
        }
        let cancel = Cancellation::new();
        if !self.claim_transfer_panel(TransferKind::Archive, "Preparing compression…", &cancel) {
            return;
        }
        self.update_archive_actions();
        let compressor = ZipCompressor::new()
            .with_level(level)
            .with_write_guard(self.context().previous_versions().write_guard())
            .with_progress(self.progress_reporter(&cancel));
        self.describe_compression(&cancel, &request);
        let folder = request.destination_uri.clone();
        let frame = frame.downgrade();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let created = compressor.compress_in_background(request, cancel).await;
                window.finish_archive_operation();
                let frame = frame.upgrade();
                match (created, frame) {
                    (Err(ArchiveError::ArchiveExists), Some(frame)) => {
                        frame.show_error(&ArchiveError::ArchiveExists.to_string());
                    }
                    (outcome, frame) => {
                        if let Some(frame) = frame {
                            frame.close();
                        }
                        // Windows Explorer says nothing after Cancel.
                        if outcome.as_ref().is_err_and(ArchiveError::is_cancelled) {
                            return;
                        }
                        if let Ok(created) = &outcome {
                            window.remember_created_archive(created);
                        }
                        let outcome = outcome
                            .map(|created| compression_success_text(&created))
                            .map_err(|error| crate::archive_view::compression_failure_text(&error));
                        window.report_in_folder(&folder, outcome, crate::archive_view::COMPRESSION_STOPPED);
                    }
                }
            }
        ));
    }

    /// Lets Undo take `created` away again, to the Recycle Bin.
    fn remember_created_archive(&self, created: &CreatedArchive) {
        let item = CreatedItem {
            uri: created.uri.clone(),
            kind: ItemKind::File,
        };
        self.context().record_operation(item.undo_record());
    }
}

/// The request for the archive the dialog's controls describe: the path
/// in the field, given the chosen method's ending when it has none.
fn archive_request(controls: &CreateArchiveControls, uris: &[String]) -> Result<CompressionRequest, String> {
    let (_, ending) = controls.method();
    let typed = controls.path.text().trim().to_owned();
    if typed.is_empty() {
        return Err(ox_core::i18n::gettext("Enter where to create the archive."));
    }
    let path = with_method_ending(&typed, ending);
    let home = glib::home_dir();
    let uri = normalise_location(&path, None, Path::new(&home)).map_err(|error| error.to_string())?;
    let folder =
        parent_location(&uri).ok_or_else(|| ox_core::i18n::gettext("Enter where to create the archive."))?;
    let name = gio::File::for_uri(&uri)
        .basename()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    validate_name(&name).map_err(|error| error.to_string())?;
    Ok(CompressionRequest {
        uris: uris.to_vec(),
        destination_uri: folder,
        archive_name: name,
    })
}

/// Whether the file name in `path` ends with the archive ending `known`,
/// in any case, and is more than that ending: a name such as `.tgz` is
/// only a name.
fn has_ending(path: &str, known: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).as_bytes();
    name.len() > known.len() && name[name.len() - known.len()..].eq_ignore_ascii_case(known.as_bytes())
}

/// `path` with its archive ending, if any, replaced by `ending`.
fn with_ending(path: &str, ending: &str) -> String {
    let stem = KNOWN_ENDINGS
        .iter()
        .find(|known| has_ending(path, known))
        .map_or(path, |known| &path[..path.len() - known.len()]);
    format!("{stem}{ending}")
}

/// The path Create writes for the typed `path` with the chosen method's
/// `ending`: kept when it already ends so, or in that ending's short
/// form (`.tgz` for `.tar.gz`, `.tbz2` for `.tar.bz2`), which the
/// compressor writes the same way; else the ending the format or method
/// gives, in place of another archive ending, so `Notes.tgz` is never
/// written as `Notes.tgz.tar.gz`.
fn with_method_ending(path: &str, ending: &str) -> String {
    let short = match ending {
        ".tar.gz" => Some(".tgz"),
        ".tar.bz2" => Some(".tbz2"),
        _ => None,
    };
    if has_ending(path, ending) || short.is_some_and(|short| has_ending(path, short)) {
        path.to_owned()
    } else {
        with_ending(path, ending)
    }
}

/// How the field shows the archive `name` in `folder`: a local path, or
/// the location's address.
fn location_text(folder: &str, name: &str) -> String {
    let file = gio::File::for_uri(folder).child(name);
    file.path().map_or_else(
        || file.uri().to_string(),
        |path| path.to_string_lossy().into_owned(),
    )
}

/// A field label, as the other dialogs' (`field-label`).
fn field_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["field-label"])
        .build()
}

/// "Files will be added to this archive:" with the path and Browse….
fn path_field(body: &gtk::Box, text: &str) -> gtk::Entry {
    let caption = field_label(&ox_core::i18n::gettext("Files will be added to this archive:"));
    let entry = gtk::Entry::builder().text(text).hexpand(true).build();
    entry.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
    caption.set_mnemonic_widget(Some(&entry));
    let browse = gtk::Button::with_label(&ox_core::i18n::gettext("Browse…"));
    browse.add_css_class(ButtonStyle::Bordered.css_class());
    // Drawn as the Extract dialog's Browse…, as tall as the field.
    browse.add_css_class("extract-browse");
    browse.set_valign(gtk::Align::Center);
    browse.connect_clicked(glib::clone!(
        #[weak]
        entry,
        move |button| pick_archive_into(button, &entry)
    ));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&entry);
    row.append(&browse);
    body.append(&caption);
    body.append(&row);
    entry
}

/// A drop-down of `labels` under the label `text`.
fn labelled_dropdown(body: &gtk::Box, text: &str, labels: &[&str]) -> gtk::DropDown {
    let caption = field_label(text);
    let dropdown = gtk::DropDown::from_strings(labels);
    dropdown.set_halign(gtk::Align::Start);
    dropdown.set_size_request(220, -1);
    dropdown.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
    body.append(&caption);
    body.append(&dropdown);
    dropdown
}

/// "Compression level:", a slider from 1 to 9 with a tick at each step,
/// "Faster" under its start and "Smaller" under its end.
fn level_slider(body: &gtk::Box) -> gtk::Scale {
    let caption = field_label(&ox_core::i18n::gettext("Compression level:"));
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 9.0, 1.0);
    scale.set_value(DEFAULT_LEVEL);
    scale.set_draw_value(false);
    scale.set_round_digits(0);
    for step in 1..=9 {
        scale.add_mark(f64::from(step), gtk::PositionType::Bottom, None);
    }
    scale.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
    let ends = gtk::CenterBox::new();
    let faster = gtk::Label::new(Some(&ox_core::i18n::gettext("Faster")));
    let smaller = gtk::Label::new(Some(&ox_core::i18n::gettext("Smaller")));
    for label in [&faster, &smaller] {
        label.add_css_class("quiet-level");
    }
    ends.set_start_widget(Some(&faster));
    ends.set_end_widget(Some(&smaller));
    body.append(&caption);
    body.append(&scale);
    body.append(&ends);
    scale
}

/// Lets the user choose where to save the archive, starting at the folder
/// and name the field shows, and writes the chosen path into `entry`.
fn pick_archive_into(button: &gtk::Button, entry: &gtk::Entry) {
    let picker = gtk::FileDialog::builder()
        .title(ox_core::i18n::gettext("Select a destination"))
        .modal(true)
        .build();
    let typed = entry.text();
    let home = glib::home_dir();
    if let Ok(uri) = normalise_location(&typed, None, Path::new(&home)) {
        let file = gio::File::for_uri(&uri);
        if let Some(name) = file.basename() {
            picker.set_initial_name(Some(&name.to_string_lossy()));
        }
        // Only a local folder is checked here, on the main thread; a share
        // could take long to answer.
        let folder = file.parent().filter(|folder| {
            folder.uri().starts_with("file://")
                && folder.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
                    == gio::FileType::Directory
        });
        if let Some(folder) = folder {
            picker.set_initial_folder(Some(&folder));
        }
    }
    let window = button.root().and_downcast::<gtk::Window>();
    picker.save(
        window.as_ref(),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            entry,
            move |chosen| {
                if let Ok(file) = chosen {
                    let text = file.path().map_or_else(
                        || file.uri().to_string(),
                        |path| path.to_string_lossy().into_owned(),
                    );
                    entry.set_text(&text);
                }
            }
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ARC-023
    #[test]
    fn the_ending_follows_the_format_and_method() {
        assert_eq!(with_ending("/a/Notes 2.zip", ".tar.gz"), "/a/Notes 2.tar.gz");
        assert_eq!(with_ending("/a/Notes 2.TAR.GZ", ".zip"), "/a/Notes 2.zip");
        assert_eq!(with_ending("/a/report.txt", ".tar"), "/a/report.txt.tar");
        assert_eq!(with_ending(".zip", ".tar"), ".zip.tar");
    }

    /// parity: ARC-023
    #[test]
    fn create_keeps_a_typed_ending_of_the_chosen_method() {
        assert_eq!(with_method_ending("/a/Site.tgz", ".tar.gz"), "/a/Site.tgz");
        assert_eq!(with_method_ending("/a/Site.TBZ2", ".tar.bz2"), "/a/Site.TBZ2");
        assert_eq!(with_method_ending("/a/Site.tar.xz", ".tar.xz"), "/a/Site.tar.xz");
        assert_eq!(with_method_ending("/a/Site.tgz", ".tar.xz"), "/a/Site.tar.xz");
        assert_eq!(with_method_ending("/a/Site.zip", ".tar"), "/a/Site.tar");
        assert_eq!(with_method_ending("/a/Site", ".zip"), "/a/Site.zip");
        assert_eq!(with_method_ending("/a/.tgz", ".tar.gz"), "/a/.tgz.tar.gz");
    }
}
