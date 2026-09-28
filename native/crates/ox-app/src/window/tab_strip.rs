// SPDX-License-Identifier: AGPL-3.0-only
//! The tab strip in the title bar.
//!
//! Ports `renderTabs` in `desktop/ui/app.js` and `.tab` in `style.css`:
//! each tab is 215 pixels wide with its icon, title and close button, and
//! tabs shrink toward 100 pixels and then scroll sideways ([`TabLayout`]),
//! so opening many tabs never widens the window. The whole tab is the
//! click target, as in app.js: it is one focusable widget announced as a
//! tab of the "Folder tabs" list with its selected state; a click or Enter
//! shows it and a middle-click closes it. The close button inside claims
//! its own clicks.

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::icons::{self, ArtKind, Glyph};

use super::appearance::ArtStyle;
use super::gestures;
use super::session::TabId;
use super::tab_layout::TabLayout;
use super::widget_tree::remove_children;
use super::window_action::WindowAction;

/// The tab icon's edge: 16 pixels (ui-spec.md I03; the web app's was 17).
const ICON_SIZE: i32 = 16;
/// The gap between the icon and the title (ui-spec.md S06).
const ICON_TO_TITLE: i32 = 10;
/// The glyph of a tab's close button.
const CLOSE_GLYPH: i32 = 12;

/// A tab's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TabIcon {
    /// A line glyph: a landing page or a device.
    Glyph(Glyph),
    /// Colour art: a folder, or a folder on a network share.
    Art(ArtKind),
}

/// What the strip shows of one tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabView {
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
    layout: TabLayout,
}

impl TabStrip {
    /// An empty tab strip.
    pub fn new() -> Self {
        let layout = TabLayout::new();
        let tabs = gtk::Box::builder()
            .accessible_role(gtk::AccessibleRole::TabList)
            .layout_manager(&layout)
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
        Self {
            root,
            viewport,
            tabs,
            layout,
        }
    }

    /// Makes tabs `width` pixels wide when there is room: 215, or less in
    /// a narrow window.
    pub fn set_tab_width(&self, width: i32) {
        self.layout.set_tab_width(width);
    }

    /// Replaces the tabs with `tabs` and scrolls the active one into view.
    pub fn show(&self, tabs: &[TabView], style: ArtStyle) {
        remove_children(&self.tabs);
        let mut active = None;
        for tab in tabs {
            let widget = tab_widget(tab, style);
            self.tabs.append(&widget);
            if tab.active {
                active = Some(widget);
            }
        }
        let Some(active) = active else {
            return;
        };
        // After the new tabs are laid out, so their positions are known.
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = viewport)]
            self.viewport,
            move || viewport.scroll_to(&active, None)
        ));
    }

    /// The tab list, for tests.
    #[cfg(test)]
    pub fn tab_list(&self) -> &gtk::Box {
        &self.tabs
    }
}

fn tab_icon(icon: TabIcon, style: ArtStyle) -> gtk::Image {
    match icon {
        TabIcon::Glyph(glyph) => icons::glyph(glyph, ICON_SIZE),
        TabIcon::Art(kind) => style.image(kind, ICON_SIZE),
    }
}

/// The widget of `tab`: its icon, title and close button, one focusable
/// target that shows the tab on a click or Enter and closes it on a
/// middle-click.
fn tab_widget(tab: &TabView, style: ArtStyle) -> gtk::Box {
    let widget = gtk::Box::builder()
        .spacing(ICON_TO_TITLE)
        .focusable(true)
        .accessible_role(gtk::AccessibleRole::Tab)
        .tooltip_text(&tab.tooltip)
        .css_classes(["tab"])
        .build();
    if tab.active {
        widget.add_css_class("active");
    }
    widget.update_property(&[gtk::accessible::Property::Label(&tab.title)]);
    widget.update_state(&[gtk::accessible::State::Selected(Some(tab.active))]);
    widget.append(&tab_icon(tab.icon, style));
    widget.append(&title(&tab.title));
    widget.append(&close_button(tab));
    let id = tab.id.to_variant();
    widget.add_controller(select_on_click(id.clone()));
    widget.add_controller(select_on_enter(id.clone()));
    widget.add_controller(gestures::middle_click(move |gesture, _, _| {
        run_on(gesture.widget(), WindowAction::CloseTabById, &id);
    }));
    widget
}

/// Runs the tab action `action` on the tab `id` from `widget`, when the
/// gesture still has one.
fn run_on(widget: Option<gtk::Widget>, action: WindowAction, id: &glib::Variant) {
    if let Some(widget) = widget {
        action.activate_from(&widget, Some(id));
    }
}

/// A primary click anywhere on the tab shows it.
fn select_on_click(id: glib::Variant) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.connect_pressed(move |gesture, _, _, _| {
        run_on(gesture.widget(), WindowAction::SelectTab, &id);
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
        run_on(keys.widget(), WindowAction::SelectTab, &id);
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

/// The tab's close button, named `Close <title>` for screen readers.
fn close_button(tab: &TabView) -> gtk::Button {
    let close = gtk::Button::builder()
        .child(&icons::glyph(Glyph::Close, CLOSE_GLYPH))
        .tooltip_text("Close tab")
        .action_name(WindowAction::CloseTabById.detailed_name())
        .action_target(&tab.id.to_variant())
        .focus_on_click(false)
        .valign(gtk::Align::Center)
        .css_classes(["tab-close"])
        .build();
    let name = format!("Close {}", tab.title);
    close.update_property(&[gtk::accessible::Property::Label(&name)]);
    close
}
