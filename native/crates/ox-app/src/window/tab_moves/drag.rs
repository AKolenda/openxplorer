// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging a tab: along its strip to reorder it, onto another window's
//! strip to move it there, or out of the window to tear it into a new one
//! (TAB-032 to TAB-035, TAB-040).
//!
//! Ports `NativeTabDrag` of `desktop/native_tab_drag.py` on GTK 4's drag
//! and drop. The tab strip is the drag source; every window takes tab
//! drops with one target over the whole window, which finds the spot
//! itself: the title bar from the first tab to the open-windows button
//! places the tab before the tab whose middle is right of the pointer, and
//! in the source window, 40 pixels or more below the title bar tears it
//! out. Anywhere else refuses the tab.
//!
//! Safety rules:
//! - "Tab drags stay in the process" (TAB-040): the drag offers the tab as
//!   a [`DraggedTab`] value, which has no MIME type, so GDK never gives it
//!   to another application, and it names a window and a tab, never a
//!   location. The only other format is GNOME's root-window drop, whose
//!   data is empty: GNOME Shell asks for it when the tab is released over
//!   the desktop, which means "tear out".
//! - "Only a tear-out makes a window" (TAB-035): a new window opens only
//!   when the tab is released in the tear-out area, on GNOME's desktop, or
//!   (on X11) where nothing takes drops and no window of this app
//!   refused it last. Escape, a refused spot and Wayland's cancel keep the tab and
//!   say how to detach it.
//! - "The original goes last": a tab dropped on another window is removed
//!   here only after that window added it, once the drag has ended.

mod source;
mod target;
#[cfg(test)]
mod tests;

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::window::session::TabId;
use crate::window::BrowserWindow;

/// GNOME Shell's format for a drop on the desktop (`ROOT_MIME`).
const ROOT_WINDOW_DROP: &str = "application/x-rootwindow-drop";

/// How far below the title bar the tear-out area starts; the gap between
/// refuses the tab, so a tab dragged a little too low is not torn out.
const TEAR_OUT_GAP: f64 = 40.0;

/// The note over the folder pane while a release would tear the tab out.
const TEAR_OUT_HINT: &str = "Release to open this tab in a new window";

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
    /// Released in the tear-out area or outside every window: a new window
    /// takes it when the drag ends.
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
    /// A window of this app refused the tab where the pointer was last
    /// (`last_rejected`): on X11 a release there reports "no target".
    is_refused_here: bool,
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
}
