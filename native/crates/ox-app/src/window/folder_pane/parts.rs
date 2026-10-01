// SPDX-License-Identifier: AGPL-3.0-only
//! The widgets of the folder pane and the folder model they show.
//!
//! Builds the `main` area of `v2.0.0:desktop/ui/index.html`: the details and icon
//! views in a stack of their own, the empty page and the landing page, all
//! in the stack of pages [`super::PanePage`] names.

use std::rc::Rc;

use gtk::prelude::*;

use crate::folder_view::cells::CellOwners;
use crate::folder_view::details::DetailsView;
use crate::folder_view::grid::{IconSize, IconView};
use crate::folder_view::model::FolderModel;
use crate::window::empty_page::EmptyPage;
use crate::window::loading_line::LoadingLine;

use super::{FolderView, PanePage};

/// The folder pane's widgets and the folder model they show.
#[derive(Debug)]
pub(super) struct PaneParts {
    /// The listing, the empty page or the landing page ([`PanePage`]).
    pub(super) stack: gtk::Stack,
    /// The details or the icon view ([`FolderView`]).
    pub(super) views: gtk::Stack,
    /// The details view.
    pub(super) details: DetailsView,
    /// The icon view.
    pub(super) icon_view: IconView,
    /// The active tab's filtered, sorted and selectable items.
    pub(super) model: FolderModel,
    /// Maps cell widgets to their rows.
    pub(super) owners: Rc<CellOwners>,
    /// The empty, loading and error page.
    pub(super) empty: EmptyPage,
    /// The landing page's contents.
    pub(super) landing: gtk::Box,
    /// The line over the pane while a folder is listed.
    pub(super) loading_line: LoadingLine,
    /// The note over the pane that says what a drag would do there, such
    /// as "Open with convert".
    pub(super) drag_hint: gtk::Label,
    /// The rubber band over the views while one is drawn (SEL-012).
    pub(super) rubber_band: gtk::Box,
}

impl PaneParts {
    /// The pane's widgets, showing nothing yet.
    pub(super) fn new() -> Self {
        let model = FolderModel::new();
        let owners = CellOwners::new();
        let details = DetailsView::new(&model, &owners);
        let icon_view = IconView::new(&owners);
        let views = view_stack(&details, &icon_view);
        let empty = EmptyPage::new();
        let (landing, landing_scroll) = landing_page();
        let stack = page_stack(&views, &empty, &landing_scroll);
        Self {
            stack,
            views,
            details,
            icon_view,
            model,
            owners,
            empty,
            landing,
            loading_line: LoadingLine::new(),
            drag_hint: drag_hint(),
            rubber_band: rubber_band(),
        }
    }
}

/// The note that says what a drag would do (`.tab-drag-hint` in
/// style.css), hidden until a drag needs it. It never takes the pointer,
/// so drops go to the view beneath it.
fn drag_hint() -> gtk::Label {
    gtk::Label::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .can_target(false)
        .visible(false)
        .css_classes(["drag-hint"])
        .build()
}

/// The rectangle a rubber band draws, hidden until one is drawn. It never
/// takes the pointer.
fn rubber_band() -> gtk::Box {
    gtk::Box::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .can_target(false)
        .visible(false)
        .css_classes(["rubber-band"])
        .build()
}

/// The details and icon views, one of them shown.
fn view_stack(details: &DetailsView, icon_view: &IconView) -> gtk::Stack {
    let views = gtk::Stack::new();
    views.add_named(details, Some(FolderView::Details.stack_name()));
    let icons = FolderView::Icons(IconSize::LARGE);
    views.add_named(icon_view, Some(icons.stack_name()));
    views
}

/// The landing page's contents and the scroller around them.
fn landing_page() -> (gtk::Box, gtk::ScrolledWindow) {
    let landing = gtk::Box::new(gtk::Orientation::Vertical, 0);
    landing.add_css_class("page");
    let landing_scroll = scrolled(&landing);
    landing_scroll.add_css_class("landing");
    (landing, landing_scroll)
}

/// The folder pane's pages, one of them shown: the listing, the empty
/// or error page and the landing page.
fn page_stack(views: &gtk::Stack, empty: &EmptyPage, landing_scroll: &gtk::ScrolledWindow) -> gtk::Stack {
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    stack.add_css_class("folder-pane");
    stack.add_named(views, Some(PanePage::Listing.name()));
    stack.add_named(&empty.root, Some(PanePage::Empty.name()));
    stack.add_named(landing_scroll, Some(PanePage::Landing.name()));
    stack
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(child)
        .build()
}
