// SPDX-License-Identifier: AGPL-3.0-only
//! Dolphin's Advanced Permissions on the Permissions tab: Read, Write and
//! Exec for the owner, the group and others, and Set UID, Set GID and
//! Sticky, behind an "Advanced Permissions" expander. While it is open,
//! its bits are what the tab applies.

use gtk::prelude::*;

/// The classes of the grid's rows, with how far their bits are shifted.
const CLASSES: [(&str, u32); 3] = [("Owner", 6), ("Group", 3), ("Others", 0)];
/// The columns of the grid, with their bit within a class.
const ACCESSES: [(&str, u32); 3] = [("Read", 0o4), ("Write", 0o2), ("Exec", 0o1)];
/// The special bits, with their labels.
const SPECIAL: [(&str, u32); 3] = [("Set UID", 0o4000), ("Set GID", 0o2000), ("Sticky", 0o1000)];

/// The expander and its check boxes, each with the bit it sets.
#[derive(Debug, Clone)]
pub(super) struct AdvancedBits {
    expander: gtk::Expander,
    checks: Vec<(gtk::CheckButton, u32)>,
}

impl AdvancedBits {
    /// The advanced controls, closed, showing `mode`.
    pub(super) fn new(mode: u32) -> Self {
        let grid = gtk::Grid::builder().column_spacing(12).row_spacing(4).build();
        let mut checks = Vec::new();
        for (column, (name, _)) in (1..).zip(ACCESSES) {
            grid.attach(&heading(name), column, 0, 1, 1);
        }
        for (line, (class, shift)) in (1..).zip(CLASSES) {
            grid.attach(&heading(class), 0, line, 1, 1);
            for (column, (access, bit)) in (1..).zip(ACCESSES) {
                let bit = bit << shift;
                let check = gtk::CheckButton::new();
                check.set_active(mode & bit != 0);
                check.update_property(&[gtk::accessible::Property::Label(&format!("{class} {access}"))]);
                grid.attach(&check, column, line, 1, 1);
                checks.push((check, bit));
            }
        }
        let special = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        for (label, bit) in SPECIAL {
            let check = gtk::CheckButton::with_label(label);
            check.set_active(mode & bit != 0);
            special.append(&check);
            checks.push((check, bit));
        }
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.append(&grid);
        content.append(&special);
        let expander = gtk::Expander::builder()
            .label(&ox_core::i18n::gettext("Advanced Permissions"))
            .child(&content)
            .build();
        Self { expander, checks }
    }

    /// The expander, to add to the tab.
    pub(super) fn widget(&self) -> &gtk::Expander {
        &self.expander
    }

    /// Whether the advanced controls are open, and so apply.
    pub(super) fn is_expanded(&self) -> bool {
        self.expander.is_expanded()
    }

    /// Runs `changed` with the new state whenever the expander opens or
    /// closes.
    pub(super) fn connect_expanded(&self, changed: impl Fn(bool) + 'static) {
        self.expander
            .connect_expanded_notify(move |expander| changed(expander.is_expanded()));
    }

    /// Runs `changed` whenever the expander opens or closes or a bit is
    /// ticked or cleared.
    pub(super) fn connect_changed(&self, changed: impl Fn() + Clone + 'static) {
        let on_expanded = changed.clone();
        self.expander.connect_expanded_notify(move |_| on_expanded());
        for (check, _) in &self.checks {
            let changed = changed.clone();
            check.connect_toggled(move |_| changed());
        }
    }

    /// Ticks the check boxes of the bits of `mode`.
    pub(super) fn show(&self, mode: u32) {
        for (check, bit) in &self.checks {
            check.set_active(mode & bit != 0);
        }
    }

    /// The bits the check boxes describe.
    pub(super) fn mode(&self) -> u32 {
        self.checks
            .iter()
            .filter(|(check, _)| check.is_active())
            .fold(0, |mode, (_, bit)| mode | bit)
    }
}

/// A column or row heading of the grid.
fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["property-name"])
        .build()
}
