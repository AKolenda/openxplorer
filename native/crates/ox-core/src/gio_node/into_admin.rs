// SPDX-License-Identifier: AGPL-3.0-only
//! Writing an item into `admin://` from anywhere else (OPS-047).
//!
//! GIO hands a copy or move from a local file to a `GVfs` location to the
//! backend as a "push": `gvfsd-admin` would then open, or rename, the
//! source by its path, as root. [`crate::admin_copy`] only vouches for the
//! destination's path; the source's folders may be the user's own, or
//! ones other accounts can change, so root must never resolve them. Here
//! the app reads the source itself, as the user, and `gvfsd-admin` only
//! creates and writes the new file in the checked destination. A move
//! into `admin://` is never native for the same reason: the engine copies
//! it this way and removes the original as the user (XFER-013).

use std::io::{Read, Write};

use gio::prelude::*;

use crate::i18n::gettext;
use crate::transfer::{Cancellation, TransferError};

/// The size of each read from the source.
const CHUNK: usize = 256 * 1024;

/// Whether `source` is written into `target` across the `admin://`
/// boundary: from another location into `admin://`.
pub(super) fn crosses_into_admin(source: &gio::File, target: &gio::File) -> bool {
    target.has_uri_scheme("admin") && !source.has_uri_scheme("admin")
}

/// The refusal of a native move into `admin://`, which the engine
/// finishes by copying.
pub(super) fn no_native_move() -> TransferError {
    TransferError::NotSupported(gettext(
        "Items are moved into an administrator location by copying them and then removing the originals.",
    ))
}

/// Copies the file or link `source` into the new item `target` under
/// `admin://`, reporting the bytes written to `progress`. The target is
/// created exclusively, so an existing item is never overwritten
/// (XFER-002); a link is copied as a link (XFER-017).
///
/// # Errors
///
/// The error of reading the source or writing the target,
/// [`TransferError::Cancelled`], and a refusal for anything but a file or a
/// link.
pub(super) fn copy_into_admin(
    source: &gio::File,
    target: &gio::File,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), TransferError> {
    let cancellable = Some(cancel.cancellable());
    let info = source.query_info(
        "standard::type,standard::size,standard::symlink-target,time::modified,time::modified-usec,unix::mode",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        cancellable,
    )?;
    match info.file_type() {
        gio::FileType::SymbolicLink => {
            let link = info.symlink_target().unwrap_or_default();
            target.make_symbolic_link(link, cancellable)?;
            return Ok(());
        }
        gio::FileType::Regular => {}
        _ => {
            return Err(TransferError::NotSupported(gettext(
                "Only files, folders and links can be copied as administrator.",
            )))
        }
    }
    let total = u64::try_from(info.size()).unwrap_or(0);
    // Read as the user: following a link here only reads what the user
    // may read anyway.
    let mut input = source.read(cancellable)?.into_read();
    let mut output = target
        .create(gio::FileCreateFlags::NONE, cancellable)?
        .into_write();
    let mut buffer = vec![0; CHUNK];
    let mut written = 0_u64;
    progress(0, total);
    loop {
        cancel.check()?;
        let read = input.read(&mut buffer).map_err(|error| io_error(&error))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| io_error(&error))?;
        written += read as u64;
        progress(written, total.max(written));
    }
    output.flush().map_err(|error| io_error(&error))?;
    drop(output);
    keep_metadata(target, &info, cancel);
    Ok(())
}

/// Gives `target` the modification time and permissions `source` had, as
/// a copy keeps them; a backend that refuses keeps its own.
fn keep_metadata(target: &gio::File, source: &gio::FileInfo, cancel: &Cancellation) {
    let cancellable = Some(cancel.cancellable());
    let flags = gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS;
    if source.has_attribute("time::modified") {
        let _ = target.set_attribute_uint64(
            "time::modified",
            source.attribute_uint64("time::modified"),
            flags,
            cancellable,
        );
        let _ = target.set_attribute_uint32(
            "time::modified-usec",
            source.attribute_uint32("time::modified-usec"),
            flags,
            cancellable,
        );
    }
    if source.has_attribute("unix::mode") {
        // Only the permission bits, without group and other write: never
        // set-user-ID, set-group-ID or sticky on a file root now owns, and
        // nothing another account could change in a folder only root may.
        let mode = source.attribute_uint32("unix::mode") & 0o755;
        let _ = target.set_attribute_uint32("unix::mode", mode, flags, cancellable);
    }
}

/// `error` as the engine reports a failed read or write.
fn io_error(error: &std::io::Error) -> TransferError {
    TransferError::failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    use std::path::Path;

    use super::*;
    use crate::test_support::temporary_folder;

    /// Only a write from elsewhere into `admin://` is done here.
    ///
    /// parity: OPS-047
    #[test]
    fn only_writes_into_admin_from_elsewhere_cross_the_boundary() {
        let local = gio::File::for_uri("file:///srv/Apps/tool.cfg");
        let admin = gio::File::for_uri("admin:///opt/Apps/tool.cfg");

        assert!(crosses_into_admin(&local, &admin));
        assert!(!crosses_into_admin(&admin, &admin));
        assert!(!crosses_into_admin(&admin, &local));
        assert!(!crosses_into_admin(&local, &local));
    }

    /// The file is read here and written to a new item, never over an
    /// existing one, with its time and its permissions less group and
    /// other write; a link stays a link. The target is a plain folder
    /// here: what `gvfsd-admin` does with the writes is its own.
    ///
    /// parity: OPS-047, XFER-002, XFER-017
    #[test]
    fn a_file_is_copied_by_the_app_and_a_link_as_a_link() {
        let folder = temporary_folder();
        let source = folder.path().join("tool.cfg");
        fs::write(&source, "settings").expect("write the source");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o4775)).expect("set the mode");
        let target = folder.path().join("copied.cfg");
        let cancel = Cancellation::new();
        let mut reported = Vec::new();

        copy_into_admin(
            &gio::File::for_path(&source),
            &gio::File::for_path(&target),
            &cancel,
            &mut |done, total| reported.push((done, total)),
        )
        .expect("the copy");

        assert_eq!(fs::read_to_string(&target).expect("the copy"), "settings");
        let (source_meta, target_meta) = (
            fs::metadata(&source).expect("source"),
            fs::metadata(&target).expect("target"),
        );
        assert_eq!(target_meta.mode() & 0o7777, 0o755);
        assert_eq!(target_meta.mtime(), source_meta.mtime());
        assert_eq!(reported.last(), Some(&(8, 8)));

        let again = copy_into_admin(
            &gio::File::for_path(&source),
            &gio::File::for_path(&target),
            &cancel,
            &mut |_, _| {},
        );
        assert!(again.is_err(), "an existing item is never overwritten");

        let link = folder.path().join("link");
        symlink("tool.cfg", &link).expect("a link");
        let copied_link = folder.path().join("copied link");
        copy_into_admin(
            &gio::File::for_path(&link),
            &gio::File::for_path(&copied_link),
            &cancel,
            &mut |_, _| {},
        )
        .expect("the link");
        assert_eq!(
            fs::read_link(&copied_link).expect("a link"),
            Path::new("tool.cfg")
        );
    }
}
