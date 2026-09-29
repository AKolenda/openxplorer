// SPDX-License-Identifier: AGPL-3.0-only
//! The parts of the window that take dropped files (DND-011, DND-014,
//! DND-016, DND-021, DND-025, TAB-018): each zone's drop target, which
//! asks [`spot`] where a drop at a point goes, [`highlight`] to show it,
//! [`spring`] to open a folder the drag stays over and [`autoscroll`] to
//! scroll a zone the drag hovers near the edge of.
//!
//! Ports `publishFileDragLayout` and `showFileDropHint` of
//! `desktop/ui/app.js`. The web app published rectangles for the native
//! side to hit-test; here each part answers for a point itself, in its
//! own coordinates at its real size, so scaling and clipping need no
//! arithmetic.

mod autoscroll;
mod highlight;
mod spot;
mod spring;
#[cfg(test)]
mod tests;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

pub(in crate::window) use autoscroll::DragScroll;

use spot::DropSpot;

use crate::window::file_drag::DraggedItems;
use crate::window::BrowserWindow;

/// A part of the window that takes dropped files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropZone {
    /// A folder view.
    FolderView,
    /// The sidebar.
    Sidebar,
    /// The breadcrumbs.
    Breadcrumbs,
    /// The tabs.
    Tabs,
}

/// The formats a file drop target takes: this process's own dragged
/// items, and every format GTK reads a file list from.
fn file_drop_formats() -> gdk::ContentFormats {
    gdk::ContentFormatsBuilder::new()
        .add_type(DraggedItems::static_type())
        .add_type(gdk::FileList::static_type())
        .build()
        .union_deserialize_mime_types()
}

/// Every action a drop may run.
fn every_action() -> gdk::DragAction {
    gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK | gdk::DragAction::ASK
}

impl BrowserWindow {
    /// Lets `widget`, the window's `zone`, take dropped files.
    pub(in crate::window) fn attach_file_drop_zone(&self, widget: &impl IsA<gtk::Widget>, zone: DropZone) {
        let target = gtk::DropTargetAsync::new(Some(file_drop_formats()), every_action());
        let hover = glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |target: &gtk::DropTargetAsync, drop: &gdk::Drop, x: f64, y: f64| {
                window.hover_drop(zone, target, drop, x, y)
            }
        );
        target.connect_drag_enter(hover.clone());
        target.connect_drag_motion(hover);
        target.connect_drag_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.leave_drop_zone(zone)
        ));
        target.connect_drop(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |target, drop, x, y| window.take_drop(zone, target, drop, x, y)
        ));
        widget.add_controller(target);
    }

    /// A drag moves over `zone`: highlights where a drop would go and
    /// returns the action it would run, or none.
    fn hover_drop(
        &self,
        zone: DropZone,
        target: &gtk::DropTargetAsync,
        drop: &gdk::Drop,
        x: f64,
        y: f64,
    ) -> gdk::DragAction {
        let spot = target
            .widget()
            .and_then(|widget| self.hover_drop_at(zone, &widget, x, y));
        let action = self.drop_action(drop);
        match (spot, action) {
            (Some(_), Some(action)) => action.as_drag_action(),
            _ => gdk::DragAction::empty(),
        }
    }

    /// A drag is at (`x`, `y`) of `widget`, the window's `zone`: scrolls
    /// the zone near its edge, and highlights where a drop would go. While
    /// the zone scrolls, the folder under the pointer changes without the
    /// pointer moving, so no folder opens by staying under it (DND-021).
    fn hover_drop_at(&self, zone: DropZone, widget: &gtk::Widget, x: f64, y: f64) -> Option<DropSpot> {
        let scrolls =
            matches!(zone, DropZone::FolderView | DropZone::Sidebar) && self.scroll_drag_near_edge(widget, y);
        let spot = self.drop_spot(zone, widget, x, y);
        self.show_drop_spot(zone, spot.as_ref());
        if scrolls {
            self.open_folder_after_hover(None);
        }
        spot
    }

    /// The drag left `zone`, or dropped there: its highlight and any
    /// scrolling stop, and what was learnt about programs under it is
    /// forgotten, as they may change before the next drag.
    fn leave_drop_zone(&self, zone: DropZone) {
        self.show_drop_spot(zone, None);
        self.stop_drag_scroll();
        if zone == DropZone::FolderView {
            self.forget_program_checks();
        }
    }

    /// Takes `drop` at (`x`, `y`) of `zone`: finds where it goes, then
    /// reads its items and sends them there once the handler returned.
    fn take_drop(
        &self,
        zone: DropZone,
        target: &gtk::DropTargetAsync,
        drop: &gdk::Drop,
        x: f64,
        y: f64,
    ) -> bool {
        let Some(widget) = target.widget() else {
            return false;
        };
        let spot = self.drop_spot(zone, &widget, x, y);
        self.leave_drop_zone(zone);
        let (Some(spot), Some(action)) = (spot, self.drop_action(drop)) else {
            return false;
        };
        if let Err(refusal) = self.check_ready() {
            self.show_message(&refusal.to_string());
            return false;
        }
        self.remember_drop_point(&widget, x, y);
        let destination = spot.destination();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            drop,
            async move {
                window.receive_drop(drop, destination, action).await;
            }
        ));
        true
    }

    /// Keeps where a drop happened, in the folder pane's coordinates, for
    /// the drop menu.
    fn remember_drop_point(&self, widget: &gtk::Widget, x: f64, y: f64) {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = graphene::Point::new(x as f32, y as f32);
        let in_pane = widget.compute_point(self.folder_pane(), &point).unwrap_or(point);
        self.imp()
            .drop_point
            .set((f64::from(in_pane.x()), f64::from(in_pane.y())));
    }
}
