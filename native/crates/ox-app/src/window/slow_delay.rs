// SPDX-License-Identifier: AGPL-3.0-only
//! When a tab's location counts as slow: once it has taken longer than
//! [`APPEARANCE_DELAY`] to open and list, counted from the moment the
//! user asked for it. Only then does the file area say "Working on it..."
//! and the status bar "Loading…", so a location that answers sooner
//! flashes nothing (ui-spec.md M06). The progress itself shows in the
//! address bar from the first moment, as in Windows Explorer (VIEW-068);
//! nothing is drawn over the items.
//!
//! Each tab keeps its own [`BusyClock`], so a tab shown again is as slow
//! as it was, and the time runs on from a typed address's lookup into its
//! listing.

use std::time::{Duration, Instant};

/// How long a location takes before it counts as slow. Shorter listings,
/// which are most of them, show nothing in the file area: Dolphin waits
/// as long before it says a folder is loading.
pub(crate) const APPEARANCE_DELAY: Duration = Duration::from_millis(300);

/// Since when a tab has been busy opening or listing its location.
#[derive(Debug, Default)]
pub(crate) struct BusyClock {
    /// When the tab became busy; `None` before the first time.
    since: Option<Instant>,
    /// A lookup has just answered and its tab moves on to list what it
    /// found: the listing goes on from the lookup's time.
    handing_over: bool,
    /// A redraw is due when the location becomes slow.
    pub(crate) redraw_pending: bool,
}

impl BusyClock {
    /// The tab starts opening or listing a location; `was_busy` says
    /// whether it already was busy, which keeps the time running.
    pub(crate) fn start(&mut self, was_busy: bool, now: Instant) {
        let goes_on = was_busy || self.handing_over;
        if !goes_on || self.since.is_none() {
            self.since = Some(now);
        }
        self.handing_over = false;
    }

    /// A lookup answered; the listing it may start goes on from its time.
    pub(crate) fn hand_over(&mut self) {
        self.handing_over = true;
    }

    /// The answer of a lookup has been acted on: anything started later
    /// counts from its own start.
    pub(crate) fn end_hand_over(&mut self) {
        self.handing_over = false;
    }

    /// Keeps the clock in step with whether the tab is `busy`: a tab seen
    /// idle forgets its time, unless a lookup is handing over to its
    /// listing, and a tab made busy without [`Self::start`] counts from
    /// now.
    pub(crate) fn follow(&mut self, busy: bool, now: Instant) {
        if busy {
            self.since.get_or_insert(now);
        } else if !self.handing_over {
            self.since = None;
        }
    }

    /// Whether the location of a `busy` tab has taken longer than the
    /// delay.
    pub(crate) fn is_slow(&self, busy: bool, now: Instant) -> bool {
        busy && self
            .since
            .is_some_and(|since| now.saturating_duration_since(since) >= APPEARANCE_DELAY)
    }

    /// How long until the location of a `busy` tab becomes slow; `None`
    /// when it is slow already or the tab is not busy.
    pub(crate) fn until_slow(&self, busy: bool, now: Instant) -> Option<Duration> {
        let since = self.since.filter(|_| busy)?;
        let left = APPEARANCE_DELAY.saturating_sub(now.saturating_duration_since(since));
        (!left.is_zero()).then_some(left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: VIEW-047
    #[test]
    fn a_location_counts_as_slow_once_it_has_run_for_the_delay() {
        let start = Instant::now();
        let mut clock = BusyClock::default();
        clock.start(false, start);
        assert!(!clock.is_slow(true, start), "a location that just started");
        assert_eq!(clock.until_slow(true, start), Some(APPEARANCE_DELAY));
        let later = start + APPEARANCE_DELAY;
        assert!(clock.is_slow(true, later));
        assert_eq!(clock.until_slow(true, later), None);
        assert!(!clock.is_slow(false, later), "a tab that is done is not slow");
    }

    /// parity: VIEW-068
    #[test]
    fn the_time_runs_on_from_a_lookup_into_its_listing_only() {
        let start = Instant::now();
        let mut clock = BusyClock::default();
        clock.start(false, start);
        clock.hand_over();
        let listing = start + APPEARANCE_DELAY / 2;
        clock.start(false, listing);
        assert!(clock.is_slow(true, start + APPEARANCE_DELAY), "from Enter");

        clock.end_hand_over();
        let next = start + APPEARANCE_DELAY * 4;
        clock.start(false, next);
        assert!(
            !clock.is_slow(true, next + APPEARANCE_DELAY / 2),
            "a new location counts anew"
        );
        clock.start(true, next + APPEARANCE_DELAY);
        assert!(
            clock.is_slow(true, next + APPEARANCE_DELAY),
            "a busy tab keeps its time"
        );
    }

    #[test]
    fn a_tab_seen_busy_or_idle_without_a_start_follows_it() {
        let start = Instant::now();
        let mut clock = BusyClock::default();
        clock.follow(false, start);
        assert!(!clock.is_slow(true, start + APPEARANCE_DELAY));
        clock.follow(true, start);
        assert!(clock.is_slow(true, start + APPEARANCE_DELAY));

        // Seen idle, it forgets that time.
        clock.follow(false, start + APPEARANCE_DELAY);
        let again = start + APPEARANCE_DELAY * 2;
        clock.follow(true, again);
        assert!(!clock.is_slow(true, again), "busy again counts anew");

        // Not while a lookup hands over to its listing.
        clock.hand_over();
        clock.follow(false, again + APPEARANCE_DELAY);
        clock.start(false, again + APPEARANCE_DELAY);
        assert!(clock.is_slow(true, again + APPEARANCE_DELAY));
    }
}
