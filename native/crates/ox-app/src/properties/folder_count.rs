// SPDX-License-Identifier: AGPL-3.0-only
//! A folder's Size, Size on disk and Contains, counting up by themselves
//! as Windows 11's Properties count them (PROP-004, PROP-027).
//!
//! The folder is measured on a worker thread from the moment its
//! Properties open; every total the scan reports so far reaches the rows
//! on the main thread, so they climb while the user reads the tab. Closing
//! the window cancels the scan. The measure is the same as Calculate folder
//! size's: links, nested mounts and snapshot collections are skipped.
//!
//! A total is never shown as more exact than it is (PROP-027): once the
//! scan left something out or could not read part of the folder, the rows
//! read `≥ …`; a folder that could not be read at all reads Unavailable,
//! never a false zero.

use gtk::glib;
use ox_core::format;
use ox_core::sizes::{scan_folder_size_in_background, FolderSize, ScanStatus};
use ox_core::transfer::Cancellation;

/// The rows a count updates.
#[derive(Debug, Clone)]
pub(super) struct FolderCountRows {
    pub size: gtk::Label,
    pub on_disk: gtk::Label,
    pub contains: gtk::Label,
}

/// Totals added up over several folders and files, for a selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Totals {
    pub bytes: u64,
    /// `None` once any item's space on disk is unknown.
    pub on_disk: Option<u64>,
    pub files: u64,
    pub folders: u64,
    /// Something was left out or could not be read: the totals are a
    /// lower bound.
    pub partial: bool,
}

/// Shown for a folder that could not be read.
const UNAVAILABLE: &str = crate::i18n::message_id("Unavailable");

impl Totals {
    /// Nothing counted yet.
    pub(super) fn zero() -> Self {
        Self {
            on_disk: Some(0),
            ..Self::default()
        }
    }

    /// These totals with `size`'s added.
    pub(super) fn plus(self, size: &FolderSize) -> Self {
        Self {
            bytes: self.bytes.saturating_add(size.bytes),
            on_disk: self
                .on_disk
                .zip(size.allocated)
                .map(|(total, allocated)| total.saturating_add(allocated)),
            files: self.files.saturating_add(size.files),
            folders: self.folders.saturating_add(size.folders),
            partial: self.partial || matches!(size.status, ScanStatus::Partial(_) | ScanStatus::Cancelled),
        }
    }
}

/// Whether a finished scan read nothing of its folder: it failed, or it
/// found the folder unreadable and counted nothing.
fn read_nothing(size: &FolderSize) -> bool {
    size.errors > 0 && size.entries == 0 && size.files == 0 && size.folders == 0
}

impl FolderCountRows {
    /// Shows `totals`, as the count has them so far, as a lower bound
    /// when they are partial.
    pub(super) fn show(&self, totals: Totals) {
        let bound = |text: String| {
            if totals.partial {
                ox_core::i18n::format_message("≥ {value}", &[("value", &text)])
            } else {
                text
            }
        };
        self.size.set_text(&bound(format::exact_size_text(totals.bytes)));
        self.on_disk.set_text(
            &totals
                .on_disk
                .map(format::exact_size_text)
                .map(bound)
                .unwrap_or_default(),
        );
        self.contains
            .set_text(&bound(format::contents_text(totals.files, totals.folders)));
    }

    /// Shows that the folder could not be read.
    pub(super) fn show_unavailable(&self) {
        let text = ox_core::i18n::gettext_static(UNAVAILABLE);
        for row in [&self.size, &self.on_disk, &self.contains] {
            row.set_text(text);
        }
    }
}

/// Counts the folders at `uris` one after the other, starting from
/// `start`, and shows the totals in `rows` as they climb; `cancel` stops
/// the count, as closing the window does. One folder that cannot be read
/// reads Unavailable; among several items, it makes the totals partial.
pub(super) fn count_folders(uris: Vec<String>, start: Totals, rows: &FolderCountRows, cancel: &Cancellation) {
    rows.show(start);
    let rows = rows.clone();
    let cancel = cancel.clone();
    glib::spawn_future_local(async move {
        let mut done = start;
        let is_alone = uris.len() == 1 && start == Totals::zero();
        for uri in uris {
            let (sender, receiver) = async_channel::unbounded::<FolderSize>();
            let progress = move |size: &FolderSize| {
                let _ = sender.try_send(size.clone());
            };
            let shown = rows.clone();
            let base = done;
            let updates = glib::spawn_future_local(async move {
                while let Ok(size) = receiver.recv().await {
                    shown.show(base.plus(&size));
                }
            });
            let result = scan_folder_size_in_background(uri, cancel.clone(), progress).await;
            // The channel closes with the scan; the last total is shown
            // before the result.
            let _ = updates.await;
            if cancel.is_cancelled() {
                return;
            }
            match result {
                Ok(size) if !read_nothing(&size) => done = done.plus(&size),
                _ if is_alone => {
                    rows.show_unavailable();
                    return;
                }
                Ok(size) => {
                    done = Totals {
                        partial: true,
                        ..done.plus(&size)
                    }
                }
                Err(_) => done.partial = true,
            }
            rows.show(done);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: PROP-004
    #[test]
    fn totals_add_up_and_forget_an_unknown_space_on_disk() {
        let size = |bytes, allocated| FolderSize {
            bytes,
            allocated,
            files: 2,
            folders: 1,
            uri: String::new(),
            entries: 3,
            skipped: 0,
            errors: 0,
            status: ox_core::sizes::ScanStatus::Complete,
            elapsed: std::time::Duration::ZERO,
            finished_at: None,
        };
        let totals = Totals::zero()
            .plus(&size(10, Some(4096)))
            .plus(&size(5, Some(4096)));
        assert_eq!(
            totals,
            Totals {
                bytes: 15,
                on_disk: Some(8192),
                files: 4,
                folders: 2,
                partial: false,
            }
        );
        assert_eq!(totals.plus(&size(1, None)).on_disk, None);
    }
}
