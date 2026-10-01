// SPDX-License-Identifier: AGPL-3.0-only
//! The folder pane: the details and icon views, the empty and error state,
//! the landing pages and the loading line.
//!
//! Ports the `main` area of `v2.0.0:desktop/ui/index.html` and `renderContent` in
//! `v2.0.0:desktop/ui/app.js`. Only the visible view is attached to the selection
//! model: a hidden `GtkGridView` still builds and binds its tiles for every
//! change, which made large folders several times slower to list.
//!
//! [`FolderPane`] is a widget subclass around a `GtkOverlay` (the pages,
//! with the loading line laid over them). The views are widgets of their
//! own: [`DetailsView`] and [`IconView`], which keeps its tiles' scale.

mod parts;
mod view;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::cells::CellOwners;
use crate::folder_view::details::DetailsView;
use crate::folder_view::grid::IconView;
use crate::folder_view::model::FolderModel;
use crate::text_size::TextSize;

use super::empty_page::EmptyState;

use parts::PaneParts;
pub(crate) use view::FolderView;

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

    /// The name of the page in the pane's stack.
    const fn name(self) -> &'static str {
        match self {
            PanePage::Listing => "listing",
            PanePage::Empty => "empty",
            PanePage::Landing => "landing",
        }
    }
}

mod imp {
    use std::cell::OnceCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::PaneParts;

    /// Private state of [`super::FolderPane`].
    #[derive(Debug, Default)]
    pub(crate) struct FolderPane {
        /// The widgets and the folder model, built by `constructed`.
        pub(super) parts: OnceCell<PaneParts>,
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
            pane.build_parts();
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
    pub(crate) struct FolderPane(ObjectSubclass<imp::FolderPane>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FolderPane {
    /// Builds the pages, empty and in the details view.
    fn build_parts(&self) {
        let parts = PaneParts::new();
        let overlay = gtk::Overlay::builder().child(&parts.stack).build();
        overlay.add_overlay(&parts.loading_line);
        overlay.add_overlay(&parts.drag_hint);
        overlay.set_parent(self);
        self.imp()
            .parts
            .set(parts)
            .expect("constructed runs once per object");
        self.show_view(FolderView::Details);
    }

    fn parts(&self) -> &PaneParts {
        self.imp().parts.get().expect("constructed builds the parts")
    }

    /// The active tab's filtered, sorted and selectable items.
    pub(super) fn model(&self) -> &FolderModel {
        &self.parts().model
    }

    /// The details view.
    pub(super) fn details(&self) -> &DetailsView {
        &self.parts().details
    }

    /// The icon view.
    pub(super) fn icon_view(&self) -> &IconView {
        &self.parts().icon_view
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
    /// [`LoadingLine::set_loading`](super::loading_line::LoadingLine::set_loading)).
    pub(super) fn set_loading(&self, loading: bool) {
        self.parts().loading_line.set_loading(loading);
    }

    /// Whether the loading line shows now, which a listing does only once
    /// it has run for a moment.
    pub(super) fn shows_loading_line(&self) -> bool {
        self.parts().loading_line.is_shown()
    }

    /// Calls `changed` whenever the loading line shows or hides.
    pub(super) fn connect_loading_line_changed(&self, changed: impl Fn() + 'static) {
        self.parts()
            .loading_line
            .connect_visible_notify(move |_| changed());
    }

    /// Shows `hint` over the pane, saying what a drag would do there, or
    /// hides the note.
    pub(super) fn show_drag_hint(&self, hint: Option<&str>) {
        let label = &self.parts().drag_hint;
        label.set_label(hint.unwrap_or_default());
        label.set_visible(hint.is_some());
    }

    /// The note over the pane while a drag shows one, for tests.
    #[cfg(test)]
    pub(super) fn drag_hint(&self) -> Option<String> {
        let label = &self.parts().drag_hint;
        label.is_visible().then(|| label.label().to_string())
    }

    /// The loading line, for tests.
    #[cfg(test)]
    pub(super) fn loading_line(&self) -> &super::loading_line::LoadingLine {
        &self.parts().loading_line
    }

    /// The empty, loading and error page, for tests.
    #[cfg(test)]
    pub(super) fn empty_page(&self) -> &super::empty_page::EmptyPage {
        &self.parts().empty
    }

    /// The view that lists items now.
    pub(super) fn view(&self) -> FolderView {
        let icons = FolderView::Icons(self.icon_view().icon_size());
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
        let column_view = parts.details.column_view();
        let grid = parts.icon_view.grid();
        match view {
            FolderView::Details => {
                grid.set_model(None::<&gtk::MultiSelection>);
                column_view.set_model(Some(selection));
            }
            FolderView::Icons(size) => {
                parts.icon_view.set_icon_size(size);
                column_view.set_model(None::<&gtk::MultiSelection>);
                grid.set_model(Some(selection));
                parts.icon_view.fit_columns();
            }
        }
        parts.views.set_visible_child_name(view.stack_name());
    }

    /// The visible view's vertical scroll adjustment.
    fn visible_vadjustment(&self) -> gtk::Adjustment {
        match self.view() {
            FolderView::Details => self.details().vadjustment(),
            FolderView::Icons(_) => self.icon_view().vadjustment(),
        }
    }

    /// The visible view's vertical scroll position.
    pub(super) fn scroll_position(&self) -> f64 {
        self.visible_vadjustment().value()
    }

    /// Scrolls the visible view to `position` once the view has measured
    /// its new items; set straight after a model change, the position
    /// would be clamped to the old, shorter list.
    pub(super) fn restore_scroll_position(&self, position: f64) {
        let adjustment = self.visible_vadjustment();
        adjustment.set_value(position);
        glib::idle_add_local_once(move || adjustment.set_value(position));
    }

    /// The visible view, as a widget.
    pub(super) fn view_widget(&self) -> gtk::Widget {
        match self.view() {
            FolderView::Details => self.details().column_view().clone().upcast(),
            FolderView::Icons(_) => self.icon_view().grid().clone().upcast(),
        }
    }

    /// True while keyboard focus is inside the visible view.
    pub(super) fn view_has_focus(&self) -> bool {
        let view = self.view_widget();
        view.has_focus() || view.focus_child().is_some()
    }

    /// The position of the item with keyboard focus in the visible view:
    /// the one it had last, when focus is elsewhere in the window.
    pub(super) fn focused_position(&self) -> Option<u32> {
        let view = self.view_widget();
        let focused = std::iter::successors(view.focus_child(), WidgetExt::focus_child).last()?;
        self.owners().position_holding(&view, focused)
    }

    /// Gives the item at `uri` keyboard focus once the view has laid out
    /// its items and scrolled to where [`Self::restore_scroll_position`]
    /// put it, if the item is still shown then.
    pub(super) fn focus_item_later(&self, uri: String) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move || {
                if let Some(position) = pane.model().position_of_uri(&uri) {
                    pane.reveal(position);
                }
            }
        ));
    }

    /// Moves keyboard focus into the visible view.
    pub(super) fn focus_view(&self) {
        self.view_widget().grab_focus();
    }

    /// Scrolls to `position` and gives it keyboard focus.
    pub(super) fn reveal(&self, position: u32) {
        self.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    }

    /// Gives `position` keyboard focus without scrolling the view.
    pub(super) fn focus_item(&self, position: u32) {
        let stay = gtk::ScrollInfo::new();
        stay.set_enable_horizontal(false);
        stay.set_enable_vertical(false);
        self.scroll_to(position, gtk::ListScrollFlags::FOCUS, Some(stay));
    }

    /// Selects only `position`, makes it the anchor Shift extends a range
    /// from, scrolls to it and gives it keyboard focus, as a click on it
    /// does.
    pub(super) fn select_and_reveal(&self, position: u32) {
        self.scroll_to(
            position,
            gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
            None,
        );
    }

    /// Moves to `position` with `flags`, scrolling as `scroll` allows.
    fn scroll_to(&self, position: u32, flags: gtk::ListScrollFlags, scroll: Option<gtk::ScrollInfo>) {
        match self.view() {
            FolderView::Details => self
                .details()
                .column_view()
                .scroll_to(position, None, flags, scroll),
            FolderView::Icons(_) => self.icon_view().grid().scroll_to(position, flags, scroll),
        }
    }

    /// Draws the icon view's cells for text of `size`.
    pub(super) fn set_text_size(&self, size: TextSize) {
        self.icon_view().set_text_size(size);
    }
}
