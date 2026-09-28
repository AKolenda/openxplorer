// SPDX-License-Identifier: AGPL-3.0-only
//! Items grouped by folder, for the operations that run the transfer
//! engine once per folder: Duplicate copies into each item's own folder,
//! and the undo of a move puts items back into the folder each came from.

use std::collections::HashMap;

/// The items that go into, or come from, one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderGroup {
    /// The folder's URI.
    pub(crate) folder_uri: String,
    /// The items, in the order they were added.
    pub(crate) uris: Vec<String>,
}

/// Collects items into [`FolderGroup`]s, keeping the order in which each
/// folder first appeared.
#[derive(Debug, Default)]
pub(crate) struct FolderGroups {
    groups: Vec<FolderGroup>,
    index_of_folder: HashMap<String, usize>,
}

impl FolderGroups {
    /// Adds `uri` to the group of `folder_uri`.
    pub(crate) fn add(&mut self, folder_uri: String, uri: String) {
        let index = match self.index_of_folder.get(&folder_uri) {
            Some(&index) => index,
            None => self.start_group(folder_uri),
        };
        self.groups[index].uris.push(uri);
    }

    /// The groups, in the order their folders first appeared.
    pub(crate) fn into_groups(self) -> Vec<FolderGroup> {
        self.groups
    }

    /// Starts an empty group for `folder_uri` and returns its index.
    fn start_group(&mut self, folder_uri: String) -> usize {
        let index = self.groups.len();
        self.index_of_folder.insert(folder_uri.clone(), index);
        self.groups.push(FolderGroup {
            folder_uri,
            uris: Vec::new(),
        });
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_are_grouped_by_folder_in_first_seen_order() {
        let mut groups = FolderGroups::default();

        groups.add("file:///b".into(), "file:///b/1".into());
        groups.add("file:///a".into(), "file:///a/1".into());
        groups.add("file:///b".into(), "file:///b/2".into());

        let expected = vec![
            FolderGroup {
                folder_uri: "file:///b".into(),
                uris: vec!["file:///b/1".into(), "file:///b/2".into()],
            },
            FolderGroup {
                folder_uri: "file:///a".into(),
                uris: vec!["file:///a/1".into()],
            },
        ];
        assert_eq!(groups.into_groups(), expected);
    }
}
