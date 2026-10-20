// SPDX-License-Identifier: AGPL-3.0-only
//! A move must retain originals whose partial destination subtree was discarded.

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use ox_core::transfer::{Cancellation, ConflictPolicy, FailureAnswer, Node, TransferError};

use crate::transfer_support::local::{LocalNode, Provider};
use crate::transfer_support::{is_staging_path, Fixture, Request};

/// A cross-filesystem move with a failed copy and one transient cleanup failure.
#[derive(Default)]
struct PartialSubtree {
    copies: AtomicUsize,
    cleanup_failed: AtomicBool,
}

impl Provider for PartialSubtree {
    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if !node.local_path().ancestors().any(is_staging_path) {
            return Err(TransferError::NotSupported(
                "Simulated cross-filesystem move.".into(),
            ));
        }
        node.local_move_native(target, cancel)
    }

    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        if self.copies.fetch_add(1, Ordering::SeqCst) == 1 {
            fs::write(target.path().expect("local test destination"), "partial")?;
            return Err(TransferError::failed("Injected second-file copy failure."));
        }
        node.local_copy_file(target, cancel, progress)
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if node.local_path().ancestors().any(is_staging_path)
            && !self.cleanup_failed.swap(true, Ordering::SeqCst)
        {
            return Err(TransferError::failed("Injected transient cleanup failure."));
        }
        node.local_delete()
    }
}

/// Skipping a file whose cleanup fails escalates to skipping its parent.
/// Removing that parent's staging must also remove its completed-copy records.
#[test]
fn skipping_a_partial_subtree_never_removes_its_originals() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("tree");
    let nested = source.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("first.txt"), "first original").unwrap();
    fs::write(nested.join("second.txt"), "second original").unwrap();
    let mut engine = fixture
        .engine(Arc::new(PartialSubtree::default()))
        .with_move_by_copying_question(|_| true)
        .with_failure_question(|_| FailureAnswer::Skip);

    let result = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

    assert_eq!(result.done.len(), 1, "{result:?}");
    assert_eq!(
        fs::read_to_string(nested.join("first.txt")).unwrap(),
        "first original"
    );
    assert_eq!(
        fs::read_to_string(nested.join("second.txt")).unwrap(),
        "second original"
    );
    assert!(!fixture.destination_folder.join("tree/nested").exists());
    fixture.assert_no_staging();
}
