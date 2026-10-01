// SPDX-License-Identifier: AGPL-3.0-only
//! A slow second click on the name of the only selected item renames it
//! in place (OPS-011).
//!
//! As in Windows File Explorer and Dolphin's "two-clicks renaming": a
//! plain click on the name of the item that is already the only one
//! selected starts the rename once a double-click interval has passed
//! with no second click. A double-click, a drag, a click elsewhere, the
//! selection changing and the window losing focus cancel it, and nothing
//! starts while renaming is not allowed there.

use std::cell::RefCell;
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::file_ops::FileCommand;
use super::BrowserWindow;
use crate::folder_view::cells::FileCell;

/// GTK's double-click time when the settings do not say (`GtkSettings`).
const DEFAULT_DOUBLE_CLICK: u32 = 400;

/// The rename a slow second click has scheduled, if any.
#[derive(Debug, Default)]
pub(super) struct SlowClickRename {
    pending: RefCell<Option<glib::SourceId>>,
}

/// What a primary press on the folder view lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NamePress {
    /// The item's position, when the press is on an item's name text.
    pub(super) on_name_of: Option<u32>,
    /// The press's click count: 2 for the second press of a double-click.
    pub(super) clicks: i32,
    /// The modifiers held.
    pub(super) modifiers: gdk::ModifierType,
}

/// Whether `press` asks for a rename of the item at its position, when
/// `selected` are the positions selected before the press.
fn asks_for_rename(press: NamePress, selected: &[u32]) -> Option<u32> {
    let position = press.on_name_of?;
    let held = gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK;
    let plain = !press.modifiers.intersects(held);
    (press.clicks == 1 && plain && selected == [position]).then_some(position)
}

/// The desktop's double-click interval.
fn double_click_interval() -> Duration {
    let millis = gtk::Settings::default()
        .map(|settings| settings.gtk_double_click_time())
        .and_then(|time| u32::try_from(time).ok())
        .unwrap_or(DEFAULT_DOUBLE_CLICK);
    Duration::from_millis(u64::from(millis))
}

impl BrowserWindow {
    /// Cancels a scheduled rename when the window loses focus.
    pub(super) fn install_slow_click_rename(&self) {
        self.connect_is_active_notify(|window| {
            if !window.is_active() {
                window.cancel_slow_click_rename();
            }
        });
    }

    /// Watches primary presses on `view` for a slow second click on a
    /// name.
    pub(super) fn attach_slow_click_rename(&self, view: &gtk::Widget) {
        let press = gtk::GestureClick::new();
        press.set_button(gdk::BUTTON_PRIMARY);
        // Before the list selects what the press lands on.
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |press, clicks, x, y| {
                let name_press = NamePress {
                    on_name_of: window.name_text_at(&view, x, y),
                    clicks,
                    modifiers: press.current_event_state(),
                };
                window.name_pressed(name_press);
            }
        ));
        view.add_controller(press);
    }

    /// The position of the item whose name text is at (`x`, `y`) in
    /// `view`, if a name is there.
    fn name_text_at(&self, view: &gtk::Widget, x: f64, y: f64) -> Option<u32> {
        let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let cell = picked
            .ancestor(FileCell::static_type())
            .and_downcast::<FileCell>()?;
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let in_view = gtk::graphene::Point::new(x as f32, y as f32);
        let point = view.compute_point(&cell, &in_view)?;
        if !cell.name_text_contains(point) {
            return None;
        }
        self.folder_pane().owners().position_holding(view, picked)
    }

    /// A primary press on the folder view: cancels the rename scheduled
    /// before, and schedules one when `press` is a slow second click on
    /// the name of the only selected item.
    pub(super) fn name_pressed(&self, press: NamePress) {
        self.cancel_slow_click_rename();
        let selected = self.folder_pane().model().selected_positions();
        let Some(position) = asks_for_rename(press, &selected) else {
            return;
        };
        if !self.allows(FileCommand::Rename) || self.are_item_clicks_paused() {
            return;
        }
        let rename = glib::timeout_add_local_once(
            double_click_interval(),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().slow_click_rename.pending.take();
                    // Losing focus cancelled it already; a changed
                    // selection cancels it here.
                    let still_selected = window.folder_pane().model().selected_positions() == [position];
                    if still_selected {
                        glib::spawn_future_local(async move { window.rename_selection().await });
                    }
                }
            ),
        );
        self.imp().slow_click_rename.pending.replace(Some(rename));
    }

    /// Drops the rename a slow second click scheduled, if any.
    pub(super) fn cancel_slow_click_rename(&self) {
        if let Some(pending) = self.imp().slow_click_rename.pending.take() {
            pending.remove();
        }
    }
}
