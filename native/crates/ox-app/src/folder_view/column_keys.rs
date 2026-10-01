// SPDX-License-Identifier: AGPL-3.0-only
//! The details view's column titles as keyboard controls (ACC-006).
//!
//! Ports the keys of the `.column-label` buttons and `.column-resizer`
//! handles of `renderColumns` in `desktop/ui/app.js`: each title takes
//! keyboard focus; Enter or Space sorts by its column, and again turns
//! the order round, as a click does; Left and Right make the column 10
//! pixels narrower or wider (40 with Shift), within the Python app's
//! limits; Home fits it to the widest text among the first 2,000 items
//! listed (`fitColumn`). A width changed this way is saved like a dragged
//! one. GTK 4.14's titles take no focus of their
//! own, so this module makes them focusable.

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::folder_view::column_titles::title_buttons;
use crate::folder_view::column_widths::{self, settings_column};
use crate::folder_view::details::DetailsView;
use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};
use crate::resizer_control::ResizerControl;

/// How far an arrow key resizes a column, and with Shift held.
const KEY_STEP: i32 = 10;
const SHIFT_KEY_STEP: i32 = 40;

/// How many listed items Home measures (`filtered().slice(0,2000)`).
const FIT_ITEMS: u32 = 2000;

/// What a key does on a focused column title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TitleKey {
    /// Sort by the column, or turn its order round.
    Sort,
    /// Change the width by this many pixels.
    Resize(i32),
    /// Fit the column to the listed items.
    FitToItems,
}

impl TitleKey {
    /// What `key`, with Shift held or not, does on a title.
    fn for_key(key: gdk::Key, shift: bool) -> Option<Self> {
        let step = if shift { SHIFT_KEY_STEP } else { KEY_STEP };
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => Some(TitleKey::Sort),
            gdk::Key::Left | gdk::Key::KP_Left => Some(TitleKey::Resize(-step)),
            gdk::Key::Right | gdk::Key::KP_Right => Some(TitleKey::Resize(step)),
            gdk::Key::Home | gdk::Key::KP_Home => Some(TitleKey::FitToItems),
            _ => None,
        }
    }
}

/// The width `column` gets when a key changes its `current` width, in
/// the pixels settings save, by `change`, within the column's limits.
fn resized_width(column: SortColumn, current: i32, change: i32) -> u32 {
    let limits = settings_column(column).width_range();
    let wanted = u32::try_from((current + change).max(0)).unwrap_or(0);
    wanted.clamp(*limits.start(), *limits.end())
}

/// The width that fits `column` to texts `text_widths` pixels wide: the
/// widest plus room for the icon on Name (64) or the cell padding (30),
/// within the column's limits, as `fitColumn` in app.js.
fn fitted_width(column: SortColumn, text_widths: impl IntoIterator<Item = i32>) -> u32 {
    let padding = if column == SortColumn::Name { 64 } else { 30 };
    let limits = settings_column(column).width_range();
    let widest = text_widths
        .into_iter()
        .map(|width| width + padding)
        .max()
        .unwrap_or(0);
    u32::try_from(widest)
        .unwrap_or(0)
        .clamp(*limits.start(), *limits.end())
}

/// The order a sort key asks for when the view sorts by `current`.
fn order_after_sort_key(column: SortColumn, current: SortOrder) -> SortOrder {
    let direction = if current.column == column {
        match current.direction {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    } else {
        SortDirection::Ascending
    };
    SortOrder { column, direction }
}

/// Makes every column title of `view` focusable and gives it the keys.
pub(crate) fn make_titles_keyboard_operable(view: &DetailsView) {
    let titles = title_buttons(view.column_view());
    for (column, title) in SortColumn::ALL.into_iter().zip(titles) {
        title.set_focusable(true);
        // A click sorts and leaves focus in the list, as before.
        title.set_focus_on_click(false);
        title.update_property(&[gtk::accessible::Property::Description(
            "Enter sorts by this column; Left and Right resize it; Home fits it to the listed items",
        )]);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            view,
            #[weak]
            title,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                let Some(action) = TitleKey::for_key(key, shift) else {
                    return glib::Propagation::Proceed;
                };
                run_title_key(&view, &title, column, action);
                glib::Propagation::Stop
            }
        ));
        title.add_controller(keys);
        add_column_resizer(&view, column, &title);
    }
}

/// Adds "Resize <column> column" (`.column-resizer`) to the title of
/// `column`: a separator carrying the width and its limits as its value,
/// which a screen reader may set like a key resizes the column. The title
/// takes the keys, so the separator is no second Tab stop.
fn add_column_resizer(view: &DetailsView, column: SortColumn, title: &gtk::Widget) {
    let (Some(view_column), Some(title_box)) = (
        view.column(column),
        title.first_child().and_downcast::<gtk::Box>(),
    ) else {
        return;
    };
    let name = view_column.title().unwrap_or_default();
    let resizer = ResizerControl::new(&format!("Resize {name} column"));
    title_box.append(&resizer);
    let announce = glib::clone!(
        #[weak]
        title,
        #[weak]
        resizer,
        move |view_column: &gtk::ColumnViewColumn| {
            let limits = settings_column(column).width_range();
            let fixed = view_column.fixed_width();
            let shown = if fixed > 0 { fixed } else { title.width() };
            #[expect(clippy::cast_possible_truncation, reason = "column widths are small")]
            let width = column_widths::saved_width(column, shown).map_or(0, |width| width as i32);
            let min = i32::try_from(*limits.start()).unwrap_or(0);
            let max = i32::try_from(*limits.end()).unwrap_or(i32::MAX);
            resizer.set_values(min, max, width.clamp(min, max));
        }
    );
    announce(&view_column);
    view_column.connect_fixed_width_notify(announce.clone());
    resizer.connect_map(glib::clone!(
        #[weak]
        view_column,
        move |_| announce(&view_column)
    ));
    resizer.connect_value_requested(glib::clone!(
        #[weak]
        view_column,
        move |value| {
            #[expect(clippy::cast_possible_truncation, reason = "column widths are small")]
            let width = resized_width(column, value.round() as i32, 0);
            view_column.set_expand(false);
            view_column.set_fixed_width(column_widths::fixed_width(column, Some(width)));
        }
    ));
}

/// Carries out `action` on the title `title` of `column`.
fn run_title_key(view: &DetailsView, title: &gtk::Widget, column: SortColumn, action: TitleKey) {
    let Some(view_column) = view.column(column) else {
        return;
    };
    match action {
        TitleKey::Sort => view.sort_by(order_after_sort_key(column, view.sort_order())),
        TitleKey::Resize(change) => {
            let fixed = view_column.fixed_width();
            let shown = if fixed > 0 { fixed } else { title.width() };
            let current = column_widths::saved_width(column, shown).unwrap_or_default();
            #[expect(clippy::cast_possible_truncation, reason = "column widths are small")]
            let width = resized_width(column, current as i32, change);
            view_column.set_expand(false);
            view_column.set_fixed_width(column_widths::fixed_width(column, Some(width)));
        }
        TitleKey::FitToItems => {
            let list = view.column_view();
            let text_widths = view
                .cell_texts(column, FIT_ITEMS)
                .iter()
                .map(|text| list.create_pango_layout(Some(text)).pixel_size().0)
                .collect::<Vec<_>>();
            let width = fitted_width(column, text_widths);
            view_column.set_expand(false);
            view_column.set_fixed_width(column_widths::fixed_width(column, Some(width)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ACC-006
    #[test]
    fn title_keys_sort_resize_within_the_limits_and_fit_the_items() {
        assert_eq!(TitleKey::for_key(gdk::Key::Return, false), Some(TitleKey::Sort));
        assert_eq!(
            TitleKey::for_key(gdk::Key::Left, false),
            Some(TitleKey::Resize(-10))
        );
        assert_eq!(
            TitleKey::for_key(gdk::Key::Right, true),
            Some(TitleKey::Resize(40))
        );
        assert_eq!(
            TitleKey::for_key(gdk::Key::Home, false),
            Some(TitleKey::FitToItems)
        );
        assert_eq!(resized_width(SortColumn::Type, 135, 10), 145);
        assert_eq!(fitted_width(SortColumn::Type, [40, 120, 90]), 150);
        assert_eq!(fitted_width(SortColumn::Name, [300]), 364, "room for the icon");
        assert_eq!(
            fitted_width(SortColumn::Size, []),
            70,
            "at least the smallest width"
        );
        assert_eq!(fitted_width(SortColumn::Size, [900]), 600, "at most the widest");
        assert_eq!(
            resized_width(SortColumn::Size, 75, -10),
            70,
            "Size is at least 70"
        );
        let by_name = SortOrder::DEFAULT;
        let by_size = order_after_sort_key(SortColumn::Size, by_name);
        assert_eq!(by_size.direction, SortDirection::Ascending);
        let again = order_after_sort_key(SortColumn::Size, by_size);
        assert_eq!(again.direction, SortDirection::Descending);
    }
}
