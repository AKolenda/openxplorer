// SPDX-License-Identifier: AGPL-3.0-only
//! Folder Options' "Single-click to open an item (point to select)"
//! (SEL-018), as Windows 11 File Explorer has it: a plain primary click on
//! an item opens it, as a double-click or Enter does otherwise, and the
//! pointer resting on an item for Windows' hover time selects it. With Ctrl
//! held the item pointed at joins the selection, and with Shift the items
//! from the one with keyboard focus to it are selected. The pointer is a
//! hand over items, and their names are underlined, always ("Underline
//! icon titles consistent with my browser") or only the one pointed at
//! ("Underline icon titles only when I point at them", Windows' default).
//!
//! A Ctrl, Shift or Alt click selects as before and opens nothing, and
//! neither do a click on an item's check box or expand arrow, a rubber
//! band, a drag, or a press and release on two different items. The second
//! press of a double-click opens nothing more: the first click opened the
//! item already. Enter still opens the selection, and a slow second click
//! no longer renames, as the first click opens. Every view of the file list
//! (Details, List, the icon sizes, Tiles, Content and Gallery) follows the
//! choice, in every open window as soon as it is saved
//! ([`BrowserWindow::follow_click_mode`]).
//!
//! Ports Dolphin's single-click activation (`KItemListController`, the
//! release that activates, and "modifier clicks and rubber-band releases
//! never open anything") with Windows' wording and point to select, which
//! Dolphin does not have.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::settings::Preferences;

use super::folder_pane::FolderPane;
use super::gestures::held_modifiers;
use super::BrowserWindow;
use crate::folder_view::cells::FileCell;
use crate::folder_view::gallery_view::GalleryCanvas;

/// How long the pointer rests on an item before pointing selects it:
/// Windows' mouse hover time (`SPI_GETMOUSEHOVERTIME`, 400 ms unless the
/// user changed it), which its list view waits for in point to select.
pub(crate) const HOVER_SELECT_DELAY: Duration = Duration::from_millis(400);

/// GTK's drag threshold when the settings do not say (`GtkSettings`).
const DEFAULT_DRAG_THRESHOLD: i32 = 8;

/// The class of a folder view while a single click opens its items.
const SINGLE_CLICK_CLASS: &str = "single-click";

/// The class of a folder view whose item names are all underlined.
const UNDERLINED_TITLES_CLASS: &str = "underlined-titles";

/// The class of the item cell the pointer is on, whose name is underlined.
pub(crate) const POINTED_CLASS: &str = "pointed";

/// The modifiers that make a click select rather than open.
fn selecting_modifiers() -> gdk::ModifierType {
    gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::ALT_MASK
        | gdk::ModifierType::SUPER_MASK
}

/// The pointer buttons that, held, keep pointing from selecting: a drag or
/// a band is under way.
fn held_buttons() -> gdk::ModifierType {
    gdk::ModifierType::BUTTON1_MASK
        | gdk::ModifierType::BUTTON2_MASK
        | gdk::ModifierType::BUTTON3_MASK
        | gdk::ModifierType::BUTTON4_MASK
        | gdk::ModifierType::BUTTON5_MASK
}

/// How a click opens an item, as Folder Options' "Click items as follows"
/// says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClickMode {
    /// A double-click opens, a single click selects: Windows' default.
    #[default]
    Double,
    /// A single click opens and pointing selects; the names are always
    /// underlined, or only the one pointed at.
    Single { underline_always: bool },
}

impl ClickMode {
    /// The mode `preferences` choose.
    pub(crate) fn of(preferences: &Preferences) -> Self {
        if preferences.single_click_open {
            ClickMode::Single {
                underline_always: preferences.underline_titles_always,
            }
        } else {
            ClickMode::Double
        }
    }
}

/// A window's single-click state.
#[derive(Debug, Default)]
pub(super) struct SingleClick {
    /// How a click opens an item now.
    mode: Cell<ClickMode>,
    /// The item a plain primary press landed on, which a release on it
    /// opens.
    pressed: RefCell<Option<Pressed>>,
    /// The item the pointer is on.
    pointed: RefCell<Option<Pointed>>,
    /// While the second press of a double-click is handled: the view's own
    /// activation must not open the item a second time.
    swallows_activation: Cell<bool>,
}

/// The item a plain primary press landed on.
#[derive(Debug)]
struct Pressed {
    view: glib::WeakRef<gtk::Widget>,
    position: u32,
    /// Its URI: the release opens this item, never another that a refresh
    /// or a sort put at its position since.
    uri: String,
    /// Where the press was in the view.
    point: (f64, f64),
}

/// The item the pointer is on.
#[derive(Debug)]
struct Pointed {
    view: glib::WeakRef<gtk::Widget>,
    position: u32,
    /// Its icon-and-name cell, marked [`POINTED_CLASS`].
    cell: glib::WeakRef<FileCell>,
    /// Selects it once the pointer has rested on it; gone once it did, or
    /// once a key or a press took over.
    timer: Option<glib::SourceId>,
}

impl BrowserWindow {
    /// How a click opens an item in this window.
    pub(crate) fn click_mode(&self) -> ClickMode {
        self.imp().single_click.mode.get()
    }

    /// Whether a single click opens items.
    pub(super) fn single_click_opens(&self) -> bool {
        matches!(self.click_mode(), ClickMode::Single { .. })
    }

    /// Follows Folder Options' "Click items as follows" in every view of
    /// both panes: their classes, which underline the names, and the
    /// pointer's item, which a change of mode forgets.
    pub(super) fn follow_click_mode(&self, preferences: &Preferences) {
        let mode = ClickMode::of(preferences);
        if self.imp().single_click.mode.replace(mode) == mode {
            return;
        }
        self.forget_pointed_item();
        let (single, underlined) = match mode {
            ClickMode::Double => (false, false),
            ClickMode::Single { underline_always } => (true, underline_always),
        };
        for pane in self.folder_panes() {
            for view in click_views(pane) {
                set_class(&view, SINGLE_CLICK_CLASS, single);
                set_class(&view, UNDERLINED_TITLES_CLASS, underlined);
                view.set_cursor(None);
            }
        }
    }

    /// Lets a single click on an item of `view` open it and the pointer
    /// select it, while Folder Options says so.
    pub(super) fn attach_single_click(&self, view: &gtk::Widget) {
        let press = gtk::GestureClick::new();
        press.set_button(gdk::BUTTON_PRIMARY);
        // Before the list selects what the press lands on, and before a
        // second press activates it.
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        press.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |press, clicks, x, y| window.single_click_pressed(
                &view,
                clicks,
                held_modifiers(press),
                (x, y)
            )
        ));
        press.connect_released(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |press, clicks, x, y| {
                window.single_click_released(&view, clicks, held_modifiers(press), (x, y));
            }
        ));
        // A move past the drag threshold: a drag, even one that never
        // starts, opens nothing where it ends.
        press.connect_update(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |press, sequence| {
                if let Some(point) = press.point(sequence) {
                    window.drop_moved_press(point);
                }
            }
        ));
        // A band, a drag or a check box that claims the press.
        press.connect_cancel(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.imp().single_click.pressed.take();
            }
        ));
        view.add_controller(press);

        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |_, x, y| window.pointer_over_click_items(&view, Some((x, y)))
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move |_| window.pointer_over_click_items(&view, None)
        ));
        view.add_controller(motion);
        // Another folder listed, or this one changed: the item pointed at
        // is no longer known to be the one there.
        if let Some(pane) = self.pane_holding(view) {
            pane.model().selection().connect_items_changed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, _, _, _| window.forget_pointed_item()
            ));
        }
    }

    /// Whether the view's own activation, which Enter and a double-click
    /// make, is the second press of a double-click while a single click
    /// opens items, which opened the item already.
    pub(super) fn swallows_activation(&self) -> bool {
        self.imp().single_click.swallows_activation.get()
    }

    /// A primary press at `point` in `view`: remembers a plain press on an
    /// item for its release, and keeps the second press of a double-click
    /// from opening anything. Pointing selects nothing more until the
    /// pointer moves to another item: the press selects as it does.
    fn single_click_pressed(
        &self,
        view: &gtk::Widget,
        clicks: i32,
        modifiers: gdk::ModifierType,
        point: (f64, f64),
    ) {
        let state = &self.imp().single_click;
        state.pressed.take();
        if !self.single_click_opens() {
            return;
        }
        self.stop_pointing_selection();
        if clicks > 1 {
            // Only the view's own activation: the release of the first
            // click may open its item after this, when the second press
            // came before the window had time to.
            state.swallows_activation.set(true);
            glib::idle_add_local_once(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.imp().single_click.swallows_activation.set(false)
            ));
            return;
        }
        if modifiers.intersects(selecting_modifiers()) || self.are_item_clicks_paused() {
            return;
        }
        let Some(position) = self.click_item_at(view, point) else {
            return;
        };
        let Some(uri) = self.uri_in(view, position) else {
            return;
        };
        state.pressed.replace(Some(Pressed {
            view: view.downgrade(),
            position,
            uri,
            point,
        }));
    }

    /// The pointer moved to `point` with the primary button held: past the
    /// drag threshold, the press opens nothing.
    fn drop_moved_press(&self, point: (f64, f64)) {
        let mut pressed = self.imp().single_click.pressed.borrow_mut();
        if pressed
            .as_ref()
            .is_some_and(|pressed| is_past_drag_threshold(pressed.point, point))
        {
            pressed.take();
        }
    }

    /// The URI of the item at `position` in `view`'s folder.
    fn uri_in(&self, view: &gtk::Widget, position: u32) -> Option<String> {
        let item = self.pane_holding(view)?.model().item(position)?;
        Some(item.entry().uri.clone())
    }

    /// A primary release at `point` in `view`: opens the item a plain press
    /// landed on when it is released on that item, once the list has
    /// handled the click, as a double-click would.
    fn single_click_released(
        &self,
        view: &gtk::Widget,
        clicks: i32,
        modifiers: gdk::ModifierType,
        point: (f64, f64),
    ) {
        let Some(pressed) = self.imp().single_click.pressed.take() else {
            return;
        };
        let position = pressed.position;
        let opens = self.single_click_opens()
            && clicks == 1
            && pressed.view.upgrade().as_ref() == Some(view)
            && !modifiers.intersects(selecting_modifiers())
            && !self.are_item_clicks_paused()
            && !self.band_is_shown()
            && !is_past_drag_threshold(pressed.point, point)
            && self.click_item_at(view, point) == Some(position)
            && self.uri_in(view, position).as_ref() == Some(&pressed.uri);
        if !opens {
            return;
        }
        // Resting on the item, or on the one at its place in a folder it
        // opens, selects nothing until the pointer moves again.
        self.forget_pointed_item();
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            view,
            move || {
                // The press made its pane the active one, as any click
                // does; the item is still the one clicked. This opens it
                // even while a second press of a double-click that came
                // first swallows the view's own activation.
                let clicked = window.side_holding(&view) == Some(window.active_side())
                    && window.uri_in(&view, position).as_ref() == Some(&pressed.uri);
                if clicked {
                    window.open_from_view(position);
                }
            }
        ));
    }

    /// The item at `point` in `view` that a single click opens and the
    /// pointer selects: its icon or its name, a whole details row once it
    /// is selected (as for a drag, [`Self::is_beside_item`]); never its
    /// check box, its expand arrow, the name field of a rename, a group's
    /// heading or blank space.
    fn click_item_at(&self, view: &gtk::Widget, (x, y): (f64, f64)) -> Option<u32> {
        if let Some(canvas) = view.downcast_ref::<GalleryCanvas>() {
            return canvas.item_at(x, y);
        }
        let pane = self.pane_holding(view)?;
        if super::group_headings::heading_at(view, x, y).is_some() {
            return None;
        }
        let picked = view.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let own_clicks = std::iter::successors(Some(picked.clone()), WidgetExt::parent)
            .take_while(|widget| widget != view)
            .any(|widget| {
                widget.is::<gtk::CheckButton>()
                    || widget.is::<gtk::Text>()
                    || widget.has_css_class("folder-expander")
                    || crate::folder_view::cells::is_in_group_slot(&widget)
            });
        if own_clicks {
            return None;
        }
        let position = pane.owners().position_holding(view, picked)?;
        let beside_name = view.is::<gtk::ColumnView>()
            && !pane.model().selection().is_selected(position)
            && pane.owners().file_cell_at(position, view).is_some_and(|cell| {
                #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
                let in_view = gtk::graphene::Point::new(x as f32, y as f32);
                view.compute_point(&cell, &in_view)
                    .is_some_and(|point| cell.is_past_name(point))
            });
        (!beside_name).then_some(position)
    }

    /// The folder pane `view` is in.
    fn pane_holding(&self, view: &gtk::Widget) -> Option<FolderPane> {
        self.folder_panes()
            .into_iter()
            .find(|pane| view.is_ancestor(*pane))
            .cloned()
    }

    /// The pointer moved to `point` in `view`, or left it: over an item it
    /// is a hand and underlines the item's name, and resting there selects
    /// the item.
    fn pointer_over_click_items(&self, view: &gtk::Widget, point: Option<(f64, f64)>) {
        if !self.single_click_opens() {
            return;
        }
        let position = point.and_then(|point| self.click_item_at(view, point));
        let same = self
            .imp()
            .single_click
            .pointed
            .borrow()
            .as_ref()
            .is_some_and(|pointed| {
                Some(pointed.position) == position && pointed.view.upgrade().as_ref() == Some(view)
            });
        if same {
            return;
        }
        // Before the hand: forgetting the item before takes its view's
        // pointer away, which is this view's when the pointer went
        // straight from one item to another.
        self.forget_pointed_item();
        view.set_cursor_from_name(position.map(|_| "pointer"));
        let Some(position) = position else {
            return;
        };
        let cell = self
            .pane_holding(view)
            .and_then(|pane| pane.owners().file_cell_at(position, view));
        if let Some(cell) = &cell {
            cell.add_css_class(POINTED_CLASS);
        }
        let timer = glib::timeout_add_local_once(
            HOVER_SELECT_DELAY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.select_pointed_item()
            ),
        );
        self.imp().single_click.pointed.replace(Some(Pointed {
            view: view.downgrade(),
            position,
            cell: cell.as_ref().map(ObjectExt::downgrade).unwrap_or_default(),
            timer: Some(timer),
        }));
    }

    /// Keeps the item pointed at from being selected by the pointer
    /// resting there, until it moves to another item: a key (any but a
    /// modifier alone, [`Self::install_slow_click_rename`]) or a press
    /// took over the selection.
    pub(super) fn stop_pointing_selection(&self) {
        let timer = self
            .imp()
            .single_click
            .pointed
            .borrow_mut()
            .as_mut()
            .and_then(|pointed| pointed.timer.take());
        if let Some(timer) = timer {
            timer.remove();
        }
    }

    /// The pointer has rested on an item: selects it, as Windows' point to
    /// select does, unless a button is held now (a drag or a band), a
    /// dialog is in front or a name is being edited. Ctrl held now adds it
    /// to the selection, Shift selects from the item with keyboard focus
    /// (Gallery: the item Shift extends from) to it.
    fn select_pointed_item(&self) {
        let (view, position) = {
            let mut pointed = self.imp().single_click.pointed.borrow_mut();
            let Some(pointed) = pointed.as_mut() else {
                return;
            };
            pointed.timer = None;
            let Some(view) = pointed.view.upgrade() else {
                return;
            };
            (view, pointed.position)
        };
        let Some((state, pointer)) = held_now(&view) else {
            return;
        };
        // The view may have scrolled under the resting pointer, which then
        // points at another item, or at none: only the item under it now
        // is selected, and the dwell starts again on another one.
        let under = pointer.and_then(|point| self.click_item_at(&view, point));
        if under != Some(position) {
            self.forget_pointed_item();
            if let Some(point) = pointer {
                self.pointer_over_click_items(&view, Some(point));
            }
            return;
        }
        let quiet = !state.intersects(held_buttons())
            && self.single_click_opens()
            && !self.band_is_shown()
            && !self.are_item_clicks_paused()
            && !self.dialog_in_front()
            && !self.focus_is_in_text_field();
        if !quiet {
            return;
        }
        if let Some(canvas) = view.downcast_ref::<GalleryCanvas>() {
            canvas.point_to_select(position, state);
            return;
        }
        let Some(pane) = self.pane_holding(&view) else {
            return;
        };
        let model = pane.model();
        if position >= model.n_items() {
            return;
        }
        let selection = model.selection();
        if state.contains(gdk::ModifierType::CONTROL_MASK) {
            selection.select_item(position, false);
        } else if let Some(anchor) = pane
            .focused_position()
            .filter(|_| state.contains(gdk::ModifierType::SHIFT_MASK))
        {
            let first = anchor.min(position);
            selection.select_range(first, anchor.max(position) - first + 1, true);
        } else if model.selected_positions() != [position] {
            model.select_only(position);
        }
    }

    /// Forgets the item the pointer is on: its name is no longer
    /// underlined and resting there selects nothing.
    fn forget_pointed_item(&self) {
        let Some(pointed) = self.imp().single_click.pointed.take() else {
            return;
        };
        if let Some(timer) = pointed.timer {
            timer.remove();
        }
        if let Some(cell) = pointed.cell.upgrade() {
            cell.remove_css_class(POINTED_CLASS);
        }
        if let Some(view) = pointed.view.upgrade() {
            view.set_cursor(None);
        }
    }
}

/// The modifiers and pointer buttons held now on `view`'s seat, read when
/// pointing selects rather than at the pointer's last move: Ctrl or Shift
/// pressed or a button let go while the pointer rests count.
fn held_now(view: &gtk::Widget) -> Option<(gdk::ModifierType, Option<(f64, f64)>)> {
    let seat = view.display().default_seat()?;
    let native = view.native()?;
    let surface = native.surface()?;
    let (x, y, pointer) = surface.device_position(&seat.pointer()?)?;
    let keys = seat
        .keyboard()
        .map_or_else(gdk::ModifierType::empty, |keyboard| keyboard.modifier_state());
    // Where the pointer is now in `view`: the surface's coordinates less
    // the native's offset in it, then from the native to the view.
    let (offset_x, offset_y) = native.surface_transform();
    #[expect(clippy::cast_possible_truncation, reason = "pixels on a screen")]
    let in_native = gtk::graphene::Point::new((x - offset_x) as f32, (y - offset_y) as f32);
    let in_view = native
        .compute_point(view, &in_native)
        .map(|point| (f64::from(point.x()), f64::from(point.y())));
    Some((pointer | keys, in_view))
}

/// Whether the pointer at `to` is past the desktop's drag threshold from
/// `from`, as GTK measures it (`gtk_drag_check_threshold`).
fn is_past_drag_threshold(from: (f64, f64), to: (f64, f64)) -> bool {
    let threshold = gtk::Settings::default().map_or(DEFAULT_DRAG_THRESHOLD, |settings| {
        settings.gtk_dnd_drag_threshold()
    });
    let threshold = f64::from(threshold);
    (to.0 - from.0).abs() > threshold || (to.1 - from.1).abs() > threshold
}

/// The views of `pane` a click opens items in: Details, the icon grid
/// (List, the icon sizes, Tiles and Content) and Gallery.
fn click_views(pane: &FolderPane) -> [gtk::Widget; 3] {
    [
        pane.details().column_view().clone().upcast(),
        pane.icon_view().grid().clone().upcast(),
        pane.gallery().canvas().clone().upcast(),
    ]
}

/// Adds `class` to `widget` or takes it away.
fn set_class(widget: &gtk::Widget, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Click items as follows" reads its two preferences: Double-click by
    /// default, as in Windows, and Single-click with names underlined
    /// always or only when pointed at.
    ///
    /// parity: SEL-018
    #[test]
    fn the_click_mode_follows_folder_options() {
        let mut preferences = Preferences::default();
        assert_eq!(ClickMode::of(&preferences), ClickMode::Double);
        preferences.underline_titles_always = true;
        assert_eq!(
            ClickMode::of(&preferences),
            ClickMode::Double,
            "underlining needs single-click"
        );
        preferences.single_click_open = true;
        assert_eq!(
            ClickMode::of(&preferences),
            ClickMode::Single {
                underline_always: true
            }
        );
        preferences.underline_titles_always = false;
        assert_eq!(
            ClickMode::of(&preferences),
            ClickMode::Single {
                underline_always: false
            }
        );
    }
}
