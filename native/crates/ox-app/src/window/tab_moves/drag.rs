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

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use super::TabMoveRefusal;
use crate::window::session::TabId;
use crate::window::tab_strip::TabInsertion;
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

    fn attach_tab_drag_source(&self) {
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        source.connect_prepare(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            None,
            move |_, x, y| window.prepare_tab_drag(x, y)
        ));
        source.connect_drag_begin(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, drag| window.begin_tab_drag(drag)
        ));
        source.connect_drag_cancel(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |_, _, reason| {
                window.cancel_tab_drag(reason);
                false
            }
        ));
        source.connect_drag_end(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _| window.end_tab_drag()
        ));
        self.tab_strip().add_controller(source);
    }

    fn attach_tab_drop_target(&self) {
        let target = gtk::DropTarget::new(DraggedTab::static_type(), gdk::DragAction::MOVE);
        target.connect_motion(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |target, x, y| window.hover_tab_drop(target, x, y)
        ));
        target.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.show_tab_drop_spot(None)
        ));
        target.connect_drop(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |_, value, x, y| window.take_tab_drop(value, x, y)
        ));
        self.add_controller(target);
    }

    /// What dragging the tab at (`x`, `y`) of the strip offers; `None`
    /// on a close button, or while the window is busy, which the toast
    /// explains (TAB-031).
    fn prepare_tab_drag(&self, x: f64, y: f64) -> Option<gdk::ContentProvider> {
        let tab = self.tab_strip().draggable_tab_at(x, y)?;
        if self.is_busy_for_tab_moves() {
            self.show_message(&TabMoveRefusal::SourceBusy.to_string());
            return None;
        }
        self.imp().outgoing_tab.replace(Some(OutgoingTabDrag {
            tab,
            outcome: TabDragOutcome::Pending,
            is_refused_here: false,
        }));
        let dragged = DraggedTab {
            source: self.downgrade(),
            tab,
        };
        Some(tab_drag_content(&dragged))
    }

    /// The drag started: the tab fades and its likeness follows the
    /// pointer.
    fn begin_tab_drag(&self, drag: &gdk::Drag) {
        let Some(tab) = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .map(|outgoing| outgoing.tab)
        else {
            return;
        };
        self.tab_strip().show_dragged_tab(Some(tab));
        if let Some(icon) = self.tab_strip().drag_icon(tab) {
            gtk::DragIcon::for_drag(drag).set_child(Some(&icon));
        }
    }

    /// The drag was cancelled: released where no window took it, refused,
    /// or Escape. Only a release where nothing takes drops, with no
    /// window of this app refusing it last, tears the tab out; everything
    /// else keeps it and says how to detach it (`drag_failed`).
    fn cancel_tab_drag(&self, reason: gdk::DragCancelReason) {
        let mut outgoing = self.imp().outgoing_tab.borrow_mut();
        let Some(outgoing) = outgoing.as_mut() else {
            return;
        };
        if outgoing.outcome != TabDragOutcome::Pending {
            return;
        }
        let is_released_outside = reason == gdk::DragCancelReason::NoTarget && !outgoing.is_refused_here;
        if is_released_outside {
            outgoing.outcome = TabDragOutcome::TornOut;
        } else {
            outgoing.outcome = TabDragOutcome::Cancelled;
            self.show_message(&TabMoveRefusal::Cancelled.to_string());
        }
    }

    /// The drag ended: the tab shows plainly again, and once GTK is done
    /// with the drag it leaves for a new window, or is removed here when
    /// another window took it. A drop no window of this app took, and
    /// that was not cancelled, is GNOME Shell's desktop drop: a tear-out.
    fn end_tab_drag(&self) {
        self.tab_strip().show_dragged_tab(None);
        self.show_tab_drop_spot(None);
        let Some(outgoing) = self.imp().outgoing_tab.take() else {
            return;
        };
        let tab = outgoing.tab;
        let finish: fn(&BrowserWindow, TabId) = match outgoing.outcome {
            TabDragOutcome::Pending | TabDragOutcome::TornOut => BrowserWindow::move_tab_to_new_window,
            TabDragOutcome::MovedAway => BrowserWindow::close_tab,
            TabDragOutcome::Reordered | TabDragOutcome::Cancelled => return,
        };
        // After the drag's own signal handlers, so a window that closes
        // with its last tab is not destroyed in the middle of them.
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || finish(&window, tab)
        ));
    }

    /// A tab drag moves over this window: shows where a drop would go and
    /// tells the source whether it is refused here.
    fn hover_tab_drop(&self, target: &gtk::DropTarget, x: f64, y: f64) -> gdk::DragAction {
        let dragged = target.current_drop().as_ref().and_then(dragged_tab);
        let source = dragged.and_then(|dragged| dragged.source.upgrade());
        let spot = source
            .as_ref()
            .and_then(|source| self.tab_drop_spot(source, x, y));
        if let Some(source) = &source {
            source.note_tab_refusal(spot.is_none());
        }
        self.show_tab_drop_spot(spot);
        if spot.is_some() {
            gdk::DragAction::MOVE
        } else {
            gdk::DragAction::empty()
        }
    }

    /// Records whether the window under the pointer refuses this window's
    /// tab drag.
    fn note_tab_refusal(&self, is_refused: bool) {
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.is_refused_here = is_refused;
        }
    }

    /// Takes the tab `value` carries at (`x`, `y`); true when it moved or
    /// will tear out.
    fn take_tab_drop(&self, value: &glib::Value, x: f64, y: f64) -> bool {
        self.show_tab_drop_spot(None);
        let Ok(dragged) = value.get::<DraggedTab>() else {
            return false;
        };
        let Some(source) = dragged.source.upgrade() else {
            return false;
        };
        let Some(spot) = self.tab_drop_spot(&source, x, y) else {
            return false;
        };
        source.settle_tab_drag(dragged.tab, self, spot)
    }

    /// Where a tab from `source` dropped at (`x`, `y`) of this window
    /// goes; `None` where it is refused, and anywhere while this window
    /// is busy (TAB-035, TAB-037).
    fn tab_drop_spot(&self, source: &BrowserWindow, x: f64, y: f64) -> Option<TabDropSpot> {
        if self.is_busy_for_tab_moves() {
            return None;
        }
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = graphene::Point::new(x as f32, y as f32);
        if self.is_in_tab_row(point) {
            let in_strip = self.compute_point(self.tab_strip(), &point)?;
            let before = self.tab_strip().tab_after(f64::from(in_strip.x()));
            return Some(TabDropSpot::Strip { before });
        }
        let is_tear_out_area = source == self && self.is_below_title_bar(y, TEAR_OUT_GAP);
        is_tear_out_area.then_some(TabDropSpot::TearOut)
    }

    /// True for a point of the title bar from the first tab to the
    /// open-windows button: the tabs, "+" and the empty drag area.
    fn is_in_tab_row(&self, point: graphene::Point) -> bool {
        let strip = self.tab_strip().compute_bounds(self);
        let windows_button = self.imp().open_windows_button.compute_bounds(self);
        let (Some(strip), Some(windows_button), Some(bottom)) =
            (strip, windows_button, self.title_bar_bottom())
        else {
            return false;
        };
        let is_across = point.x() >= strip.x() && point.x() < windows_button.x();
        let is_down = point.y() >= 0.0 && point.y() < bottom;
        is_across && is_down
    }

    /// True when `y` is at least `gap` below the title bar.
    fn is_below_title_bar(&self, y: f64, gap: f64) -> bool {
        self.title_bar_bottom()
            .is_some_and(|bottom| y >= f64::from(bottom) + gap)
    }

    /// Where the title bar ends, in the window's coordinates.
    fn title_bar_bottom(&self) -> Option<f32> {
        let bar = self.titlebar()?.compute_bounds(self)?;
        Some(bar.y() + bar.height())
    }

    /// The source's side of a drop of its tab `tab` on `destination` at
    /// `spot`: reorders, hands the tab over or marks the tear-out; true
    /// when the tab is taken. A late or repeated drop of a drag that is
    /// over takes nothing.
    fn settle_tab_drag(&self, tab: TabId, destination: &BrowserWindow, spot: TabDropSpot) -> bool {
        let is_this_drag = self
            .imp()
            .outgoing_tab
            .borrow()
            .as_ref()
            .is_some_and(|outgoing| outgoing.tab == tab && outgoing.outcome == TabDragOutcome::Pending);
        if !is_this_drag {
            return false;
        }
        let outcome = match spot {
            TabDropSpot::TearOut => TabDragOutcome::TornOut,
            TabDropSpot::Strip { before } if destination == self => {
                self.reorder_tab(tab, before);
                TabDragOutcome::Reordered
            }
            TabDropSpot::Strip { before } => match self.hand_over_tab(tab, destination, before) {
                Ok(()) => TabDragOutcome::MovedAway,
                Err(refusal) => {
                    self.show_message(&refusal.to_string());
                    return false;
                }
            },
        };
        if let Some(outgoing) = self.imp().outgoing_tab.borrow_mut().as_mut() {
            outgoing.outcome = outcome;
        }
        true
    }

    /// Shows where a dropped tab would go: the strip's insertion mark, or
    /// the tear-out note; nothing for `None`.
    fn show_tab_drop_spot(&self, spot: Option<TabDropSpot>) {
        let insertion = match spot {
            Some(TabDropSpot::Strip { before }) => Some(before),
            Some(TabDropSpot::TearOut) | None => None,
        };
        self.tab_strip()
            .show_tab_insertion(insertion.map_or(TabInsertion::Hidden, TabInsertion::before));
        let hint = (spot == Some(TabDropSpot::TearOut)).then_some(TEAR_OUT_HINT);
        self.folder_pane().show_drag_hint(hint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{
        capture, settle, wait_for, wait_for_frames, wait_until, windows_besides, Fixture, OpenedWindows,
        TestWindow, ThemeGuard,
    };

    /// parity: TAB-040
    #[test]
    fn a_tab_drag_offers_the_tab_to_this_process_only() {
        let content = tab_drag_content(&DraggedTab {
            source: glib::WeakRef::new(),
            tab: TabId::from_raw(1),
        });

        let formats = content.formats();

        assert!(formats.contains_type(DraggedTab::static_type()));
        let mime_types: Vec<String> = formats.mime_types().iter().map(ToString::to_string).collect();
        assert_eq!(mime_types, [ROOT_WINDOW_DROP], "no file, text or location format");
        let served = formats.union_serialize_mime_types();
        let served: Vec<String> = served.mime_types().iter().map(ToString::to_string).collect();
        assert_eq!(
            served,
            [ROOT_WINDOW_DROP],
            "GTK serializes nothing else for other apps"
        );
    }

    fn tab_ids(window: &BrowserWindow) -> Vec<TabId> {
        let session = window.imp().session.borrow();
        session.tabs().iter().map(|tab| tab.id).collect()
    }

    /// The tab widget at `index` of `window`'s strip.
    fn tab_widget(window: &BrowserWindow, index: usize) -> gtk::Widget {
        crate::window::widget_tree::children(&window.tab_strip().tab_list())
            .nth(index)
            .expect("the window shows the tab")
    }

    /// The middle of `widget` in `ancestor`'s coordinates.
    fn middle_in(widget: &impl IsA<gtk::Widget>, ancestor: &impl IsA<gtk::Widget>) -> (f64, f64) {
        let bounds = widget
            .compute_bounds(ancestor)
            .expect("a shown widget has bounds");
        let x = bounds.x() + bounds.width() / 2.0;
        let y = bounds.y() + bounds.height() / 2.0;
        (f64::from(x), f64::from(y))
    }

    /// Starts dragging the tab at `index`, as a press and a move there do;
    /// the dragged tab.
    fn start_dragging(window: &BrowserWindow, index: usize) -> TabId {
        wait_for_frames(window, 3);
        let (x, y) = middle_in(&tab_widget(window, index), window.tab_strip());
        window.prepare_tab_drag(x, y).expect("the tab can be dragged");
        let outgoing = window.imp().outgoing_tab.borrow();
        outgoing
            .as_ref()
            .map(|outgoing| outgoing.tab)
            .expect("the drag is prepared")
    }

    /// A window with tabs on `fixture`, its Documents folder and
    /// `fixture` again.
    fn three_tabs(fixture: &Fixture) -> TestWindow {
        let test = TestWindow::open(&fixture.uri());
        for uri in [fixture.uri_of("Documents"), fixture.uri()] {
            test.window.add_tab(&uri).expect("a folder");
            test.wait_for_listing("the new tab");
        }
        test
    }

    /// parity: TAB-032
    #[gtk::test]
    fn a_tab_dropped_on_its_own_strip_goes_before_the_tab_right_of_the_pointer() {
        let fixture = Fixture::standard();
        let test = three_tabs(&fixture);
        let [first, second, third] = tab_ids(&test.window)[..] else {
            panic!("three tabs are open");
        };
        let dragged = start_dragging(&test.window, 0);
        let past_the_last = tab_widget(&test.window, 2)
            .compute_bounds(&test.window)
            .expect("a shown tab");
        let x = f64::from(past_the_last.x() + past_the_last.width() - 2.0);
        let (_, y) = middle_in(&tab_widget(&test.window, 2), &test.window);

        let spot = test.window.tab_drop_spot(&test.window, x, y);
        let taken = spot.is_some_and(|spot| test.window.settle_tab_drag(dragged, &test.window, spot));
        test.window.end_tab_drag();
        settle();

        assert_eq!(spot, Some(TabDropSpot::Strip { before: None }));
        assert!(taken);
        assert_eq!(tab_ids(&test.window), [second, third, first]);
        assert!(
            windows_besides(&[&test.window]).is_empty(),
            "reordering opens no window"
        );
    }

    /// parity: TAB-033, TAB-039
    #[gtk::test]
    fn a_tab_dropped_on_another_windows_strip_moves_there_and_leaves_after_the_drag() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let other = test.open_beside(&fixture.uri());
        let dragged = start_dragging(&test.window, 1);
        wait_for_frames(&other.window, 3);
        let (x, y) = middle_in(&tab_widget(&other.window, 0), &other.window);
        let other_first = tab_ids(&other.window)[0];

        let spot = other.window.tab_drop_spot(&test.window, f64::min(x, 20.0), y);
        let taken = spot.is_some_and(|spot| test.window.settle_tab_drag(dragged, &other.window, spot));
        let kept_while_dragging = test.window.tab_count();
        test.window.end_tab_drag();
        wait_until("the original to go", || test.window.tab_count() == 1);

        assert_eq!(
            spot,
            Some(TabDropSpot::Strip {
                before: Some(other_first)
            })
        );
        assert!(taken);
        assert_eq!(kept_while_dragging, 2, "the original stays until the drag ends");
        assert_eq!(other.window.tab_count(), 2);
        assert_eq!(other.window.current_uri(), Some(fixture.uri_of("Documents")));
        assert_eq!(
            tab_ids(&other.window)[1],
            other_first,
            "the tab went before the first"
        );
    }

    /// parity: TAB-034, TAB-035
    #[gtk::test]
    fn only_the_source_window_well_below_its_strip_tears_a_tab_out() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        let other = test.open_beside(&fixture.uri());
        let bottom = f64::from(test.window.title_bar_bottom().expect("a laid out title bar"));
        let dragged = start_dragging(&test.window, 1);

        let in_the_gap = test.window.tab_drop_spot(&test.window, 300.0, bottom + 20.0);
        let well_below = test.window.tab_drop_spot(&test.window, 300.0, bottom + 60.0);
        let in_another_window = other.window.tab_drop_spot(&test.window, 300.0, bottom + 60.0);
        let taken = test
            .window
            .settle_tab_drag(dragged, &test.window, TabDropSpot::TearOut);
        test.window.end_tab_drag();
        let opened = OpenedWindows::only(&[&test.window, &other.window]);

        assert_eq!(in_the_gap, None);
        assert_eq!(well_below, Some(TabDropSpot::TearOut));
        assert_eq!(in_another_window, None, "another window's body refuses the tab");
        assert!(taken);
        assert_eq!(opened.window().current_uri(), Some(fixture.uri_of("Documents")));
        assert_eq!(test.window.tab_count(), 1);
    }

    /// How a drag ends without a window of this app taking the tab.
    struct UnclaimedEnd {
        /// The cancel reason GTK reports first, or `None` when the drop
        /// was performed (GNOME Shell's desktop drop).
        cancel: Option<gdk::DragCancelReason>,
        /// A window of this app refused the tab where it was released.
        refused_here: bool,
        /// The tab tears out into a new window.
        tears_out: bool,
    }

    /// parity: TAB-034, TAB-035, TAB-036
    #[gtk::test]
    fn a_tab_released_outside_every_window_tears_out_and_a_cancelled_one_stays() {
        let cases = [
            UnclaimedEnd {
                cancel: None,
                refused_here: false,
                tears_out: true,
            },
            UnclaimedEnd {
                cancel: Some(gdk::DragCancelReason::NoTarget),
                refused_here: false,
                tears_out: true,
            },
            UnclaimedEnd {
                cancel: Some(gdk::DragCancelReason::NoTarget),
                refused_here: true,
                tears_out: false,
            },
            UnclaimedEnd {
                cancel: Some(gdk::DragCancelReason::UserCancelled),
                refused_here: false,
                tears_out: false,
            },
            UnclaimedEnd {
                cancel: Some(gdk::DragCancelReason::Error),
                refused_here: false,
                tears_out: false,
            },
        ];
        let fixture = Fixture::standard();
        for case in cases {
            let test = TestWindow::open(&fixture.uri());
            test.window
                .add_tab(&fixture.uri_of("Documents"))
                .expect("a folder");
            test.wait_for_listing("the second tab");
            let dragged = start_dragging(&test.window, 1);

            test.window.note_tab_refusal(case.refused_here);
            if let Some(reason) = case.cancel {
                test.window.cancel_tab_drag(reason);
            }
            test.window.end_tab_drag();
            let late_drop = test
                .window
                .settle_tab_drag(dragged, &test.window, TabDropSpot::TearOut);

            assert!(!late_drop, "a drop after the drag ended takes nothing");
            if case.tears_out {
                let opened = OpenedWindows::only(&[&test.window]);
                assert_eq!(opened.window().current_uri(), Some(fixture.uri_of("Documents")));
                assert_eq!(test.window.tab_count(), 1);
            } else {
                wait_for(std::time::Duration::from_millis(100));
                assert!(windows_besides(&[&test.window]).is_empty());
                assert_eq!(test.window.tab_count(), 2, "the tab stays");
                assert_eq!(test.window.shown_message(), TabMoveRefusal::Cancelled.to_string());
            }
        }
    }

    /// parity: TAB-031
    #[gtk::test]
    fn no_tab_drag_starts_from_a_close_button_or_while_the_window_is_busy() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        wait_for_frames(&test.window, 3);
        let tab = tab_widget(&test.window, 0);
        let close = tab.last_child().expect("the tab has a close button");
        let (close_x, close_y) = middle_in(&close, test.window.tab_strip());
        let (x, y) = middle_in(&tab, test.window.tab_strip());

        let from_close_button = test.window.prepare_tab_drag(close_x, close_y);
        let operation = test.window.begin_operation("Preparing copy…");
        let while_busy = test.window.prepare_tab_drag(x, y);
        let busy_message = test.window.shown_message();
        test.window.end_operation();

        assert!(from_close_button.is_none());
        assert!(operation.is_some());
        assert!(while_busy.is_none());
        assert_eq!(busy_message, TabMoveRefusal::SourceBusy.to_string());
        assert!(
            test.window.prepare_tab_drag(x, y).is_some(),
            "a tab drags once the operation ended"
        );
    }

    /// parity: TAB-032, TAB-034
    #[gtk::test]
    fn a_tab_drag_shows_where_the_tab_would_go() {
        let fixture = Fixture::standard();
        let test = three_tabs(&fixture);
        let [_, second, third] = tab_ids(&test.window)[..] else {
            panic!("three tabs are open");
        };
        let dragged = start_dragging(&test.window, 0);
        test.window.tab_strip().show_dragged_tab(Some(dragged));

        test.window
            .show_tab_drop_spot(Some(TabDropSpot::Strip { before: Some(second) }));
        let before_second = classes_of(&test.window, second);
        let strip_marked = test.window.tab_strip().has_css_class("tab-drop-active");
        let fades_the_dragged_tab = classes_of(&test.window, dragged);
        test.window
            .show_tab_drop_spot(Some(TabDropSpot::Strip { before: None }));
        let after_last = classes_of(&test.window, third);
        test.window.show_tab_drop_spot(Some(TabDropSpot::TearOut));
        let tear_out_hint = test.window.folder_pane().drag_hint();
        test.window.end_tab_drag();

        assert!(before_second.contains(&"tab-insert-before".to_owned()));
        assert!(strip_marked);
        assert!(fades_the_dragged_tab.contains(&"tab-drag-source".to_owned()));
        assert!(after_last.contains(&"tab-insert-after".to_owned()));
        assert_eq!(tear_out_hint.as_deref(), Some(TEAR_OUT_HINT));
        assert!(
            !test.window.tab_strip().has_css_class("tab-drop-active"),
            "the marks go with the drag"
        );
        assert_eq!(test.window.folder_pane().drag_hint(), None);
        assert!(!classes_of(&test.window, dragged).contains(&"tab-drag-source".to_owned()));
    }

    /// The style classes of tab `id` of `window`.
    fn classes_of(window: &BrowserWindow, id: TabId) -> Vec<String> {
        let classes = window.tab_strip().tab_classes();
        classes
            .into_iter()
            .find_map(|(tab, classes)| (tab == id).then_some(classes))
            .expect("the tab is shown")
    }

    /// With `OX_NATIVE_CAPTURE_DIR` set, saves a tab drag's insertion mark
    /// and its tear-out note in both themes.
    #[gtk::test]
    fn a_tab_drags_marks_are_captured_light_and_dark() {
        let _theme = ThemeGuard::keep();
        let fixture = Fixture::standard();
        let test = three_tabs(&fixture);
        let second = tab_ids(&test.window)[1];
        let dragged = start_dragging(&test.window, 0);
        for theme in ["light", "dark"] {
            test.activate("theme", Some(theme));
            // A new theme draws the tabs anew; a drag's next motion would
            // mark the new ones.
            wait_for_frames(&test.window, 3);
            test.window.tab_strip().show_dragged_tab(Some(dragged));
            test.window
                .show_tab_drop_spot(Some(TabDropSpot::Strip { before: Some(second) }));
            capture(&test.window, &format!("native-tab-drag-insert-{theme}.png"));
            test.window.show_tab_drop_spot(Some(TabDropSpot::TearOut));
            capture(&test.window, &format!("native-tab-drag-tear-out-{theme}.png"));
        }
        test.window.end_tab_drag();
    }
}
