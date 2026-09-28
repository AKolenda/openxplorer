// SPDX-License-Identifier: AGPL-3.0-only
//! The folder pane: the details and icon views, the empty and error state,
//! the landing pages and the loading line.
//!
//! Ports the `main` area of `desktop/ui/index.html` and `renderContent` in
//! `desktop/ui/app.js`. Only the visible view is attached to the selection
//! model: a hidden `GtkGridView` still builds and binds its tiles for every
//! change, which made large folders several times slower to list.
//!
//! [`FolderPane`] is a widget subclass around a `GtkOverlay` (the pages,
//! with the loading line laid over them), so the scale of the icon view's
//! tiles lives in the widget that its resize handler reads.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::cells::{CellOwners, IconCells};
use crate::folder_view::grid::{self, IconSize};
use crate::folder_view::{details, model::FolderModel};
use crate::theme::Appearance;

use super::empty_page::{EmptyPage, EmptyState};
use super::loading_line::LoadingLine;

/// What the folder pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanePage {
    /// The folder's items.
    Listing,
    /// The empty, filtered-out, loading or error state.
    Empty,
    /// A landing page (This PC, Network).
    Landing,
}

impl PanePage {
    /// Every page, in the order the pane stacks them.
    const ALL: [PanePage; 3] = [PanePage::Listing, PanePage::Empty, PanePage::Landing];

    const fn name(self) -> &'static str {
        match self {
            PanePage::Listing => "listing",
            PanePage::Empty => "empty",
            PanePage::Landing => "landing",
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

/// What sizes the icon view's cells: the icon size and the text size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GridScale {
    icon_size: IconSize,
    /// In percent.
    text_size: u32,
}

impl Default for GridScale {
    /// Large icons at the default text size, as a new window starts.
    fn default() -> Self {
        Self {
            icon_size: IconSize::Large,
            text_size: crate::text_size::DEFAULT,
        }
    }
}

/// The folder pane's widgets and the folder model they show.
#[derive(Debug)]
struct PaneParts {
    /// The listing, the empty page or the landing page ([`PanePage`]).
    stack: gtk::Stack,
    /// The details or the icon view ([`FolderView`]).
    views: gtk::Stack,
    details: gtk::ColumnView,
    details_scroll: gtk::ScrolledWindow,
    grid: gtk::GridView,
    grid_scroll: gtk::ScrolledWindow,
    /// The active tab's filtered, sorted and selectable items.
    model: FolderModel,
    /// Bound item icons, redrawn when the theme or scale changes.
    icons: Rc<IconCells>,
    /// Maps cell widgets to their rows.
    owners: Rc<CellOwners>,
    /// The empty, loading and error page.
    empty: EmptyPage,
    /// The landing page's contents.
    landing: gtk::Box,
    /// The line over the pane while a folder is listed.
    loading_line: LoadingLine,
}

impl PaneParts {
    /// The pane's widgets, drawing item icons in `appearance`.
    fn new(appearance: Appearance) -> Self {
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

mod imp {
    use std::cell::{Cell, OnceCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{GridScale, PaneParts};

    /// Private state of [`super::FolderPane`].
    #[derive(Debug, Default)]
    pub struct FolderPane {
        /// The widgets and the folder model, built by
        /// [`super::FolderPane::new`].
        pub(super) parts: OnceCell<PaneParts>,
        /// What sizes the icon view's tiles.
        pub(super) grid_scale: Cell<GridScale>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FolderPane {
        const NAME: &'static str = "OxFolderPane";
        type Type = super::FolderPane;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for FolderPane {
        fn constructed(&self) {
            self.parent_constructed();
            // The pane takes all the room beside the details pane, as its
            // pages do.
            let pane = self.obj();
            pane.set_hexpand(true);
            pane.set_vexpand(true);
        }

        fn dispose(&self) {
            // The overlay is the pane's one child.
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for FolderPane {}
}

glib::wrapper! {
    /// The folder pane, with the shared folder model of its window.
    pub struct FolderPane(ObjectSubclass<imp::FolderPane>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FolderPane {
    /// An empty folder pane in the details view, drawing icons in `appearance`.
    pub(super) fn new(appearance: Appearance) -> Self {
        let pane: Self = glib::Object::new();
        let parts = PaneParts::new(appearance);
        let overlay = gtk::Overlay::builder().child(&parts.stack).build();
        overlay.add_overlay(&parts.loading_line);
        overlay.set_parent(&pane);
        pane.imp().parts.set(parts).expect("a new pane has no parts yet");
        pane.show_view(FolderView::Details);
        pane.fit_grid_columns_to_width();
        pane
    }

    fn parts(&self) -> &PaneParts {
        self.imp().parts.get().expect("FolderPane::new builds the parts")
    }

    /// The active tab's filtered, sorted and selectable items.
    pub(super) fn model(&self) -> &FolderModel {
        &self.parts().model
    }

    /// The details view.
    pub(super) fn details(&self) -> &gtk::ColumnView {
        &self.parts().details
    }

    /// The icon view.
    pub(super) fn grid(&self) -> &gtk::GridView {
        &self.parts().grid
    }

    /// Bound item icons, redrawn when the theme or scale changes.
    pub(super) fn icons(&self) -> &IconCells {
        &self.parts().icons
    }

    /// Maps cell widgets to their rows.
    pub(super) fn owners(&self) -> &CellOwners {
        &self.parts().owners
    }

    /// The landing page's contents, which the window draws.
    pub(super) fn landing(&self) -> &gtk::Box {
        &self.parts().landing
    }

    /// Shows `page`.
    pub(super) fn show_page(&self, page: PanePage) {
        self.parts().stack.set_visible_child_name(page.name());
    }

    /// The page shown now.
    pub(super) fn page(&self) -> Option<PanePage> {
        let name = self.parts().stack.visible_child_name()?;
        PanePage::ALL
            .into_iter()
            .find(|page| page.name() == name.as_str())
    }

    /// Shows the empty page in `state`.
    pub(super) fn show_empty(&self, state: &EmptyState) {
        self.parts().empty.show(state);
        self.show_page(PanePage::Empty);
    }

    /// Shows the loading line over the items while `loading` lasts (see
    /// [`LoadingLine::set_loading`]).
    pub(super) fn set_loading(&self, loading: bool) {
        self.parts().loading_line.set_loading(loading);
    }

    /// The loading line, for tests.
    #[cfg(test)]
    pub(super) fn loading_line(&self) -> &LoadingLine {
        &self.parts().loading_line
    }

    /// The empty, loading and error page, for tests.
    #[cfg(test)]
    pub(super) fn empty_page(&self) -> &EmptyPage {
        &self.parts().empty
    }

    /// The view that lists items now.
    pub(super) fn view(&self) -> FolderView {
        let icons = FolderView::Icons(self.imp().grid_scale.get().icon_size);
        let shown = self.parts().views.visible_child_name();
        if shown.as_deref() == Some(icons.stack_name()) {
            icons
        } else {
            FolderView::Details
        }
    }

    /// Switches views. Only the visible view holds the selection model.
    pub(super) fn show_view(&self, view: FolderView) {
        let parts = self.parts();
        let selection = parts.model.selection();
        match view {
            FolderView::Details => {
                parts.grid.set_model(None::<&gtk::MultiSelection>);
                parts.details.set_model(Some(selection));
            }
            FolderView::Icons(size) => {
                self.use_icon_size(size);
                parts.details.set_model(None::<&gtk::MultiSelection>);
                parts.grid.set_model(Some(selection));
                self.set_grid_columns();
            }
        }
        parts.views.set_visible_child_name(view.stack_name());
    }

    /// Draws the icon view's tiles at `size`, when they are not already.
    fn use_icon_size(&self, size: IconSize) {
        let scale = self.imp().grid_scale.get();
        if scale.icon_size == size {
            return;
        }
        self.imp().grid_scale.set(GridScale {
            icon_size: size,
            ..scale
        });
        let parts = self.parts();
        grid::set_icon_size(&parts.grid, &parts.icons, &parts.owners, size);
    }

    fn visible_scroll(&self) -> &gtk::ScrolledWindow {
        match self.view() {
            FolderView::Details => &self.parts().details_scroll,
            FolderView::Icons(_) => &self.parts().grid_scroll,
        }
    }

    /// The visible view's vertical scroll position.
    pub(super) fn scroll_position(&self) -> f64 {
        self.visible_scroll().vadjustment().value()
    }

    /// Scrolls the visible view to `position` once the view has measured
    /// its new items; set straight after a model change, the position
    /// would be clamped to the old, shorter list.
    pub(super) fn restore_scroll_position(&self, position: f64) {
        let adjustment = self.visible_scroll().vadjustment();
        adjustment.set_value(position);
        glib::idle_add_local_once(move || adjustment.set_value(position));
    }

    /// The visible view, as a widget.
    pub(super) fn view_widget(&self) -> gtk::Widget {
        match self.view() {
            FolderView::Details => self.details().clone().upcast(),
            FolderView::Icons(_) => self.grid().clone().upcast(),
        }
    }

    /// True while keyboard focus is inside the visible view.
    pub(super) fn view_has_focus(&self) -> bool {
        let view = self.view_widget();
        view.has_focus() || view.focus_child().is_some()
    }

    /// Moves keyboard focus into the visible view.
    pub(super) fn focus_view(&self) {
        self.view_widget().grab_focus();
    }

    /// Scrolls to `position` and gives it keyboard focus.
    pub(super) fn reveal(&self, position: u32) {
        let focus = gtk::ListScrollFlags::FOCUS;
        match self.view() {
            FolderView::Details => self.details().scroll_to(position, None, focus, None),
            FolderView::Icons(_) => self.grid().scroll_to(position, focus, None),
        }
    }

    /// Draws the icon view's cells for text of `percent` size.
    pub(super) fn set_text_size(&self, percent: u32) {
        let scale = self.imp().grid_scale.get();
        self.imp().grid_scale.set(GridScale {
            text_size: percent,
            ..scale
        });
        self.set_grid_columns();
    }

    /// Keeps the icon view's columns at what fits its pane (see
    /// [`grid::columns_for_width`]).
    fn fit_grid_columns_to_width(&self) {
        let adjustment = self.parts().grid_scroll.hadjustment();
        adjustment.connect_page_size_notify(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_| pane.set_grid_columns_when_allocated()
        ));
    }

    /// Sets the icon view's columns for its pane's width, once GTK has
    /// finished allocating the pane. The page size changes while GTK
    /// allocates the grid, and GTK ignores a resize the grid queues then,
    /// so a window that opened in the icon view kept one column.
    fn set_grid_columns_when_allocated(&self) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move || pane.set_grid_columns()
        ));
    }

    /// Gives the icon view the columns its scroller's width holds at the
    /// current scale.
    fn set_grid_columns(&self) {
        let parts = self.parts();
        let scale = self.imp().grid_scale.get();
        let cell = grid::cell_size(scale.icon_size, scale.text_size);
        let columns = grid::columns_for_width(cell.width, parts.grid_scroll.width());
        if parts.grid.max_columns() != columns {
            parts.grid.set_max_columns(columns);
        }
    }
}
