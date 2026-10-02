// SPDX-License-Identifier: AGPL-3.0-only
//! F6 and Shift+F6: keyboard focus from one region of the window to the
//! next (ACC-015).
//!
//! Windows Explorer cycles its panes with F6; here the regions are, in
//! order, the tab strip, the address bar, the search box, the command
//! bar, the sidebar, the file list and the Details pane, wrapping round.
//! A hidden or disabled region is skipped. Focus lands on the region's
//! current item: the front tab, the highlighted sidebar row, the item that
//! last had focus in the file list; elsewhere on the region's first
//! control. The selection never changes. The keys work from text fields
//! too, but not while a dialog is open.

use gtk::glib;
use gtk::prelude::*;

use super::BrowserWindow;

/// A region F6 visits, in the order it visits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Region {
    Tabs,
    Address,
    Search,
    Commands,
    Sidebar,
    Files,
    Details,
}

impl Region {
    /// Every region, in F6's order.
    const ALL: [Region; 7] = [
        Region::Tabs,
        Region::Address,
        Region::Search,
        Region::Commands,
        Region::Sidebar,
        Region::Files,
        Region::Details,
    ];
}

/// The region after `current` among `reachable` (before it `backward`),
/// wrapping round; the first reachable one when focus is in none.
fn next_region(current: Option<Region>, reachable: &[Region], backward: bool) -> Option<Region> {
    let count = reachable.len();
    if count == 0 {
        return None;
    }
    let index = current.and_then(|region| reachable.iter().position(|candidate| *candidate == region));
    let next = match (index, backward) {
        (None, false) => 0,
        (None, true) => count - 1,
        (Some(index), false) => (index + 1) % count,
        (Some(index), true) => (index + count - 1) % count,
    };
    Some(reachable[next])
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
                window.focus_next_region(backward);
                glib::Propagation::Stop
            });
            let trigger = gtk::ShortcutTrigger::parse_string(keys);
            shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(action)));
        }
        self.add_controller(shortcuts);
    }

    /// Moves keyboard focus to the next region that can take it, or the
    /// previous one `backward`; returns the region focused.
    pub(super) fn focus_next_region(&self, backward: bool) -> Option<Region> {
        let reachable: Vec<Region> = Region::ALL
            .into_iter()
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
            Region::Files => self.folder_pane().clone().upcast(),
            Region::Details => self.details_pane().clone().upcast(),
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
            Region::Files => {
                let pane = self.folder_pane();
                match pane.focused_position() {
                    Some(position) => pane.focus_item(position),
                    None => pane.focus_view(),
                }
                pane.view_has_focus()
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
}
