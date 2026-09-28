// SPDX-License-Identifier: AGPL-3.0-only
//! The widgets of the folder pane and the folder model they show.
//!
//! Builds the `main` area of `desktop/ui/index.html`: the details and icon
//! views in a stack of their own, the empty page and the landing page, all
//! in the stack of pages [`super::PanePage`] names.

use std::rc::Rc;

use gtk::prelude::*;

use crate::folder_view::cells::{CellOwners, IconCells};
use crate::folder_view::grid::{self, IconSize};
use crate::folder_view::{details, model::FolderModel};
use crate::theme::Appearance;
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
    pub(super) details: gtk::ColumnView,
    /// The scroller around the details view.
    pub(super) details_scroll: gtk::ScrolledWindow,
    /// The icon view.
    pub(super) grid: gtk::GridView,
    /// The scroller around the icon view.
    pub(super) grid_scroll: gtk::ScrolledWindow,
    /// The active tab's filtered, sorted and selectable items.
    pub(super) model: FolderModel,
    /// Bound item icons, redrawn when the theme or scale changes.
    pub(super) icons: Rc<IconCells>,
    /// Maps cell widgets to their rows.
    pub(super) owners: Rc<CellOwners>,
    /// The empty, loading and error page.
    pub(super) empty: EmptyPage,
    /// The landing page's contents.
    pub(super) landing: gtk::Box,
    /// The line over the pane while a folder is listed.
    pub(super) loading_line: LoadingLine,
}

impl PaneParts {
    /// The pane's widgets, drawing item icons in `appearance`.
    pub(super) fn new(appearance: Appearance) -> Self {
        let model = FolderModel::new();
        let icons = IconCells::new(appearance);
        let owners = CellOwners::new();
        let details = details::build(&model, &icons, &owners);
        let grid = grid::build(&icons, &owners, IconSize::Large);
        let details_scroll = scrolled(&details);
        let grid_scroll = scrolled(&grid);
        let views = view_stack(&details_scroll, &grid_scroll);
        let empty = EmptyPage::new();
        let (landing, landing_scroll) = landing_page();
        let stack = page_stack(&views, &empty, &landing_scroll);
        Self {
            stack,
            views,
            details,
            details_scroll,
            grid,
            grid_scroll,
            model,
            icons,
            owners,
            empty,
            landing,
            loading_line: LoadingLine::new(),
        }
    }
}

/// The details and icon views, one of them shown.
fn view_stack(details_scroll: &gtk::ScrolledWindow, grid_scroll: &gtk::ScrolledWindow) -> gtk::Stack {
    let views = gtk::Stack::new();
    views.add_named(details_scroll, Some(FolderView::Details.stack_name()));
    let icons = FolderView::Icons(IconSize::Large);
    views.add_named(grid_scroll, Some(icons.stack_name()));
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
