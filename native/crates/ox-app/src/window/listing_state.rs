// SPDX-License-Identifier: AGPL-3.0-only
//! Where a tab's listing stands: never listed, listing or listed.
//!
//! Ports each tab's `loaded` and `busy` flags in `desktop/ui/app.js`, and
//! the folder monitor's rule in `desktop/winspace.py` that a change seen
//! while a folder is listed lists it once more afterwards. One enum holds
//! what were three flags, so a pending reload can only exist while a
//! listing runs.

/// Where a tab's listing stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ListingState {
    /// Never listed: a tab opened in the background stays so until it is
    /// first shown.
    #[default]
    NotListed,
    /// A listing runs.
    Listing {
        /// An earlier listing finished, so the tab has been listed before.
        listed_before: bool,
        /// The folder changed while this listing ran; it is listed again
        /// once this one ends.
        reload_pending: bool,
    },
    /// The last listing finished. A landing page counts as listed.
    Listed,
}

/// What follows the end of a tab's listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListingEnd {
    /// The tab closed while it was listed, so nothing is left to show.
    TabClosed,
    /// The folder is current.
    Done,
    /// The folder changed while it was listed, so it is listed again.
    ListAgain,
}

/// When a tab whose folder changed is listed again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReloadTiming {
    /// No listing runs, so the folder is listed now.
    Now,
    /// A listing runs; the folder is listed again once it ends.
    AfterRunningListing,
}

impl ListingState {
    /// Whether the last listing finished and no other runs.
    pub fn is_listed(self) -> bool {
        self == ListingState::Listed
    }

    /// Whether a listing runs.
    pub fn is_listing(self) -> bool {
        matches!(self, ListingState::Listing { .. })
    }

    /// Whether the tab has never been listed and no listing runs: a
    /// background tab that is shown for the first time.
    pub fn needs_listing(self) -> bool {
        self == ListingState::NotListed
    }

    /// Whether a change seen during the running listing waits for it.
    pub fn has_pending_reload(self) -> bool {
        matches!(
            self,
            ListingState::Listing {
                reload_pending: true,
                ..
            }
        )
    }

    /// Whether any listing has finished, even if another runs now.
    fn has_been_listed(self) -> bool {
        match self {
            ListingState::NotListed => false,
            ListingState::Listing { listed_before, .. } => listed_before,
            ListingState::Listed => true,
        }
    }

    /// A listing begins. It replaces any listing that ran, and it covers
    /// a reload that was pending.
    pub fn begin(&mut self) {
        *self = ListingState::Listing {
            listed_before: self.has_been_listed(),
            reload_pending: false,
        };
    }

    /// The running listing stops without finishing, so the tab stands as
    /// it did before the listing began.
    pub fn stop(&mut self) {
        *self = if self.has_been_listed() {
            ListingState::Listed
        } else {
            ListingState::NotListed
        };
    }

    /// The running listing finished: [`ListingEnd::ListAgain`] when the
    /// folder changed while it ran, else [`ListingEnd::Done`].
    pub fn finish(&mut self) -> ListingEnd {
        let list_again = self.has_pending_reload();
        *self = ListingState::Listed;
        if list_again {
            ListingEnd::ListAgain
        } else {
            ListingEnd::Done
        }
    }

    /// The watched folder changed. A running listing may have read the
    /// folder before the change, so it is listed again after that one.
    pub fn schedule_reload(&mut self) -> ReloadTiming {
        let ListingState::Listing { reload_pending, .. } = self else {
            return ReloadTiming::Now;
        };
        *reload_pending = true;
        ReloadTiming::AfterRunningListing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_listing_that_finishes_leaves_the_tab_listed() {
        let mut state = ListingState::default();
        assert!(state.needs_listing(), "a new tab has never been listed");
        state.begin();
        assert!(state.is_listing());
        assert!(!state.needs_listing(), "a running listing is enough");
        assert_eq!(state.finish(), ListingEnd::Done);
        assert!(state.is_listed());
    }

    /// parity: VIEW-055
    #[test]
    fn a_change_seen_while_listing_lists_the_folder_again_after() {
        let mut state = ListingState::default();
        state.begin();
        assert_eq!(state.schedule_reload(), ReloadTiming::AfterRunningListing);
        assert!(state.has_pending_reload());
        assert_eq!(state.finish(), ListingEnd::ListAgain);
        assert_eq!(state.schedule_reload(), ReloadTiming::Now, "nothing runs now");
        assert!(state.is_listed(), "a reload that runs now does not wait");
    }

    #[test]
    fn a_new_listing_covers_a_pending_reload() {
        let mut state = ListingState::default();
        state.begin();
        state.schedule_reload();
        state.begin();
        assert!(!state.has_pending_reload());
        assert_eq!(state.finish(), ListingEnd::Done);
    }

    #[test]
    fn a_stopped_listing_leaves_the_tab_as_it_was_before() {
        let mut first = ListingState::default();
        first.begin();
        first.stop();
        assert!(first.needs_listing(), "a tab never listed stays unlisted");
        let mut again = ListingState::Listed;
        again.begin();
        again.stop();
        assert!(again.is_listed(), "a tab listed before stays listed");
    }
}
