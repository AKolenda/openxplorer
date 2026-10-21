// SPDX-License-Identifier: AGPL-3.0-only
//! Error messages name the item the user asked for, not the private
//! staging it is built in.
//!
//! A backend error names the path it failed on, and while an item is built
//! that path is inside its staging: pasting `one.txt` into a read-only
//! folder failed with "Error creating directory
//! …/Locked/.winspace-transfer-<hex>.part: Permission denied". The user
//! never sees the staging, and Windows Explorer names the item it could not
//! create, so the message names `…/Locked/one.txt` and keeps the system's
//! reason.
//!
//! Staging always sits in the folder of the final item, so putting the
//! final name in place of the staging name, and of the item inside a
//! staging folder, turns every spelling of a staged path into the real
//! one: a local path, a FUSE path or a URI. A staging name is random, so it
//! appears nowhere else in a message. Staging whose items end up directly
//! in an existing folder is left out of the path instead, so the message
//! names that folder.

use std::ffi::OsStr;

use super::error::TransferError;

/// The private name an item is built under, and the real name messages
/// use for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedName {
    /// The staging's own name in its folder.
    stage: String,
    /// Each staged spelling and what replaces it, in the order they are
    /// replaced.
    replacements: Vec<(String, String)>,
}

impl StagedName {
    /// An item built as `stage`, or as one of `inner` in a staging folder
    /// called `stage`, and published as `real` beside it.
    pub(crate) fn new(stage: &str, inner: &[&OsStr], real: &str) -> Self {
        let mut staged: Vec<String> = inner
            .iter()
            .map(|inner| format!("{stage}/{}", inner.to_string_lossy()))
            .collect();
        staged.sort_by_key(|path| std::cmp::Reverse(path.len()));
        staged.push(stage.to_owned());
        Self {
            stage: stage.to_owned(),
            replacements: staged
                .into_iter()
                .map(|staged| (staged, real.to_owned()))
                .collect(),
        }
    }

    /// Private folders `stage` and then `private` inside it, whose items
    /// end up directly in the existing folder called `folder` that holds
    /// them: both are left out of a path, and a bare name becomes `folder`.
    pub(crate) fn inside(stage: &str, private: &str, folder: &str) -> Self {
        let replacements = [stage, private]
            .into_iter()
            .flat_map(|name| {
                [
                    (format!("/{name}"), String::new()),
                    (name.to_owned(), folder.to_owned()),
                ]
            })
            .collect();
        Self {
            stage: stage.to_owned(),
            replacements,
        }
    }

    /// The staging's own name.
    pub(crate) fn stage(&self) -> &str {
        &self.stage
    }

    /// `message` with the real name in place of the staging.
    pub(crate) fn reword(&self, message: &str) -> String {
        self.replacements
            .iter()
            .fold(message.to_owned(), |message, (staged, real)| {
                message.replace(staged, real)
            })
    }

    /// `error` naming the real item. A location the user must recover by
    /// hand stays exactly as reported.
    pub(crate) fn apply(&self, error: TransferError) -> TransferError {
        match error {
            TransferError::NotFound(message) => TransferError::NotFound(self.reword(&message)),
            TransferError::Exists(message) => TransferError::Exists(self.reword(&message)),
            TransferError::NotMounted(message) => TransferError::NotMounted(self.reword(&message)),
            TransferError::NotSupported(message) => TransferError::NotSupported(self.reword(&message)),
            TransferError::ReplaceUnsupported(message) => {
                TransferError::ReplaceUnsupported(self.reword(&message))
            }
            TransferError::Failed(message) => TransferError::Failed(self.reword(&message)),
            TransferError::Cancelled | TransferError::RecoveryRequired(_) => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stage of one copy, named like the engine's.
    fn copy_of_one() -> StagedName {
        let stage = format!(".winspace-transfer-{}.part", "a".repeat(32));
        StagedName::new(&stage, &[OsStr::new("payload")], "one.txt")
    }

    /// parity: XFER-001
    #[test]
    fn staged_paths_become_the_real_path_in_every_spelling() {
        let name = copy_of_one();
        let stage = name.stage().to_owned();
        let cases = [
            (
                format!("Error creating directory /home/user/Locked/{stage}: Permission denied"),
                "Error creating directory /home/user/Locked/one.txt: Permission denied",
            ),
            (
                format!("Error opening file /home/user/Locked/{stage}/payload: Permission denied"),
                "Error opening file /home/user/Locked/one.txt: Permission denied",
            ),
            (
                format!("Error opening file smb://nas.example/Share/{stage}/payload/sub/b.txt: No space"),
                "Error opening file smb://nas.example/Share/one.txt/sub/b.txt: No space",
            ),
            (format!("{stage} is busy"), "one.txt is busy"),
        ];
        for (staged, real) in cases {
            assert_eq!(name.reword(&staged), real);
        }
    }

    /// parity: ARC-013
    #[test]
    fn private_folders_inside_an_existing_folder_are_left_out_of_paths() {
        let stage = format!(".openxplorer-extract-{}.part", "b".repeat(32));
        let private = format!(".openxplorer-extract-{}", "c".repeat(32));
        let name = StagedName::inside(&stage, &private, "Target");
        let cases = [
            (
                format!("Error creating directory /home/user/Target/{stage}: Permission denied"),
                "Error creating directory /home/user/Target: Permission denied".to_owned(),
            ),
            (
                format!("Error opening file smb://nas.example/Share/Target/{stage}/Docs/a.txt: No space"),
                "Error opening file smb://nas.example/Share/Target/Docs/a.txt: No space".to_owned(),
            ),
            (
                format!("Error moving {stage} to /home/user/Target/{private}: Busy"),
                "Error moving Target to /home/user/Target: Busy".to_owned(),
            ),
        ];
        for (staged, real) in cases {
            assert_eq!(name.reword(&staged), real);
        }
    }

    /// parity: XFER-001
    #[test]
    fn messages_without_staging_are_kept() {
        let name = copy_of_one();
        let message = "Error opening file /home/user/source/payload: Permission denied";

        assert_eq!(name.reword(message), message);
    }

    /// parity: XFER-001, XFER-003
    #[test]
    fn the_kind_of_error_is_kept_and_recovery_locations_stay_exact() {
        let name = copy_of_one();
        let staged = format!("/Locked/{}", name.stage());

        assert_eq!(
            name.apply(TransferError::Exists(staged.clone())),
            TransferError::Exists("/Locked/one.txt".into())
        );
        assert_eq!(
            name.apply(TransferError::NotMounted(staged.clone())),
            TransferError::NotMounted("/Locked/one.txt".into())
        );
        assert_eq!(name.apply(TransferError::Cancelled), TransferError::Cancelled);
        let recovery = TransferError::RecoveryRequired(staged);
        assert_eq!(name.apply(recovery.clone()), recovery);
    }
}
