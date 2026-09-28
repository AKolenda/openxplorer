// SPDX-License-Identifier: AGPL-3.0-only
//! The folder pane: the details and icon views, the empty and error state,
//! the landing pages and the loading line.
//!
//! Ports the `main` area of `desktop/ui/index.html` and `renderContent` in
//! `desktop/ui/app.js`. Only the visible view is attached to the selection
//! model: a hidden `GtkGridView` still builds and binds its tiles for every
//! change, which made large folders several times slower to list.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::folder_view::cells::{BoundIcons, CellOwners};
use crate::folder_view::grid::{IconSize, IconView};
use crate::folder_view::{details, model::FolderModel};
use crate::theme::Appearance;

use super::empty_page::{EmptyPage, EmptyState};
use super::loading_line::LoadingLine;

/// What the folder pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContentPage {
    /// The folder's items.
    Listing,
    /// The empty, filtered-out, loading or error state.
    Empty,
    /// A landing page (This PC, Network).
    Landing,
}

impl ContentPage {
    const fn name(self) -> &'static str {
        match self {
            ContentPage::Listing => "listing",
            ContentPage::Empty => "empty",
            ContentPage::Landing => "landing",
        }
    }
}

/// Which view lists the items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderView {
    /// Rows with Name, Date modified, Type and Size columns.
    Details,
    /// Icon tiles of one size.
    Icons(IconSize),
}

impl FolderView {
    /// The `win.view` action state: `details` or an icon size key.
    pub fn key(self) -> &'static str {
        match self {
            FolderView::Details => "details",
            FolderView::Icons(size) => size.key(),
        }
    }

    /// The view for a `win.view` action state.
    pub fn from_key(key: &str) -> Option<FolderView> {
        if key == "details" {
            return Some(FolderView::Details);
        }
        IconSize::from_key(key).map(FolderView::Icons)
    }

    /// The view saved in settings. The Python app knows one icon view,
    /// "grid", which is Large icons.
    pub fn from_setting(view: &str) -> FolderView {
        if view == "grid" {
            FolderView::Icons(IconSize::Large)
        } else {
            FolderView::Details
        }
    }

    /// The value settings store: only `details` and `grid` are valid for
    /// the Python app, so every icon size is saved as `grid`.
    pub fn setting(self) -> &'static str {
        match self {
            FolderView::Details => "details",
            FolderView::Icons(_) => "grid",
        }
    }

    const fn stack_name(self) -> &'static str {
        match self {
            FolderView::Details => "details",
            FolderView::Icons(_) => "grid",
        }
    }
}

/// The details and icon views, one of them shown.
fn view_stack(details_scroll: &gtk::ScrolledWindow, icon_view: &IconView) -> gtk::Stack {
    let views = gtk::Stack::new();
    views.add_named(details_scroll, Some(FolderView::Details.stack_name()));
    let icons = FolderView::Icons(IconSize::Large);
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
    stack.add_named(views, Some(ContentPage::Listing.name()));
    stack.add_named(&empty.root, Some(ContentPage::Empty.name()));
    stack.add_named(landing_scroll, Some(ContentPage::Landing.name()));
    stack
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(child)
        .build()
}

/// The folder pane's widgets and the shared folder model.
#[derive(Debug)]
pub(super) struct Content {
    /// The folder pane, with the loading line over it.
    pub root: gtk::Overlay,
    stack: gtk::Stack,
    views: gtk::Stack,
    /// The details view.
    pub details: gtk::ColumnView,
    details_scroll: gtk::ScrolledWindow,
    /// The icon view.
    pub icon_view: IconView,
    /// The active tab's filtered, sorted and selectable items.
    pub model: FolderModel,
    /// Bound item icons, redrawn when the theme or scale changes.
    pub icons: Rc<BoundIcons>,
    /// Maps cell widgets to their rows.
    pub owners: Rc<CellOwners>,
    /// The empty, loading and error page.
    pub empty: EmptyPage,
    /// The landing page's contents.
    pub landing: gtk::Box,
    loading_line: LoadingLine,
}

impl Content {
    /// An empty folder pane in the details view, drawing icons in `appearance`.
    pub fn new(appearance: Appearance) -> Self {
        let model = FolderModel::new();
        let icons = BoundIcons::new(appearance);
        let owners = CellOwners::new();
        let details = details::build(&model, &icons, &owners);
        let icon_view = IconView::new(&icons, &owners);
        let details_scroll = scrolled(&details);
        let views = view_stack(&details_scroll, &icon_view);
        let empty = EmptyPage::new();
        let (landing, landing_scroll) = landing_page();
        let stack = page_stack(&views, &empty, &landing_scroll);
        let loading_line = LoadingLine::new();
        let root = gtk::Overlay::builder().child(&stack).build();
        root.add_overlay(&loading_line.widget);
        let content = Self {
            root,
            stack,
            views,
            details,
            details_scroll,
            icon_view,
            model,
            icons,
            owners,
            empty,
            landing,
            loading_line,
        };
        content.show_view(FolderView::Details);
        content
    }

    /// Shows `page`.
    pub fn show_page(&self, page: ContentPage) {
        self.stack.set_visible_child_name(page.name());
    }

    /// The page shown now.
    pub fn page(&self) -> Option<ContentPage> {
        let name = self.stack.visible_child_name()?;
        [ContentPage::Listing, ContentPage::Empty, ContentPage::Landing]
            .into_iter()
            .find(|page| page.name() == name.as_str())
    }

    /// Shows the empty page in `state`.
    pub fn show_empty(&self, state: &EmptyState) {
        self.empty.show(state);
        self.show_page(ContentPage::Empty);
    }

    /// Shows the loading line over the items while `loading` lasts (see
    /// [`LoadingLine::set_loading`]).
    pub fn show_loading_line(&self, loading: bool) {
        self.loading_line.set_loading(loading);
    }

    /// The loading line, for tests.
    #[cfg(test)]
    pub fn loading_line(&self) -> &gtk::Box {
        &self.loading_line.widget
    }

    /// The view that lists items now.
    pub fn view(&self) -> FolderView {
        let icons = FolderView::Icons(self.icon_view.icon_size());
        let shown = self.views.visible_child_name();
        if shown.as_deref() == Some(icons.stack_name()) {
            icons
        } else {
            FolderView::Details
        }
    }

    /// Switches views. Only the visible view holds the selection model.
    pub fn show_view(&self, view: FolderView) {
        let selection = self.model.selection();
        let grid = self.icon_view.grid();
        match view {
            FolderView::Details => {
                grid.set_model(None::<&gtk::MultiSelection>);
                self.details.set_model(Some(selection));
            }
            FolderView::Icons(size) => {
                self.icon_view.set_icon_size(size);
                self.details.set_model(None::<&gtk::MultiSelection>);
                grid.set_model(Some(selection));
                self.icon_view.fit_columns();
            }
        }
        self.views.set_visible_child_name(view.stack_name());
    }

    /// The visible view's vertical scroll adjustment.
    fn visible_vadjustment(&self) -> gtk::Adjustment {
        match self.view() {
            FolderView::Details => self.details_scroll.vadjustment(),
            FolderView::Icons(_) => self.icon_view.vadjustment(),
        }
    }

    /// The visible view's vertical scroll position.
    pub fn scroll_position(&self) -> f64 {
        self.visible_vadjustment().value()
    }

    /// Scrolls the visible view to `position` once the view has measured
    /// its new items; set straight after a model change, the position
    /// would be clamped to the old, shorter list.
    pub fn restore_scroll_position(&self, position: f64) {
        let adjustment = self.visible_vadjustment();
        adjustment.set_value(position);
        glib::idle_add_local_once(move || adjustment.set_value(position));
    }

    /// True while keyboard focus is inside the visible view.
    pub fn has_focus(&self) -> bool {
        let grid = self.icon_view.grid();
        match self.view() {
            FolderView::Details => self.details.has_focus() || self.details.focus_child().is_some(),
            FolderView::Icons(_) => grid.has_focus() || grid.focus_child().is_some(),
        }
    }

    /// Moves keyboard focus into the visible view.
    pub fn focus(&self) {
        match self.view() {
            FolderView::Details => self.details.grab_focus(),
            FolderView::Icons(_) => self.icon_view.grid().grab_focus(),
        };
    }

    /// Scrolls to `position` and gives it keyboard focus.
    pub fn reveal(&self, position: u32) {
        let grid = self.icon_view.grid();
        match self.view() {
            FolderView::Details => self
                .details
                .scroll_to(position, None, gtk::ListScrollFlags::FOCUS, None),
            FolderView::Icons(_) => grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None),
        }
    }

    /// The visible view, as a widget.
    pub fn view_widget(&self) -> gtk::Widget {
        match self.view() {
            FolderView::Details => self.details.clone().upcast(),
            FolderView::Icons(_) => self.icon_view.grid().clone().upcast(),
        }
    }

    /// Draws the icon view's cells for text of `percent` size.
    pub fn set_text_size(&self, percent: u32) {
        self.icon_view.set_text_size(percent);
    }
}
