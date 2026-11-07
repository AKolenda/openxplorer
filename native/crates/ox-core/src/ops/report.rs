// SPDX-License-Identifier: AGPL-3.0-only
//! What the user is told when an operation ends (OPS-023).
//!
//! Nothing, as in Windows Explorer, which has no summary of a file
//! operation: complete success says nothing, and every item that fails is
//! asked about while the operation runs, in its progress window (OPS-047),
//! so nothing is left to say afterwards. The 1.x app's "Operation result"
//! dialog is gone. Only a request refused before any item ran, such as a
//! destination that does not exist, opens "Operation stopped" with the
//! refusal.

/// The title of the dialog for a request that failed before any item was
/// handled; its text is the error's message.
pub const STOPPED_TITLE: &str = crate::i18n::message_id("Operation stopped");
