// SPDX-License-Identifier: AGPL-3.0-only
//! Items as a fresh query reports them, for the opening and terminal tests
//! of the integration service. The Python tests passed dictionaries such
//! as `{'kind': 'file', 'name': 'a.pdf'}`; [`item`] builds the matching
//! [`Entry`], and a test sets any other field with struct update syntax.

use ox_core::entry::{Entry, EntryKind};

/// An item named `name` of `kind` in `/tmp`, a folder exactly when `kind`
/// is [`EntryKind::Directory`], with nothing else known about it.
pub fn item(kind: EntryKind, name: &str) -> Entry {
    Entry {
        uri: format!("file:///tmp/{name}"),
        name: name.to_owned(),
        kind,
        is_dir: kind == EntryKind::Directory,
        is_virtual: false,
        can_operate: true,
        target_uri: None,
        size: None,
        type_label: String::new(),
        content_type: None,
        modified: None,
        is_hidden: false,
        is_symlink: false,
        trash_orig_path: None,
        trash_deletion_date: None,
        can_rename: None,
        can_trash: None,
        can_delete: None,
        can_write: None,
        serialized_icon: None,
        created: None,
        owner: None,
        unix_mode: None,
    }
}
