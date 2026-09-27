// SPDX-License-Identifier: AGPL-3.0-only
//! Validating a run's request before anything changes: the number of items,
//! items selected twice, and the destination folder. Ports the checks at the
//! start of `TransferEngine.run` in `desktop/operations.py` (XFER-019).

use std::collections::HashSet;

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::node::{Node, NodeFactory};
use super::types::TransferMode;

/// The most items one run accepts.
pub const MAX_ITEMS: usize = 100_000;

/// The selected URIs in their original order, each once.
///
/// # Errors
///
/// No items, or more than [`MAX_ITEMS`].
pub(crate) fn distinct_items(uris: &[String]) -> Result<Vec<String>, TransferError> {
    if uris.is_empty() || uris.len() > MAX_ITEMS {
        return Err(TransferError::failed("Select between 1 and 100,000 items."));
    }
    let mut seen = HashSet::new();
    let distinct = uris
        .iter()
        .filter(|uri| seen.insert(uri.as_str()))
        .cloned()
        .collect();
    Ok(distinct)
}

/// The destination folder of a copy or move, resolved with `factory`;
/// `None` for Trash and delete, which take no destination.
///
/// # Errors
///
/// No destination, a destination that is not a folder, or a failure or
/// cancellation while it is checked.
pub(crate) fn destination_folder(
    factory: &NodeFactory,
    mode: TransferMode,
    target: Option<&str>,
    cancel: &Cancellation,
) -> Result<Option<Box<dyn Node>>, TransferError> {
    if mode.is_removal() {
        return Ok(None);
    }
    let Some(target) = target.filter(|target| !target.is_empty()) else {
        return Err(TransferError::failed("Choose a destination folder."));
    };
    let folder = factory(target)?;
    if !folder.is_directory(Some(cancel))? {
        return Err(TransferError::failed("The destination is not a folder."));
    }
    Ok(Some(folder))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::gio_node::GioNode;

    fn gio_factory() -> NodeFactory {
        Arc::new(|uri: &str| Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>))
    }

    fn uris(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    /// Port of `test_duplicate_sources_deduplicated` in
    /// `desktop/tests/test_operations.py`, at the level of the request.
    ///
    /// parity: XFER-019
    #[test]
    fn duplicates_are_dropped_in_order() {
        let selected = uris(&["b", "a", "b", "c", "a"]);

        let distinct = distinct_items(&selected);

        assert_eq!(distinct, Ok(uris(&["b", "a", "c"])));
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
    fn copies_and_moves_need_an_existing_destination_folder() {
        let temp = tempfile::tempdir().expect("a temp dir");
        let file = temp.path().join("file");
        std::fs::write(&file, b"not a folder").expect("write a file");
        let file_uri = GioNode::from_file(gio::File::for_path(&file)).uri();
        let folder_uri = GioNode::from_file(gio::File::for_path(temp.path())).uri();
        let factory = gio_factory();
        let cancel = Cancellation::new();

        for mode in [TransferMode::Copy, TransferMode::Move] {
            let missing = destination_folder(&factory, mode, None, &cancel).err();
            let empty = destination_folder(&factory, mode, Some(""), &cancel).err();
            let not_folder = destination_folder(&factory, mode, Some(&file_uri), &cancel).err();
            let folder = destination_folder(&factory, mode, Some(&folder_uri), &cancel);

            let choose = TransferError::failed("Choose a destination folder.");
            assert_eq!(missing, Some(choose.clone()), "{mode:?}");
            assert_eq!(empty, Some(choose), "{mode:?}");
            let refusal = TransferError::failed("The destination is not a folder.");
            assert_eq!(not_folder, Some(refusal), "{mode:?}");
            let resolved = folder
                .expect("a folder is accepted")
                .expect("copies have a folder");
            assert_eq!(resolved.uri(), folder_uri);
        }
    }

    /// parity: XFER-015, XFER-019
    #[test]
    fn trash_and_delete_take_no_destination() {
        let factory = gio_factory();
        let cancel = Cancellation::new();

        for mode in [TransferMode::Trash, TransferMode::Delete] {
            let folder = destination_folder(&factory, mode, Some("file:///tmp"), &cancel);

            assert!(matches!(folder, Ok(None)), "{mode:?}");
        }
    }
}
