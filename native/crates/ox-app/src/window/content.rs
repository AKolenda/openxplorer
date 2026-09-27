// SPDX-License-Identifier: AGPL-3.0-only
//! The folder pane: the details and icon views, the empty and error state,
//! the landing pages and the loading line.
//!
//! Ports the `main` area of `desktop/ui/index.html` and `renderContent` in
//! `desktop/ui/app.js`. Only the visible view is attached to the selection
//! model: a hidden `GtkGridView` still builds and binds its tiles for every
//! change, which made large folders several times slower to list.

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::folder_view::cells::{CellOwners, IconCells};
use crate::folder_view::grid::{self, IconSize};
use crate::folder_view::{details, model::FolderModel};
use crate::icons::{self, Glyph};
use crate::theme::Appearance;

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

/// What the empty page says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EmptyState {
    /// The folder is still being listed.
    Loading,
    /// The folder could not be listed: the error text and a Try again button.
    Unavailable(String),
    /// The filter hides every item.
    NoMatches,
    /// The folder has no items.
    EmptyFolder,
}

/// The empty page's widgets.
#[derive(Debug)]
struct EmptyPage {
    root: gtk::Box,
    spinner: gtk::Spinner,
    icon: gtk::Image,
    title: gtk::Label,
    message: gtk::Label,
    retry: gtk::Button,
}

impl EmptyPage {
    fn new() -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["empty-state"])
            .build();
        let spinner = gtk::Spinner::new();
        let icon = icons::glyph(Glyph::FolderLine, 44);
        let title = gtk::Label::builder().css_classes(["empty-title"]).build();
        let message = gtk::Label::builder()
            .wrap(true)
            .max_width_chars(65)
            .justify(gtk::Justification::Center)
            .selectable(true)
            .build();
        let retry = gtk::Button::builder()
            .label("Try again")
            .action_name("win.refresh")
            .halign(gtk::Align::Center)
            .visible(false)
            .build();
        root.append(&spinner);
        root.append(&icon);
        root.append(&title);
        root.append(&message);
        root.append(&retry);
        Self {
            root,
            spinner,
            icon,
            title,
            message,
            retry,
        }
    }

    /// Shows `state`, with the app.js wording (`renderRows`).
    fn show(&self, state: &EmptyState) {
        let loading = *state == EmptyState::Loading;
        self.spinner.set_visible(loading);
        self.spinner.set_spinning(loading);
        self.icon.set_visible(!loading);
        let glyph = match state {
            EmptyState::Unavailable(_) => Glyph::Network,
            _ => Glyph::FolderLine,
        };
        icons::set_glyph(&self.icon, glyph, 44);
        let (title, message) = match state {
            EmptyState::Loading => ("Loading…", ""),
            EmptyState::Unavailable(error) => ("This location is unavailable", error.as_str()),
            EmptyState::NoMatches => ("No matching items", "Try a different filter."),
            EmptyState::EmptyFolder => ("This folder is empty", ""),
        };
        self.title.set_text(title);
        self.message.set_text(message);
        self.message.set_visible(!message.is_empty());
        self.retry
            .set_visible(matches!(state, EmptyState::Unavailable(_)));
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
    let landing = gtk::Box::new(gtk::Orientation::Vertical, 8);
    landing.add_css_class("page");
    let landing_scroll = scrolled(&landing);
    landing_scroll.add_css_class("landing");
    (landing, landing_scroll)
}

/// The thin line that runs above the items while a folder is listed.
fn loading_line() -> gtk::ProgressBar {
    gtk::ProgressBar::builder()
        .css_classes(["loading-line"])
        .visible(false)
        .build()
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
    /// The folder pane, including the loading line.
    pub root: gtk::Box,
    stack: gtk::Stack,
    views: gtk::Stack,
    /// The details view.
    pub details: gtk::ColumnView,
    details_scroll: gtk::ScrolledWindow,
    /// The icon view.
    pub grid: gtk::GridView,
    grid_scroll: gtk::ScrolledWindow,
    grid_size: Rc<Cell<IconSize>>,
    /// The active tab's filtered, sorted and selectable items.
    pub model: FolderModel,
    /// Bound item icons, redrawn when the theme or scale changes.
    pub icons: Rc<IconCells>,
    /// Maps cell widgets to their rows.
    pub owners: Rc<CellOwners>,
    empty: EmptyPage,
    /// The landing page's contents.
    pub landing: gtk::Box,
    loading_line: gtk::ProgressBar,
}

impl Content {
    /// An empty folder pane in the details view, drawing icons in `appearance`.
    pub fn new(appearance: Appearance) -> Self {
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
        let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
        stack.add_css_class("folder-pane");
        stack.add_named(&views, Some(ContentPage::Listing.name()));
        stack.add_named(&empty.root, Some(ContentPage::Empty.name()));
        stack.add_named(&landing_scroll, Some(ContentPage::Landing.name()));
        let loading_line = loading_line();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&loading_line);
        root.append(&stack);
        let content = Self {
            root,
            stack,
            views,
            details,
            details_scroll,
            grid,
            grid_scroll,
            grid_size: Rc::new(Cell::new(IconSize::Large)),
            model,
            icons,
            owners,
            empty,
            landing,
            loading_line,
        };
        content.show_view(FolderView::Details);
        content.fit_grid_columns_to_width();
        content
    }

    /// Shows `page`.
    pub fn show_page(&self, page: ContentPage) {
        self.stack.set_visible_child_name(page.name());
    }

    /// The page shown now, for tests.
    #[cfg(test)]
    pub fn page(&self) -> Option<ContentPage> {
        let name = self.stack.visible_child_name()?;
        [ContentPage::Listing, ContentPage::Empty, ContentPage::Landing]
            .into_iter()
            .find(|page| page.name() == name.as_str())
    }

    /// The empty page's title, for tests.
    #[cfg(test)]
    pub fn empty_title(&self) -> String {
        self.empty.title.text().to_string()
    }

    /// True when the empty page offers a Try again button that refreshes,
    /// for tests.
    #[cfg(test)]
    pub fn offers_try_again(&self) -> bool {
        let retry = &self.empty.retry;
        retry.is_visible() && retry.action_name().as_deref() == Some("win.refresh")
    }

    /// Shows the empty page in `state`.
    pub fn show_empty(&self, state: &EmptyState) {
        self.empty.show(state);
        self.show_page(ContentPage::Empty);
    }

    /// Shows or hides the loading line above the items.
    pub fn show_loading_line(&self, loading: bool) {
        self.loading_line.set_visible(loading);
        if loading {
            self.loading_line.pulse();
        }
    }

    /// The view that lists items now.
    pub fn view(&self) -> FolderView {
        let icons = FolderView::Icons(self.grid_size.get());
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
        match view {
            FolderView::Details => {
                self.grid.set_model(None::<&gtk::MultiSelection>);
                self.details.set_model(Some(selection));
            }
            FolderView::Icons(size) => {
                if self.grid_size.replace(size) != size {
                    grid::set_icon_size(&self.grid, &self.icons, &self.owners, size);
                }
                self.details.set_model(None::<&gtk::MultiSelection>);
                self.grid.set_model(Some(selection));
                self.update_grid_columns();
            }
        }
        self.views.set_visible_child_name(view.stack_name());
    }

    fn visible_scroll(&self) -> &gtk::ScrolledWindow {
        match self.view() {
            FolderView::Details => &self.details_scroll,
            FolderView::Icons(_) => &self.grid_scroll,
        }
    }

    /// The visible view's vertical scroll position.
    pub fn scroll_position(&self) -> f64 {
        self.visible_scroll().vadjustment().value()
    }

    /// Scrolls the visible view to `position` once the view has measured
    /// its new items; set straight after a model change, the position
    /// would be clamped to the old, shorter list.
    pub fn restore_scroll_position(&self, position: f64) {
        let adjustment = self.visible_scroll().vadjustment();
        adjustment.set_value(position);
        glib::idle_add_local_once(move || adjustment.set_value(position));
    }

    /// True while keyboard focus is inside the visible view.
    pub fn has_focus(&self) -> bool {
        match self.view() {
            FolderView::Details => self.details.has_focus() || self.details.focus_child().is_some(),
            FolderView::Icons(_) => self.grid.has_focus() || self.grid.focus_child().is_some(),
        }
    }

    /// Moves keyboard focus into the visible view.
    pub fn focus(&self) {
        match self.view() {
            FolderView::Details => self.details.grab_focus(),
            FolderView::Icons(_) => self.grid.grab_focus(),
        };
    }

    /// Scrolls to `position` and gives it keyboard focus.
    pub fn reveal(&self, position: u32) {
        match self.view() {
            FolderView::Details => self
                .details
                .scroll_to(position, None, gtk::ListScrollFlags::FOCUS, None),
            FolderView::Icons(_) => self.grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None),
        }
    }

    /// The visible view, as a widget.
    pub fn view_widget(&self) -> gtk::Widget {
        match self.view() {
            FolderView::Details => self.details.clone().upcast(),
            FolderView::Icons(_) => self.grid.clone().upcast(),
        }
    }

    /// Keeps the icon view's column cap near what fits (see
    /// [`grid::columns_for_width`]).
    fn fit_grid_columns_to_width(&self) {
        let grid = self.grid.downgrade();
        let grid_size = Rc::clone(&self.grid_size);
        self.grid_scroll
            .hadjustment()
            .connect_page_size_notify(move |adjustment| {
                if let Some(grid) = grid.upgrade() {
                    let columns = grid::columns_for_width(grid_size.get(), adjustment.page_size());
                    grid.set_max_columns(columns);
                }
            });
    }

    fn update_grid_columns(&self) {
        let width = self.grid_scroll.hadjustment().page_size();
        let columns = grid::columns_for_width(self.grid_size.get(), width);
        self.grid.set_max_columns(columns);
    }
}
