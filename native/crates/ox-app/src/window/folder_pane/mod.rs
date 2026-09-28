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
//! with the loading line laid over them). The views are widgets of their
//! own: [`DetailsView`] and [`IconView`], which keeps its tiles' scale.

mod parts;
mod view;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::cells::{BoundIcons, CellOwners};
use crate::folder_view::details::DetailsView;
use crate::folder_view::grid::IconView;
use crate::folder_view::model::FolderModel;
use crate::theme::Appearance;

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
    pub struct FolderPane {
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
    pub struct FolderPane(ObjectSubclass<imp::FolderPane>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FolderPane {
    /// Builds the pages, empty and in the details view. Item icons are
    /// drawn light until the window sets the skin's appearance through
    /// [`BoundIcons::set_appearance`], before anything is listed.
    fn build_parts(&self) {
        let parts = PaneParts::new(Appearance::Light);
        let overlay = gtk::Overlay::builder().child(&parts.stack).build();
        overlay.add_overlay(&parts.loading_line);
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

    /// Bound item icons, redrawn when the theme or scale changes.
    pub(super) fn icons(&self) -> &BoundIcons {
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
    /// [`LoadingLine::set_loading`](super::loading_line::LoadingLine::set_loading)).
    pub(super) fn set_loading(&self, loading: bool) {
        self.parts().loading_line.set_loading(loading);
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

    /// Moves keyboard focus into the visible view.
    pub(super) fn focus_view(&self) {
        self.view_widget().grab_focus();
    }

    /// Scrolls to `position` and gives it keyboard focus.
    pub(super) fn reveal(&self, position: u32) {
        let focus = gtk::ListScrollFlags::FOCUS;
        match self.view() {
            FolderView::Details => self
                .details()
                .column_view()
                .scroll_to(position, None, focus, None),
            FolderView::Icons(_) => self.icon_view().grid().scroll_to(position, focus, None),
        }
    }

    /// Draws the icon view's cells for text of `percent` size.
    pub(super) fn set_text_size(&self, percent: u32) {
        self.icon_view().set_text_size(percent);
    }
}
