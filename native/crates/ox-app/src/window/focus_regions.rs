// SPDX-License-Identifier: AGPL-3.0-only
//! F6 and Shift+F6: keyboard focus from one region of the window to the
//! next (ACC-015).
//!
//! Windows Explorer cycles its panes with F6; here the regions are, in
//! order, the tab strip, the address bar, the search box, the command
//! bar, the sidebar, the file list (each pane of a split tab, the left
//! one first, as Tab visits them, `tab_order.rs`), the Details pane and
//! the status bar, wrapping round. Microsoft gives Windows 11's F6 order
//! as "Tabs", "Navigation pane", "Content pane", "Details pane (if
//! shown)" and "Status bar" ("Use a screen reader to explore and navigate
//! File Explorer in Windows"); the address bar, search box and command
//! bar, which Windows reaches with Tab from the tabs, are F6 stops here
//! too.
//! With Settings' "F6 moves like Windows" on, F6 visits only the regions
//! Microsoft lists (the tabs, the sidebar, the file list and each pane
//! of a split tab, the Details pane and the status bar), and the address
//! bar, search box and command bar are left to Tab; from one of those
//! three, F6 goes on to the next region in the order above.
//! A hidden or disabled region is skipped. Focus lands on the region's
//! current item: the front tab, the highlighted sidebar row, the item that
//! last had focus in the file list; elsewhere on the region's first
//! control. The selection never changes. The keys work from text fields
//! too, but not while a dialog is open.
//!
//! Alt pressed and released with no other key or click between moves the
//! keyboard to the command bar's first button, with its focus ring, as
//! Microsoft says of File Explorer: "Alt moves focus to the app bar" ("Use
//! a screen reader to explore and navigate File Explorer in Windows").
//! Escape there gives the keyboard back to the control it left. Alt with
//! another key is only that shortcut.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::session::PaneSide;
use super::BrowserWindow;

/// A region F6 visits, in the order it visits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Region {
    Tabs,
    Address,
    Search,
    Commands,
    Sidebar,
    /// The file list; the left pane of a split tab.
    Files,
    /// The right pane of a split tab, shown only then.
    EndPane,
    Details,
    /// The status bar's view buttons.
    StatusBar,
}

impl Region {
    /// Every region, in F6's order.
    const ALL: [Region; 9] = [
        Region::Tabs,
        Region::Address,
        Region::Search,
        Region::Commands,
        Region::Sidebar,
        Region::Files,
        Region::EndPane,
        Region::Details,
        Region::StatusBar,
    ];

    /// Whether F6 visits this region when it moves like Windows 11's:
    /// not the address bar, search box or command bar, which Windows
    /// reaches with Tab from the tabs.
    fn in_windows_cycle(self) -> bool {
        !matches!(self, Region::Address | Region::Search | Region::Commands)
    }

    /// Where this region comes in F6's full order.
    fn order(self) -> usize {
        Region::ALL
            .iter()
            .position(|region| *region == self)
            .unwrap_or_default()
    }
}

/// The region after `current` among `reachable`, in F6's order (before
/// it `backward`), wrapping round; the first reachable one (the last
/// `backward`) when focus is in none. `current` need not be reachable
/// itself: from a region F6 passes over, it goes on from where that
/// region sits.
fn next_region(current: Option<Region>, reachable: &[Region], backward: bool) -> Option<Region> {
    let (first, last) = (reachable.first().copied()?, reachable.last().copied()?);
    let Some(current) = current else {
        return Some(if backward { last } else { first });
    };
    let at = current.order();
    let next = if backward {
        reachable
            .iter()
            .rev()
            .find(|region| region.order() < at)
            .copied()
            .unwrap_or(last)
    } else {
        reachable
            .iter()
            .find(|region| region.order() > at)
            .copied()
            .unwrap_or(first)
    };
    Some(next)
}

/// F6 moves to the next region, Shift+F6 to the previous one.
const REGION_KEYS: [(&str, bool); 2] = [("F6", false), ("<Shift>F6", true)];

/// F6 and Shift+F6 under the names the keyboard shortcuts window knows
/// them by (CMD-032): `region-next` and `region-previous`.
pub(super) fn region_key_bindings() -> impl Iterator<Item = (String, &'static str)> {
    REGION_KEYS.into_iter().map(|(keys, backward)| {
        let name = if backward {
            "region-previous"
        } else {
            "region-next"
        };
        (name.to_owned(), keys)
    })
}

impl BrowserWindow {
    /// Adds F6 and Shift+F6, before any focused widget sees them.
    pub(super) fn install_focus_regions(&self) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        for (keys, backward) in REGION_KEYS {
            let action = gtk::CallbackAction::new(move |widget, _| {
                let Some(window) = widget.downcast_ref::<BrowserWindow>() else {
                    return glib::Propagation::Proceed;
                };
                if window.dialog_layer().shown().is_some() {
                    return glib::Propagation::Proceed;
                }
                // F6 moves the keyboard, so it shows where it went, as
                // Tab does (focus_ring.rs).
                if window.focus_next_region(backward).is_some() {
                    window.set_focus_visible(true);
                }
                glib::Propagation::Stop
            });
            let trigger = gtk::ShortcutTrigger::parse_string(keys);
            shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(action)));
        }
        self.add_controller(shortcuts);
        self.install_alt_alone();
    }

    /// Alt alone moves the keyboard to the command bar, Escape there
    /// gives it back (see the module's text).
    fn install_alt_alone(&self) {
        // Whether the Alt held now has had no other key or click with it.
        let alone = Rc::new(Cell::new(false));
        // The control Alt took the keyboard from, while it is in the
        // command bar because of Alt.
        let left: Rc<RefCell<glib::WeakRef<gtk::Widget>>> = Rc::default();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[strong]
            alone,
            #[strong]
            left,
            move |keys, key, _, modifiers| {
                let is_alt = matches!(key, gdk::Key::Alt_L | gdk::Key::Alt_R);
                // Only the modifiers that make a shortcut count; Caps Lock
                // and Num Lock do not.
                let others = gdk::ModifierType::SHIFT_MASK
                    | gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::SUPER_MASK
                    | gdk::ModifierType::META_MASK;
                alone.set(is_alt && !modifiers.intersects(others));
                if key != gdk::Key::Escape || modifiers.intersects(others | gdk::ModifierType::ALT_MASK) {
                    return glib::Propagation::Proceed;
                }
                let Some(window) = keys.widget().and_downcast::<BrowserWindow>() else {
                    return glib::Propagation::Proceed;
                };
                let back = left.replace(glib::WeakRef::new()).upgrade();
                match back {
                    Some(widget)
                        if window.focused_region() == Some(Region::Commands) && widget.is_mapped() =>
                    {
                        widget.grab_focus();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        ));
        keys.connect_key_released(glib::clone!(
            #[strong]
            alone,
            #[strong]
            left,
            move |keys, key, _, _| {
                let was_alone = alone.replace(false);
                if !was_alone || !matches!(key, gdk::Key::Alt_L | gdk::Key::Alt_R) {
                    return;
                }
                let Some(window) = keys.widget().and_downcast::<BrowserWindow>() else {
                    return;
                };
                if window.dialog_layer().shown().is_some()
                    || window.focused_region() == Some(Region::Commands)
                    || !window.region_is_reachable(Region::Commands)
                {
                    return;
                }
                let before = gtk::prelude::GtkWindowExt::focus(&window);
                if window.focus_region(Region::Commands) {
                    window.set_focus_visible(true);
                    left.replace(before.map(|widget| widget.downgrade()).unwrap_or_default());
                }
            }
        ));
        self.add_controller(keys);
        // A click with Alt held makes it no Alt alone, as in Windows.
        let clicks = gtk::GestureClick::new();
        clicks.set_button(0);
        clicks.set_propagation_phase(gtk::PropagationPhase::Capture);
        clicks.connect_pressed(glib::clone!(
            #[strong]
            alone,
            move |_, _, _, _| alone.set(false)
        ));
        self.add_controller(clicks);
        // Leaving the window (Alt+Tab) ends an Alt, so its release seen on
        // coming back does not count as Alt alone.
        self.connect_is_active_notify(move |_| alone.set(false));
        // Escape gives the keyboard back only while it is still in the
        // command bar Alt moved it to: once it leaves (F6, a click, Tab),
        // Escape there is Escape again.
        self.connect_notify_local(Some("focus-widget"), move |window, _| {
            if window.focused_region() != Some(Region::Commands) {
                left.replace(glib::WeakRef::new());
            }
        });
    }

    /// Moves keyboard focus to the next region that can take it, or the
    /// previous one `backward`; returns the region focused.
    pub(super) fn focus_next_region(&self, backward: bool) -> Option<Region> {
        let like_windows = self.context().settings_data().preferences.f6_like_windows;
        let reachable: Vec<Region> = Region::ALL
            .into_iter()
            .filter(|region| !like_windows || region.in_windows_cycle())
            .filter(|region| self.region_is_reachable(*region))
            .collect();
        let mut candidate = next_region(self.focused_region(), &reachable, backward);
        // A region that turns out to hold nothing focusable is passed over.
        for _ in 0..reachable.len() {
            let region = candidate?;
            if self.focus_region(region) {
                return Some(region);
            }
            candidate = next_region(Some(region), &reachable, backward);
        }
        None
    }

    /// The region holding keyboard focus, if any.
    pub(super) fn focused_region(&self) -> Option<Region> {
        let focus = gtk::prelude::GtkWindowExt::focus(self)?;
        Region::ALL.into_iter().find(|region| {
            let widget = self.region_widget(*region);
            focus == widget || focus.is_ancestor(&widget)
        })
    }

    /// The widget that holds `region`.
    fn region_widget(&self, region: Region) -> gtk::Widget {
        match region {
            Region::Tabs => self.tab_strip().clone().upcast(),
            Region::Address => self.address_bar().clone().upcast(),
            Region::Search => self.search_box().clone().upcast(),
            Region::Commands => self.command_bar().clone().upcast(),
            Region::Sidebar => self.sidebar().clone().upcast(),
            Region::Files => self.pane_on(PaneSide::Start).clone().upcast(),
            Region::EndPane => self.pane_on(PaneSide::End).clone().upcast(),
            Region::Details => self.details_pane().clone().upcast(),
            Region::StatusBar => self.status_bar().clone().upcast(),
        }
    }

    /// Whether `region` is shown and enabled.
    fn region_is_reachable(&self, region: Region) -> bool {
        let widget = self.region_widget(region);
        widget.is_mapped() && widget.is_sensitive()
    }

    /// Focuses the current item of `region`, else its first control;
    /// false when it has nothing to focus.
    fn focus_region(&self, region: Region) -> bool {
        match region {
            Region::Files | Region::EndPane => {
                let side = if region == Region::Files {
                    PaneSide::Start
                } else {
                    PaneSide::End
                };
                let pane = self.pane_on(side);
                match pane.focused_position() {
                    Some(position) => pane.focus_item(position),
                    None => pane.focus_view(),
                }
                pane.view_has_focus()
            }
            Region::Tabs => {
                let strip = self.tab_strip();
                strip.focus_front_tab() || strip.child_focus(gtk::DirectionType::TabForward)
            }
            Region::Sidebar => {
                let list = self.sidebar().list();
                match list.selected_row() {
                    Some(row) => row.grab_focus(),
                    None => list.child_focus(gtk::DirectionType::TabForward),
                }
            }
            _ => self
                .region_widget(region)
                .child_focus(gtk::DirectionType::TabForward),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ACC-015
    #[test]
    fn f6_goes_round_the_shown_regions_and_shift_goes_back() {
        let shown = [Region::Address, Region::Sidebar, Region::Files];
        assert_eq!(next_region(None, &shown, false), Some(Region::Address));
        assert_eq!(
            next_region(Some(Region::Address), &shown, false),
            Some(Region::Sidebar)
        );
        assert_eq!(
            next_region(Some(Region::Files), &shown, false),
            Some(Region::Address)
        );
        assert_eq!(
            next_region(Some(Region::Address), &shown, true),
            Some(Region::Files)
        );
        assert_eq!(
            next_region(Some(Region::Details), &shown, false),
            Some(Region::Address)
        );
        assert_eq!(next_region(None, &[], false), None);
    }

    /// From a region F6 passes over, such as the address bar while F6
    /// moves like Windows, F6 goes on from where that region sits.
    ///
    /// parity: ACC-015
    #[test]
    fn f6_goes_on_from_a_region_it_passes_over() {
        let cycle: Vec<Region> = Region::ALL
            .into_iter()
            .filter(|region| region.in_windows_cycle())
            .collect();
        assert_eq!(
            cycle,
            [
                Region::Tabs,
                Region::Sidebar,
                Region::Files,
                Region::EndPane,
                Region::Details,
                Region::StatusBar
            ]
        );
        for skipped in [Region::Address, Region::Search, Region::Commands] {
            assert_eq!(next_region(Some(skipped), &cycle, false), Some(Region::Sidebar));
            assert_eq!(next_region(Some(skipped), &cycle, true), Some(Region::Tabs));
        }
        assert_eq!(next_region(None, &cycle, true), Some(Region::StatusBar));
    }
}
