// SPDX-License-Identifier: AGPL-3.0-only
//! The changes `Settings` can make, as pure functions on [`SettingsData`].
//!
//! Ports the bodies of `bookmark`, `pin_many` and `remember_open` in
//! `v2.0.0:desktop/core.py`. Locking, reloading and saving happen in
//! [`Settings`](super::Settings). Safety rule "validate before changing"
//! (`pin_many` in core.py): each function checks its whole request before
//! it changes anything, so a rejected request leaves the settings as they
//! were.

use super::labels::{bookmark_fallback_label, pin_fallback_label};
use super::model::{Bookmark, RecentEntry, SettingsData, MAX_BOOKMARKS, MAX_ORDER, MAX_RECENT};
use super::stored_location::{normalise, require_share};
use super::SettingsError;
use crate::location::safe_label;

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

/// A location the user asked to pin or map, with its label, before
/// validation: a folder dragged onto Quick access, or a share connected
/// as a drive. An empty label is replaced by the folder or host name.
///
/// Unlike [`Bookmark`], which holds a checked, canonical location, this is
/// what the user supplied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookmarkRequest {
    /// The location, in any form [`normalise`] accepts.
    pub uri: String,
    /// The sidebar label, or empty for the folder or host name.
    pub label: String,
}

impl BookmarkRequest {
    /// A request for `uri` with the given label.
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
///
/// # Errors
///
/// [`SettingsError::Location`] for a location or label the location rules
/// refuse; nothing is changed then.
pub(super) fn apply_bookmark(
    settings: &mut SettingsData,
    action: BookmarkAction,
    kind: BookmarkKind,
    request: &BookmarkRequest,
) -> Result<(), SettingsError> {
    let uri = match kind {
        BookmarkKind::Share => require_share(&request.uri)?,
        BookmarkKind::Pin => normalise(&request.uri)?,
    };
    let added = match action {
        BookmarkAction::Add => Some(Bookmark {
            label: safe_label(&request.label, &bookmark_fallback_label(&uri))?,
            uri: uri.clone(),
        }),
        BookmarkAction::Remove => None,
    };
    // Safety rule "validate before changing": every fallible step is above
    // this line; nothing below can fail.
    let bookmarks = match kind {
        BookmarkKind::Share => &mut settings.shares,
        BookmarkKind::Pin => &mut settings.pins,
    };
    bookmarks.retain(|bookmark| bookmark.uri != uri);
    bookmarks.extend(added);
    if kind == BookmarkKind::Pin {
        show_or_hide_in_quick_access(settings, action, &uri);
    }
    Ok(())
}

/// Shows the standard folder `uri` in Quick access again after it was
/// hidden, without pinning it (SIDE-010).
pub(super) fn show_in_quick_access(settings: &mut SettingsData, uri: &str) -> Result<(), SettingsError> {
    let uri = normalise(uri)?;
    settings.hidden_quick.retain(|hidden| *hidden != uri);
    Ok(())
}

/// Adding a pin shows it in Quick access again; removing one hides it,
/// which also unpins a standard folder, and forgets its place in the order.
fn show_or_hide_in_quick_access(settings: &mut SettingsData, action: BookmarkAction, uri: &str) {
    match action {
        BookmarkAction::Add => settings.hidden_quick.retain(|hidden| hidden != uri),
        BookmarkAction::Remove => {
            if !settings.hidden_quick.iter().any(|hidden| hidden == uri) {
                settings.hidden_quick.push(uri.to_owned());
            }
            settings.quick_order.retain(|ordered| ordered != uri);
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
///
/// # Errors
///
/// [`SettingsError::Invalid`] for an empty or oversized batch or order,
/// and [`SettingsError::Location`] for an invalid location or label;
/// nothing is changed then.
pub(super) fn pin_many(
    settings: &mut SettingsData,
    items: &[BookmarkRequest],
    before: Option<&str>,
    quick_order: Option<&[String]>,
) -> Result<Vec<Bookmark>, SettingsError> {
    if items.is_empty() || items.len() > MAX_BOOKMARKS {
        return Err(SettingsError::invalid(format!(
            "Drag between 1 and {MAX_BOOKMARKS} folders at a time."
        )));
    }
    let dragged = clean_pins(items)?;
    let before = before.map(normalise).transpose()?;
    let shown_order = clean_order(quick_order.unwrap_or_default())?;
    let pins = merge_pins(&settings.pins, &dragged)?;
    let mut order = if shown_order.is_empty() {
        saved_then_pinned(&settings.quick_order, &pins)
    } else {
        shown_order
    };
    place_dragged(&mut order, &dragged, before.as_deref());
    // Safety rule "validate before changing": every fallible step is above
    // this line; nothing below can fail.
    settings.pins = pins;
    settings.quick_order = order;
    settings.hidden_quick.retain(|uri| !contains_uri(&dragged, uri));
    Ok(dragged)
}

/// Normalises and labels a batch, keeping the first of any duplicates.
fn clean_pins(items: &[BookmarkRequest]) -> Result<Vec<Bookmark>, SettingsError> {
    let mut clean: Vec<Bookmark> = Vec::with_capacity(items.len());
    for item in items {
        let uri = normalise(&item.uri)?;
        let label = safe_label(&item.label, &pin_fallback_label(&uri))?;
        if !contains_uri(&clean, &uri) {
            clean.push(Bookmark { uri, label });
        }
    }
    Ok(clean)
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
        return Err(SettingsError::invalid(format!(
            "Quick access supports up to {MAX_BOOKMARKS} custom pins."
        )));
    }
    Ok(pins)
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

/// Moves the dragged pins in front of `before` (or to the end). A drop on
/// one of the dragged pins keeps the order, only appending new pins.
fn place_dragged(order: &mut Vec<String>, dragged: &[Bookmark], before: Option<&str>) {
    let is_dropped_on_itself = before.is_some_and(|before| contains_uri(dragged, before));
    if is_dropped_on_itself {
        for pin in dragged {
            push_unique(order, pin.uri.clone());
        }
        return;
    }
    order.retain(|uri| !contains_uri(dragged, uri));
    let index = before
        .and_then(|before| order.iter().position(|uri| uri == before))
        .unwrap_or(order.len());
    let dragged_uris = dragged.iter().map(|pin| pin.uri.clone());
    order.splice(index..index, dragged_uris);
}

/// True if one of `pins` is at `uri`.
fn contains_uri(pins: &[Bookmark], uri: &str) -> bool {
    pins.iter().any(|pin| pin.uri == uri)
}

/// Appends `uri` unless it is already present.
fn push_unique(order: &mut Vec<String>, uri: String) {
    if !order.contains(&uri) {
        order.push(uri);
    }
}

/// Puts `entry` first in the recent files, removing an older entry for the
/// same file and keeping at most 30. The entry is stored as it will read
/// back: canonical URI, bounded name and type, never a folder.
///
/// # Errors
///
/// [`SettingsError::Location`] for an invalid file location; nothing is
/// changed then.
pub(super) fn remember_open(settings: &mut SettingsData, entry: RecentEntry) -> Result<(), SettingsError> {
    let opened = RecentEntry {
        uri: normalise(&entry.uri)?,
        ..entry
    };
    let stored = opened.into_stored();
    // Safety rule "validate before changing": nothing below can fail.
    settings.recent.retain(|recent| recent.uri != stored.uri);
    settings.recent.insert(0, stored);
    settings.recent.truncate(MAX_RECENT);
    Ok(())
}

/// Forgets the recent files opened before `opened_before`, and those
/// with no time of opening; `None` forgets them all (SAFE-022). True when
/// any were forgotten.
pub(super) fn forget_recent(settings: &mut SettingsData, opened_before: Option<u64>) -> bool {
    let before = settings.recent.len();
    settings.recent.retain(|recent| recent.is_kept_by(opened_before));
    settings.recent.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::test_support::{pin, shown_quick_order, DESKTOP, DOCUMENTS, DOWNLOADS};

    /// `count` distinct share folders, `smb://nas/s/0` onwards.
    fn numbered_pins(count: usize) -> Vec<BookmarkRequest> {
        (0..count).map(|i| pin(&format!("smb://nas/s/{i}"))).collect()
    }

    /// Adds `uri` as a bookmark of `kind`; the request must be valid.
    fn add(settings: &mut SettingsData, kind: BookmarkKind, uri: &str, label: &str) {
        let request = BookmarkRequest::new(uri, label);
        apply_bookmark(settings, BookmarkAction::Add, kind, &request).expect("a valid bookmark");
    }

    /// Unpins `uri`; the location must be valid.
    fn unpin(settings: &mut SettingsData, uri: &str) {
        apply_bookmark(settings, BookmarkAction::Remove, BookmarkKind::Pin, &pin(uri))
            .expect("a valid location");
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_insert_before_documents`
    /// parity: SIDE-007, SIDE-008
    #[test]
    fn pins_dropped_on_a_folder_are_inserted_before_it() {
        let mut settings = SettingsData::default();
        let items = [BookmarkRequest::new("smb://nas/work", "work")];
        pin_many(&mut settings, &items, Some(DOCUMENTS), Some(&shown_quick_order())).unwrap();
        assert_eq!(
            settings.quick_order,
            [DESKTOP, DOWNLOADS, "smb://nas/work", DOCUMENTS]
        );
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_duplicate_batch_dedupes_canonical_uri`
    /// parity: SIDE-007
    #[test]
    fn duplicate_batch_dedupes_canonical_uri() {
        let mut settings = SettingsData::default();
        let items = [pin("smb://NAS/work/"), pin("smb://nas/work")];
        pin_many(&mut settings, &items, None, None).unwrap();
        assert_eq!(settings.pins.len(), 1);
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_repeat_drag_does_not_duplicate`
    /// parity: SIDE-007
    #[test]
    fn repeat_drag_does_not_duplicate() {
        let mut settings = SettingsData::default();
        let items = [pin("smb://nas/work")];
        pin_many(&mut settings, &items, None, Some(&shown_quick_order())).unwrap();
        let order = settings.quick_order.clone();
        pin_many(&mut settings, &items, None, Some(&order)).unwrap();
        assert_eq!(settings.pins.len(), 1);
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_reorder_offline_pin_without_querying_nas`
    /// parity: SIDE-008
    #[test]
    fn reorder_offline_pin_without_querying_nas() {
        let mut settings = SettingsData::default();
        let items = [pin("smb://offline/work")];
        pin_many(&mut settings, &items, None, Some(&shown_quick_order())).unwrap();
        let order = settings.quick_order.clone();
        pin_many(&mut settings, &items, Some(DESKTOP), Some(&order)).unwrap();
        assert_eq!(settings.quick_order[0], "smb://offline/work");
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_drop_on_self_keeps_order`
    /// parity: SIDE-008
    #[test]
    fn drop_on_self_keeps_order() {
        let mut settings = SettingsData::default();
        let items = [pin("smb://nas/work")];
        pin_many(&mut settings, &items, None, Some(&shown_quick_order())).unwrap();
        let before = settings.quick_order.clone();
        pin_many(&mut settings, &items, Some("smb://nas/work"), Some(&before)).unwrap();
        assert_eq!(settings.quick_order, before);
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_unpin_removes_order_only_not_share`
    /// parity: SIDE-009
    #[test]
    fn unpin_removes_order_only_not_share() {
        let mut settings = SettingsData::default();
        let work = "smb://nas/work";
        add(&mut settings, BookmarkKind::Share, work, "Drive");
        pin_many(&mut settings, &[pin(work)], None, Some(&shown_quick_order())).unwrap();

        unpin(&mut settings, work);

        assert!(!settings.quick_order.contains(&work.to_owned()));
        assert_eq!(settings.shares.len(), 1);
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_invalid_batch_has_no_partial_writes`
    /// parity: SIDE-007
    #[test]
    fn invalid_batch_has_no_partial_writes() {
        let mut settings = SettingsData::default();
        let items = [pin("smb://nas/work"), pin("smb://u:secret@nas/work")];
        assert!(pin_many(&mut settings, &items, None, None).is_err());
        assert_eq!(settings, SettingsData::default());
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_bad_label_does_not_mutate`
    /// parity: SIDE-007
    #[test]
    fn bad_label_does_not_mutate() {
        let mut settings = SettingsData::default();
        let items = [BookmarkRequest::new("smb://nas/work", "bad\nlabel")];
        assert!(pin_many(&mut settings, &items, None, None).is_err());
        assert_eq!(settings, SettingsData::default());
    }

    /// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_pin_cap`
    /// parity: SIDE-007
    #[test]
    fn pins_are_capped_per_batch_and_in_total() {
        let mut settings = SettingsData::default();
        assert!(pin_many(&mut settings, &numbered_pins(201), None, None).is_err());
        pin_many(&mut settings, &numbered_pins(200), None, None).unwrap();

        let error = pin_many(&mut settings, &[pin("smb://nas/s/extra")], None, None).unwrap_err();

        assert_eq!(error.to_string(), "Quick access supports up to 200 custom pins.");
    }

    /// Ported from `v2.0.0:desktop/tests/test_core.py::CoreTests::test_hidden_builtin_pin`
    /// parity: SIDE-009
    #[test]
    fn unpinning_a_standard_folder_hides_it_until_it_is_pinned_again() {
        let mut settings = SettingsData::default();
        let desktop = "file:///home/a/Desktop";
        unpin(&mut settings, desktop);
        assert!(settings.hidden_quick.contains(&desktop.to_owned()));

        add(&mut settings, BookmarkKind::Pin, desktop, "Desktop");
        assert!(!settings.hidden_quick.contains(&desktop.to_owned()));
    }

    /// Ported from `v2.0.0:desktop/tests/test_core.py::CoreTests::test_credential_bookmark_rejected`
    /// parity: SAFE-010, NET-017
    #[test]
    fn a_bookmark_with_credentials_is_refused_and_leaves_the_settings_unchanged() {
        let mut settings = SettingsData::default();
        let with_password = pin("smb://u:secret@nas/share");
        let result = apply_bookmark(
            &mut settings,
            BookmarkAction::Add,
            BookmarkKind::Share,
            &with_password,
        );
        assert!(matches!(result, Err(SettingsError::Location(_))));
        assert_eq!(settings, SettingsData::default());
    }

    /// parity: SIDE-007
    #[test]
    fn pin_labels_fall_back_to_the_folder_or_host() {
        let mut settings = SettingsData::default();
        let items = [pin("smb://nas/"), pin("/tmp/Work%20Files/")];
        let clean = pin_many(&mut settings, &items, None, None).unwrap();
        let labels: Vec<&str> = clean.iter().map(|pin| pin.label.as_str()).collect();
        assert_eq!(labels, ["nas", "Work%20Files"]);
    }

    /// parity: NET-017
    #[test]
    fn re_adding_a_bookmark_moves_it_last_with_the_new_label() {
        let mut settings = SettingsData::default();
        add(&mut settings, BookmarkKind::Share, "smb://nas/a", "A");
        add(&mut settings, BookmarkKind::Share, "smb://nas/b", "");
        add(&mut settings, BookmarkKind::Share, "smb://nas/a", "New");
        let shares: Vec<(&str, &str)> = settings
            .shares
            .iter()
            .map(|share| (share.uri.as_str(), share.label.as_str()))
            .collect();
        assert_eq!(shares, [("smb://nas/b", "b"), ("smb://nas/a", "New")]);
    }

    /// parity: HOME-011
    #[test]
    fn remember_open_keeps_thirty_newest_unique_files() {
        let mut settings = SettingsData::default();
        for i in 0..35 {
            let entry = RecentEntry {
                uri: format!("/tmp/file{i}.txt"),
                name: format!("file{i}.txt"),
                type_label: "Text".into(),
                is_dir: true,
                size: i,
                modified: 1,
                opened: None,
            };
            remember_open(&mut settings, entry).unwrap();
        }
        let older_copy = settings
            .recent
            .iter()
            .find(|entry| entry.uri.ends_with("/file20.txt"))
            .expect("file20 is among the 30 newest")
            .clone();
        let again = RecentEntry {
            uri: "file:///tmp/file20.txt".into(),
            ..older_copy
        };

        remember_open(&mut settings, again).unwrap();

        assert_eq!(settings.recent.len(), MAX_RECENT);
        assert_eq!(settings.recent[0].uri, "file:///tmp/file20.txt");
        assert_eq!(settings.recent[1].uri, "file:///tmp/file34.txt");
        assert!(settings.recent.iter().all(|entry| !entry.is_dir));
        let copies = settings
            .recent
            .iter()
            .filter(|entry| entry.uri.ends_with("file20.txt"))
            .count();
        assert_eq!(copies, 1);
    }
}
