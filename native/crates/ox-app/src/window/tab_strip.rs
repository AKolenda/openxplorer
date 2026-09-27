// SPDX-License-Identifier: AGPL-3.0-only
//! The tab strip in the title bar.
//!
//! Ports `renderTabs` in `desktop/ui/app.js` and `.tab` in `style.css`:
//! each tab is 215 pixels wide with its icon, title and close button, and
//! tabs shrink toward 80 pixels and then scroll sideways ([`TabLayout`]),
//! so opening many tabs never widens the window. The whole tab is the
//! click target, as in app.js: it is one focusable widget announced as a
//! tab of the "Folder tabs" list with its selected state; a click or Enter
//! shows it and a middle-click closes it. The close button inside claims
//! its own clicks.

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::icons::{self, ArtKind, Glyph};
use crate::theme::Appearance;

use super::gestures;
use super::session::TabId;
use super::tab_layout::TabLayout;

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
    /// The tab shown.
    pub id: TabId,
    /// The tab's title.
    pub title: String,
    /// The full address.
    pub tooltip: String,
    /// The tab's icon.
    pub icon: TabIcon,
    /// The tab is in front.
    pub active: bool,
}

/// The tab strip's widgets.
#[derive(Debug)]
pub(super) struct TabStrip {
    /// The scrolling strip.
    pub root: gtk::ScrolledWindow,
    viewport: gtk::Viewport,
    tabs: gtk::Box,
}

impl TabStrip {
    /// An empty tab strip.
    pub fn new() -> Self {
        let tabs = gtk::Box::builder()
            .accessible_role(gtk::AccessibleRole::TabList)
            .layout_manager(&TabLayout::new())
            .css_classes(["tabs"])
            .build();
        tabs.update_property(&[gtk::accessible::Property::Label("Folder tabs")]);
        let viewport = gtk::Viewport::builder().child(&tabs).build();
        // External: scrollable without a visible bar, and never asking the
        // window to be as wide as all the tabs. Not expanding: the "+"
        // follows the last tab and the drag area takes the rest.
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(true)
            .hexpand(false)
            .valign(gtk::Align::End)
            .child(&viewport)
            .build();
        gestures::scroll_sideways_with_wheel(&root);
        Self { root, viewport, tabs }
    }

    /// Replaces the tabs and scrolls the active one into view.
    pub fn show(&self, labels: &[TabLabel], appearance: Appearance, scale: i32) {
        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        let mut active = None;
        for label in labels {
            let tab = tab(label, appearance, scale);
            self.tabs.append(&tab);
            if label.active {
                active = Some(tab);
            }
        }
        if let Some(active) = active {
            // After the new tabs are laid out, so their positions are known.
            let viewport = self.viewport.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(viewport) = viewport.upgrade() {
                    viewport.scroll_to(&active, None);
                }
            });
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
        TabIcon::Glyph(glyph) => icons::glyph(glyph, 17),
        TabIcon::Art(kind) => icons::art_image(kind, 17, appearance, scale),
    }
}

fn tab(label: &TabLabel, appearance: Appearance, scale: i32) -> gtk::Box {
    let tab = gtk::Box::builder()
        .spacing(9)
        .focusable(true)
        .accessible_role(gtk::AccessibleRole::Tab)
        .tooltip_text(&label.tooltip)
        .css_classes(["tab"])
        .build();
    if label.active {
        tab.add_css_class("active");
    }
    tab.update_property(&[gtk::accessible::Property::Label(&label.title)]);
    tab.update_state(&[gtk::accessible::State::Selected(Some(label.active))]);
    tab.append(&tab_icon(label.icon, appearance, scale));
    tab.append(&title(&label.title));
    tab.append(&close_button(label));
    let id = label.id.to_variant();
    tab.add_controller(select_on_click(id.clone()));
    tab.add_controller(select_on_enter(id.clone()));
    tab.add_controller(gestures::middle_click(move |gesture, _, _| {
        run_on(gesture.widget(), "win.close-tab-by-id", &id);
    }));
    tab
}

/// Runs the window action `action` from `widget`, when there is one.
fn run_on(widget: Option<gtk::Widget>, action: &str, target: &glib::Variant) {
    if let Some(widget) = widget {
        // Every browser window has the tab actions.
        let _ = widget.activate_action(action, Some(target));
    }
}

/// A primary click anywhere on the tab shows it.
fn select_on_click(id: glib::Variant) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.connect_pressed(move |gesture, _, _, _| {
        run_on(gesture.widget(), "win.select-tab", &id);
    });
    click
}

/// Enter or Space on a focused tab shows it.
fn select_on_enter(id: glib::Variant) -> gtk::EventControllerKey {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |keys, key, _, _| {
        let activates = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space);
        if !activates {
            return glib::Propagation::Proceed;
        }
        run_on(keys.widget(), "win.select-tab", &id);
        glib::Propagation::Stop
    });
    keys
}

fn title(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["tab-title"])
        .build()
}

/// The tab's close button, named "Close <title>" for screen readers.
fn close_button(label: &TabLabel) -> gtk::Button {
    let close = gtk::Button::builder()
        .child(&icons::glyph(Glyph::Close, 12))
        .tooltip_text("Close tab")
        .action_name("win.close-tab-by-id")
        .action_target(&label.id.to_variant())
        .focus_on_click(false)
        .valign(gtk::Align::Center)
        .css_classes(["tab-close"])
        .build();
    let name = format!("Close {}", label.title);
    close.update_property(&[gtk::accessible::Property::Label(&name)]);
    close
}
