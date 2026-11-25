// SPDX-License-Identifier: AGPL-3.0-only
//! What a folder's infotip says of its contents, as Windows Explorer's
//! does: its size, and the names of the first folders and files in it
//! ("Folders: Photos, Taxes" and "Files: notes.txt, plan.odt, ..."), or
//! "Empty folder".
//!
//! A tooltip is asked for on the main thread and must answer at once, so
//! the contents are read on a worker thread the first time a folder's
//! tooltip is asked for, and the tooltip is asked for again once they are
//! read ([`summary_or_read`]). Only folders on this computer are read: a
//! tooltip never waits on a share. The size is counted for at most
//! [`SIZE_TIME`]; a folder too large to count in that time shows no size,
//! never a number that is too small. A few folders' summaries are kept,
//! each for [`KEPT_FOR`], so a pointer moving back and forth reads each
//! folder once.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::format;
use ox_core::i18n::format_message;
use ox_core::sizes::{scan_folder_size, ScanStatus};
use ox_core::transfer::Cancellation;

use crate::folder_view::sorting::SortKey;

/// How many names of each kind a summary lists, then "...", as Windows
/// 11 lists seven of eight files.
pub(crate) const NAMES_LISTED: usize = 7;

/// How long a summary's size is counted before it is left out.
const SIZE_TIME: Duration = Duration::from_secs(1);

/// The most items of the folder itself a summary reads for its names.
const MOST_NAMES_READ: usize = 5_000;

/// How long a summary is kept before the folder is read again.
const KEPT_FOR: Duration = Duration::from_secs(10);

/// How many folders' summaries are kept.
const KEPT_FOLDERS: usize = 32;

/// What a folder holds, as its infotip says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FolderSummary {
    /// The bytes of every file inside, when they were all counted in time.
    pub size: Option<u64>,
    /// The first subfolders' names, in name order.
    pub folders: Vec<String>,
    /// Whether it holds more subfolders than are listed.
    pub more_folders: bool,
    /// The first files' names, in name order.
    pub files: Vec<String>,
    /// Whether it holds more files than are listed.
    pub more_files: bool,
}

impl FolderSummary {
    /// The infotip's lines: "Size: ...", then "Folders: ..." and "Files:
    /// ..." where it holds any, or "Empty folder" where it holds nothing.
    pub(crate) fn lines(&self) -> Vec<String> {
        if self.folders.is_empty() && self.files.is_empty() {
            return vec![ox_core::i18n::gettext("Empty folder")];
        }
        let mut lines = Vec::new();
        if let Some(size) = self.size {
            let size = format::short_size(size);
            lines.push(format_message("Size: {size}", &[("size", &size)]));
        }
        if !self.folders.is_empty() {
            let names = listed(&self.folders, self.more_folders);
            lines.push(format_message("Folders: {names}", &[("names", &names)]));
        }
        if !self.files.is_empty() {
            let names = listed(&self.files, self.more_files);
            lines.push(format_message("Files: {names}", &[("names", &names)]));
        }
        lines
    }
}

/// `names` joined with commas, ending in ", ..." when there are `more`.
fn listed(names: &[String], more: bool) -> String {
    let mut text = names.join(", ");
    if more {
        text.push_str(", ...");
    }
    text
}

/// Reads the summary of the local folder at `path`, hidden items left out
/// as the views leave them out by default; `None` when it cannot be read.
/// Blocks: run it off the main thread.
pub(crate) fn read_summary(path: &std::path::Path, uri: &str) -> Option<FolderSummary> {
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .take(MOST_NAMES_READ)
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // A link is listed by what it is, as the views show it.
        let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_folder {
            folders.push(name);
        } else {
            files.push(name);
        }
    }
    let (folders, more_folders) = first_names(folders);
    let (files, more_files) = first_names(files);
    let empty = folders.is_empty() && files.is_empty();
    Some(FolderSummary {
        size: if empty { None } else { size_in_time(uri) },
        folders,
        more_folders,
        files,
        more_files,
    })
}

/// The first [`NAMES_LISTED`] of `names` in natural order, and whether
/// there are more.
fn first_names(names: Vec<String>) -> (Vec<String>, bool) {
    let mut keyed: Vec<(SortKey, String)> = names
        .into_iter()
        .map(|name| (SortKey::new(&name), name))
        .collect();
    keyed.sort_by(|(a, first), (b, second)| a.natural_cmp(b).then_with(|| first.cmp(second)));
    let more = keyed.len() > NAMES_LISTED;
    (
        keyed
            .into_iter()
            .take(NAMES_LISTED)
            .map(|(_, name)| name)
            .collect(),
        more,
    )
}

/// The bytes of every file in the folder at `uri`, when they can all be
/// counted within [`SIZE_TIME`].
fn size_in_time(uri: &str) -> Option<u64> {
    let cancel = Cancellation::new();
    let started = Instant::now();
    let stop = cancel.clone();
    let scanned = scan_folder_size(uri, &cancel, move |_| {
        if started.elapsed() > SIZE_TIME {
            stop.cancel();
        }
    })
    .ok()?;
    (scanned.status == ScanStatus::Complete).then_some(scanned.bytes)
}

/// Where reading a folder's summary is.
#[derive(Debug, Clone)]
enum Reading {
    /// It is being read.
    Running,
    /// It was read: `None` when the folder could not be.
    Done(Option<FolderSummary>),
}

/// One folder's summary, as read or being read.
#[derive(Debug)]
struct Kept {
    uri: String,
    summary: Reading,
    read_at: Instant,
}

thread_local! {
    /// The folders' summaries read lately, newest last.
    static KEPT: RefCell<VecDeque<Kept>> = const { RefCell::new(VecDeque::new()) };
}

/// The summary of the folder at `uri`, if it was read lately; otherwise
/// starts reading it, for a folder on this computer, and asks `cell` for
/// its tooltip again once it is read. `None` until then, and for a folder
/// elsewhere or one that cannot be read.
pub(crate) fn summary_or_read(uri: &str, cell: &gtk::Widget) -> Option<FolderSummary> {
    let known = KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        kept.retain(|folder| folder.read_at.elapsed() < KEPT_FOR);
        kept.iter()
            .find(|folder| folder.uri == uri)
            .map(|folder| folder.summary.clone())
    });
    match known {
        Some(Reading::Done(summary)) => return summary,
        Some(Reading::Running) => return None,
        None => {}
    }
    // A share's folder has a local path too, through GVfs; it is not read.
    let file = gio::File::for_uri(uri);
    if !file.has_uri_scheme("file") {
        return None;
    }
    let path = file.path()?;
    KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.len() >= KEPT_FOLDERS {
            kept.pop_front();
        }
        kept.push_back(Kept {
            uri: uri.to_owned(),
            summary: Reading::Running,
            read_at: Instant::now(),
        });
    });
    let cell = cell.downgrade();
    let uri = uri.to_owned();
    glib::spawn_future_local(async move {
        let reading = uri.clone();
        let summary = gio::spawn_blocking(move || read_summary(&path, &reading))
            .await
            .ok()
            .flatten();
        KEPT.with(|kept| {
            if let Some(folder) = kept.borrow_mut().iter_mut().find(|folder| folder.uri == uri) {
                folder.summary = Reading::Done(summary);
                folder.read_at = Instant::now();
            }
        });
        if let Some(cell) = cell.upgrade() {
            cell.trigger_tooltip_query();
        }
    });
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder's summary names its first folders and files in name
    /// order, says when there are more, and counts its size; an empty one
    /// says so.
    ///
    /// parity: VIEW-001
    #[test]
    fn a_summary_lists_the_first_names_and_the_size() {
        let folder = tempfile::tempdir().expect("a folder");
        for name in ["Taxes", "Photos", ".cache"] {
            std::fs::create_dir(folder.path().join(name)).expect("a subfolder");
        }
        for (index, name) in [
            "b.txt", "a10.txt", "a2.txt", "c.txt", "d.txt", "e.txt", "f.txt", "g.txt", ".hidden",
        ]
        .into_iter()
        .enumerate()
        {
            std::fs::write(folder.path().join(name), vec![b'x'; index + 1]).expect("a file");
        }
        let uri = ox_core::location::file_uri(folder.path());

        let summary = read_summary(folder.path(), &uri).expect("a readable folder");

        assert_eq!(summary.folders, ["Photos", "Taxes"]);
        assert!(!summary.more_folders);
        assert_eq!(
            summary.files,
            ["a2.txt", "a10.txt", "b.txt", "c.txt", "d.txt", "e.txt", "f.txt"]
        );
        assert!(summary.more_files);
        assert_eq!(summary.size, Some((1..=9).sum()));
        assert_eq!(
            summary.lines(),
            [
                format!("Size: {}", format::short_size(45)),
                "Folders: Photos, Taxes".to_owned(),
                "Files: a2.txt, a10.txt, b.txt, c.txt, d.txt, e.txt, f.txt, ...".to_owned(),
            ]
        );

        let empty = tempfile::tempdir().expect("a folder");
        let summary = read_summary(empty.path(), &ox_core::location::file_uri(empty.path()))
            .expect("a readable folder");
        assert_eq!(summary.lines(), ["Empty folder"]);
    }

    /// A tooltip never waits for a folder's contents: the first ask
    /// starts reading them and answers without them, a later ask has
    /// them, and a folder that is not on this computer is never read.
    ///
    /// parity: VIEW-001
    #[gtk::test]
    fn the_summary_is_read_off_the_main_thread_once() {
        let folder = tempfile::tempdir().expect("a folder");
        std::fs::write(folder.path().join("notes.txt"), b"notes").expect("a file");
        let uri = ox_core::location::file_uri(folder.path());
        let cell = gtk::Label::new(None);

        assert_eq!(summary_or_read(&uri, cell.upcast_ref()), None, "not read yet");
        crate::test_support::harness::wait_until("the summary", || {
            summary_or_read(&uri, cell.upcast_ref()).is_some()
        });
        let summary = summary_or_read(&uri, cell.upcast_ref()).expect("kept");
        assert_eq!(summary.files, ["notes.txt"]);
        assert_eq!(
            summary_or_read("smb://nas/share/Projects", cell.upcast_ref()),
            None
        );
    }
}
