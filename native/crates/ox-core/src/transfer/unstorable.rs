// SPDX-License-Identifier: AGPL-3.0-only
//! Items the destination's file system cannot store (XFER-028): names with
//! characters FAT, exFAT and NTFS forbid, and symbolic links on FAT and
//! exFAT. The engine asks the app about each one, as KIO's
//! `handleMsdosFsQuirks` asks Dolphin's user: "Replace invalid characters"
//! (with `_`), "Replace all", "Skip", "Skip all" or "Cancel". Without a
//! question installed, the item is attempted as it is and the backend's
//! error is reported, as before.

use std::ffi::{OsStr, OsString};

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::limits::{replace_forbidden_characters, StorageRules};
use super::node::{Node, NodeKind};

/// Why an item cannot be stored as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnstorableReason {
    /// Its name has characters the file system forbids.
    InvalidCharacters,
    /// It is a symbolic link, which the file system cannot store.
    SymbolicLink,
}

/// An item the destination cannot store, as the question shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnstorableItem {
    /// The item's name.
    pub name: String,
    /// Why it cannot be stored.
    pub reason: UnstorableReason,
}

/// The user's answer about an [`UnstorableItem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnstorableAnswer {
    /// Replace the forbidden characters of this name with `_`.
    Replace,
    /// Do that for every such name of the run.
    ReplaceAll,
    /// Leave this item out.
    Skip,
    /// Leave out every item of the run with the same problem.
    SkipAll,
    /// Stop the run.
    Cancel,
}

/// Asks the user about an item; runs on the engine's worker thread.
pub type UnstorableQuestion = dyn FnMut(&UnstorableItem) -> UnstorableAnswer + Send;

/// What happens to one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fix {
    /// It is copied under this name.
    Name(OsString),
    /// It is left out.
    Skip,
}

/// The "all" answers that apply to the rest of a run.
#[derive(Debug, Default)]
struct RunAnswers {
    /// Replace all: every forbidden name gets `_` instead.
    replace_all: bool,
    /// Skip all for names.
    skip_all_names: bool,
    /// Skip all for links.
    skip_all_links: bool,
}

/// The rules of the run's destination and the answers that apply to the
/// rest of the run.
#[derive(Default)]
pub(crate) struct Unstorable {
    question: Option<Box<UnstorableQuestion>>,
    pub(crate) rules: StorageRules,
    /// The destination's `id::filesystem`, when known.
    destination_id: Option<String>,
    /// True while the current item is on the destination's own file
    /// system, which already stores its names and links.
    same_filesystem: bool,
    /// The "all" answers given so far in this run.
    answers: RunAnswers,
    /// Items left out since [`Unstorable::take_skipped`] was last called.
    skipped: usize,
}

impl Unstorable {
    /// Asks `question` about the items a destination cannot store.
    pub(crate) fn with_question(question: Box<UnstorableQuestion>) -> Self {
        Self {
            question: Some(question),
            ..Self::default()
        }
    }

    /// Starts a run into a file system with `rules` and the id
    /// `destination_id`; earlier "all" answers no longer apply.
    pub(crate) fn start_run(&mut self, rules: StorageRules, destination_id: Option<String>) {
        self.rules = rules;
        self.destination_id = destination_id;
        self.same_filesystem = false;
        self.answers = RunAnswers::default();
        self.skipped = 0;
    }

    /// Starts the top-level item `source`. An item already on the
    /// destination's file system is stored there as it is, so nothing about
    /// it is asked: `fuseblk` names both NTFS and exFAT through FUSE, and
    /// ntfs-3g without `windows_names` stores the characters Windows forbids.
    pub(crate) fn start_item(&mut self, source: &dyn Node, cancel: &Cancellation) {
        self.skipped = 0;
        let restricts = self.rules != StorageRules::default();
        self.same_filesystem = restricts
            && self.destination_id.is_some()
            && source.filesystem(Some(cancel)).and_then(|info| info.id) == self.destination_id;
    }

    /// The number of items left out since the last call.
    pub(crate) fn take_skipped(&mut self) -> usize {
        std::mem::take(&mut self.skipped)
    }

    /// The name `item` gets in the destination, or [`Fix::Skip`]. `kind` is
    /// the item's kind when known; a link's kind is queried when the
    /// destination cannot store links.
    ///
    /// # Errors
    ///
    /// [`TransferError::Cancelled`] when the user answered Cancel (the
    /// whole run is cancelled), or a failure to inspect the item.
    pub(crate) fn fix(
        &mut self,
        item: &dyn Node,
        kind: Option<NodeKind>,
        cancel: &Cancellation,
    ) -> Result<Fix, TransferError> {
        let name = item.name();
        if self.same_filesystem {
            return Ok(Fix::Name(name));
        }
        if !self.rules.stores_links && self.question.is_some() {
            let kind = match kind {
                Some(kind) => kind,
                None => item.info(Some(cancel))?.kind,
            };
            if kind == NodeKind::Symlink && self.skips_link(&name, cancel)? {
                self.skipped += 1;
                return Ok(Fix::Skip);
            }
        }
        if !self.rules.forbids_name(&name) || self.question.is_none() {
            return Ok(Fix::Name(name));
        }
        if self.answers.replace_all {
            return Ok(Fix::Name(replace_forbidden_characters(&name)));
        }
        if self.answers.skip_all_names {
            self.skipped += 1;
            return Ok(Fix::Skip);
        }
        match self.ask(&name, UnstorableReason::InvalidCharacters, cancel)? {
            UnstorableAnswer::ReplaceAll => {
                self.answers.replace_all = true;
                Ok(Fix::Name(replace_forbidden_characters(&name)))
            }
            UnstorableAnswer::Replace => Ok(Fix::Name(replace_forbidden_characters(&name))),
            answer => {
                self.answers.skip_all_names |= answer == UnstorableAnswer::SkipAll;
                self.skipped += 1;
                Ok(Fix::Skip)
            }
        }
    }

    /// True when the link `name` is left out; links cannot be renamed into
    /// something the file system stores, so every answer but Cancel skips.
    fn skips_link(&mut self, name: &OsStr, cancel: &Cancellation) -> Result<bool, TransferError> {
        if !self.answers.skip_all_links {
            let answer = self.ask(name, UnstorableReason::SymbolicLink, cancel)?;
            self.answers.skip_all_links = answer == UnstorableAnswer::SkipAll;
        }
        Ok(true)
    }

    /// Asks the question; Cancel cancels the whole run.
    fn ask(
        &mut self,
        name: &OsStr,
        reason: UnstorableReason,
        cancel: &Cancellation,
    ) -> Result<UnstorableAnswer, TransferError> {
        cancel.check()?;
        let Some(question) = self.question.as_mut() else {
            return Ok(UnstorableAnswer::Skip);
        };
        let item = UnstorableItem {
            name: name.to_string_lossy().into_owned(),
            reason,
        };
        let answer = question(&item);
        if answer == UnstorableAnswer::Cancel {
            cancel.cancel();
            return Err(TransferError::Cancelled);
        }
        Ok(answer)
    }
}
