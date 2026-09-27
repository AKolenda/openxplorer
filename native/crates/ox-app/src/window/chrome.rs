// SPDX-License-Identifier: AGPL-3.0-only
//! Window controls and layout. Navigation behavior lives in the controller.

use gtk::prelude::*;
use gtk::{gio, pango};

use crate::icons;

pub(super) struct Chrome {
    pub tabs: gtk::Box,
    pub address: gtk::Stack,
    pub entry: gtk::Entry,
    pub crumbs: gtk::Box,
    pub search: gtk::SearchEntry,
    pub status: gtk::Label,
    pub hint: gtk::Label,
    pub message: gtk::Label,
    pub workspace: gtk::Paned,
}

pub(super) fn icon_button(glyph: &str, tooltip: &str, action: &str) -> gtk::Button {
    gtk::Button::builder()
        .child(&icons::glyph(glyph, 16))
        .tooltip_text(tooltip)
        .action_name(action)
        .build()
}

pub(super) fn text_button(label: &str, glyph: &str, action: &str) -> gtk::Button {
    let child = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    child.append(&icons::glyph(glyph, 17));
    child.append(&gtk::Label::new(Some(label)));
    gtk::Button::builder()
        .child(&child)
        .action_name(action)
        .css_classes(["text-command"])
        .build()
}

fn menu_button(label: &str, glyph: &str, menu: &gio::Menu) -> gtk::MenuButton {
    let child = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    child.append(&icons::glyph(glyph, 17));
    child.append(&gtk::Label::new(Some(label)));
    child.append(&icons::glyph("down", 10));
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.add_css_class("ox-menu");
    gtk::MenuButton::builder()
        .child(&child)
        .popover(&popover)
        .css_classes(["text-command"])
        .build()
}

fn command_bar() -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.add_css_class("commandbar");
    bar.append(&text_button("New tab", "plus", "win.new-tab"));
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    // File mutation commands are added only when their native workflows exist.
    bar.append(&text_button("Open", "folderline", "win.open"));
    let sort = gio::Menu::new();
    for column in crate::folder_view::sorting::SortColumn::ALL {
        sort.append(Some(column.label()), Some(&format!("win.sort::{}", column.key())));
    }
    let direction = gio::Menu::new();
    direction.append(Some("Ascending"), Some("win.direction::ascending"));
    direction.append(Some("Descending"), Some("win.direction::descending"));
    sort.append_section(None, &direction);
    bar.append(&menu_button("Sort", "sort", &sort));

    let view = gio::Menu::new();
    view.append(Some("Details"), Some("win.view::details"));
    for size in crate::folder_view::grid::IconSize::ALL {
        view.append(Some(size.label()), Some(&format!("win.view::{}", size.key())));
    }
    let visibility = gio::Menu::new();
    visibility.append(Some("Show hidden files"), Some("win.hidden"));
    visibility.append(Some("Details pane"), Some("win.details-pane"));
    view.append_section(None, &visibility);
    bar.append(&menu_button("View", "grid", &view));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    bar.append(&text_button("Details", "details", "win.details-pane"));

    let appearance = gio::Menu::new();
    for (label, value) in [("System", "system"), ("Light", "light"), ("Dark", "dark")] {
        appearance.append(Some(label), Some(&format!("win.theme::{value}")));
    }
    let text = gio::Menu::new();
    text.append(Some("Larger text"), Some("win.text-larger"));
    text.append(Some("Smaller text"), Some("win.text-smaller"));
    text.append(Some("Reset text size"), Some("win.text-reset"));
    appearance.append_section(Some("Text size"), &text);
    bar.append(&menu_button("Appearance", "sun", &appearance));
    bar
}

impl Chrome {
    pub fn new(window: &gtk::ApplicationWindow) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title.add_css_class("ox-titlebar");
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tabs.add_css_class("tabs");
        let tab_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .child(&tabs)
            .build();
        title.append(&tab_scroll);
        let new_tab = icon_button("plus", "New tab (Ctrl+T)", "win.new-tab");
        new_tab.add_css_class("newtab");
        title.append(&new_tab);
        let controls = gtk::WindowControls::new(gtk::PackType::End);
        title.append(&controls);
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&title));
        window.set_titlebar(Some(&handle));

        let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        navigation.add_css_class("navrow");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        buttons.add_css_class("nav-buttons");
        for (glyph, label, action) in [
            ("back", "Back (Alt+Left)", "win.back"),
            ("forward", "Forward (Alt+Right)", "win.forward"),
            ("up", "Up (Alt+Up)", "win.up"),
            ("refresh", "Refresh (F5)", "win.refresh"),
        ] {
            buttons.append(&icon_button(glyph, label, action));
        }
        navigation.append(&buttons);
        let address_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        address_box.add_css_class("address");
        address_box.set_hexpand(true);
        address_box.append(&icons::glyph("folderline", 17));
        let address = gtk::Stack::builder().hexpand(true).hhomogeneous(false).build();
        let entry = gtk::Entry::builder().hexpand(true).build();
        entry.update_property(&[gtk::accessible::Property::Label("Folder location")]);
        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let crumb_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&crumbs)
            .build();
        address.add_named(&crumb_scroll, Some("crumbs"));
        address.add_named(&entry, Some("entry"));
        address_box.append(&address);
        address_box.append(&icon_button("down", "Edit location (Ctrl+L)", "win.location"));
        navigation.append(&address_box);
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Filter this folder")
            .width_request(205)
            .build();
        search.update_property(&[gtk::accessible::Property::Label("Filter this folder")]);
        search.add_css_class("search");
        navigation.append(&search);
        root.append(&navigation);
        root.append(&command_bar());

        let message = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .visible(false)
            .build();
        message.add_css_class("window-message");
        message.set_selectable(true);
        root.append(&message);
        let workspace = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .vexpand(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .resize_start_child(false)
            .build();
        workspace.add_css_class("workspace");
        root.append(&workspace);

        let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        status_bar.add_css_class("statusbar");
        let status = gtk::Label::builder().xalign(0.0).build();
        let hint = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        hint.add_css_class("typeahead-hint");
        status_bar.append(&status);
        status_bar.append(&hint);
        let details = icon_button("list", "Details view", "win.view");
        details.set_action_target_value(Some(&"details".to_variant()));
        let grid = icon_button("grid", "Large icons", "win.view");
        grid.set_action_target_value(Some(&"large".to_variant()));
        status_bar.append(&details);
        status_bar.append(&grid);
        root.append(&status_bar);
        window.set_child(Some(&root));
        Self {
            tabs,
            address,
            entry,
            crumbs,
            search,
            status,
            hint,
            message,
            workspace,
        }
    }
}
