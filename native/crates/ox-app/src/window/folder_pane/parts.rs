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
use crate::folder_view::keep_top::TopKeeper;
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
    /// The whole area of the empty page, which takes the pane's
    /// right-click menu, keys and drops while it shows.
    pub(super) empty_area: gtk::Box,
    /// The landing page's contents.
    pub(super) landing: gtk::Box,
    /// The line over the pane while a folder is listed.
    pub(super) loading_line: LoadingLine,
    /// "This folder is empty." near the top of the list, as Windows
    /// Explorer says it, while the folder shown has no items.
    pub(super) empty_note: gtk::Label,
    /// The note over the pane that says what a drag would do there, such
    /// as "Open with convert".
    pub(super) drag_hint: gtk::Label,
    /// The rubber band over the views while one is drawn (SEL-012).
    pub(super) rubber_band: gtk::Box,
    /// Keeps the visible view at its top when its items change there.
    pub(super) top_keeper: TopKeeper,
}

impl PaneParts {
    /// The pane's widgets, showing nothing yet.
    pub(super) fn new() -> Self {
        let model = FolderModel::new();
        let owners = CellOwners::new();
        let details = DetailsView::new(&model, &owners);
        let icon_view = IconView::new(&owners);
        let views = view_stack(&details, &icon_view);
        let top_keeper = keep_views_at_their_top(&model, &views, &details, &icon_view);
        let empty = EmptyPage::new();
        let (landing, landing_scroll) = landing_page();
        let (stack, empty_area) = page_stack(&views, &empty, &landing_scroll);
        Self {
            stack,
            views,
            details,
            icon_view,
            model,
            owners,
            empty,
            empty_area,
            landing,
            loading_line: LoadingLine::new(),
            empty_note: empty_note(),
            drag_hint: drag_hint(),
            rubber_band: rubber_band(),
            top_keeper,
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

/// The note an empty folder shows near the top of its list, under the
/// column titles, as Windows Explorer does. It never takes the pointer,
/// so clicks, right-clicks and drops go to the list beneath it.
fn empty_note() -> gtk::Label {
    gtk::Label::builder()
        .label(ox_core::i18n::gettext_static(EMPTY_NOTE))
        // A longer translation wraps inside a narrow pane.
        .wrap(true)
        .justify(gtk::Justification::Center)
        .can_target(false)
        .visible(false)
        .css_classes(["empty-folder-note"])
        .build()
}

/// What the note says.
pub(super) const EMPTY_NOTE: &str = crate::i18n::message_id("This folder is empty.");

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
/// Keeps whichever of the views is shown at its top when the items change
/// while it is there: the details view scrolls down, the icons down too
/// and the compact list sideways.
fn keep_views_at_their_top(
    model: &FolderModel,
    views: &gtk::Stack,
    details: &DetailsView,
    icon_view: &IconView,
) -> TopKeeper {
    let grid = icon_view.both_scroll_adjustments();
    let mut adjustments = vec![details.vadjustment()];
    adjustments.extend(grid);
    let shown = views.downgrade();
    let details = details.downgrade();
    let icon_view = icon_view.downgrade();
    TopKeeper::follow(views, model.selection(), &adjustments, move || {
        let name = shown.upgrade()?.visible_child_name()?;
        if name == FolderView::Details.stack_name() {
            Some(details.upgrade()?.vadjustment())
        } else {
            Some(icon_view.upgrade()?.scroll_adjustment())
        }
    })
}

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
/// or error page and the landing page; and the empty page's whole area.
fn page_stack(
    views: &gtk::Stack,
    empty: &EmptyPage,
    landing_scroll: &gtk::ScrolledWindow,
) -> (gtk::Stack, gtk::Box) {
    let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
    stack.add_css_class("folder-pane");
    stack.add_named(views, Some(PanePage::Listing.name()));
    // The empty page scrolls down, never across, as the other pages do. Its
    // wrapping message then never sets the pane's minimum height: a split's
    // GtkPaned measures each pane at the divider's width, and a message
    // wrapping onto one more line there made the pane taller than the
    // height it reports free of width, which GTK's box layout rejects
    // ("Expect overlapping widgets"). The stack counts the page even while
    // another one shows.
    // The page fills the whole pane, so a right-click, a key or a drop
    // anywhere on it reaches the empty folder; its words stay centred.
    // Keyboard focus rests on it while the page shows, so the folder's
    // keys work in an empty folder too.
    let empty_area = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .focusable(true)
        .css_classes(["empty-folder-area"])
        .build();
    empty.root.set_vexpand(true);
    empty_area.append(&empty.root);
    // A screen reader says what the page says ("This folder is empty"),
    // not the list's type-to-select hint: there is nothing to select.
    empty.label_area(&empty_area);
    let empty_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&empty_area)
        .build();
    stack.add_named(&empty_scroll, Some(PanePage::Empty.name()));
    stack.add_named(landing_scroll, Some(PanePage::Landing.name()));
    (stack, empty_area)
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(child)
        .build()
}
