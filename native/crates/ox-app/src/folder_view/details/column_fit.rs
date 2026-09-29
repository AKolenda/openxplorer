// SPDX-License-Identifier: AGPL-3.0-only
//! Fitting and nudging the details columns: a double-click on a title's
//! resize edge, or Home on a focused title, fits the column to the widest
//! text among the first 2,000 shown items; Left and Right on a focused
//! title make it 10 pixels narrower or wider. Dragged widths stay within
//! the column's limits, and Name stops filling the spare width once it
//! has a width of its own; wider columns then scroll sideways.
//!
//! Ports `fitColumn`, `setColumnWidth` and the `.column-resizer` handlers
//! of `renderColumns` in `desktop/ui/app.js` (VIEW-028, VIEW-029,
//! VIEW-030). GTK draws the resize edge itself and has no widget for it,
//! so the title stands in for the web app's focusable separator. Every
//! change goes through the column's fixed width, so it is saved like a
//! drag once it settles.

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::{cell_text, DetailsView};
use crate::folder_view::column_widths;
use crate::folder_view::item::FileItem;
use crate::folder_view::sorting::SortColumn;

/// The most items a fitted column is measured against (`fitColumn`).
const FIT_ITEM_LIMIT: u32 = 2_000;

/// How close to a title's end edge a double-click fits the column: the
/// width of the web app's resize handle, and of GTK's resize edge.
const RESIZE_EDGE: f64 = 8.0;

/// How many pixels Left and Right change a focused title's column.
const NUDGE_STEP: i32 = 10;

/// What a title says about its edge (`handle.title` in app.js).
const TITLE_TOOLTIP: &str = "Drag to resize · double-click to fit loaded items (up to 2,000)";

impl DetailsView {
    /// Lets the titles fit and nudge their columns, and keeps dragged
    /// widths within the columns' limits.
    pub(super) fn install_column_fit(&self) {
        let Some(header) = self.header() else { return };
        for (column, title) in Self::titles(&header) {
            title.set_focusable(true);
            title.set_tooltip_text(Some(TITLE_TOOLTIP));
            describe_resizing(&title, column);
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(glib::clone!(
                #[weak(rename_to = view)]
                self,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, _| view.title_key(column, key)
            ));
            title.add_controller(keys);
        }
        let double_click = gtk::GestureClick::new();
        double_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        double_click.connect_pressed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[weak]
            header,
            move |gesture, presses, x, _| {
                if presses != 2 {
                    return;
                }
                if let Some(column) = Self::column_at_edge(&header, x) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    view.fit_column(column);
                }
            }
        ));
        header.add_controller(double_click);
        let titles = Self::titles(&header);
        for ((column, view_column), (_, title)) in self.view_columns().zip(titles) {
            // The title holds its column, so the column holds it weakly.
            let title = title.downgrade();
            view_column.connect_fixed_width_notify(move |view_column| {
                let width = view_column.fixed_width();
                let saved = column_widths::saved_width(column, width);
                if let (Some(saved), Some(title)) = (saved, title.upgrade()) {
                    title.update_property(&[gtk::accessible::Property::ValueNow(saved)]);
                }
                if width > 0 {
                    // A column with a width of its own stops filling the
                    // space: after a resize, every width is exact.
                    view_column.set_expand(false);
                    let clamped = column_widths::clamped_fixed_width(column, width);
                    if clamped != width {
                        view_column.set_fixed_width(clamped);
                    }
                }
            });
        }
    }

    /// The column view's row of titles.
    pub(crate) fn header(&self) -> Option<gtk::Widget> {
        self.column_view()
            .first_child()
            .filter(|child| child.css_name() == "header")
    }

    /// Each column with its title, in column order.
    fn titles(header: &gtk::Widget) -> Vec<(SortColumn, gtk::Widget)> {
        let titles = std::iter::successors(header.first_child(), WidgetExt::next_sibling);
        SortColumn::ALL.into_iter().zip(titles).collect()
    }

    /// The shown column whose title ends within [`RESIZE_EDGE`] of `x`,
    /// a position in `header`.
    fn column_at_edge(header: &gtk::Widget, x: f64) -> Option<SortColumn> {
        let titles = Self::titles(header);
        let shown = titles.iter().filter(|(_, title)| title.is_visible());
        shown
            .filter_map(|(column, title)| Some((*column, title.compute_bounds(header)?)))
            .find(|(_, bounds)| (f64::from(bounds.x() + bounds.width()) - x).abs() <= RESIZE_EDGE)
            .map(|(column, _)| column)
    }

    /// Left, Right and Home on `column`'s focused title.
    fn title_key(&self, column: SortColumn, key: gdk::Key) -> glib::Propagation {
        match key {
            gdk::Key::Left | gdk::Key::KP_Left => self.nudge_column(column, -NUDGE_STEP),
            gdk::Key::Right | gdk::Key::KP_Right => self.nudge_column(column, NUDGE_STEP),
            gdk::Key::Home | gdk::Key::KP_Home => self.fit_column(column),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// Makes `column` `step` pixels wider (narrower when negative), from
    /// the width it has on screen, within its limits.
    pub(crate) fn nudge_column(&self, column: SortColumn, step: i32) {
        let Some(view_column) = self.column(column) else {
            return;
        };
        let fixed = view_column.fixed_width();
        let current = if fixed > 0 {
            fixed
        } else {
            self.header()
                .and_then(|header| {
                    let titles = Self::titles(&header);
                    let title = titles.into_iter().find(|(shown, _)| *shown == column)?.1;
                    Some(title.width())
                })
                .unwrap_or_default()
        };
        view_column.set_expand(false);
        view_column.set_fixed_width(column_widths::clamped_fixed_width(column, current + step));
    }

    /// Fits `column` to the widest text among the first
    /// [`FIT_ITEM_LIMIT`] shown items (`fitColumn`).
    pub(crate) fn fit_column(&self, column: SortColumn) {
        let Some(view_column) = self.column(column) else {
            return;
        };
        let widest = self.widest_text(column);
        let width = column_widths::fitted_width(column, widest);
        view_column.set_expand(false);
        view_column.set_fixed_width(column_widths::fixed_width(column, Some(width)));
    }

    /// The width in pixels of the widest text `column` shows for the first
    /// [`FIT_ITEM_LIMIT`] items.
    pub(crate) fn widest_text(&self, column: SortColumn) -> f64 {
        let view = self.column_view();
        let Some(model) = view.model() else { return 0.0 };
        let shown = model.n_items().min(FIT_ITEM_LIMIT);
        let widths = (0..shown)
            .filter_map(|position| model.item(position).and_downcast::<FileItem>())
            .map(|item| {
                let layout = view.create_pango_layout(Some(&cell_text(column, &item)));
                layout.pixel_size().0
            });
        f64::from(widths.max().unwrap_or_default())
    }
}

/// Tells screen readers that `title` resizes `column` from the keyboard,
/// and within which widths, as the web app's focusable separator did
/// (`aria-label`, `aria-valuemin`, `aria-valuemax`).
fn describe_resizing(title: &gtk::Widget, column: SortColumn) {
    let limits = column_widths::width_limits(column);
    let description = format!(
        "Resize {} column: Left and Right change its width, Home fits it",
        column.label()
    );
    title.update_property(&[
        gtk::accessible::Property::Description(&description),
        gtk::accessible::Property::ValueMin(f64::from(*limits.start())),
        gtk::accessible::Property::ValueMax(f64::from(*limits.end())),
    ]);
}

#[cfg(test)]
mod tests {
    use gtk::gio;

    use super::*;
    use crate::folder_view::cells::CellOwners;
    use crate::folder_view::model::FolderModel;
    use crate::test_support::file_entry;

    /// A details view showing files named `names`.
    fn view_of(names: &[&str]) -> (DetailsView, FolderModel) {
        let model = FolderModel::new();
        let store = gio::ListStore::new::<FileItem>();
        for name in names {
            store.append(&FileItem::new(file_entry(name)));
        }
        model.set_store(Some(&store));
        let view = DetailsView::new(&model, &CellOwners::new());
        view.column_view().set_model(Some(model.selection()));
        (view, model)
    }

    /// Home fits a column to its widest name; Left and Right step it by
    /// 10 pixels, never past its limits. The titles are focusable and
    /// describe their keys to screen readers.
    ///
    /// parity: VIEW-029, VIEW-030
    #[gtk::test]
    fn titles_fit_and_nudge_their_columns() {
        let long_name = "A rather long file name that needs a wide column.txt";
        let (view, _model) = view_of(&["a.txt", long_name]);
        let header = view.header().expect("column titles");
        let titles = DetailsView::titles(&header);
        assert!(titles.iter().all(|(_, title)| title.is_focusable()));
        let name = view.column(SortColumn::Name).expect("a Name column");
        assert_eq!(
            view.title_key(SortColumn::Name, gdk::Key::Home),
            glib::Propagation::Stop
        );
        let widest = view.widest_text(SortColumn::Name);
        assert!(widest > 100.0, "the long name was measured");
        let fitted = column_widths::fitted_width(SortColumn::Name, widest);
        assert_eq!(
            name.fixed_width(),
            column_widths::fixed_width(SortColumn::Name, Some(fitted))
        );
        assert!(!name.expands(), "a fitted Name no longer fills the space");

        let size = view.column(SortColumn::Size).expect("a Size column");
        let before = size.fixed_width();
        view.title_key(SortColumn::Size, gdk::Key::Right);
        assert_eq!(size.fixed_width(), before + NUDGE_STEP);
        for _ in 0..20 {
            view.title_key(SortColumn::Size, gdk::Key::Left);
        }
        let narrowest = column_widths::clamped_fixed_width(SortColumn::Size, 0);
        assert_eq!(
            size.fixed_width(),
            narrowest,
            "Size stops at its 70-pixel minimum"
        );
    }

    /// Name fills the spare width until it is resized; then its width is
    /// exact.
    ///
    /// parity: VIEW-031
    #[gtk::test]
    fn name_fills_the_width_until_it_is_resized() {
        let (view, _model) = view_of(&["a.txt"]);
        let name = view.column(SortColumn::Name).expect("a Name column");
        assert!(name.expands());
        name.set_fixed_width(320);
        assert!(!name.expands());
        assert_eq!(name.fixed_width(), 320);
    }
}
