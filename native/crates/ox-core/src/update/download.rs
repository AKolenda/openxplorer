// SPDX-License-Identifier: AGPL-3.0-only
//! Downloading the installer into a private folder and verifying it.
//! Ports the download part of `Updater.install` in `desktop/updater.py`.

use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::release::sha256_checksum;
use super::{FetchError, Installer, ReleaseServer, UpdateError};
use crate::private_storage::{private_directory, FILE_MODE};
use crate::transfer::Cancellation;

/// How much is read and written at a time.
const BLOCK_SIZE: usize = 128 * 1024;

/// The mode of each download's folder, as Python's `mkdtemp` makes it.
const PRIVATE_FOLDER_MODE: u32 = 0o700;

/// The name prefix of each download's private folder.
const STAGING_PREFIX: &str = "release-";

/// A downloaded installer whose size and SHA-256 digest matched the
/// release.
///
/// Safety rule "temporary files are always removed"
/// (`tempfile.TemporaryDirectory` in `Updater.install`): the installer
/// lives in its own folder, which is deleted with everything in it when
/// this is dropped, whether the installation succeeded, failed or
/// panicked.
#[derive(Debug)]
pub(super) struct StagedInstaller {
    /// The private folder, deleted on drop.
    folder: TempDir,
    /// The installer's name inside `folder`.
    file_name: String,
}

impl StagedInstaller {
    /// Where the installer is.
    pub(super) fn path(&self) -> PathBuf {
        self.folder.path().join(&self.file_name)
    }

    /// The private folder it is in.
    #[cfg(test)]
    pub(super) fn folder(&self) -> &Path {
        self.folder.path()
    }
}

/// Downloads `installer` into a new private folder inside
/// `updates_folder` and verifies it.
///
/// Safety rule "downloads are private" (`private_directory` and the
/// `0o600` file in `Updater.install`): `updates_folder` must be an owned,
/// real folder and becomes mode 0700, each download gets a fresh 0700
/// folder inside it, and the installer is a new 0600 file.
///
/// Safety rule "verify before anything runs": the download stops as soon
/// as it is larger than the release said, and it is accepted only if its
/// size and SHA-256 digest are exactly the release's.
///
/// # Errors
///
/// [`UpdateError::Refused`] or [`UpdateError::Io`] for the folder or file,
/// a failed fetch in the download's wording, [`UpdateError::Cancelled`],
/// [`UpdateError::InstallerTooLarge`] and
/// [`UpdateError::ChecksumMismatch`].
pub(super) fn download_installer(
    server: &dyn ReleaseServer,
    installer: &Installer,
    updates_folder: &Path,
    cancel: &Cancellation,
) -> Result<StagedInstaller, UpdateError> {
    // Safety rule "downloads are private": an owned, real 0700 folder...
    private_directory(updates_folder)?;
    // ...and a fresh 0700 folder per download. Without the explicit mode,
    // tempfile would leave it as the umask allows, often 0775.
    let folder = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .permissions(Permissions::from_mode(PRIVATE_FOLDER_MODE))
        .tempdir_in(updates_folder)
        .map_err(|error| UpdateError::io(updates_folder, error))?;
    let path = folder.path().join(&installer.name);
    let body = server
        .open(&installer.url, cancel)
        .map_err(FetchError::into_download_error)?;
    let mut output = create_private_file(&path)?;
    let (received, digest) = copy_limited(body, &mut output, &path, installer.size, cancel)?;
    output.sync_all().map_err(|error| UpdateError::io(&path, error))?;
    // Safety rule "verify before anything runs": exactly the release's
    // size and digest, or nothing is installed.
    if received != installer.size || digest != installer.sha256.as_str() {
        return Err(UpdateError::ChecksumMismatch);
    }
    Ok(StagedInstaller {
        folder,
        file_name: installer.name.clone(),
    })
}

/// Creates `path` as a new file (never an existing one) with mode 0600.
fn create_private_file(path: &Path) -> Result<File, UpdateError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
        .map_err(|error| UpdateError::io(path, error))?;
    // The umask can only narrow the creation mode; this makes it exact.
    file.set_permissions(Permissions::from_mode(FILE_MODE))
        .map_err(|error| UpdateError::io(path, error))?;
    Ok(file)
}

/// Copies `body` into `output`, the file at `path`, refusing more than
/// `limit` bytes, and returns how many bytes came and their SHA-256 digest
/// in hexadecimal.
fn copy_limited(
    mut body: impl Read,
    output: &mut File,
    path: &Path,
    limit: u64,
    cancel: &Cancellation,
) -> Result<(u64, String), UpdateError> {
    let mut checksum = sha256_checksum();
    let mut block = vec![0; BLOCK_SIZE];
    let mut received: u64 = 0;
    loop {
        if cancel.is_cancelled() {
            return Err(UpdateError::Cancelled);
        }
        let count = match body.read(&mut block) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(FetchError::interrupted(cancel).into_download_error()),
        };
        received += count as u64;
        // Never write more than the release said the installer holds.
        if received > limit {
            return Err(UpdateError::InstallerTooLarge);
        }
        let bytes = &block[..count];
        output
            .write_all(bytes)
            .map_err(|error| UpdateError::io(path, error))?;
        checksum.update(bytes);
    }
    let digest = checksum
        .string()
        .expect("a SHA-256 checksum has a hexadecimal form");
    Ok((received, digest))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::test_support::permission_bits;
    use crate::update::{Sha256Digest, TrustedUrl};

    const PAYLOAD: &[u8] = b"Fictional package bytes.\n";

    /// Answers every fetch with `PAYLOAD`.
    struct FixedBody;

    impl ReleaseServer for FixedBody {
        fn open(&self, _url: &TrustedUrl, _cancel: &Cancellation) -> Result<Box<dyn Read>, FetchError> {
            Ok(Box::new(Cursor::new(PAYLOAD)))
        }
    }

    fn installer() -> Installer {
        let digest = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, PAYLOAD).unwrap();
        Installer {
            name: "openxplorer_1.0.1_all.deb".to_owned(),
            url: TrustedUrl::parse("https://github.com/fixture").unwrap(),
            sha256: Sha256Digest::from_github(&format!("sha256:{digest}")).unwrap(),
            size: PAYLOAD.len() as u64,
        }
    }

    #[test]
    fn the_installer_is_private_and_its_folder_disappears_with_it() {
        let root = tempfile::tempdir().unwrap();
        let updates = root.path().join("updates");

        let staged = download_installer(&FixedBody, &installer(), &updates, &Cancellation::new()).unwrap();

        assert_eq!(std::fs::read(staged.path()).unwrap(), PAYLOAD);
        assert_eq!(permission_bits(&staged.path()), 0o600);
        assert_eq!(permission_bits(staged.folder()), 0o700);
        assert_eq!(permission_bits(&updates), 0o700);
        drop(staged);
        assert_eq!(std::fs::read_dir(&updates).unwrap().count(), 0);
    }

    #[test]
    fn a_cancelled_download_writes_nothing_more() {
        let root = tempfile::tempdir().unwrap();
        let cancel = Cancellation::new();
        cancel.cancel();

        let result = download_installer(&FixedBody, &installer(), root.path(), &cancel);

        assert!(matches!(result, Err(UpdateError::Cancelled)));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
