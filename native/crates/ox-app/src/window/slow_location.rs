// SPDX-License-Identifier: AGPL-3.0-only
//! Showing that a tab is opening a location, as Windows Explorer does
//! (VIEW-068).
//!
//! A tab is busy from the moment the user asks for a location until it is
//! listed: while a typed address or a folder opened from the list is
//! looked up, while its share is mounted (with any sign-in), and while it
//! is listed. As in Explorer, from the first moment:
//!
//! - the address bar shows its progress, a fill crawling along behind the
//!   address (still, with animations off), and says it is busy to screen
//!   readers;
//! - Refresh turns into Stop, which cancels whatever runs.
//!
//! Once the location has taken longer than
//! [`APPEARANCE_DELAY`](super::slow_delay::APPEARANCE_DELAY), the file
//! area says "Working on it..." where Explorer says it, near the top of
//! the list, in place of the previous folder's rows; a location that
//! answers sooner never flashes it (see `pane_content.rs`).
//!
//! Everything is per tab: another tab or pane stays usable, and a tab
//! keeps its state while the user looks at another one. A lookup in a
//! tab the user left may still open its folder there, but never launches
//! a file (SAFE-013).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::loader::Listing;

use super::session::TabId;
use super::BrowserWindow;

/// What a tab is going to before it moves there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpeningKind {
    /// An address typed into the address bar.
    Typed,
    /// An address pasted and gone to: a file it names is shown in its
    /// folder rather than opened.
    Pasted,
    /// A folder opened from the list, which marks the tab as activating.
    Item,
}

/// A location a tab is going to: looked up, or mounted, before the tab
/// moves there. Dropping it cancels the work.
#[derive(Debug)]
pub(in crate::window) struct Opening {
    /// The location, as the lookup resolved it.
    target: String,
    kind: OpeningKind,
    /// The lookup; dropping it cancels it.
    work: Listing,
}

impl Opening {
    /// The location the tab is going to.
    pub(in crate::window) fn target(&self) -> &str {
        &self.target
    }
}

/// The CSS class of the address bar while the tab in front is busy.
const LOADING_CLASS: &str = "loading";

impl BrowserWindow {
    /// Whether tab `id` is opening or listing a location.
    pub(super) fn is_tab_busy(&self, id: TabId) -> bool {
        let session = self.imp().session.borrow();
        session.tab(id).is_some_and(super::session::Tab::is_busy)
    }

    /// Whether the tab in front has taken longer than the delay to open
    /// and list its location.
    pub(super) fn is_slow(&self) -> bool {
        let session = self.imp().session.borrow();
        session.active().is_some_and(super::session::Tab::is_slow)
    }

    /// Redraws tab `id` once its location becomes slow, if it is busy
    /// and not slow yet, so a pane on screen then says it is working.
    /// Each tab keeps its own time, so a tab shown again is as slow as
    /// it was (VIEW-068).
    pub(super) fn redraw_when_slow(&self, id: TabId) {
        let wait = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else { return };
            let busy = tab.is_busy();
            let now = std::time::Instant::now();
            tab.busy_clock.follow(busy, now);
            let wait = tab.busy_clock.until_slow(busy, now);
            if wait.is_none() || tab.busy_clock.redraw_pending {
                return;
            }
            tab.busy_clock.redraw_pending = true;
            wait
        };
        let Some(wait) = wait else { return };
        glib::timeout_add_local_once(
            wait,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    if let Some(tab) = window.imp().session.borrow_mut().tab_mut(id) {
                        tab.busy_clock.redraw_pending = false;
                    }
                    window.redraw_pane(id);
                }
            ),
        );
    }

    /// Whether the tab in front is opening or listing a location.
    pub(super) fn is_busy(&self) -> bool {
        let id = self.imp().session.borrow().active_id();
        id.is_some_and(|id| self.is_tab_busy(id))
    }

    /// Whether tab `id` is going to `target` already, looked up or
    /// listed: Enter on the same address does not start it over.
    pub(super) fn is_going_to(&self, id: TabId, target: &str) -> bool {
        let session = self.imp().session.borrow();
        let Some(tab) = session.tab(id) else { return false };
        match &tab.opening {
            Some(opening) => opening.target == target,
            None => tab.listing_state.is_listing() && !tab.reloading && tab.uri() == target,
        }
    }

    /// What tab `id` is going to, if it is looking a location up.
    pub(super) fn opening_kind(&self, id: TabId) -> Option<OpeningKind> {
        let session = self.imp().session.borrow();
        session
            .tab(id)
            .and_then(|tab| tab.opening.as_ref())
            .map(|opening| opening.kind)
    }

    /// Tab `id` starts going to `target`, which `work` looks up; any
    /// earlier lookup of the tab is cancelled.
    pub(super) fn begin_opening(&self, id: TabId, target: &str, kind: OpeningKind, work: Listing) {
        let earlier = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else { return };
            let was_busy = tab.is_busy();
            tab.busy_clock.start(was_busy, std::time::Instant::now());
            // A folder from the list that is still being opened holds the
            // tab's one activation; replaced, it gives it back, as when
            // it is cancelled.
            if tab
                .opening
                .as_ref()
                .is_some_and(|opening| opening.kind == OpeningKind::Item)
            {
                tab.is_activating = false;
            }
            tab.opening.replace(Opening {
                target: target.to_owned(),
                kind,
                work,
            })
        };
        drop(earlier);
        self.redraw_pane(id);
    }

    /// The lookup of tab `id` has its answer, which it acts on now: the
    /// tab stops waiting for it. False once the tab has closed.
    pub(super) fn end_opening(&self, id: TabId) -> bool {
        let opening = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else {
                return false;
            };
            let opening = tab.opening.take();
            if opening.is_some() {
                // What the lookup found may be listed now, going on from
                // the lookup's time.
                tab.busy_clock.hand_over();
            }
            opening
        };
        if let Some(opening) = opening {
            opening.work.detach();
        }
        self.redraw_pane(id);
        true
    }

    /// The answer of tab `id`'s lookup has been acted on: a location the
    /// tab opens later counts its own time.
    pub(super) fn after_opening(&self, id: TabId) {
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.busy_clock.end_hand_over();
        }
    }

    /// Cancels the lookup tab `id` has running, if any; true when there
    /// was one.
    pub(super) fn cancel_opening(&self, id: TabId) -> bool {
        let opening = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else {
                return false;
            };
            let opening = tab.opening.take();
            if opening
                .as_ref()
                .is_some_and(|opening| opening.kind == OpeningKind::Item)
            {
                tab.is_activating = false;
            }
            opening
        };
        let cancelled = opening.is_some();
        // Dropping the work cancels its lookup or mount, and any sign-in
        // the mount asked for.
        drop(opening);
        if cancelled {
            self.redraw_pane(id);
        }
        cancelled
    }

    /// Escape in the address bar: a typed address being opened in the
    /// tab in front is cancelled with the edit, as the address it showed
    /// goes.
    pub(super) fn cancel_typed_opening(&self) {
        let id = {
            let session = self.imp().session.borrow();
            let Some(tab) = session.active() else { return };
            let is_typed = tab
                .opening
                .as_ref()
                .is_some_and(|opening| opening.kind != OpeningKind::Item);
            if !is_typed {
                return;
            }
            tab.id
        };
        self.cancel_opening(id);
    }

    /// Shows whether the tab in front is busy: progress in the address
    /// bar and Stop in place of Refresh, at once.
    pub(super) fn show_busy_state(&self) {
        let busy = self.is_busy();
        let bar = self.address_bar();
        if busy {
            bar.add_css_class(LOADING_CLASS);
        } else {
            bar.remove_css_class(LOADING_CLASS);
        }
        bar.update_state(&[gtk::accessible::State::Busy(busy)]);
        self.show_stop_or_refresh();
    }
}
