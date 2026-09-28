// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar and landing pages composed from settings and mounted volumes.

use std::rc::Rc;

use gtk::prelude::*;

use crate::locations::Page;
use crate::{icons, locations, volumes};

use super::BrowserWindow;

impl BrowserWindow {
    pub(super) fn render_sidebar(self: &Rc<Self>) {
        while let Some(child) = self.sidebar.first_child() {
            self.sidebar.remove(&child);
        }
        self.sidebar_uris.borrow_mut().clear();
        for page in Page::ALL {
            self.sidebar_row(page.title(), Some(page.uri()), page.glyph(), None);
        }
        self.sidebar_separator();
        self.sidebar_row("Home folder", Some(&self.home_uri), "home", None);
        for place in ox_core::places::quick_access(&self.settings) {
            self.sidebar_row(
                &place.label,
                Some(&place.uri),
                place.glyph().unwrap_or("folderline"),
                place.glyph_color(),
            );
        }
        self.sidebar_separator();
        for volume in volumes::from_monitor(&self.volume_monitor) {
            let glyph = if volume.kind == volumes::VolumeKind::Device {
                "phone"
            } else {
                "drive"
            };
            self.sidebar_row(&volume.label, volume.uri.as_deref(), glyph, None);
        }
        for share in &self.settings.shares {
            self.sidebar_row(&share.label, Some(&share.uri), "network", Some("#2f9a67"));
        }
        if let Some(uri) = self.current_uri() {
            self.select_sidebar(&uri);
        }
    }

    fn sidebar_row(&self, label: &str, uri: Option<&str>, glyph: &str, color: Option<&str>) {
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 11);
        let pill = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pill.add_css_class("pill");
        pill.set_valign(gtk::Align::Center);
        content.append(&pill);
        let icon = match color {
            Some(color) => icons::colored_glyph(glyph, 18, color),
            None => icons::glyph(glyph, 18),
        };
        content.append(&icon);
        content.append(
            &gtk::Label::builder()
                .label(label)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        row.set_child(Some(&content));
        row.set_sensitive(uri.is_some());
        row.set_tooltip_text(Some(&uri.map(locations::address_text).unwrap_or_else(|| {
            "Mount this device in your desktop before browsing it.".to_string()
        })));
        self.sidebar.append(&row);
        self.sidebar_uris.borrow_mut().push(uri.map(str::to_string));
    }

    fn sidebar_separator(&self) {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(false);
        row.add_css_class("separator-row");
        let line = gtk::Separator::new(gtk::Orientation::Horizontal);
        line.add_css_class("side-separator");
        row.set_child(Some(&line));
        self.sidebar.append(&row);
        self.sidebar_uris.borrow_mut().push(None);
    }

    pub(super) fn select_sidebar(&self, uri: &str) {
        let selected = self.sidebar_uris.borrow().iter().position(|candidate| {
            candidate
                .as_deref()
                .is_some_and(|candidate| locations::same_location(candidate, uri))
        });
        let row = selected.and_then(|index| self.sidebar.row_at_index(index as i32));
        self.sidebar.select_row(row.as_ref());
    }

    pub(super) fn render_landing(self: &Rc<Self>) {
        let Some(page) = self.current_uri().as_deref().and_then(Page::from_uri) else {
            return;
        };
        let body = &self.content.landing;
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }
        let title = gtk::Label::builder().label(page.title()).xalign(0.0).build();
        title.add_css_class("page-title");
        body.append(&title);
        let subtitle = gtk::Label::builder()
            .label(page.subtitle())
            .xalign(0.0)
            .wrap(true)
            .build();
        subtitle.add_css_class("page-subtitle");
        body.append(&subtitle);
        if page != Page::Network {
            self.landing_heading("Quick access");
            let cards = self.card_grid();
            cards.insert(&self.location_card("Home folder", &self.home_uri, "home"), -1);
            for place in ox_core::places::quick_access(&self.settings) {
                let glyph_name = place.glyph().unwrap_or("folderline");
                cards.insert(&self.location_card(&place.label, &place.uri, glyph_name), -1);
            }
            body.append(&cards);
        }
        if page == Page::ThisPc {
            self.landing_heading("Devices and drives");
            let cards = self.card_grid();
            cards.insert(&self.location_card("Local Disk", "file:///", "drive"), -1);
            for volume in volumes::from_monitor(&self.volume_monitor) {
                if let Some(uri) = volume.uri {
                    let glyph = if volume.kind == volumes::VolumeKind::Device {
                        "phone"
                    } else {
                        "drive"
                    };
                    cards.insert(&self.location_card(&volume.label, &uri, glyph), -1);
                }
            }
            body.append(&cards);
        }
        if page != Page::ThisPc {
            self.landing_heading("Network locations");
            let cards = self.card_grid();
            for share in &self.settings.shares {
                cards.insert(&self.location_card(&share.label, &share.uri, "network"), -1);
            }
            body.append(&cards);
            if self.settings.shares.is_empty() {
                let empty = gtk::Label::builder()
                    .label("No saved network locations. Enter a mounted share's address in the location bar.")
                    .xalign(0.0)
                    .wrap(true)
                    .build();
                empty.add_css_class("quiet");
                body.append(&empty);
            }
        }
    }

    fn landing_heading(&self, text: &str) {
        let heading = gtk::Label::builder().label(text).xalign(0.0).build();
        heading.add_css_class("section-title");
        self.content.landing.append(&heading);
    }

    fn card_grid(&self) -> gtk::FlowBox {
        gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .column_spacing(10)
            .row_spacing(10)
            .min_children_per_line(1)
            .max_children_per_line(4)
            .homogeneous(true)
            .build()
    }

    fn location_card(self: &Rc<Self>, name: &str, uri: &str, glyph: &str) -> gtk::Button {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 15);
        content.append(&icons::glyph(glyph, 32));
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 4);
        texts.set_hexpand(true);
        texts.set_valign(gtk::Align::Center);
        let label = gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        label.add_css_class("card-name");
        texts.append(&label);
        let description = gtk::Label::builder()
            .label(locations::address_text(uri))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(24)
            .build();
        description.add_css_class("card-sub");
        texts.append(&description);
        content.append(&texts);
        let button = gtk::Button::builder()
            .child(&content)
            .css_classes(["quick-card"])
            .build();
        let weak = Rc::downgrade(self);
        let uri = uri.to_string();
        button.connect_clicked(move |_| {
            if let Some(browser) = weak.upgrade() {
                browser.navigate_or_report(&uri);
            }
        });
        button
    }

    pub(super) fn watch_volumes(self: &Rc<Self>) {
        let changed = |weak: std::rc::Weak<Self>| {
            move || {
                if let Some(browser) = weak.upgrade() {
                    browser.render_sidebar();
                    browser.render_landing();
                }
            }
        };
        let mut signals = self.volume_signals.borrow_mut();
        let callback = changed(Rc::downgrade(self));
        signals.push(self.volume_monitor.connect_mount_added(move |_, _| callback()));
        let callback = changed(Rc::downgrade(self));
        signals.push(self.volume_monitor.connect_mount_removed(move |_, _| callback()));
        let callback = changed(Rc::downgrade(self));
        signals.push(self.volume_monitor.connect_mount_changed(move |_, _| callback()));
        let callback = changed(Rc::downgrade(self));
        signals.push(self.volume_monitor.connect_volume_added(move |_, _| callback()));
        let callback = changed(Rc::downgrade(self));
        signals.push(self.volume_monitor.connect_volume_removed(move |_, _| callback()));
        let callback = changed(Rc::downgrade(self));
        signals.push(self.volume_monitor.connect_volume_changed(move |_, _| callback()));
    }
}
