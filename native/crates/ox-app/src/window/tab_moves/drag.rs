// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging a tab: along its strip to reorder it, onto another window's
//! strip to move it there, or out of the window to tear it into a new one
//! (TAB-032 to TAB-035, TAB-040), as in Windows Explorer and Chrome.
//!
//! Ports `NativeTabDrag` of `v2.0.0:desktop/native_tab_drag.py` on GTK 4. A
//! press on a tab that moves past the drag threshold is the tab drag's: the
//! drag source on the title bar's window handle claims it, so dragging a
//! tab never moves the window ([`source`] says why it must sit there).
//! Along the strip the tab itself follows the pointer and the other tabs
//! slide aside (`tab_strip::live_drag`), with no drag and drop at all.
//! Once the pointer leaves the strip by more than Chrome's 15 pixels, GTK's
//! drag and drop takes over, carrying the tab itself, drawn as in its
//! strip, and the tab beside it comes to the front, as in Chrome. Every window
//! takes tab drops with one target over the whole window, which finds the
//! spot itself: the title bar from the first tab to the caption buttons
//! places the tab where the tabs made room for it, and anywhere else in a
//! window tears it out, as Chrome does. Back over its own strip the tab
//! follows the pointer again, in front, and the carried tab hides; over
//! another window's strip a stand-in for it follows the pointer there.
//!
//! Safety rules:
//! - "Tab drags stay in the process" (TAB-040): the drag offers the tab as
//!   a [`DraggedTab`] value, which has no MIME type, so GDK never gives it
//!   to another application, and it names a window and a tab, never a
//!   location. The only other format is GNOME's root-window drop, whose
//!   data is empty: GNOME Shell asks for it when the tab is released over
//!   the desktop, which means "tear out".
//! - "Only a tear-out makes a window" (TAB-035): a new window opens only
//!   when the tab is released in a window of this app outside every tab
//!   strip, on GNOME's desktop, or outside every window of this app. A busy
//!   window refuses the tab, and Escape keeps it. On Wayland GTK reports a
//!   release outside every window and Escape alike, so Escape keeps the
//!   tab only while the pointer is over a window of this app.
//! - "The original goes last": a tab dropped on another window is removed
//!   here only after that window added it, once the drag has ended.

mod source;
mod target;
#[cfg(test)]
mod tests;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::folder_view::sorting::SortOrder;
use crate::window::folder_pane::FolderView;
use crate::window::folder_search::KeptSearch;
use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// GNOME Shell's format for a drop on the desktop (`ROOT_MIME`).
const ROOT_WINDOW_DROP: &str = "application/x-rootwindow-drop";

/// Why a tab that is being dragged does not close or navigate.
const TAB_IS_MOVING: &str = crate::i18n::message_id("Wait for this tab to finish moving.");

/// The tab a drag carries: the window it comes from and its id there.
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "OxDraggedTab")]
pub(in crate::window) struct DraggedTab {
    /// The window the tab is in.
    source: glib::WeakRef<BrowserWindow>,
    /// The tab.
    tab: TabId,
}

/// Where a dropped tab goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabDropSpot {
    /// Into the strip, before this tab or at the end.
    Strip { before: Option<TabId> },
    /// Into a new window of its own.
    TearOut,
}

/// What became of the tab this window is dragging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabDragOutcome {
    /// Nothing has taken it yet.
    Pending,
    /// This window's strip took it: it has moved already.
    Reordered,
    /// Another window has it: it is removed here when the drag ends.
    MovedAway,
    /// Released outside every tab strip: a new window takes it when the
    /// drag ends.
    TornOut,
    /// Cancelled or refused: it stays.
    Cancelled,
}

/// The tab drag this window started, while it lasts.
#[derive(Debug)]
pub(in crate::window) struct OutgoingTabDrag {
    /// The dragged tab.
    tab: TabId,
    /// What became of it.
    outcome: TabDragOutcome,
    /// The window of this app the pointer is over, if any.
    over: Option<Hover>,
    /// What follows the pointer, once the drag and drop started.
    carried: Option<Carried>,
    /// The pointer is over this window's own strip.
    over_own_strip: bool,
    /// The tab was in front when it left the strip, and the tab beside it
    /// came to the front meanwhile.
    stepped_back: bool,
    /// The search the window showed then, which comes back with the tab.
    stepped_back_search: Option<KeptSearch>,
    /// The view and sort order of the pane the tab showed in then, which
    /// it takes along if it moves: the pane in front shows the other tab's.
    stepped_back_style: Option<(FolderView, SortOrder)>,
    /// The window that took the tab, once one did.
    destination: Option<glib::WeakRef<BrowserWindow>>,
}

/// What follows the pointer while GTK's drag and drop carries a tab: the
/// tab itself, which the drag shows or hides by its hotspot alone.
#[derive(Debug)]
struct Carried {
    /// The drag.
    drag: gdk::Drag,
    /// The tab that follows the pointer, for tests.
    #[cfg(test)]
    tab: gtk::Widget,
    /// Where it is held: where the tab was grabbed.
    hold: (i32, i32),
    /// The drag's hotspot now.
    at: std::cell::Cell<(i32, i32)>,
}

/// Whether the carried tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CarriedLook {
    /// Under the pointer, held where it was grabbed.
    Shown,
    /// Away: back in its own strip, the tab there follows the pointer.
    Hidden,
}

/// What the window of this app under the pointer does with the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hover {
    /// It takes it.
    Takes,
    /// It is busy and refuses it (TAB-037).
    Refuses,
}

/// The tab `drop` carries, when it is a tab drag of this process.
fn dragged_tab(drop: &gdk::Drop) -> Option<DraggedTab> {
    let content = drop.drag()?.content();
    let value = content.value(DraggedTab::static_type()).ok()?;
    value.get::<DraggedTab>().ok()
}

/// What a drag of `tab` offers: the tab itself, and an empty answer to
/// GNOME Shell's desktop drop.
fn tab_drag_content(tab: &DraggedTab) -> gdk::ContentProvider {
    let own = gdk::ContentProvider::for_value(&tab.to_value());
    let desktop = gdk::ContentProvider::for_bytes(ROOT_WINDOW_DROP, &glib::Bytes::from_static(b""));
    gdk::ContentProvider::new_union(&[own, desktop])
}

impl BrowserWindow {
    /// Lets tabs be dragged out of the strip, and the window take dragged
    /// tabs.
    pub(in crate::window) fn connect_tab_drag_and_drop(&self) {
        self.attach_tab_drag_source();
        self.attach_tab_drop_target();
    }

    /// True, after saying so, while tab `id` is being dragged: it cannot
    /// close or change location until the drag has settled where it goes
    /// (TAB-003), whether GTK's drag and drop carries it or it moves along
    /// the strip.
    pub(in crate::window) fn refuse_while_moving(&self, id: TabId) -> bool {
        let is_moving = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .is_some_and(|outgoing| outgoing.tab == id)
            || self.tab_strip().live_tab() == Some(id);
        if is_moving {
            self.show_message(ox_core::i18n::gettext_static(TAB_IS_MOVING));
        }
        is_moving
    }

    /// [`Self::refuse_while_moving`] for the tab in front.
    pub(in crate::window) fn refuse_while_active_tab_moves(&self) -> bool {
        let active = self.imp().session.borrow().active_id();
        active.is_some_and(|id| self.refuse_while_moving(id))
    }

    /// [`Self::refuse_while_active_tab_moves`] for what gives a tab of the
    /// strip another id, as splitting it, closing one of its panes or
    /// switching between them does, or closes it: refused for any tab while
    /// one follows the pointer along the row, as where the dragged tab
    /// lands or goes back is kept as the ids of the tabs after it, and the
    /// strip shows which tab is in front only among the tabs it had when
    /// the drag began. A tab let go a moment ago lands at once first.
    pub(in crate::window) fn refuse_while_tabs_move(&self) -> bool {
        let strip = self.tab_strip();
        strip.land_settling_tab();
        if strip.is_following() {
            self.show_message(ox_core::i18n::gettext_static(TAB_IS_MOVING));
            return true;
        }
        self.refuse_while_active_tab_moves()
    }
}
