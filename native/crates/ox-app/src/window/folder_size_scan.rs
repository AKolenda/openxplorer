// SPDX-License-Identifier: AGPL-3.0-only
//! Measuring folders on request (PROP-026, PROP-027, PROP-029).
//!
//! Ports `scanFolderSizes`, `receiveFolderSize` and `stopSizeScan` of
//! `desktop/ui/app.js`. Calculate folder size measures the selected
//! folders, Calculate folder sizes every folder shown, and Properties the
//! folder it describes. A window runs one scan at a time and measures the
//! folders of a run one after another; the bar at the bottom follows the
//! scan, and Cancel scan stops the running folder and the ones after it.
//! Results last for the window's session and show in the Size column, the
//! details pane and Properties. Opening a folder never measures it.

use std::cell::{Cell, OnceCell, RefCell};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::entry::EntryError;
use ox_core::location::{is_smb_server, split_location, LocationKind};
use ox_core::sizes::{scan_folder_size_in_background, FolderSize, ScanStatus, SizeError};
use ox_core::transfer::Cancellation;

use crate::folder_view::item::FileItem;
use crate::properties::{
    progress_text, size_key, FolderSizeState, FolderSizes, RunEnd, RunPosition, SizeScanStrip,
};

use super::actions::plain_action;
use super::session::TabId;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// Shown when a request names no folder that can be measured.
const NO_FOLDER: &str = "Select a folder or share to calculate its size.";
/// Shown when a scan is asked for while one runs.
const SCAN_RUNNING: &str = "Cancel or finish the current folder-size scan first.";

/// The run in progress.
#[derive(Debug)]
struct SizeRun {
    /// Identifies the run, so progress of an earlier one is dropped.
    number: u64,
    /// Stops the folder being measured.
    cancel: Cancellation,
    /// Set once the user pressed Cancel scan.
    is_cancelled: bool,
    /// Which folder of how many is being measured.
    position: RunPosition,
    /// The folder being measured, as it was asked for; `None` between two
    /// folders. Progress crosses threads, so a report can arrive after its
    /// folder's result, and must not overwrite it.
    measuring: Option<String>,
}

/// Which folder of which run a progress report belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FolderScanId {
    /// The run's number.
    run: u64,
    /// The folder's position in the run.
    index: usize,
}

impl SizeRun {
    /// The folder `scan` names, while this run still measures it.
    fn measuring(&self, scan: FolderScanId) -> Option<&str> {
        let is_current = self.number == scan.run && self.position.index == scan.index;
        self.measuring.as_deref().filter(|_| is_current)
    }
}

/// The window's measured folder sizes and its running scan.
#[derive(Debug, Default)]
pub(super) struct SizeScans {
    /// The bar at the bottom; set by `install_size_scans`.
    strip: OnceCell<SizeScanStrip>,
    /// What every scan of this window found.
    measured: RefCell<FolderSizes>,
    /// The run in progress, if any.
    run: RefCell<Option<SizeRun>>,
    /// The number of the latest run.
    runs: Cell<u64>,
}

impl BrowserWindow {
    fn size_scans(&self) -> &SizeScans {
        &self.imp().size_scans
    }

    /// The bar at the bottom of the window.
    pub(super) fn size_strip(&self) -> &SizeScanStrip {
        self.size_scans()
            .strip
            .get()
            .expect("BrowserWindow::new installs the size-scan bar")
    }

    /// Puts the size-scan bar above the status bar and adds the
    /// folder-size actions.
    pub(super) fn install_size_scans(&self) {
        let strip = SizeScanStrip::default();
        if let Some(column) = self.status_bar().parent().and_downcast::<gtk::Box>() {
            column.insert_child_after(&strip, self.status_bar().prev_sibling().as_ref());
        }
        self.size_scans().strip.set(strip).expect("installed once");
        self.add_action_entries([
            plain_action(WindowAction::CalculateFolderSize, |window| {
                let selected = window.folder_pane().model().selected_items();
                window.calculate_folder_sizes(&selected);
            }),
            plain_action(WindowAction::CalculateFolderSizes, |window| {
                let shown = window.shown_items();
                window.calculate_folder_sizes(&shown);
            }),
            plain_action(WindowAction::CancelSizeScan, BrowserWindow::stop_size_scan),
            gtk::gio::ActionEntry::builder(WindowAction::CalculateFolderSizeOf.name())
                .parameter_type(Some(glib::VariantTy::STRING))
                .activate(|window: &BrowserWindow, _, target| {
                    if let Some(uri) = target.and_then(glib::Variant::str) {
                        let folders = vec![uri.to_owned()];
                        // A pinned share may be unmounted (NET-004).
                        window.after_mounting(uri, move |window| window.start_size_run(folders));
                    }
                })
                .build(),
        ]);
    }

    /// The items the folder view shows, in display order.
    fn shown_items(&self) -> Vec<FileItem> {
        let model = self.folder_pane().model();
        (0..model.n_items())
            .filter_map(|position| model.item(position))
            .collect()
    }

    /// Enables Calculate folder size for a selection with a folder, and
    /// both commands only while no scan runs.
    pub(super) fn update_size_actions(&self) {
        let is_idle = self.size_scans().run.borrow().is_none();
        let selected = self.folder_pane().model().selected_items();
        let has_folder = selected.iter().any(|item| item.entry().is_dir);
        self.set_action_enabled(WindowAction::CalculateFolderSize, is_idle && has_folder);
        self.set_action_enabled(WindowAction::CalculateFolderSizes, is_idle);
    }

    /// Measures the folders among `items`.
    fn calculate_folder_sizes(&self, items: &[FileItem]) {
        let folders = items
            .iter()
            .filter(|item| item.entry().is_dir)
            .map(|item| item.entry().navigation_uri().to_owned())
            .collect();
        self.start_size_run(folders);
    }

    /// Measures `uris` one after another: folders and shares only, each
    /// once, never a whole server (`scanFolderSizes`).
    fn start_size_run(&self, uris: Vec<String>) {
        if self.size_scans().run.borrow().is_some() {
            self.show_message(SCAN_RUNNING);
            return;
        }
        let folders = measurable_folders(uris);
        if folders.is_empty() {
            self.show_message(NO_FOLDER);
            return;
        }
        let number = self.size_scans().runs.get().wrapping_add(1);
        self.size_scans().runs.set(number);
        self.size_scans().run.replace(Some(SizeRun {
            number,
            cancel: Cancellation::new(),
            is_cancelled: false,
            position: RunPosition {
                index: 0,
                total: folders.len(),
            },
            measuring: None,
        }));
        self.size_strip().start();
        self.update_size_actions();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move { window.measure_folders(number, folders).await }
        ));
    }

    /// Measures each folder of run `number` in turn, then shows how the
    /// run ended.
    async fn measure_folders(&self, number: u64, folders: Vec<String>) {
        let mut complete = 0;
        let mut partial = 0;
        for (index, uri) in folders.iter().enumerate() {
            let Some(cancel) = self.begin_folder(index, uri) else {
                break;
            };
            let progress = self.size_progress_sender(FolderScanId { run: number, index });
            let result = scan_folder_size_in_background(uri.clone(), cancel, progress).await;
            match self.finish_folder(uri, result) {
                Some(true) => complete += 1,
                Some(false) => partial += 1,
                None => {}
            }
        }
        let is_cancelled = self.size_scans().run.take().is_some_and(|run| run.is_cancelled);
        let end = if is_cancelled {
            RunEnd::Cancelled
        } else {
            RunEnd::Finished { complete, partial }
        };
        self.size_strip().show_end(end);
        self.update_size_actions();
    }

    /// Marks the folder at `uri` as being measured and returns its
    /// cancellation, or `None` once the run was cancelled.
    fn begin_folder(&self, index: usize, uri: &str) -> Option<Cancellation> {
        let cancel = {
            let mut run = self.size_scans().run.borrow_mut();
            let run = run.as_mut()?;
            if run.is_cancelled {
                return None;
            }
            run.position.index = index;
            run.measuring = Some(uri.to_owned());
            run.cancel.clone()
        };
        let scanning = FolderSizeState::Measured(zero_size(uri));
        self.receive_folder_size(uri, &scanning);
        Some(cancel)
    }

    /// Records how measuring `uri` ended. `Some(true)` for a complete
    /// total, `Some(false)` for a partial or failed one, `None` when the
    /// user cancelled before the folder was read.
    fn finish_folder(&self, uri: &str, result: Result<FolderSize, SizeError>) -> Option<bool> {
        if let Some(run) = self.size_scans().run.borrow_mut().as_mut() {
            run.measuring = None;
        }
        let state = match result {
            Ok(size) => FolderSizeState::Measured(size),
            Err(SizeError::Read(EntryError::Cancelled)) => {
                self.forget_folder_size(uri);
                return None;
            }
            Err(error) => FolderSizeState::Unavailable(error.to_string()),
        };
        let is_complete = state.is_complete();
        self.receive_folder_size(uri, &state);
        Some(is_complete)
    }

    /// Where the worker thread sends the totals while the folder `scan`
    /// names is measured: to this window, on the main thread.
    fn size_progress_sender(&self, scan: FolderScanId) -> impl FnMut(&FolderSize) + Send + 'static {
        let window = glib::SendWeakRef::from(self.downgrade());
        move |size: &FolderSize| {
            let size = size.clone();
            let window = window.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(window) = window.upgrade() {
                    window.receive_progress(scan, &size);
                }
            });
        }
    }

    /// Shows the totals of the folder `scan` names so far, unless its
    /// result, or another folder or run, came first.
    fn receive_progress(&self, scan: FolderScanId, size: &FolderSize) {
        let (uri, text) = {
            let run = self.size_scans().run.borrow();
            let Some(run) = run.as_ref() else {
                return;
            };
            let Some(uri) = run.measuring(scan) else {
                return;
            };
            let name = self.imp().locations.borrow().base_name(uri);
            (
                uri.to_owned(),
                progress_text(run.position, &name, size, run.is_cancelled),
            )
        };
        self.size_strip().show_progress(&text);
        if size.status == ScanStatus::Scanning {
            self.receive_folder_size(&uri, &FolderSizeState::Measured(size.clone()));
        }
    }

    /// Cancel scan: stops the run; Dismiss: hides the finished bar.
    fn stop_size_scan(&self) {
        let mut run = self.size_scans().run.borrow_mut();
        let Some(run) = run.as_mut() else {
            self.size_strip().set_visible(false);
            return;
        };
        run.is_cancelled = true;
        run.cancel.cancel();
        self.size_strip().show_cancelling();
    }

    /// What this window measured for the folder at `uri`.
    pub(super) fn measured_folder_size(&self, uri: &str) -> Option<FolderSizeState> {
        self.size_scans().measured.borrow().get(uri).cloned()
    }

    /// Records `state` for the folder at `uri` and shows it wherever the
    /// folder is: the rows of every tab, the details pane and Properties.
    fn receive_folder_size(&self, uri: &str, state: &FolderSizeState) {
        self.size_scans().measured.borrow_mut().set(uri, state.clone());
        let stores: Vec<gtk::gio::ListStore> = {
            let session = self.imp().session.borrow();
            session.tabs().iter().map(|tab| tab.store.clone()).collect()
        };
        for store in stores {
            update_rows_of(&store, uri, state);
        }
        for view in self.properties_views() {
            view.show_folder_size(uri, state);
        }
        self.update_details_pane();
    }

    /// Forgets the folder at `uri` again, as if it had never been
    /// measured: its scan was cancelled before it read anything.
    fn forget_folder_size(&self, uri: &str) {
        self.size_scans().measured.borrow_mut().remove(uri);
        self.update_details_pane();
    }

    /// Gives the rows of tab `id` the sizes measured this session, after
    /// the tab was listed again.
    pub(super) fn apply_measured_folder_sizes(&self, id: TabId) {
        let Some(store) = self.tab_store(id) else {
            return;
        };
        let measured = self.size_scans().measured.borrow();
        for position in 0..store.n_items() {
            let Some(item) = store.item(position).and_downcast::<FileItem>() else {
                continue;
            };
            let is_unmarked = item.entry().is_dir && item.folder_size().is_none();
            if let Some(state) = measured.get(&item.entry().uri).filter(|_| is_unmarked) {
                item.set_folder_size(state.clone());
            }
        }
    }
}

/// Gives the rows of `store` for the folder at `uri` the new `state`, and
/// has the views draw and sort them again.
fn update_rows_of(store: &gtk::gio::ListStore, uri: &str, state: &FolderSizeState) {
    for position in 0..store.n_items() {
        let Some(item) = store.item(position).and_downcast::<FileItem>() else {
            continue;
        };
        if size_key(item.entry().navigation_uri()) != size_key(uri) {
            continue;
        }
        item.set_folder_size(state.clone());
        store.items_changed(position, 1, 1);
    }
}

/// The folders of `uris` a scan can measure, each once: local folders and
/// SMB shares, never a whole server (`scanFolderSizes`).
fn measurable_folders(uris: Vec<String>) -> Vec<String> {
    let mut folders: Vec<String> = Vec::new();
    for uri in uris {
        let kind = split_location(&uri).map(|parts| parts.kind());
        let is_measurable =
            matches!(kind, Ok(LocationKind::Local | LocationKind::Smb)) && !is_smb_server(&uri);
        let is_new = !folders.iter().any(|known| size_key(known) == size_key(&uri));
        if is_measurable && is_new {
            folders.push(uri);
        }
    }
    folders
}

/// The totals of a scan of `uri` that has not counted anything yet.
fn zero_size(uri: &str) -> FolderSize {
    FolderSize {
        uri: uri.to_owned(),
        bytes: 0,
        files: 0,
        folders: 0,
        entries: 0,
        skipped: 0,
        errors: 0,
        status: ScanStatus::Scanning,
        elapsed: Duration::ZERO,
        finished_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: PROP-026
    #[test]
    fn only_folders_and_shares_are_measured_each_once() {
        let uris = vec![
            "file:///home/demo/Projects".to_owned(),
            "file:///home/demo/Projects/".to_owned(),
            "smb://nas/".to_owned(),
            "smb://nas/media".to_owned(),
            "mtp://phone/DCIM".to_owned(),
        ];

        let folders = measurable_folders(uris);

        assert_eq!(folders, ["file:///home/demo/Projects", "smb://nas/media"]);
    }
}
