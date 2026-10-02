// SPDX-License-Identifier: AGPL-3.0-only
//! Validating a run's request before anything changes: the operation and
//! its destination folder, the number of items, and items selected twice.
//! Ports the checks at the start of `TransferEngine.run` in
//! `v2.0.0:desktop/operations.py` (XFER-019).

use std::collections::HashSet;

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::node::{Node, NodeFactory};
use super::types::{ConflictPolicy, Operation, TransferMode};

/// The most items one run accepts.
pub const MAX_ITEMS: usize = 100_000;

impl<'a> Operation<'a> {
    /// The operation of a request in the app's protocol: a mode, the URI
    /// of the destination folder (`target`) and a conflict policy, as the
    /// Python bridge passes them to `TransferEngine.run`. Trash and delete
    /// ignore `target` and `policy`, like the Python engine.
    ///
    /// The Python engine counts the items first, so a request with neither
    /// items nor a destination is refused here with the destination's
    /// message rather than the items' one.
    ///
    /// # Errors
    ///
    /// A copy or move without a destination folder.
    pub fn from_request(
        mode: TransferMode,
        target: Option<&'a str>,
        policy: ConflictPolicy,
    ) -> Result<Self, TransferError> {
        let operation = match (mode, target) {
            (TransferMode::Trash, _) => Operation::Trash,
            (TransferMode::Delete, _) => Operation::Delete,
            (TransferMode::Copy | TransferMode::Move, None) => return Err(missing_destination()),
            (TransferMode::Copy, Some(destination_folder)) => Operation::Copy {
                destination_folder,
                policy,
            },
            (TransferMode::Move, Some(destination_folder)) => Operation::Move {
                destination_folder,
                policy,
            },
        };
        Ok(operation)
    }
}

/// The refusal of a copy or move that names no destination folder.
fn missing_destination() -> TransferError {
    TransferError::failed(crate::i18n::gettext("Choose a destination folder."))
}

/// The selected URIs in their original order, each once.
///
/// # Errors
///
/// No items, or more than [`MAX_ITEMS`].
pub(crate) fn distinct_items(uris: &[String]) -> Result<Vec<&str>, TransferError> {
    if uris.is_empty() || uris.len() > MAX_ITEMS {
        return Err(TransferError::failed(crate::i18n::gettext(
            "Select between 1 and 100,000 items.",
        )));
    }
    let mut seen = HashSet::new();
    let distinct = uris
        .iter()
        .map(String::as_str)
        .filter(|uri| seen.insert(*uri))
        .collect();
    Ok(distinct)
}

/// The destination folder of a copy or move at `uri`, resolved with
/// `factory`.
///
/// # Errors
///
/// An empty URI, a destination that is not a folder, or a failure or
/// cancellation while it is checked.
pub(crate) fn destination_folder(
    factory: &NodeFactory,
    uri: &str,
    cancel: &Cancellation,
) -> Result<Box<dyn Node>, TransferError> {
    if uri.is_empty() {
        return Err(missing_destination());
    }
    let folder = factory(uri)?;
    if !folder.is_directory(Some(cancel))? {
        return Err(TransferError::failed(crate::i18n::gettext(
            "The destination is not a folder.",
        )));
    }
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::gio_node::GioNode;
    use crate::test_support::temporary_folder;

    /// Resolves every URI with the production GIO adapter.
    fn gio_factory() -> NodeFactory {
        Arc::new(|uri: &str| Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>))
    }

    /// The selection `names`, as the engine receives it.
    fn uris(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    /// Ported from `v2.0.0:desktop/tests/test_operations.py::TransferTests::test_duplicate_sources_deduplicated`,
    /// at the level of the request.
    ///
    /// parity: XFER-019
    #[test]
    fn duplicates_are_dropped_in_order() {
        let selected = uris(&["b", "a", "b", "c", "a"]);

        let distinct = distinct_items(&selected);

        assert_eq!(distinct, Ok(vec!["b", "a", "c"]));
    }

    /// parity: XFER-019
    #[test]
    fn an_empty_or_oversized_selection_is_refused() {
        let refusal = Err(TransferError::failed("Select between 1 and 100,000 items."));
        let oversized = vec![String::from("file:///tmp/a"); MAX_ITEMS + 1];

        assert_eq!(distinct_items(&[]), refusal);
        assert_eq!(distinct_items(&oversized), refusal);
        assert!(distinct_items(&oversized[..MAX_ITEMS]).is_ok());
    }

    /// parity: XFER-019
    #[test]
    fn copies_and_moves_need_a_destination_folder() {
        for mode in [TransferMode::Copy, TransferMode::Move] {
            let missing = Operation::from_request(mode, None, ConflictPolicy::Skip);

            let choose = TransferError::failed("Choose a destination folder.");
            assert_eq!(missing, Err(choose), "{mode:?}");
        }
    }

    /// parity: XFER-019
    #[test]
    fn the_destination_must_be_an_existing_folder() {
        let root = temporary_folder();
        let file = root.path().join("file");
        std::fs::write(&file, b"not a folder").expect("write a file");
        let file_uri = GioNode::from_file(gio::File::for_path(&file)).uri();
        let folder_uri = GioNode::from_file(gio::File::for_path(root.path())).uri();
        let factory = gio_factory();
        let cancel = Cancellation::new();

        let empty = destination_folder(&factory, "", &cancel).err();
        let not_folder = destination_folder(&factory, &file_uri, &cancel).err();
        let folder = destination_folder(&factory, &folder_uri, &cancel);

        let choose = TransferError::failed("Choose a destination folder.");
        assert_eq!(empty, Some(choose));
        let refusal = TransferError::failed("The destination is not a folder.");
        assert_eq!(not_folder, Some(refusal));
        let resolved = folder.expect("a folder is accepted");
        assert_eq!(resolved.uri(), folder_uri);
    }

    /// parity: XFER-019
    #[test]
    fn a_copy_or_move_request_keeps_its_destination_and_policy() {
        let target = Some("file:///tmp");

        let copy = Operation::from_request(TransferMode::Copy, target, ConflictPolicy::Replace);
        let moved = Operation::from_request(TransferMode::Move, target, ConflictPolicy::KeepBoth);

        let expected_copy = Operation::Copy {
            destination_folder: "file:///tmp",
            policy: ConflictPolicy::Replace,
        };
        let expected_move = Operation::Move {
            destination_folder: "file:///tmp",
            policy: ConflictPolicy::KeepBoth,
        };
        assert_eq!(copy, Ok(expected_copy));
        assert_eq!(moved, Ok(expected_move));
    }

    /// parity: XFER-015, XFER-019
    #[test]
    fn trash_and_delete_take_no_destination() {
        for mode in [TransferMode::Trash, TransferMode::Delete] {
            let with_target = Operation::from_request(mode, Some("file:///tmp"), ConflictPolicy::Skip)
                .expect("a removal is accepted with a target");
            let without_target = Operation::from_request(mode, None, ConflictPolicy::Skip)
                .expect("a removal is accepted without a target");

            assert!(
                matches!(with_target, Operation::Trash | Operation::Delete),
                "{mode:?}"
            );
            assert_eq!(with_target.mode(), mode);
            assert_eq!(without_target, with_target, "{mode:?}");
        }
    }
}
