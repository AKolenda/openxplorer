// SPDX-License-Identifier: AGPL-3.0-only
//! The production GIO adapter ([`GioNode`]) on isolated temporary local
//! files. Each case file covers one part of the adapter; the adapter on a
//! simulated phone is tested in `transfer_cases/mtp_adapter.rs`.
//!
//! | Case file | What it covers |
//! |---|---|
//! | `local_adapter` | Listing, copying, moving and publishing, each without following links and without overwriting |
//! | `removal` | Trash and permanent deletion |
//! | `device_capabilities` | Device capabilities, inspected without contacting device backends |

#[path = "gio_node_cases/device_capabilities.rs"]
mod device_capabilities;
#[path = "gio_node_cases/local_adapter.rs"]
mod local_adapter;
#[path = "gio_node_cases/removal.rs"]
mod removal;
#[path = "transfer_support/shared.rs"]
mod shared;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use ox_core::gio_node::GioNode;

/// The adapter for the local item at `path`.
fn node(path: &Path) -> GioNode {
    GioNode::from_file(gio::File::for_path(path))
}

/// A folder `kept` holding `data`, and a symbolic link `link` to it.
struct LinkedFolder {
    kept: PathBuf,
    link: PathBuf,
}

impl LinkedFolder {
    /// Creates the folder and the link in `root`.
    fn create(root: &Path) -> Self {
        let kept = root.join("kept");
        fs::create_dir(&kept).expect("the fixture folder is created");
        fs::write(kept.join("data"), b"retained").expect("the fixture file is written");
        let link = root.join("link");
        symlink(&kept, &link).expect("the fixture link is created");
        Self { kept, link }
    }

    /// Asserts the folder kept its data and the link is still a link.
    fn assert_untouched(&self) {
        assert_eq!(
            fs::read(self.kept.join("data")).expect("the file can be read"),
            b"retained"
        );
        assert!(fs::symlink_metadata(&self.link)
            .expect("the link still exists")
            .file_type()
            .is_symlink());
    }
}
