// SPDX-License-Identifier: AGPL-3.0-only
//! The user's consent to finish moves by copying (XFER-011 and XFER-013).
//!
//! Where a backend cannot move an item natively (another file system,
//! share or device), the Python app refused the move and kept the source
//! (XFER-011); Dolphin copies and then deletes. The engine does the latter
//! only when the app asked the user and the user agreed, once per
//! operation. Without a question installed, or when the user declines,
//! such a move is refused as before and the source is kept.

/// A move the backend cannot do natively, as the question shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveByCopyingItem {
    /// The first item that cannot be moved natively.
    pub name: String,
    /// The folder it was to be moved into.
    pub destination: String,
}

/// Asks the user whether moves the backend cannot do natively are finished
/// by copying and then removing the originals; runs on the engine's worker
/// thread. `true` agrees for the rest of the operation.
pub type MoveByCopyingQuestion = dyn FnMut(&MoveByCopyingItem) -> bool + Send;

/// The question and the answer given in this operation.
#[derive(Default)]
pub(crate) struct MoveByCopying {
    question: Option<Box<MoveByCopyingQuestion>>,
    answer: Option<bool>,
}

impl MoveByCopying {
    /// Asks `question` the first time a move cannot be done natively.
    pub(crate) fn with_question(question: Box<MoveByCopyingQuestion>) -> Self {
        Self {
            question: Some(question),
            answer: None,
        }
    }

    /// True when the user agreed that `item`, and every later such move of
    /// the operation, is finished by copying. The user is asked once.
    pub(crate) fn allowed(&mut self, item: impl FnOnce() -> MoveByCopyingItem) -> bool {
        if let Some(answer) = self.answer {
            return answer;
        }
        let Some(question) = self.question.as_mut() else {
            return false;
        };
        let answer = question(&item());
        self.answer = Some(answer);
        answer
    }
}
