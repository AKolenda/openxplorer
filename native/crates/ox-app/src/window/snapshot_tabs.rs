// SPDX-License-Identifier: AGPL-3.0-only
//! Tabs that show a previous version: Browse, the tab marker and the
//! banner (PROP-021, PROP-022).
//!
//! Ports Browse of `renderVersionsPanel`, `snapshotFor`, the snapshot part
//! of `renderTabs` and `renderSnapshotBanner` in `v2.0.0:desktop/ui/app.js`.
//! Browse opens a snapshot folder in a new foreground tab tagged with the
//! snapshot; a tab inside a snapshot, tagged or found by its location,
//! gets the amber edge and "Previous version" badge, and the banner under
//! the command bar describes it while its tab is in front.

use gtk::subclass::prelude::*;
use ox_core::versions::{snapshot_location, SnapshotLocation};

use crate::properties::SnapshotTarget;

use super::session::{TabId, TabPlacement};
use super::tab_strip::TabView;
use super::BrowserWindow;

/// Added to the tooltip of a tab whose Properties dialog is open.
const PROPERTIES_OPEN: &str = crate::i18n::message_id(" · Properties open");

impl BrowserWindow {
    /// Opens a snapshot folder in a new tab, tagged with its snapshot
    /// (Browse).
    pub(super) fn browse_snapshot(&self, target: SnapshotTarget) {
        self.refresh_snapshot_roots();
        if let Err(error) = self.open_tab(&target.uri, TabPlacement::Foreground) {
            self.show_message(&error.to_string());
            return;
        }
        let Some(tab) = self.imp().session.borrow().active_id() else {
            return;
        };
        let snapshot = SnapshotLocation::Snapshot {
            root: target.root,
            name: target.label,
        };
        self.imp()
            .item_dialogs
            .snapshot_tabs
            .borrow_mut()
            .push((tab, snapshot));
        self.render_location();
    }

    /// The snapshot tab `id` shows, if any: the one Browse tagged it with
    /// while it stays inside it, else the one its location is in
    /// (`snapshotFor`).
    fn snapshot_of_tab(&self, id: TabId, uri: &str) -> Option<SnapshotLocation> {
        let tagged = self
            .imp()
            .item_dialogs
            .snapshot_tabs
            .borrow()
            .iter()
            .find(|(tab, snapshot)| *tab == id && is_within(uri, snapshot.root()))
            .map(|(_, snapshot)| snapshot.clone());
        tagged.or_else(|| snapshot_location(uri, &self.imp().locations.borrow().snapshot_roots))
    }

    /// Adds `· Properties open` to the tooltips of tabs with Properties,
    /// and marks the tabs that show a previous version.
    pub(super) fn mark_tabs_with_dialogs_and_snapshots(&self, views: &mut [TabView]) {
        let uris: Vec<(TabId, String)> = {
            let session = self.imp().session.borrow();
            session
                .tabs()
                .iter()
                .map(|tab| (tab.id, tab.uri().to_owned()))
                .collect()
        };
        for view in views.iter_mut() {
            if self.has_properties(view.id) {
                view.tooltip
                    .push_str(ox_core::i18n::gettext_static(PROPERTIES_OPEN));
            }
            let uri = uris
                .iter()
                .find(|(id, _)| *id == view.id)
                .map(|(_, uri)| uri.as_str());
            let snapshot = uri.and_then(|uri| self.snapshot_of_tab(view.id, uri));
            if let Some(snapshot) = snapshot {
                view.tooltip.push_str(" · Previous version · ");
                view.tooltip.push_str(snapshot.label());
                view.previous_version = Some(snapshot.label().to_owned());
            }
        }
    }

    /// Shows the banner when the active tab is inside a snapshot.
    pub(super) fn show_snapshot_banner(&self) {
        let active = self
            .imp()
            .session
            .borrow()
            .active()
            .map(|tab| (tab.id, tab.uri().to_owned()));
        let snapshot = active.and_then(|(id, uri)| self.snapshot_of_tab(id, &uri));
        self.snapshot_banner().show_snapshot(snapshot.as_ref());
    }

    /// Reads the known snapshot collections again, which mark tabs and
    /// the banner.
    pub(super) fn refresh_snapshot_roots(&self) {
        let roots = self.context().previous_versions().snapshot_roots();
        self.imp().locations.borrow_mut().snapshot_roots = roots;
    }
}

/// True when `uri` is `root` or lies below it (`within` in
/// `snapshot-meta.js`).
fn is_within(uri: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    let uri_trimmed = uri.trim_end_matches('/');
    uri_trimmed == root || uri.starts_with(&format!("{root}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: PROP-021
    #[test]
    fn a_tagged_root_covers_its_folder_and_what_is_below_it_only() {
        let root = "smb://nas/share/.snapshot/daily.2026-09-05";

        assert!(is_within(root, root));
        assert!(is_within(&format!("{root}/Projects"), root));
        assert!(!is_within("smb://nas/share/.snapshot/daily.2026-09-050", root));
    }
}
