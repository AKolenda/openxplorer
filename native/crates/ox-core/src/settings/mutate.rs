// SPDX-License-Identifier: AGPL-3.0-only
//! The changes `Settings` can make, as pure functions on [`SettingsData`].
//!
//! Ports the bodies of `bookmark`, `pin_many` and `remember_open` in
//! `desktop/core.py`. Locking, reloading and saving happen in
//! [`Settings`](super::Settings); these functions validate everything
//! before changing anything, so a rejected request leaves the data as it
//! was.

use super::labels::{bookmark_fallback_label, pin_fallback_label};
use super::model::{Bookmark, RecentEntry, SettingsData};
use super::read::{first_chars, MAX_BOOKMARKS, MAX_NAME_CHARS, MAX_ORDER, MAX_RECENT, MAX_TYPE_CHARS};
use super::SettingsError;
use crate::location::{normalise, require_share, safe_label};

/// Whether [`Settings::bookmark`](super::Settings::bookmark) adds or
/// removes the location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkAction {
    /// Add it, or move it to the end with a new label.
    Add,
    /// Remove it.
    Remove,
}

/// Which list [`Settings::bookmark`](super::Settings::bookmark) changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkKind {
    /// A Quick access pin (any supported location).
    Pin,
    /// A mapped network share (an SMB shared folder).
    Share,
}

/// A folder dragged onto Quick access. An empty label is replaced by the
/// folder name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PinRequest {
    /// The folder location, in any form `normalise_location` accepts.
    pub uri: String,
    /// The sidebar label, or empty for the folder name.
    pub label: String,
}

impl PinRequest {
    /// A pin request for `uri` with the given label.
    pub fn new(uri: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            label: label.into(),
        }
    }
}

/// Adds or removes a pin or share. Removing a pin also hides it (so a
/// known folder can be unpinned) and drops it from the Quick access
/// order; adding a pin un-hides it.
pub(super) fn apply_bookmark(
    data: &mut SettingsData,
    action: BookmarkAction,
    kind: BookmarkKind,
    uri: &str,
    label: &str,
) -> Result<(), SettingsError> {
    let uri = match kind {
        BookmarkKind::Share => require_share(uri)?,
        BookmarkKind::Pin => normalise(uri)?,
    };
    let added = match action {
        BookmarkAction::Add => Some(Bookmark {
            label: safe_label(label, &bookmark_fallback_label(&uri))?,
            uri: uri.clone(),
        }),
        BookmarkAction::Remove => None,
    };
    let list = match kind {
        BookmarkKind::Share => &mut data.shares,
        BookmarkKind::Pin => &mut data.pins,
    };
    list.retain(|bookmark| bookmark.uri != uri);
    list.extend(added);
    if kind == BookmarkKind::Pin {
        show_or_hide_in_quick_access(data, action, &uri);
    }
    Ok(())
}

/// Adding a pin shows it in Quick access again; removing one hides it,
/// which also unpins a standard folder, and forgets its place in the order.
fn show_or_hide_in_quick_access(data: &mut SettingsData, action: BookmarkAction, uri: &str) {
    match action {
        BookmarkAction::Add => data.hidden_quick.retain(|hidden| hidden != uri),
        BookmarkAction::Remove => {
            if !data.hidden_quick.iter().any(|hidden| hidden == uri) {
                data.hidden_quick.push(uri.to_owned());
            }
            data.quick_order.retain(|ordered| ordered != uri);
        }
    }
}

/// Adds or reorders Quick access pins and returns the cleaned batch.
///
/// The whole batch is validated first. `before` is the entry the folders
/// were dropped on (they are inserted before it, or at the end); a drop on
/// one of the dragged folders keeps its place. `quick_order` is the order
/// the sidebar showed, used as the base order when given. Performs no file
/// I/O: callers verify new folders exist before pinning them.
pub(super) fn pin_many(
    data: &mut SettingsData,
    items: &[PinRequest],
    before: Option<&str>,
    quick_order: Option<&[String]>,
) -> Result<Vec<Bookmark>, SettingsError> {
    if items.is_empty() || items.len() > MAX_BOOKMARKS {
        return Err(SettingsError::invalid(
            "Drag between 1 and 200 folders at a time.",
        ));
    }
    let dragged = clean_pins(items)?;
    let before = before.map(normalise).transpose()?;
    let shown_order = clean_order(quick_order.unwrap_or_default())?;
    let pins = merge_pins(&data.pins, &dragged)?;
    let mut order = if shown_order.is_empty() {
        saved_then_pinned(&data.quick_order, &pins)
    } else {
        shown_order
    };
    place_dragged(&mut order, &dragged, before.as_deref());
    data.pins = pins;
    data.quick_order = order;
    data.hidden_quick.retain(|uri| !is_dragged(&dragged, uri));
    Ok(dragged)
}

/// The sidebar order as shown, normalised and without duplicates.
fn clean_order(shown: &[String]) -> Result<Vec<String>, SettingsError> {
    if shown.len() > MAX_ORDER {
        return Err(SettingsError::invalid("Invalid sidebar order."));
    }
    let mut order = Vec::with_capacity(shown.len());
    for uri in shown {
        push_unique(&mut order, normalise(uri)?);
    }
    Ok(order)
}

/// The saved order followed by every pin missing from it, as Python's
/// `dict.fromkeys(quickOrder + pins)`.
fn saved_then_pinned(saved_order: &[String], pins: &[Bookmark]) -> Vec<String> {
    let mut order = Vec::new();
    let pinned = pins.iter().map(|pin| &pin.uri);
    for uri in saved_order.iter().chain(pinned) {
        push_unique(&mut order, uri.clone());
    }
    order
}

/// The saved pins with `dragged` added at the end or relabelled in place.
fn merge_pins(saved: &[Bookmark], dragged: &[Bookmark]) -> Result<Vec<Bookmark>, SettingsError> {
    let mut pins = saved.to_vec();
    for pin in dragged {
        match pins.iter_mut().find(|existing| existing.uri == pin.uri) {
            Some(existing) => existing.label.clone_from(&pin.label),
            None => pins.push(pin.clone()),
        }
    }
    if pins.len() > MAX_BOOKMARKS {
        return Err(SettingsError::invalid(
            "Quick access supports up to 200 custom pins.",
        ));
    }
    Ok(pins)
}

/// Moves the dragged pins in front of `before` (or to the end). A drop on
/// one of the dragged pins keeps the order, only appending new pins.
fn place_dragged(order: &mut Vec<String>, dragged: &[Bookmark], before: Option<&str>) {
    let dropped_on_itself = before.is_some_and(|before| dragged.iter().any(|pin| pin.uri == before));
    if dropped_on_itself {
        for pin in dragged {
            push_unique(order, pin.uri.clone());
        }
        return;
    }
    order.retain(|uri| !is_dragged(dragged, uri));
    let index = before
        .and_then(|before| order.iter().position(|uri| uri == before))
        .unwrap_or(order.len());
    let dragged_uris = dragged.iter().map(|pin| pin.uri.clone());
    order.splice(index..index, dragged_uris);
}

/// True if `uri` is one of the dragged pins.
fn is_dragged(dragged: &[Bookmark], uri: &str) -> bool {
    dragged.iter().any(|pin| pin.uri == uri)
}

/// Puts `entry` first in the recent files, removing an older entry for the
/// same file and keeping at most 30. The entry is stored as it will read
/// back: canonical URI, bounded name and type, never a folder.
pub(super) fn remember_open(data: &mut SettingsData, entry: &RecentEntry) -> Result<(), SettingsError> {
    let clean = RecentEntry {
        uri: normalise(&entry.uri)?,
        name: first_chars(&entry.name, MAX_NAME_CHARS),
        type_name: first_chars(&entry.type_name, MAX_TYPE_CHARS),
        is_dir: false,
        size: entry.size,
        modified: entry.modified,
    };
    data.recent.retain(|recent| recent.uri != clean.uri);
    data.recent.insert(0, clean);
    data.recent.truncate(MAX_RECENT);
    Ok(())
}

/// Normalises and labels a batch, keeping the first of any duplicates.
fn clean_pins(items: &[PinRequest]) -> Result<Vec<Bookmark>, SettingsError> {
    let mut clean: Vec<Bookmark> = Vec::with_capacity(items.len());
    for item in items {
        let uri = normalise(&item.uri)?;
        let label = safe_label(&item.label, &pin_fallback_label(&uri))?;
        if clean.iter().all(|pin| pin.uri != uri) {
            clean.push(Bookmark { uri, label });
        }
    }
    Ok(clean)
}

/// Appends `uri` unless it is already present.
fn push_unique(order: &mut Vec<String>, uri: String) {
    if !order.contains(&uri) {
        order.push(uri);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUICK: [&str; 3] = [
        "file:///home/test/Desktop",
        "file:///home/test/Downloads",
        "file:///home/test/Documents",
    ];

    fn quick() -> Vec<String> {
        QUICK.map(String::from).to_vec()
    }

    fn pin(uri: &str) -> PinRequest {
        PinRequest::new(uri, "")
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_insert_before_documents`
    /// parity: SIDE-007, SIDE-008
    #[test]
    fn insert_before_documents() {
        let mut data = SettingsData::default();
        let items = [PinRequest::new("smb://nas/work", "work")];
        pin_many(&mut data, &items, Some(QUICK[2]), Some(&quick())).unwrap();
        assert_eq!(data.quick_order, [QUICK[0], QUICK[1], "smb://nas/work", QUICK[2]]);
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_duplicate_batch_dedupes_canonical_uri`
    /// parity: SIDE-007
    #[test]
    fn duplicate_batch_dedupes_canonical_uri() {
        let mut data = SettingsData::default();
        pin_many(
            &mut data,
            &[pin("smb://NAS/work/"), pin("smb://nas/work")],
            None,
            None,
        )
        .unwrap();
        assert_eq!(data.pins.len(), 1);
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_repeat_drag_does_not_duplicate`
    /// parity: SIDE-007
    #[test]
    fn repeat_drag_does_not_duplicate() {
        let mut data = SettingsData::default();
        pin_many(&mut data, &[pin("smb://nas/work")], None, Some(&quick())).unwrap();
        let order = data.quick_order.clone();
        pin_many(&mut data, &[pin("smb://nas/work")], None, Some(&order)).unwrap();
        assert_eq!(data.pins.len(), 1);
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_reorder_offline_pin_without_querying_nas`
    /// parity: SIDE-008
    #[test]
    fn reorder_offline_pin_without_querying_nas() {
        let mut data = SettingsData::default();
        pin_many(&mut data, &[pin("smb://offline/work")], None, Some(&quick())).unwrap();
        let order = data.quick_order.clone();
        pin_many(
            &mut data,
            &[pin("smb://offline/work")],
            Some(QUICK[0]),
            Some(&order),
        )
        .unwrap();
        assert_eq!(data.quick_order[0], "smb://offline/work");
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_drop_on_self_keeps_order`
    /// parity: SIDE-008
    #[test]
    fn drop_on_self_keeps_order() {
        let mut data = SettingsData::default();
        pin_many(&mut data, &[pin("smb://nas/work")], None, Some(&quick())).unwrap();
        let before = data.quick_order.clone();
        pin_many(
            &mut data,
            &[pin("smb://nas/work")],
            Some("smb://nas/work"),
            Some(&before),
        )
        .unwrap();
        assert_eq!(data.quick_order, before);
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_unpin_removes_order_only_not_share`
    /// parity: SIDE-009
    #[test]
    fn unpin_removes_order_only_not_share() {
        let mut data = SettingsData::default();
        apply_bookmark(
            &mut data,
            BookmarkAction::Add,
            BookmarkKind::Share,
            "smb://nas/work",
            "Drive",
        )
        .unwrap();
        pin_many(&mut data, &[pin("smb://nas/work")], None, Some(&quick())).unwrap();
        apply_bookmark(
            &mut data,
            BookmarkAction::Remove,
            BookmarkKind::Pin,
            "smb://nas/work",
            "",
        )
        .unwrap();
        assert!(!data.quick_order.contains(&"smb://nas/work".to_owned()));
        assert_eq!(data.shares.len(), 1);
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_invalid_batch_has_no_partial_writes`
    /// parity: SIDE-007
    #[test]
    fn invalid_batch_has_no_partial_writes() {
        let mut data = SettingsData::default();
        let items = [pin("smb://nas/work"), pin("smb://u:secret@nas/work")];
        assert!(pin_many(&mut data, &items, None, None).is_err());
        assert_eq!(data, SettingsData::default());
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_bad_label_does_not_mutate`
    /// parity: SIDE-007
    #[test]
    fn bad_label_does_not_mutate() {
        let mut data = SettingsData::default();
        let items = [PinRequest::new("smb://nas/work", "bad\nlabel")];
        assert!(pin_many(&mut data, &items, None, None).is_err());
        assert_eq!(data, SettingsData::default());
    }

    /// Ported from `desktop/tests/test_pins.py::PinTests::test_pin_cap`
    /// parity: SIDE-007
    #[test]
    fn pin_cap() {
        let mut data = SettingsData::default();
        let items: Vec<PinRequest> = (0..201).map(|i| pin(&format!("smb://nas/s/{i}"))).collect();
        assert!(pin_many(&mut data, &items, None, None).is_err());
        let merged: Vec<PinRequest> = (0..200).map(|i| pin(&format!("smb://nas/s/{i}"))).collect();
        pin_many(&mut data, &merged, None, None).unwrap();
        let error = pin_many(&mut data, &[pin("smb://nas/s/extra")], None, None).unwrap_err();
        assert_eq!(error.to_string(), "Quick access supports up to 200 custom pins.");
    }

    /// Ported from `desktop/tests/test_core.py::CoreTests::test_hidden_builtin_pin`
    /// parity: SIDE-009
    #[test]
    fn hidden_builtin_pin() {
        let mut data = SettingsData::default();
        let desktop = "file:///home/a/Desktop";
        apply_bookmark(&mut data, BookmarkAction::Remove, BookmarkKind::Pin, desktop, "").unwrap();
        assert!(data.hidden_quick.contains(&desktop.to_owned()));
        apply_bookmark(
            &mut data,
            BookmarkAction::Add,
            BookmarkKind::Pin,
            desktop,
            "Desktop",
        )
        .unwrap();
        assert!(!data.hidden_quick.contains(&desktop.to_owned()));
    }

    /// Ported from `desktop/tests/test_core.py::CoreTests::test_credential_bookmark_rejected`
    /// parity: SAFE-010, NET-017
    #[test]
    fn credential_bookmark_rejected() {
        let mut data = SettingsData::default();
        let result = apply_bookmark(
            &mut data,
            BookmarkAction::Add,
            BookmarkKind::Share,
            "smb://u:secret@nas/share",
            "",
        );
        assert!(result.is_err());
        assert_eq!(data, SettingsData::default());
    }

    /// parity: SIDE-007
    #[test]
    fn pin_labels_fall_back_to_the_folder_or_host() {
        let mut data = SettingsData::default();
        let clean = pin_many(
            &mut data,
            &[pin("smb://nas/"), pin("/tmp/Work%20Files/")],
            None,
            None,
        )
        .unwrap();
        let labels: Vec<&str> = clean.iter().map(|pin| pin.label.as_str()).collect();
        assert_eq!(labels, ["nas", "Work%20Files"]);
    }

    /// parity: NET-017
    #[test]
    fn re_adding_a_bookmark_moves_it_last_with_the_new_label() {
        let mut data = SettingsData::default();
        for (uri, label) in [("smb://nas/a", "A"), ("smb://nas/b", ""), ("smb://nas/a", "New")] {
            apply_bookmark(&mut data, BookmarkAction::Add, BookmarkKind::Share, uri, label).unwrap();
        }
        let shares: Vec<(&str, &str)> = data
            .shares
            .iter()
            .map(|share| (share.uri.as_str(), share.label.as_str()))
            .collect();
        assert_eq!(shares, [("smb://nas/b", "b"), ("smb://nas/a", "New")]);
    }

    /// parity: HOME-011
    #[test]
    fn remember_open_keeps_thirty_newest_unique_files() {
        let mut data = SettingsData::default();
        for i in 0..35 {
            let entry = RecentEntry {
                uri: format!("/tmp/file{i}.txt"),
                name: format!("file{i}.txt"),
                type_name: "Text".into(),
                is_dir: true,
                size: i,
                modified: 1,
            };
            remember_open(&mut data, &entry).unwrap();
        }
        let again = RecentEntry {
            uri: "file:///tmp/file20.txt".into(),
            ..data.recent[14].clone()
        };
        remember_open(&mut data, &again).unwrap();
        assert_eq!(data.recent.len(), MAX_RECENT);
        assert_eq!(data.recent[0].uri, "file:///tmp/file20.txt");
        assert_eq!(data.recent[1].uri, "file:///tmp/file34.txt");
        assert!(data.recent.iter().all(|entry| !entry.is_dir));
        let copies = data
            .recent
            .iter()
            .filter(|entry| entry.uri.ends_with("file20.txt"))
            .count();
        assert_eq!(copies, 1);
    }
}
