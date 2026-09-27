// SPDX-License-Identifier: AGPL-3.0-only
//! The tab strip in the title bar.
//!
//! Ports `renderTabs` in `desktop/ui/app.js`. Tabs shrink toward their
//! minimum width and then scroll sideways, so opening many tabs never
//! widens the window. Each tab is announced as a tab of the "Folder tabs"
//! list with its selected state; a middle-click closes it.

use gtk::prelude::*;

use crate::icons::{self, ArtKind, Glyph};
use crate::theme::Appearance;

use super::gestures;
use super::session::TabId;

/// A tab's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TabIcon {
    /// A line glyph: a landing page or a device.
    Glyph(Glyph),
    /// Colour art: a folder, or a folder on a network share.
    Art(ArtKind),
}

/// One tab as the strip shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabLabel {
    pub id: TabId,
    pub title: String,
    /// The full address.
    pub tooltip: String,
    pub icon: TabIcon,
    pub active: bool,
}

/// The tab strip's widgets.
#[derive(Debug)]
pub(super) struct TabStrip {
    /// The scrolling strip.
    pub root: gtk::ScrolledWindow,
    tabs: gtk::Box,
}

impl TabStrip {
    pub fn new() -> Self {
        let tabs = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .accessible_role(gtk::AccessibleRole::TabList)
            .css_classes(["tabs"])
            .build();
        tabs.update_property(&[gtk::accessible::Property::Label("Folder tabs")]);
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(true)
            .hexpand(true)
            .child(&tabs)
            .build();
        gestures::scroll_sideways_with_wheel(&root);
        Self { root, tabs }
    }

    /// Replaces the tabs.
    pub fn show(&self, labels: &[TabLabel], appearance: Appearance, scale: i32) {
        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        for label in labels {
            self.tabs.append(&tab(label, appearance, scale));
        }
    }

    /// The tab list, for tests.
    #[cfg(test)]
    pub fn tab_list(&self) -> &gtk::Box {
        &self.tabs
    }
}

fn tab_icon(icon: TabIcon, appearance: Appearance, scale: i32) -> gtk::Image {
    match icon {
        TabIcon::Glyph(glyph) => icons::glyph(glyph, 16),
        TabIcon::Art(kind) => icons::art_image(kind, 17, appearance, scale),
    }
}

fn tab(label: &TabLabel, appearance: Appearance, scale: i32) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("tab");
    if label.active {
        row.add_css_class("active");
    }
    row.append(&tab_icon(label.icon, appearance, scale));
    row.append(&select_button(label));
    row.append(&close_button(label));
    let id = label.id.to_variant();
    let closer = gestures::middle_click(move |gesture, _, _| {
        if let Some(row) = gesture.widget() {
            // The action exists on every browser window.
            let _ = row.activate_action("win.close-tab-by-id", Some(&id));
        }
    });
    row.add_controller(closer);
    row
}

/// The focusable part of a tab, which screen readers announce as the tab.
fn select_button(label: &TabLabel) -> gtk::Button {
    let title = gtk::Label::builder()
        .label(&label.title)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(19)
        .build();
    let select = gtk::Button::builder()
        .child(&title)
        .tooltip_text(&label.tooltip)
        .accessible_role(gtk::AccessibleRole::Tab)
        .action_name("win.select-tab")
        .action_target(&label.id.to_variant())
        .build();
    select.update_state(&[gtk::accessible::State::Selected(Some(label.active))]);
    select
}

/// The tab's close button, named "Close <title>" for screen readers.
fn close_button(label: &TabLabel) -> gtk::Button {
    let close = gtk::Button::builder()
        .child(&icons::glyph(Glyph::Close, 12))
        .tooltip_text("Close tab")
        .action_name("win.close-tab-by-id")
        .action_target(&label.id.to_variant())
        .css_classes(["tab-close"])
        .build();
    let name = format!("Close {}", label.title);
    close.update_property(&[gtk::accessible::Property::Label(&name)]);
    close
}
