// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar: breadcrumbs, or an editable address.
//!
//! Ports `renderNavigation`, `editAddress` and `finishAddress` in
//! `desktop/ui/app.js`: the location's icon, then crumbs divided by `/`
//! (`\` on SMB, none after the `/` root) as the current app draws them.
//! The crumbs scroll sideways (a plain mouse wheel scrolls them) and stay
//! scrolled to the current folder, so a deep path never widens the
//! window. Clicking blank space or pressing Ctrl+L edits the address;
//! leaving the entry returns to the breadcrumbs.
//!
//! Each crumb activates `win.go-to`, names itself "Go to …" for screen
//! readers, shows its full address as a tooltip and opens in a background
//! tab on a middle-click. GTK's own Left and Right focus movement already
//! walks between crumbs.

use gtk::gdk;
use gtk::prelude::*;
use ox_core::location::Crumb;

use crate::icons::{self, ArtKind, Glyph};
use crate::theme::Appearance;

use super::gestures;

/// What the address bar shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressMode {
    /// One button per ancestor.
    Crumbs,
    /// A text entry holding the address.
    Entry,
}

impl AddressMode {
    const fn name(self) -> &'static str {
        match self {
            AddressMode::Crumbs => "crumbs",
            AddressMode::Entry => "entry",
        }
    }
}

/// One crumb as the bar shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CrumbButton {
    /// The folder or page.
    pub crumb: Crumb,
    /// The full address, for the tooltip.
    pub address: String,
    /// The `/` or `\\` drawn before the crumb, if any
    /// ([`ox_core::location::crumb_divider`]).
    pub divider_before: Option<&'static str>,
}

/// The icon at the start of the address bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AddressIcon {
    /// A line glyph: a page, a network location or a device.
    Glyph(Glyph),
    /// The colour folder of local folders.
    Folder,
}

/// How art is drawn: the appearance and the screen's scale factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtStyle {
    /// Light or dark art.
    pub appearance: Appearance,
    /// The screen's scale factor.
    pub scale: i32,
}

/// The address bar's widgets.
#[derive(Debug)]
pub(super) struct AddressBar {
    /// The bordered box in the navigation row.
    pub root: gtk::Box,
    icon: gtk::Image,
    stack: gtk::Stack,
    /// The editable address.
    pub entry: gtk::Entry,
    crumbs: gtk::Box,
    crumb_scroll: gtk::ScrolledWindow,
}

impl AddressBar {
    /// An address bar that shows no location yet.
    pub fn new() -> Self {
        let root = gtk::Box::builder()
            .spacing(10)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .css_classes(["address"])
            .build();
        let icon = icons::glyph(Glyph::FolderLine, 17);
        icon.add_css_class("address-icon");
        let entry = gtk::Entry::builder().hexpand(true).build();
        entry.update_property(&[gtk::accessible::Property::Label("Location")]);
        let crumbs = gtk::Box::builder()
            .spacing(2)
            .css_classes(["breadcrumbs"])
            .build();
        let crumb_scroll = crumb_scroller(&crumbs);
        let stack = gtk::Stack::builder().hexpand(true).hhomogeneous(false).build();
        stack.add_named(&crumb_scroll, Some(AddressMode::Crumbs.name()));
        stack.add_named(&entry, Some(AddressMode::Entry.name()));
        root.append(&icon);
        root.append(&stack);
        root.append(&edit_button());
        let bar = Self {
            root,
            icon,
            stack,
            entry,
            crumbs,
            crumb_scroll,
        };
        bar.keep_current_folder_visible();
        gestures::scroll_sideways_with_wheel(&bar.crumb_scroll);
        bar.edit_on_blank_click();
        bar.show_crumbs_when_focus_leaves();
        bar
    }

    /// Scrolls to the last crumb whenever the crumbs or the width change,
    /// as `crumbs.scrollLeft = crumbs.scrollWidth` in app.js. `changed`
    /// fires for new bounds only, so the user can still scroll back.
    fn keep_current_folder_visible(&self) {
        self.crumb_scroll.hadjustment().connect_changed(|adjustment| {
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
        });
    }

    /// A click on blank space around the crumbs starts editing. Crumb
    /// buttons claim their own clicks, so only blank space gets here.
    fn edit_on_blank_click(&self) {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let stack = self.stack.downgrade();
        click.connect_released(move |gesture, _, _, _| {
            let Some(stack) = stack.upgrade() else {
                return;
            };
            if stack.visible_child_name().as_deref() != Some(AddressMode::Crumbs.name()) {
                return;
            }
            // The action exists on every browser window.
            let _ = stack.activate_action("win.location", None);
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        self.crumb_scroll.add_controller(click);
    }

    /// Leaving the entry, for another widget or another window, returns to
    /// the breadcrumbs (`blur` → `finishAddress`).
    fn show_crumbs_when_focus_leaves(&self) {
        let focus = gtk::EventControllerFocus::new();
        let stack = self.stack.downgrade();
        focus.connect_leave(move |_| {
            let Some(stack) = stack.upgrade() else {
                return;
            };
            // Hiding the entry makes it lose focus again; do nothing then.
            if stack.visible_child_name().as_deref() == Some(AddressMode::Entry.name()) {
                stack.set_visible_child_name(AddressMode::Crumbs.name());
            }
        });
        self.entry.add_controller(focus);
    }

    /// What the bar shows now.
    pub fn mode(&self) -> AddressMode {
        let shown = self.stack.visible_child_name();
        if shown.as_deref() == Some(AddressMode::Entry.name()) {
            AddressMode::Entry
        } else {
            AddressMode::Crumbs
        }
    }

    /// Shows the location's crumbs, address text and icon. Text the user
    /// is typing is left alone.
    pub fn show_location(&self, crumbs: &[CrumbButton], address: &str, icon: AddressIcon, style: ArtStyle) {
        match icon {
            AddressIcon::Glyph(glyph) => icons::set_glyph(&self.icon, glyph, 17),
            AddressIcon::Folder => {
                icons::set_art(&self.icon, ArtKind::Folder, 17, style.appearance, style.scale);
            }
        }
        self.root.set_tooltip_text(Some(&format!(
            "{address} · Click blank space or press Ctrl+L to edit"
        )));
        if self.mode() == AddressMode::Crumbs {
            self.entry.set_text(address);
        }
        while let Some(child) = self.crumbs.first_child() {
            self.crumbs.remove(&child);
        }
        let last = crumbs.len().saturating_sub(1);
        for (index, crumb) in crumbs.iter().enumerate() {
            if let Some(divider) = crumb.divider_before {
                self.crumbs.append(&divider_label(divider));
            }
            let button = crumb_button(crumb);
            if index == last {
                button.update_property(&[gtk::accessible::Property::Description("Current location")]);
            }
            self.crumbs.append(&button);
        }
    }

    /// Replaces the entry with the breadcrumbs, resetting the entry to
    /// `address` so typed text is discarded.
    pub fn show_crumbs(&self, address: &str) {
        self.entry.set_text(address);
        self.stack.set_visible_child_name(AddressMode::Crumbs.name());
    }

    /// Shows the entry holding `address`, focused with all text selected.
    pub fn edit(&self, address: &str) {
        self.entry.set_text(address);
        self.stack.set_visible_child_name(AddressMode::Entry.name());
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    /// The crumb buttons shown, for tests.
    #[cfg(test)]
    pub fn crumb_buttons(&self) -> Vec<gtk::Button> {
        let mut buttons = Vec::new();
        let mut child = self.crumbs.first_child();
        while let Some(widget) = child {
            buttons.extend(widget.clone().downcast::<gtk::Button>().ok());
            child = widget.next_sibling();
        }
        buttons
    }

    /// The crumbs' horizontal scroll position, for tests.
    #[cfg(test)]
    pub fn crumb_adjustment(&self) -> gtk::Adjustment {
        self.crumb_scroll.hadjustment()
    }
}

/// The scroller around `crumbs`. External: scrollable without a visible
/// bar, and never asking the window to be as wide as the path.
fn crumb_scroller(crumbs: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(true)
        .hexpand(true)
        .child(crumbs)
        .build()
}

/// The chevron at the end of the bar that edits the address
/// (`#address-edit`).
fn edit_button() -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::glyph(Glyph::Down, 12))
        .tooltip_text("Edit location (Ctrl+L)")
        .action_name("win.location")
        .valign(gtk::Align::Center)
        .css_classes(["address-chevron"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Edit location")]);
    button
}

/// The `/` or `\\` between crumbs, hidden from screen readers as
/// `aria-hidden` hides it in app.js.
fn divider_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .css_classes(["crumb-divider"])
        .build()
}

/// A crumb that opens its folder, named "Go to …" for screen readers.
fn crumb_button(crumb: &CrumbButton) -> gtk::Button {
    let uri = crumb.crumb.uri.as_str();
    let button = gtk::Button::builder()
        .label(&crumb.crumb.label)
        .tooltip_text(&crumb.address)
        .action_name("win.go-to")
        .action_target(&uri.to_variant())
        .css_classes(["crumb"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&format!(
        "Go to {}",
        crumb.crumb.label
    ))]);
    gestures::open_folder_on_middle_click(&button, uri);
    button
}
