// SPDX-License-Identifier: AGPL-3.0-only
//! Compress to… (ARC-023), as Dolphin's "Compress → Compress to…": a
//! dialog asks for the new archive's name and format, ZIP by default or
//! `.tar.xz`, then compresses the selection beside it with progress and
//! Cancel. A taken name is refused in the dialog; nothing is replaced.

use gtk::glib;
use gtk::prelude::*;
use ox_core::archive::{ArchiveError, CompressionRequest, ZipCompressor};
use ox_core::location::validate_name;
use ox_core::transfer::Cancellation;

use crate::archive_view::{compressed_file_name, compression_success_text};
use crate::dialog::{labelled_entry, DialogFrame, DialogWidth};

use super::transfer_panel::TransferKind;
use super::{BrowserWindow, ButtonStyle};

/// The formats offered, with the ending each adds.
const FORMATS: [(&str, &str); 2] = [("ZIP (.zip)", ".zip"), ("TAR.XZ (.tar.xz)", ".tar.xz")];

impl BrowserWindow {
    /// Asks for the name and format of a new archive of the selection.
    pub(super) fn ask_compress_to(&self) {
        let selected = self.folder_pane().model().selected_items();
        let (Some(first), Some(folder)) = (selected.first(), self.current_uri()) else {
            return;
        };
        let uris: Vec<String> = selected.iter().map(|item| item.entry().uri.clone()).collect();
        let suggested = compressed_file_name(&first.entry().name)
            .next()
            .unwrap_or_default()
            .trim_end_matches(".zip")
            .to_owned();
        let frame = DialogFrame::new(&ox_core::i18n::gettext("Compress"), DialogWidth::Standard);
        let name = labelled_entry(&frame.body(), &ox_core::i18n::gettext("Archive name"), &suggested);
        let formats: Vec<&str> = FORMATS.iter().map(|(label, _)| *label).collect();
        let format = gtk::DropDown::from_strings(&formats);
        format.set_halign(gtk::Align::Start);
        format.update_property(&[gtk::accessible::Property::Label("Format")]);
        frame.body().append(&format);
        let compress = frame.add_button(&ox_core::i18n::gettext("Compress"), ButtonStyle::Accent);
        frame.add_closing_button(&ox_core::i18n::gettext("Cancel"), ButtonStyle::Bordered, || {});
        compress.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            frame,
            #[weak]
            name,
            #[weak]
            format,
            move |_| {
                let ending = FORMATS
                    .get(format.selected() as usize)
                    .map_or(".zip", |(_, ending)| ending);
                let archive_name = format!("{}{ending}", name.text().trim());
                if let Err(error) = validate_name(&archive_name) {
                    frame.show_error(&error.to_string());
                    return;
                }
                let request = CompressionRequest {
                    uris: uris.clone(),
                    destination_uri: folder.clone(),
                    archive_name,
                };
                window.compress_to(request, &frame);
            }
        ));
        name.connect_activate(glib::clone!(
            #[weak]
            compress,
            move |_| compress.emit_clicked()
        ));
        self.present_window_dialog(&frame);
        name.grab_focus();
    }

    /// Compresses as `request` says; a taken name keeps `frame` open with
    /// the reason, anything else closes it and reports in the folder.
    fn compress_to(&self, request: CompressionRequest, frame: &DialogFrame) {
        if !self.may_start_archive_operation() {
            return;
        }
        let cancel = Cancellation::new();
        self.transfer_panel()
            .start(TransferKind::Archive, "Preparing compression…", cancel.clone());
        self.update_archive_actions();
        let compressor = ZipCompressor::new()
            .with_write_guard(self.context().previous_versions().write_guard())
            .with_progress(self.operation_progress_sender());
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
                        let outcome = outcome
                            .map(|created| compression_success_text(&created))
                            .map_err(|error| crate::archive_view::compression_failure_text(&error));
                        window.report_in_folder(&folder, outcome, crate::archive_view::COMPRESSION_STOPPED);
                    }
                }
            }
        ));
    }
}
