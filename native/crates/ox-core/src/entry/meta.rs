// SPDX-License-Identifier: AGPL-3.0-only
//! What a listed item carries beyond the Details columns: the other times,
//! the owner, the permissions and a link's target, which the Sort menu
//! offers as Dolphin's further sort roles (VIEW-019).

use super::attributes::{string_attribute, time_attribute};

/// The permission bits of a Unix mode, without the file type.
const PERMISSION_BITS: u32 = 0o7777;

/// An item's further properties; each is `None` where the backend did not
/// report it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryMeta {
    /// Creation time, seconds since the Unix epoch.
    pub created: Option<u64>,
    /// Last access time, seconds since the Unix epoch.
    pub accessed: Option<u64>,
    /// The owner's user name.
    pub owner: Option<String>,
    /// The owning group's name.
    pub group: Option<String>,
    /// The permission bits (`0o755`).
    pub permissions: Option<u32>,
    /// Where a symbolic link points.
    pub link_target: Option<String>,
}

impl EntryMeta {
    /// The properties `info` reports.
    pub(super) fn from_info(info: &gio::FileInfo) -> Self {
        let permissions = info
            .has_attribute("unix::mode")
            .then(|| info.attribute_uint32("unix::mode") & PERMISSION_BITS);
        Self {
            created: time_attribute(info, "time::created"),
            accessed: time_attribute(info, "time::access"),
            owner: string_attribute(info, "owner::user"),
            group: string_attribute(info, "owner::group"),
            permissions,
            link_target: string_attribute(info, "standard::symlink-target"),
        }
    }

    /// The permissions as `ls` writes them, such as `rwxr-xr-x`; empty when
    /// unknown.
    pub fn permissions_text(&self) -> String {
        let Some(mode) = self.permissions else {
            return String::new();
        };
        let mut text = String::with_capacity(9);
        for shift in [6, 3, 0] {
            let bits = (mode >> shift) & 0o7;
            text.push(if bits & 0o4 == 0 { '-' } else { 'r' });
            text.push(if bits & 0o2 == 0 { '-' } else { 'w' });
            text.push(if bits & 0o1 == 0 { '-' } else { 'x' });
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_properties_are_read_and_permissions_written_as_ls_does() {
        let info = gio::FileInfo::new();
        info.set_attribute_uint64("time::created", 100);
        info.set_attribute_string("owner::user", "ada");
        info.set_attribute_uint32("unix::mode", 0o100_754);
        let meta = EntryMeta::from_info(&info);
        assert_eq!(meta.created, Some(100));
        assert_eq!(meta.accessed, None);
        assert_eq!(meta.owner.as_deref(), Some("ada"));
        assert_eq!(meta.permissions, Some(0o754));
        assert_eq!(meta.permissions_text(), "rwxr-xr--");
        assert_eq!(EntryMeta::default().permissions_text(), "");
    }
}
