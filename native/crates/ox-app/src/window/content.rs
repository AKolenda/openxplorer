// SPDX-License-Identifier: AGPL-3.0-only
//! Native folder views, empty states and the selection's details pane.

use std::rc::Rc;

use gtk::prelude::*;

use crate::folder_view::cells::{CellOwners, IconCells};
use crate::folder_view::{details, grid, model::FolderModel};
use crate::icons;
use crate::theme::Appearance;

pub(super) struct Content {
    pub root: gtk::Box,
    pub stack: gtk::Stack,
    pub views: gtk::Stack,
    pub details: gtk::ColumnView,
    pub grid: gtk::GridView,
    pub model: FolderModel,
    pub icons: Rc<IconCells>,
    pub owners: Rc<CellOwners>,
    pub empty_title: gtk::Label,
    pub empty_message: gtk::Label,
    pub landing: gtk::Box,
    pub inspector: gtk::Box,
    pub inspector_icon: gtk::Image,
    pub inspector_name: gtk::Label,
    pub inspector_info: gtk::Label,
    pub spinner: gtk::Spinner,
}

fn scroll(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(child)
        .build()
}

impl Content {
    pub fn new(appearance: Appearance) -> Self {
        let model = FolderModel::new();
        let icons = IconCells::new(appearance);
        let owners = CellOwners::new();
        let details = details::build(&model, &icons, &owners);
        let grid = grid::build(&model, &icons, &owners, grid::IconSize::Large);
        let views = gtk::Stack::new();
        views.add_named(&scroll(&details), Some("details"));
        views.add_named(&scroll(&grid), Some("grid"));

        let empty = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["empty-state"])
            .build();
        let spinner = gtk::Spinner::new();
        let empty_title = gtk::Label::new(None);
        empty_title.add_css_class("empty-title");
        let empty_message = gtk::Label::builder().wrap(true).max_width_chars(65).build();
        empty.append(&spinner);
        empty.append(&empty_title);
        empty.append(&empty_message);
        let landing = gtk::Box::new(gtk::Orientation::Vertical, 8);
        landing.add_css_class("page");
        let landing_scroll = scroll(&landing);
        landing_scroll.add_css_class("landing");
        let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
        stack.add_css_class("folder-pane");
        stack.add_named(&views, Some("listing"));
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&landing_scroll, Some("landing"));

        let inspector = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .width_request(240)
            .css_classes(["details", "details-inner"])
            .build();
        let header = gtk::Label::builder().label("Details").xalign(0.0).build();
        header.add_css_class("detail-header");
        inspector.append(&header);
        let inspector_icon = icons::art_image(&icons::ArtKind::Folder, 112, appearance, 1);
        inspector_icon.set_halign(gtk::Align::Center);
        inspector_icon.set_valign(gtk::Align::Center);
        let preview = gtk::CenterBox::new();
        preview.add_css_class("preview");
        preview.set_center_widget(Some(&inspector_icon));
        inspector.append(&preview);
        let inspector_name = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .max_width_chars(28)
            .build();
        inspector_name.add_css_class("dname");
        inspector.append(&inspector_name);
        let inspector_info = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .selectable(true)
            .max_width_chars(28)
            .build();
        inspector_info.add_css_class("dtype");
        inspector.append(&inspector_info);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&stack);
        root.append(&inspector);
        Self {
            root,
            stack,
            views,
            details,
            grid,
            model,
            icons,
            owners,
            empty_title,
            empty_message,
            landing,
            inspector,
            inspector_icon,
            inspector_name,
            inspector_info,
            spinner,
        }
    }

    pub fn focus(&self) {
        if self.views.visible_child_name().as_deref() == Some("grid") {
            self.grid.grab_focus();
        } else {
            self.details.grab_focus();
        }
    }

    pub fn reveal(&self, position: u32) {
        if self.views.visible_child_name().as_deref() == Some("grid") {
            self.grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        } else {
            self.details
                .scroll_to(position, None, gtk::ListScrollFlags::FOCUS, None);
        }
    }
}
