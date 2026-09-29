// SPDX-License-Identifier: AGPL-3.0-only
//! Extract here without nesting (ARC-025): when an archive holds exactly
//! one top-level folder, that folder becomes the output instead of
//! sitting inside a folder named after the archive, as Dolphin's "Extract
//! here, autodetect subfolder" does.
//!
//! The extraction itself is unchanged; afterwards the lone folder is
//! renamed up beside the archive under a free name. Every step is a
//! rename inside the destination folder that never replaces anything and
//! never falls back to copying, and a failed step puts the output back as
//! it was, so no file is lost and nothing existing is touched.

use gio::prelude::*;

use super::ExtractedFolder;
use crate::random::{random_hex, NAME_BYTES};

/// How many names are tried for the lifted folder: `X`, `X (2)`, …
const MAX_NAME_TRIES: u32 = 100;

/// A rename in place: never replaces, never copies, never follows a link.
const RENAME: gio::FileCopyFlags = gio::FileCopyFlags::NOFOLLOW_SYMLINKS
    .union(gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE);

/// The names tried for the lifted folder `name`, in order.
fn free_names(name: &str) -> impl Iterator<Item = String> + '_ {
    let numbered = (2..=MAX_NAME_TRIES).map(move |number| format!("{name} ({number})"));
    std::iter::once(name.to_owned()).chain(numbered)
}

/// The only child of `folder`, if it is a folder with a visible name.
/// Anything else (no child, several, a file, a link, a hidden folder)
/// keeps the output as it is.
fn lone_folder(folder: &gio::File) -> Option<String> {
    let children = folder
        .enumerate_children(
            "standard::name,standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let first = children.next_file(gio::Cancellable::NONE).ok()??;
    if children.next_file(gio::Cancellable::NONE).ok()?.is_some() {
        return None;
    }
    let name = first.name().to_str()?.to_owned();
    let is_folder = first.file_type() == gio::FileType::Directory;
    (is_folder && !name.starts_with('.')).then_some(name)
}

/// Moves `from` to `to`, never replacing `to`.
fn rename(from: &gio::File, to: &gio::File) -> Result<(), glib::Error> {
    from.move_(to, RENAME, gio::Cancellable::NONE, None)
}

/// Makes the lone folder inside `extracted` the output, beside the
/// archive under the first free name based on its own. Returns the
/// output as it ends up: lifted, or `extracted` unchanged when there is
/// nothing to lift or a rename fails. Blocking.
pub fn lift_single_folder(extracted: ExtractedFolder) -> ExtractedFolder {
    let outer = gio::File::for_uri(&extracted.uri);
    let destination = gio::File::for_uri(&extracted.destination_uri);
    let Some(inner_name) = lone_folder(&outer) else {
        return extracted;
    };
    // The outer folder steps aside under a private name first, so the
    // lone folder can take the archive's name when they are the same.
    let Ok(digits) = random_hex(NAME_BYTES) else {
        return extracted;
    };
    let aside = destination.child(format!(".openxplorer-extract-{digits}"));
    if rename(&outer, &aside).is_err() {
        return extracted;
    }
    let inner = aside.child(&inner_name);
    for name in free_names(&inner_name) {
        let target = destination.child(&name);
        match rename(&inner, &target) {
            Ok(()) => {
                // The folder is empty now; one left behind holds nothing.
                let _ = aside.delete(gio::Cancellable::NONE);
                return ExtractedFolder {
                    uri: target.uri().to_string(),
                    name,
                    ..extracted
                };
            }
            Err(error) if error.matches(gio::IOErrorEnum::Exists) => {}
            Err(_) => break,
        }
    }
    // Put the output back where the extraction published it.
    let _ = rename(&aside, &outer);
    extracted
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::archive::ExtractionSummary;
    use crate::location::file_uri;

    /// An extraction into `root/<name>` as the extractor reports it.
    fn extracted(root: &std::path::Path, name: &str) -> ExtractedFolder {
        ExtractedFolder {
            uri: file_uri(&root.join(name)),
            name: name.to_owned(),
            archive_uri: file_uri(&root.join(format!("{name}.zip"))),
            destination_uri: file_uri(root),
            summary: ExtractionSummary::default(),
        }
    }

    /// parity: ARC-025
    #[test]
    fn a_lone_top_folder_becomes_the_output_under_a_free_name() {
        let root = tempfile::tempdir().expect("a folder");
        fs::create_dir_all(root.path().join("Bundle/Project/src")).expect("output");
        fs::write(root.path().join("Bundle/Project/src/a.txt"), b"a").expect("file");
        fs::create_dir(root.path().join("Project")).expect("a taken name");

        let lifted = lift_single_folder(extracted(root.path(), "Bundle"));

        assert_eq!(lifted.name, "Project (2)");
        assert_eq!(fs::read(root.path().join("Project (2)/src/a.txt")).expect("moved"), b"a");
        assert!(!root.path().join("Bundle").exists());
        let mut names: Vec<_> = fs::read_dir(root.path())
            .expect("list")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["Project", "Project (2)"], "nothing left behind");
    }

    /// parity: ARC-025
    #[test]
    fn a_folder_named_like_the_archive_takes_its_name_and_mixed_content_stays() {
        let root = tempfile::tempdir().expect("a folder");
        fs::create_dir_all(root.path().join("Photos/Photos")).expect("output");
        fs::create_dir_all(root.path().join("Mixed/Docs")).expect("output");
        fs::write(root.path().join("Mixed/readme.txt"), b"r").expect("file");

        let same = lift_single_folder(extracted(root.path(), "Photos"));
        let mixed = lift_single_folder(extracted(root.path(), "Mixed"));

        assert_eq!(same.name, "Photos");
        assert!(root.path().join("Photos").is_dir());
        assert!(!root.path().join("Photos/Photos").exists());
        assert_eq!(mixed, extracted(root.path(), "Mixed"));
        assert!(root.path().join("Mixed/readme.txt").exists());
    }
}
