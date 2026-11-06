// SPDX-License-Identifier: AGPL-3.0-only
//! Paste a picture on the clipboard as a new file (CLIP-015).
//!
//! Nautilus's paste of a screenshot and Dolphin's "Paste Clipboard
//! Contents…" write the picture on the clipboard into the folder as one
//! file. The bytes are those [`PastedImage`] carried, and the write is the one
//! New ▸ Text document uses: the content goes to a private stage file in
//! the destination folder and is published under the new name by a
//! same-folder rename that never overwrites, so a paste in a folder that
//! changed meanwhile can neither replace an item nor leave a partial file
//! (OPS-048, XFER-001, XFER-007). A name that is taken moves the new file
//! to the next number, `2026-02-05 14-30-00 (2).png`, as a new folder and
//! a new document do; the file it found is never touched.
//!
//! The free space is checked before the first byte is written, with the
//! refusal a copy gives (XFER-028). A paste writes one file of a size the
//! app already knows, so unlike a copy nothing has to be measured first.

use super::context::{on_worker, OperationContext};
use super::create::CreatedItem;
use super::error::OpsError;
use super::new_from_template::{create_file_blocking, first_free_name};
use crate::clipboard::PastedImage;
use crate::gio_node::GioNode;
use crate::transfer::payload_refusal;

/// Writes `image` into the folder at `folder_uri` as the file `name`,
/// under the first free numbered name of it, and returns the item.
///
/// # Errors
///
/// An invalid name, a server listing ("Open a share before creating a
/// file."), a protected folder, not enough free space ("Not enough free
/// space on …"), cancellation, or the backend's failure. A failed stage
/// file is deleted.
pub async fn create_pasted_file(
    folder_uri: &str,
    name: &str,
    image: &PastedImage,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    let folder_uri = folder_uri.to_owned();
    let name = name.to_owned();
    let bytes = image.bytes.clone();
    let context = context.clone();
    on_worker(move || {
        // XFER-028 before the first byte: the payload is measured already.
        // A folder whose free space cannot be read is not refused here;
        // writing that does not fit reports the backend's own error.
        if let Some(refusal) = payload_refusal(
            &GioNode::new(&folder_uri),
            bytes.len() as u64,
            Some(&context.cancel),
        ) {
            return Err(refusal.into());
        }
        first_free_name(&name, |name| {
            create_file_blocking(&folder_uri, name, || Ok(bytes.clone()), &context)
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gio::prelude::*;

    use super::*;

    /// The folder at `path` as a URI.
    fn folder_uri(path: &std::path::Path) -> String {
        gio::File::for_path(path).uri().to_string()
    }

    /// parity: CLIP-015
    #[test]
    fn the_pasted_bytes_are_written_as_they_are() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let image = PastedImage {
            extension: "jpg",
            bytes: vec![0xff, 0xd8, 0xff, 0xe0, b'\r', b'\n', 0x00],
        };

        let created = glib::MainContext::new().block_on(create_pasted_file(
            &folder_uri(temp.path()),
            "2026-02-05 14-30-00.jpg",
            &image,
            &OperationContext::default(),
        ));

        let created = created.expect("the file is written");
        assert_eq!(
            created.uri,
            folder_uri(&temp.path().join("2026-02-05 14-30-00.jpg"))
        );
        assert_eq!(
            fs::read(temp.path().join("2026-02-05 14-30-00.jpg")).expect("the pasted file"),
            [0xff, 0xd8, 0xff, 0xe0, b'\r', b'\n', 0x00],
            "the picture is kept byte for byte, never re-encoded"
        );
    }

    /// parity: CLIP-015
    #[test]
    fn a_pasted_name_that_is_taken_moves_the_new_file_to_the_next_number() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let taken = temp.path().join("2026-02-05 14-30-00.png");
        fs::write(&taken, b"kept").expect("an existing item");
        let image = PastedImage {
            extension: "png",
            bytes: vec![0x89, b'P', b'N', b'G'],
        };

        let created = glib::MainContext::new().block_on(create_pasted_file(
            &folder_uri(temp.path()),
            "2026-02-05 14-30-00.png",
            &image,
            &OperationContext::default(),
        ));

        let created = created.expect("the file is written");
        assert_eq!(
            created.uri,
            folder_uri(&temp.path().join("2026-02-05 14-30-00 (2).png"))
        );
        assert_eq!(fs::read(&taken).expect("the existing item stays"), b"kept");
        assert_eq!(
            fs::read(temp.path().join("2026-02-05 14-30-00 (2).png")).expect("the pasted file"),
            [0x89, b'P', b'N', b'G']
        );
    }

    /// A folder that is gone refuses the paste in the app's wording, and
    /// writes nothing.
    #[test]
    fn a_folder_that_is_gone_refuses_the_paste_and_writes_nothing() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let gone = temp.path().join("gone");
        let image = PastedImage {
            extension: "png",
            bytes: vec![0x89, b'P', b'N', b'G'],
        };

        let refused = glib::MainContext::new().block_on(create_pasted_file(
            &folder_uri(&gone),
            "1.png",
            &image,
            &OperationContext::default(),
        ));

        assert!(refused.is_err(), "nothing is written into a folder that is gone");
        assert!(!gone.exists());
    }
}
