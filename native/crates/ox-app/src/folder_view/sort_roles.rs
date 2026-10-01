// SPDX-License-Identifier: AGPL-3.0-only
//! The sort keys beyond the details columns, and the one name the Sort menu
//! gives every key.
//!
//! Dolphin sorts by any of its item roles (`sort_by_*` in
//! `dolphinviewactionhandler.cpp`), shown as a column or not. The details
//! view sorts by its column titles ([`SortColumn`]); the further keys here
//! ([`SortRole`]) sort the folder model directly while the titles show no
//! arrow, as Dolphin's header does for a role it does not show.

use std::cmp::Ordering;

use crate::folder_view::item::FileItem;
use crate::folder_view::sorting::{SortColumn, SortDirection, SortKey};

/// A sort key that is not a details column (VIEW-019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortRole {
    /// When the item was created.
    Created,
    /// When the item was last opened.
    Accessed,
    /// The name's extension, `pdf` in `report.pdf`.
    Extension,
    /// The permission bits.
    Permissions,
    /// The owner's user name.
    Owner,
    /// The owning group.
    Group,
    /// Where a symbolic link points.
    LinkTarget,
}

impl SortRole {
    /// Every role, in the order the Sort menu lists them.
    pub(crate) const ALL: [SortRole; 7] = [
        SortRole::Created,
        SortRole::Accessed,
        SortRole::Extension,
        SortRole::Permissions,
        SortRole::Owner,
        SortRole::Group,
        SortRole::LinkTarget,
    ];

    /// Key used in action targets and saved view styles.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            SortRole::Created => "created",
            SortRole::Accessed => "accessed",
            SortRole::Extension => "extension",
            SortRole::Permissions => "permissions",
            SortRole::Owner => "owner",
            SortRole::Group => "group",
            SortRole::LinkTarget => "linkTarget",
        }
    }

    /// The Sort menu's label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            SortRole::Created => "Date created",
            SortRole::Accessed => "Date accessed",
            SortRole::Extension => "File extension",
            SortRole::Permissions => "Permissions",
            SortRole::Owner => "Owner",
            SortRole::Group => "User group",
            SortRole::LinkTarget => "Link destination",
        }
    }

    /// Compares two items by this role, ascending; unknown values first.
    pub(crate) fn compare(self, a: &FileItem, b: &FileItem) -> Ordering {
        let (a, b) = (a.entry(), b.entry());
        let text = |value: Option<&str>| value.map(SortKey::new);
        let natural = |a: Option<SortKey>, b: Option<SortKey>| match (a, b) {
            (Some(a), Some(b)) => a.natural_cmp(&b),
            (a, b) => a.is_some().cmp(&b.is_some()),
        };
        match self {
            SortRole::Created => a.meta.created.cmp(&b.meta.created),
            SortRole::Accessed => a.meta.accessed.cmp(&b.meta.accessed),
            SortRole::Extension => natural(
                text(extension(&a.name, a.is_dir)),
                text(extension(&b.name, b.is_dir)),
            ),
            SortRole::Permissions => a.meta.permissions.cmp(&b.meta.permissions),
            SortRole::Owner => natural(text(a.meta.owner.as_deref()), text(b.meta.owner.as_deref())),
            SortRole::Group => natural(text(a.meta.group.as_deref()), text(b.meta.group.as_deref())),
            SortRole::LinkTarget => natural(
                text(a.meta.link_target.as_deref()),
                text(b.meta.link_target.as_deref()),
            ),
        }
    }
}

/// The extension of a file called `name`; folders and names without a dot
/// past their first character have none.
pub(crate) fn extension(name: &str, is_dir: bool) -> Option<&str> {
    if is_dir {
        return None;
    }
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then_some(extension)
}

/// Any key the Sort menu offers: a details column or a further role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortBy {
    /// A details column, sorted by its title.
    Column(SortColumn),
    /// A key without a column.
    Role(SortRole),
}

impl SortBy {
    /// Key used in action targets and saved view styles.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            SortBy::Column(column) => column.as_str(),
            SortBy::Role(role) => role.as_str(),
        }
    }

    /// The key for an action target or a saved style.
    pub(crate) fn from_key(key: &str) -> Option<SortBy> {
        if let Some(column) = SortColumn::from_key(key) {
            return Some(SortBy::Column(column));
        }
        SortRole::ALL
            .into_iter()
            .find(|role| role.as_str() == key)
            .map(SortBy::Role)
    }
}

/// A sort key and the way it sorts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SortState {
    /// The key.
    pub by: SortBy,
    /// Which way it sorts.
    pub direction: SortDirection,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::file_entry;

    fn item_owned_by(name: &str, owner: Option<&str>, created: Option<u64>) -> FileItem {
        let mut entry = file_entry(name);
        entry.meta.owner = owner.map(str::to_owned);
        entry.meta.created = created;
        FileItem::new(entry)
    }

    /// The further keys compare their values, unknown values first, and
    /// every key round-trips through its name.
    ///
    /// parity: VIEW-019
    #[gtk::test]
    fn further_sort_keys_compare_their_values() {
        let ada = item_owned_by("b.txt", Some("ada"), Some(20));
        let bob = item_owned_by("a.PDF", Some("bob"), Some(10));
        let nobody = item_owned_by("c", None, None);
        assert_eq!(SortRole::Owner.compare(&ada, &bob), Ordering::Less);
        assert_eq!(SortRole::Owner.compare(&nobody, &ada), Ordering::Less);
        assert_eq!(SortRole::Created.compare(&ada, &bob), Ordering::Greater);
        assert_eq!(
            SortRole::Extension.compare(&bob, &ada),
            Ordering::Less,
            "pdf before txt"
        );
        assert_eq!(extension(".bashrc", false), None);
        for role in SortRole::ALL {
            assert_eq!(SortBy::from_key(role.as_str()), Some(SortBy::Role(role)));
        }
        assert_eq!(SortBy::from_key("size"), Some(SortBy::Column(SortColumn::Size)));
    }
}
