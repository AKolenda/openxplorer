// SPDX-License-Identifier: AGPL-3.0-only
//! The addresses typed into the address bar, most recent first, kept for
//! the user across windows and restarts (NAV-043).
//!
//! Windows 11 File Explorer keeps what was typed into its address bar for
//! the user (`Explorer\TypedPaths`), so every window's history drop-down
//! lists it and it survives a restart; Folder Options' Clear File Explorer
//! history forgets it. The app keeps them in `typed-addresses.json` in the
//! settings folder, at most [`MAX_TYPED_ADDRESSES`], written privately
//! (0600 in a 0700 folder) and atomically like `settings.json`. An
//! address with a user name or a control character is never kept, as
//! settings never keep one (SAFE-010).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::private_storage::{
    private_directory, private_file_if_present, read_limited_text, replace_file_atomically,
    PrivateFileOptions, StorageError,
};

/// The file in the settings folder.
pub const TYPED_ADDRESSES_FILE: &str = "typed-addresses.json";

/// The most addresses kept.
pub const MAX_TYPED_ADDRESSES: usize = 20;

/// The longest address kept, in bytes.
const MAX_ADDRESS_BYTES: usize = 4096;

/// The largest file read; [`MAX_TYPED_ADDRESSES`] entries need far less.
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// The prefix of the temporary files of an atomic save.
const TEMPORARY_PREFIX: &str = ".typed-addresses-";

/// The file's contents.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    addresses: Vec<String>,
}

/// The typed addresses, most recent first.
#[derive(Debug, Clone)]
pub struct TypedAddresses {
    path: PathBuf,
    addresses: Vec<String>,
}

impl TypedAddresses {
    /// The history in the settings folder `directory`; never fails. A
    /// missing, damaged or refused file is read as empty.
    pub fn open(directory: &Path) -> Self {
        let path = directory.join(TYPED_ADDRESSES_FILE);
        let addresses = read(&path).unwrap_or_default();
        Self { path, addresses }
    }

    /// The addresses, most recent first.
    pub fn addresses(&self) -> &[String] {
        &self.addresses
    }

    /// Puts `address` at the top, once.
    ///
    /// # Errors
    ///
    /// The [`StorageError`] of writing the file; the history in memory
    /// changes either way.
    pub fn remember(&mut self, address: &str) -> Result<(), StorageError> {
        let address = address.trim();
        if !may_keep(address) {
            return Ok(());
        }
        // Re-read, so an address typed in another process is not lost.
        if let Some(stored) = read(&self.path) {
            self.addresses = stored;
        }
        self.addresses.retain(|kept| kept != address);
        self.addresses.insert(0, address.to_owned());
        self.addresses.truncate(MAX_TYPED_ADDRESSES);
        self.save()
    }

    /// Forgets every address and removes the file.
    ///
    /// # Errors
    ///
    /// The [`StorageError`] of removing the file.
    pub fn clear(&mut self) -> Result<(), StorageError> {
        self.addresses.clear();
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(StorageError::io(&self.path, error))
            }
            _ => Ok(()),
        }
    }

    /// Writes the history privately and atomically.
    fn save(&self) -> Result<(), StorageError> {
        let text = serde_json::to_string_pretty(&Stored {
            addresses: self.addresses.clone(),
        })
        .unwrap_or_default();
        private_directory(crate::private_storage::parent_directory(&self.path))?;
        replace_file_atomically(&self.path, TEMPORARY_PREFIX, text.as_bytes())
    }
}

/// The addresses kept at `path`, those that may not be kept left out;
/// `None` when there is no file or it is unusable.
fn read(path: &Path) -> Option<Vec<String>> {
    let file = private_file_if_present(path, PrivateFileOptions::default()).ok()??;
    let text = read_limited_text(file, path, MAX_FILE_BYTES).ok()?;
    let stored: Stored = serde_json::from_str(&text).ok()?;
    let mut addresses: Vec<String> = Vec::new();
    for address in stored.addresses {
        if may_keep(&address) && address.trim() == address && !addresses.contains(&address) {
            addresses.push(address);
        }
    }
    addresses.truncate(MAX_TYPED_ADDRESSES);
    Some(addresses)
}

/// Whether `address` may be kept: not empty, short enough, without a
/// control character or a user name.
fn may_keep(address: &str) -> bool {
    let authority = address
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest));
    !address.trim().is_empty()
        && address.len() <= MAX_ADDRESS_BYTES
        && !address.chars().any(char::is_control)
        && !authority.is_some_and(|authority| authority.contains('@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: NAV-043
    #[test]
    fn typed_addresses_go_to_the_top_once_and_the_oldest_drop_off() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let mut history = TypedAddresses::open(folder.path());
        for number in 0..25 {
            history.remember(&format!("/demo/{number}")).expect("saved");
        }
        history.remember("  /demo/20  ").expect("saved");

        let addresses = history.addresses();
        assert_eq!(addresses.len(), MAX_TYPED_ADDRESSES);
        assert_eq!(addresses[0], "/demo/20");
        assert_eq!(addresses[1], "/demo/24");
        assert_eq!(addresses.iter().filter(|kept| *kept == "/demo/20").count(), 1);
        assert_eq!(addresses[19], "/demo/5", "moving one up drops none");
    }

    /// parity: NAV-043
    #[test]
    fn the_history_is_kept_privately_and_read_back() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let mut history = TypedAddresses::open(folder.path());
        history.remember("~/Music").expect("saved");
        history.remember("\\\\nas\\Projects").expect("saved");

        let again = TypedAddresses::open(folder.path());

        assert_eq!(again.addresses(), ["\\\\nas\\Projects", "~/Music"]);
        let file = folder.path().join(TYPED_ADDRESSES_FILE);
        let mode =
            std::os::unix::fs::PermissionsExt::mode(&fs::metadata(&file).expect("saved").permissions());
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_user_name_or_a_control_character_is_never_kept() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let mut history = TypedAddresses::open(folder.path());
        for refused in [
            "",
            "   ",
            "smb://sam@nas/Projects",
            "sftp://sam:secret@host/srv",
            "/demo/a\nb",
            &"x".repeat(MAX_ADDRESS_BYTES + 1),
        ] {
            history.remember(refused).expect("nothing to save");
        }
        history.remember("smb://nas/Team@2026").expect("saved");

        assert_eq!(history.addresses(), ["smb://nas/Team@2026"]);
        let stored = r#"{"addresses": ["/ok", "smb://sam@nas/x", "/a\u0007b", "/ok", 7]}"#;
        fs::write(folder.path().join(TYPED_ADDRESSES_FILE), stored).expect("written");
        assert!(
            TypedAddresses::open(folder.path()).addresses().is_empty(),
            "a file of the wrong shape is read as empty"
        );
        let stored = "{\"addresses\": [\"/ok\", \"smb://sam@nas/x\", \"/a\\u0007b\", \"/ok\"]}";
        fs::write(folder.path().join(TYPED_ADDRESSES_FILE), stored).expect("written");
        assert_eq!(TypedAddresses::open(folder.path()).addresses(), ["/ok"]);
    }
}
