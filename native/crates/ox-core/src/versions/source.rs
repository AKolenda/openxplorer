// SPDX-License-Identifier: AGPL-3.0-only
//! A snapshot source: the snapshot collection folder that holds dated
//! copies of a live folder, and how the copies are laid out in it.
//!
//! Ports the validation of `PreviousVersions.sources` and
//! `PreviousVersions.configure` in `desktop/previous_versions.py`. Sources
//! are saved as `{"live": …, "snapshots": …, "layout": …}` objects, the
//! format the Python app reads and writes.

use std::str::FromStr;

use serde::Serialize;

use super::paths::is_within;
use super::VersionsError;
use crate::location::normalise;

/// How the snapshots in a collection folder are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotLayout {
    /// `<collection>/<snapshot name>/<relative path>`: `.snapshot`,
    /// `#snapshot` and `.zfs/snapshot` folders, and plain backup folders.
    #[default]
    Direct,
    /// `<collection>/<snapshot id>/snapshot/<relative path>`: Snapper's
    /// `.snapshots` folder on Btrfs.
    Snapper,
}

impl SnapshotLayout {
    /// The name saved in the sources file: `direct` or `snapper`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Snapper => "snapper",
        }
    }
}

impl FromStr for SnapshotLayout {
    type Err = VersionsError;

    /// Parses a saved or requested layout name.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "direct" => Ok(Self::Direct),
            "snapper" => Ok(Self::Snapper),
            _ => Err(VersionsError::UnknownLayout),
        }
    }
}

/// A live folder and the snapshot collection folder that holds its
/// history.
///
/// The collection never contains the live folder: otherwise the live
/// folder would itself become read-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotSource {
    /// The canonical URI of the live folder.
    live: String,
    /// The canonical URI of the snapshot collection folder.
    #[serde(rename = "snapshots")]
    collection: String,
    /// How the snapshots are laid out in the collection.
    layout: SnapshotLayout,
}

impl SnapshotSource {
    /// A source mapping the live folder `live` to the snapshot collection
    /// folder `collection`. Both are normalised first, so any form the
    /// address bar accepts works.
    ///
    /// # Errors
    ///
    /// [`VersionsError::Location`] for an address the location rules
    /// refuse, and [`VersionsError::SnapshotFolderContainsLiveFolder`] when
    /// the collection is the live folder or contains it.
    pub fn new(live: &str, collection: &str, layout: SnapshotLayout) -> Result<Self, VersionsError> {
        let live = normalise(live)?;
        let collection = normalise(collection)?;
        // Safety rule (`configure` in previous_versions.py): a collection
        // that contains the live folder would make the live folder
        // read-only.
        if is_within(&live, &collection) {
            return Err(VersionsError::SnapshotFolderContainsLiveFolder);
        }
        Ok(Self {
            live,
            collection,
            layout,
        })
    }

    /// A source guessed next to an item (see `lookup`), whose URIs are
    /// derived from an already canonical URI and so are not normalised
    /// again.
    pub(crate) fn nearby(live: &str, collection_name: &str, layout: SnapshotLayout) -> Self {
        Self {
            live: live.to_owned(),
            collection: format!("{live}/{collection_name}"),
            layout,
        }
    }

    /// A source read back from the sources file, or `None` for an entry
    /// the Python app would drop: not an object, a missing or invalid
    /// location, or a collection that contains its live folder. An unknown
    /// or missing layout becomes [`SnapshotLayout::Direct`], as in Python.
    pub(crate) fn from_saved(saved: &serde_json::Value) -> Option<Self> {
        let live = saved.get("live")?.as_str()?;
        let collection = saved.get("snapshots")?.as_str()?;
        let layout = saved
            .get("layout")
            .and_then(serde_json::Value::as_str)
            .and_then(|name| name.parse().ok())
            .unwrap_or_default();
        Self::new(live, collection, layout).ok()
    }

    /// The canonical URI of the live folder.
    pub fn live(&self) -> &str {
        &self.live
    }

    /// The canonical URI of the snapshot collection folder.
    pub fn collection(&self) -> &str {
        &self.collection
    }

    /// How the snapshots are laid out in the collection.
    pub fn layout(&self) -> SnapshotLayout {
        self.layout
    }

    /// True when `uri` is the live folder or lies below it.
    pub fn covers(&self, uri: &str) -> bool {
        is_within(uri, &self.live)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn layouts_round_trip_through_their_saved_names() {
        for layout in [SnapshotLayout::Direct, SnapshotLayout::Snapper] {
            assert_eq!(layout.as_str().parse::<SnapshotLayout>().ok(), Some(layout));
        }
    }

    /// parity: PROP-023
    #[test]
    fn an_unknown_layout_is_refused_in_the_python_wording() {
        let error = "zfs".parse::<SnapshotLayout>().unwrap_err();

        assert_eq!(error.to_string(), "Unknown snapshot folder layout.");
    }

    /// parity: PROP-023
    #[test]
    fn a_collection_that_contains_its_live_folder_is_refused() {
        for (live, collection) in [
            ("smb://nas/share", "smb://nas/share"),
            ("smb://nas/share/work", "smb://nas/share"),
            ("/srv/data", "/srv"),
        ] {
            let error = SnapshotSource::new(live, collection, SnapshotLayout::Direct).unwrap_err();

            assert_eq!(
                error.to_string(),
                "The snapshot folder must not contain the current live folder.",
                "{live} in {collection}"
            );
        }
    }

    #[test]
    fn locations_are_normalised_and_a_nested_collection_is_accepted() {
        let source = SnapshotSource::new(
            "\\\\NAS\\Share",
            "smb://nas/Share/.snapshot/",
            SnapshotLayout::Snapper,
        )
        .expect("a collection inside the live folder is valid");

        assert_eq!(source.live(), "smb://nas/Share");
        assert_eq!(source.collection(), "smb://nas/Share/.snapshot");
        assert_eq!(source.layout(), SnapshotLayout::Snapper);
    }

    /// parity: PROP-023
    #[test]
    fn saved_entries_the_python_app_would_drop_are_dropped() {
        let dropped = [
            json!("smb://nas/share"),
            json!(["smb://nas/share", "smb://nas/share/.snapshot"]),
            json!({"live": "smb://nas/share"}),
            json!({"live": 7, "snapshots": "smb://nas/share/.snapshot"}),
            json!({"live": "C:\\Work", "snapshots": "smb://nas/share/.snapshot"}),
            json!({"live": "smb://nas/share/a", "snapshots": "smb://nas/share"}),
        ];
        for saved in dropped {
            assert_eq!(SnapshotSource::from_saved(&saved), None, "{saved}");
        }
    }

    #[test]
    fn a_saved_entry_with_an_unknown_layout_uses_the_direct_layout() {
        let saved = json!({"live": "/srv/data", "snapshots": "/srv/history", "layout": ["snapper"]});

        let source = SnapshotSource::from_saved(&saved).expect("a valid entry");

        assert_eq!(source.layout(), SnapshotLayout::Direct);
        assert_eq!(source.live(), "file:///srv/data");
    }

    #[test]
    fn a_source_saves_in_the_python_format() {
        let source = SnapshotSource::new("/srv/data", "/srv/history", SnapshotLayout::Snapper).unwrap();

        let saved = serde_json::to_value(&source).unwrap();

        assert_eq!(
            saved,
            json!({"live": "file:///srv/data", "snapshots": "file:///srv/history", "layout": "snapper"})
        );
    }
}
