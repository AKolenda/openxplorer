// SPDX-License-Identifier: AGPL-3.0-only
//! "Choose Details", which "More..." in the column titles' menu opens, as
//! in Windows 11 File Explorer (VIEW-033, VIEW-034).
//!
//! The window lists every column the app can fill as a check box: the
//! shown ones first, in their order (Name at the top unless it was moved
//! down), then the others in
//! alphabetical order. Beside the list, Move Up, Move Down, Show and Hide
//! act on the selected column, grayed where they cannot, as in Windows:
//! Name always shows, its check box drawn checked and never cleared, but
//! it moves like any other column. Under it, "Width of selected column (in
//! pixels):" holds the selected column's width, digits only, kept within
//! the column's limits. OK shows the checked columns in the list's order
//! with the widths typed and saves them as the titles' menu does, for the
//! folder or for every folder (Folder Options); Cancel, Escape and the
//! close button change nothing.
//!
//! The list follows what the view lists: a search adds Folder path and
//! the Recycle Bin Original location and Date deleted, right after Name
//! where the view shows them, checked, never hidden or moved; Date modified, which neither shows, is left out and stays
//! chosen for folders. OK applies the choice only while the pane still
//! shows the tab, location and listing it was opened for: a folder opened
//! from another application meanwhile is left as it is.
//!
//! The keyboard reaches everything: the arrows move through the columns,
//! Space checks or clears the selected one, Tab leaves the list for the
//! buttons and the width, Enter in the list or the width is OK, Escape is
//! Cancel.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::i18n::gettext;

use super::session::{PaneSide, TabId};
use super::{BrowserWindow, ButtonStyle, Dialog};
use crate::folder_view::column_widths;
use crate::folder_view::details::{lists_column, DetailsListing, FOLLOWING_NAME};
use crate::folder_view::sorting::SortColumn;

/// The widget name of the columns' list, by which an open Choose Details
/// is found again.
const LIST_NAME: &str = "choose-details-list";

/// How wide the window is, in pixels.
const WINDOW_WIDTH: i32 = 440;

/// How tall the columns' list is, in pixels: nine rows.
const LIST_HEIGHT: i32 = 250;

/// One column in the window's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChoiceRow {
    /// The column.
    pub(crate) column: SortColumn,
    /// Whether it is checked, so it shows.
    pub(crate) shown: bool,
    /// Its width in pixels, as settings save it.
    pub(crate) width: u32,
    /// Whether the user typed its width.
    pub(crate) width_typed: bool,
}

/// Whether `column` is one a listing always shows right after Name:
/// Folder path in a search, Original location and Date deleted in the
/// Recycle Bin. Choose Details lists it checked there, and it neither
/// hides nor moves.
fn is_fixed(column: SortColumn) -> bool {
    FOLLOWING_NAME.contains(&column)
}

/// The window's list of columns and what the user changed in it, before
/// OK applies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetailsChoice {
    rows: Vec<ChoiceRow>,
    /// The chosen columns the listing does not have, such as Date
    /// modified in a search or the Recycle Bin, with their places among
    /// the chosen ones: they are not listed, and OK keeps them chosen for
    /// the listings that have them.
    unlisted: Vec<(usize, SortColumn)>,
}

impl DetailsChoice {
    /// The list for a folder's view showing the `chosen` columns (after
    /// Name, unless they place it), each `width_of` pixels wide: the shown
    /// ones in their order, then the other columns in alphabetical order.
    #[cfg(test)]
    pub(crate) fn new(chosen: &[SortColumn], width_of: impl Fn(SortColumn) -> u32) -> Self {
        Self::for_listing(chosen, DetailsListing::Folder, width_of)
    }

    /// The list for a view of `listing` showing the `chosen` columns, as
    /// [`DetailsChoice::new`] makes it, with only the columns `listing`
    /// has: Folder path in a search, Original location and Date deleted
    /// in the Recycle Bin, after Name as the view shows them, and no Date
    /// modified in either.
    pub(crate) fn for_listing(
        chosen: &[SortColumn],
        listing: DetailsListing,
        width_of: impl Fn(SortColumn) -> u32,
    ) -> Self {
        let listed = |column: &SortColumn| lists_column(listing, *column);
        let unlisted = chosen
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, column)| !listed(column))
            .collect();
        let mut hidden: Vec<SortColumn> = SortColumn::CHOOSABLE
            .into_iter()
            .filter(|column| !chosen.contains(column) && listed(column))
            .collect();
        hidden.sort_by_cached_key(|column| column.label().to_lowercase());
        let leading_name = (!chosen.contains(&SortColumn::Name)).then_some(SortColumn::Name);
        let fixed = SortColumn::ALL
            .into_iter()
            .filter(|column| is_fixed(*column) && listed(column));
        let shown = leading_name
            .into_iter()
            .chain(chosen.iter().copied().filter(listed))
            .chain(fixed)
            .map(|column| (column, true));
        let rows = shown
            .chain(hidden.into_iter().map(|column| (column, false)))
            .map(|(column, shown)| ChoiceRow {
                column,
                shown,
                width: width_of(column),
                width_typed: false,
            })
            .collect();
        let mut choice = Self { rows, unlisted };
        choice.place_fixed_rows();
        choice
    }

    /// Puts the listing's own columns where the view shows them: right
    /// after Name, wherever Name is.
    fn place_fixed_rows(&mut self) {
        let (fixed, mut rows): (Vec<ChoiceRow>, Vec<ChoiceRow>) =
            self.rows.drain(..).partition(|row| is_fixed(row.column));
        let after_name = rows
            .iter()
            .position(|row| row.column == SortColumn::Name)
            .map_or(0, |name| name + 1);
        rows.splice(after_name..after_name, fixed);
        self.rows = rows;
    }

    /// The rows, in order.
    pub(crate) fn rows(&self) -> &[ChoiceRow] {
        &self.rows
    }

    /// The index of `column`'s row.
    pub(crate) fn index_of(&self, column: SortColumn) -> Option<usize> {
        self.rows.iter().position(|row| row.column == column)
    }

    /// The row a move of the row at `index` swaps with, among `others`
    /// (the indexes above or below it, nearest first): the nearest one
    /// that moves, as the listing's own columns keep their places.
    fn swap_partner(&self, index: usize, mut others: impl Iterator<Item = usize>) -> Option<usize> {
        let row = self.rows.get(index)?;
        if is_fixed(row.column) {
            return None;
        }
        others.find(|other| !is_fixed(self.rows[*other].column))
    }

    /// Whether Move Up applies to the row at `index`: every row but the
    /// first, as in Windows, and none of the listing's own columns.
    pub(crate) fn can_move_up(&self, index: usize) -> bool {
        self.swap_partner(index, (0..index).rev()).is_some()
    }

    /// Whether Move Down applies to the row at `index`: every row but the
    /// last, Name included, as in Windows, and none of the listing's own
    /// columns.
    pub(crate) fn can_move_down(&self, index: usize) -> bool {
        self.swap_partner(index, index + 1..self.rows.len()).is_some()
    }

    /// Whether the row at `index` can be cleared, as every one but Name
    /// and the listing's own columns can.
    fn can_clear(&self, index: usize) -> bool {
        self.rows
            .get(index)
            .is_some_and(|row| row.column != SortColumn::Name && !is_fixed(row.column))
    }

    /// Whether Show applies to the row at `index`: it is not checked.
    pub(crate) fn can_show(&self, index: usize) -> bool {
        self.rows.get(index).is_some_and(|row| !row.shown)
    }

    /// Whether Hide applies to the row at `index`: it is checked and can
    /// be cleared.
    pub(crate) fn can_hide(&self, index: usize) -> bool {
        self.can_clear(index) && self.rows[index].shown
    }

    /// Swaps the row at `index` with `partner`, if it has one, and
    /// returns where the row is then.
    fn move_to(&mut self, index: usize, partner: Option<usize>) -> usize {
        let Some(partner) = partner else {
            return index;
        };
        let column = self.rows[index].column;
        self.rows.swap(index, partner);
        self.place_fixed_rows();
        self.index_of(column).unwrap_or(partner)
    }

    /// Moves the row at `index` up one, past the listing's own columns,
    /// if it can, and returns where it is.
    pub(crate) fn move_up(&mut self, index: usize) -> usize {
        let partner = self.swap_partner(index, (0..index).rev());
        self.move_to(index, partner)
    }

    /// Moves the row at `index` down one, past the listing's own columns,
    /// if it can, and returns where it is.
    pub(crate) fn move_down(&mut self, index: usize) -> usize {
        let partner = self.swap_partner(index, index + 1..self.rows.len());
        self.move_to(index, partner)
    }

    /// Checks or clears the row at `index`; Name and the listing's own
    /// columns stay checked.
    pub(crate) fn set_shown(&mut self, index: usize, shown: bool) {
        if self.can_clear(index) {
            self.rows[index].shown = shown;
        }
    }

    /// Takes `text`, typed in the width field, as the width of the row at
    /// `index`, kept within its column's limits, and returns that width:
    /// `None` for an empty field or one with anything but digits, which
    /// changes nothing.
    pub(crate) fn set_width_text(&mut self, index: usize, text: &str) -> Option<u32> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let row = self.rows.get_mut(index)?;
        let limits = column_widths::width_limits(row.column);
        // All digits: only a number too large for `u32` fails.
        let width = text.parse::<u32>().unwrap_or(u32::MAX);
        row.width = width.clamp(*limits.start(), *limits.end());
        row.width_typed = true;
        Some(row.width)
    }

    /// The checked columns in the list's order, Name among them only
    /// when it is not first, with the chosen columns the listing does not
    /// have back in their places.
    pub(crate) fn chosen(&self) -> Vec<SortColumn> {
        let shown = self
            .rows
            .iter()
            .filter(|row| row.shown && !is_fixed(row.column))
            .map(|row| row.column);
        let mut chosen = crate::folder_view::details::without_leading_name(shown.collect());
        for (place, column) in &self.unlisted {
            chosen.insert((*place).min(chosen.len()), *column);
        }
        chosen
    }

    /// The columns whose width the user typed, with that width.
    pub(crate) fn changed_widths(&self) -> Vec<(SortColumn, u32)> {
        self.rows
            .iter()
            .filter(|row| row.width_typed)
            .map(|row| (row.column, row.width))
            .collect()
    }
}

/// What a Choose Details window was opened for: the tab its pane showed,
/// that tab's location and what the details view listed there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChoiceTarget {
    /// The tab.
    tab: TabId,
    /// Its location.
    uri: String,
    /// What the details view listed.
    listing: DetailsListing,
}

/// The digits of `text`, which is all the width field takes.
pub(crate) fn digits_only(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// An open Choose Details window and its controls.
#[derive(Debug, Clone)]
pub(crate) struct ChooseDetails {
    /// The window.
    pub(crate) dialog: Dialog,
    /// The columns, one check box per row.
    pub(crate) list: gtk::ListBox,
    /// Moves the selected column up.
    pub(crate) move_up: gtk::Button,
    /// Moves the selected column down.
    pub(crate) move_down: gtk::Button,
    /// Checks the selected column.
    pub(crate) show: gtk::Button,
    /// Clears the selected column.
    pub(crate) hide: gtk::Button,
    /// The selected column's width.
    pub(crate) width: gtk::Entry,
    /// Applies the choice.
    pub(crate) ok: gtk::Button,
    /// Discards it.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tests press it; the dialog answers it")
    )]
    pub(crate) cancel: gtk::Button,
}

/// Choose Details as its controls' callbacks hold it: weakly, so the
/// window is freed once it closes.
#[derive(Debug, Clone)]
struct WeakChooseDetails(glib::WeakRef<Dialog>);

impl WeakChooseDetails {
    /// The window's controls, while it is still there.
    fn upgrade(&self) -> Option<ChooseDetails> {
        ChooseDetails::of(self.0.upgrade()?)
    }
}

/// What the window's controls share while it is open.
struct State {
    /// The list as the user has changed it.
    choice: RefCell<DetailsChoice>,
    /// Set while the controls are being filled from the choice, so their
    /// own signals change nothing.
    filling: Cell<bool>,
}

impl ChooseDetails {
    /// The Choose Details window shown over `window`, if one is.
    #[cfg(test)]
    pub(crate) fn shown_over(window: &BrowserWindow) -> Option<Self> {
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<Dialog>().ok())
            .filter(|dialog| {
                dialog.is_visible() && dialog.transient_for().as_ref() == Some(window.upcast_ref())
            })
            .find(|dialog| list_of(dialog).is_some())?;
        Self::of(dialog)
    }

    /// A handle that does not keep the window alive.
    fn weak(&self) -> WeakChooseDetails {
        WeakChooseDetails(self.dialog.downgrade())
    }

    /// The controls of `dialog`, a Choose Details window.
    fn of(dialog: Dialog) -> Option<Self> {
        let list = list_of(&dialog)?;
        let buttons = super::widget_tree::descendants::<gtk::Button>(&dialog);
        let button = |label: &str| {
            buttons
                .iter()
                .find(|button| button.label().as_deref() == Some(label))
                .cloned()
        };
        let width = super::widget_tree::descendants::<gtk::Entry>(&dialog)
            .into_iter()
            .next()?;
        Some(Self {
            move_up: button(&gettext("Move Up"))?,
            move_down: button(&gettext("Move Down"))?,
            show: button(&gettext("Show"))?,
            hide: button(&gettext("Hide"))?,
            ok: button(&gettext("OK"))?,
            cancel: button("Cancel")?,
            width,
            list,
            dialog,
        })
    }

    /// Each row's label, whether it is checked and whether its check box
    /// is drawn usable (not grayed).
    #[cfg(test)]
    pub(crate) fn row_states(&self) -> Vec<(String, bool, bool)> {
        super::widget_tree::descendants::<gtk::CheckButton>(&self.list)
            .into_iter()
            .map(|check| {
                let label = check.label().map(String::from).unwrap_or_default();
                (label, check.is_active(), check.is_sensitive())
            })
            .collect()
    }

    /// The index of the selected row.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        let row = self.list.selected_row()?;
        usize::try_from(row.index()).ok()
    }

    /// Selects the row at `index`, as a click on it does.
    pub(crate) fn select(&self, index: usize) {
        let row = i32::try_from(index)
            .ok()
            .and_then(|index| self.list.row_at_index(index));
        self.list.select_row(row.as_ref());
    }
}

/// The columns' list of `dialog`, if it is a Choose Details window.
fn list_of(dialog: &Dialog) -> Option<gtk::ListBox> {
    super::widget_tree::descendants::<gtk::ListBox>(dialog)
        .into_iter()
        .find(|list| list.widget_name() == LIST_NAME)
}

/// A button of the column the list selects.
fn side_button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class(ButtonStyle::Bordered.css_class());
    button.add_css_class("dialog-button");
    button
}

/// A row of the columns' list: the column's check box. The row takes the
/// keyboard; its check box is clicked, or checked with Space on the row.
/// Name's is drawn like the others, checked, as Windows draws it: a click
/// or Space leaves it checked.
fn choice_row(row: ChoiceRow) -> gtk::ListBoxRow {
    let check = gtk::CheckButton::builder()
        .label(row.column.label())
        .active(row.shown)
        .focusable(false)
        .focus_on_click(false)
        .build();
    let list_row = gtk::ListBoxRow::builder().child(&check).build();
    list_row.update_property(&[gtk::accessible::Property::Label(row.column.label())]);
    list_row
}

/// The check box of `row`.
fn check_of(row: &gtk::ListBoxRow) -> Option<gtk::CheckButton> {
    row.child().and_downcast::<gtk::CheckButton>()
}

impl ChooseDetails {
    /// Builds the window over `window` for its active pane's details view.
    fn build(window: &BrowserWindow) -> Self {
        let dialog = Dialog::with_title_bar(
            window,
            &gettext("Choose Details"),
            &gettext("Select the details you want to display for the items in this folder."),
        );
        dialog.set_default_width(WINDOW_WIDTH);
        dialog.add_css_class("choose-details");

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(false)
            .css_classes(["choose-details-list"])
            .build();
        list.set_widget_name(LIST_NAME);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(LIST_HEIGHT)
            .max_content_height(LIST_HEIGHT)
            .hexpand(true)
            .has_frame(true)
            .build();
        let caption = gtk::Label::builder()
            .label(gettext("Details:"))
            .xalign(0.0)
            .css_classes(["field-label"])
            .mnemonic_widget(&list)
            .build();
        list.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);

        let move_up = side_button(&gettext("Move Up"));
        let move_down = side_button(&gettext("Move Down"));
        let show = side_button(&gettext("Show"));
        let hide = side_button(&gettext("Hide"));
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .valign(gtk::Align::Start)
            .build();
        for button in [&move_up, &move_down, &show, &hide] {
            buttons.append(button);
        }
        let columns = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .build();
        columns.append(&scroller);
        columns.append(&buttons);

        let width = gtk::Entry::builder()
            .width_chars(6)
            .max_width_chars(6)
            .input_purpose(gtk::InputPurpose::Digits)
            .activates_default(true)
            .build();
        let width_caption = gtk::Label::builder()
            .label(gettext("Width of selected column (in pixels):"))
            .xalign(0.0)
            .mnemonic_widget(&width)
            .build();
        width.update_relation(&[gtk::accessible::Relation::LabelledBy(&[
            width_caption.upcast_ref()
        ])]);
        let width_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .css_classes(["choose-details-width"])
            .build();
        width_row.append(&width_caption);
        width_row.append(&width);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        body.append(&caption);
        body.append(&columns);
        body.append(&width_row);
        dialog.add_field(&body);

        dialog.add_button(&gettext("OK"), ButtonStyle::Accent);
        dialog.add_cancel_button();
        Self::of(dialog).expect("the window was just built")
    }

    /// Lists the rows of the choice again, the row at `selected` selected.
    fn fill_rows(&self, state: &Rc<State>, selected: usize) {
        self.list.remove_all();
        let rows: Vec<ChoiceRow> = state.choice.borrow().rows().to_vec();
        for row in &rows {
            let list_row = choice_row(*row);
            self.follow_check_box(state, &list_row);
            self.list.append(&list_row);
        }
        self.select(selected);
    }

    /// Greys the buttons that do not apply to the selected column and
    /// shows its width.
    fn refresh(&self, state: &State) {
        let choice = state.choice.borrow();
        let selected = self.selected_index();
        let applies =
            |test: fn(&DetailsChoice, usize) -> bool| selected.is_some_and(|index| test(&choice, index));
        self.move_up.set_sensitive(applies(DetailsChoice::can_move_up));
        self.move_down
            .set_sensitive(applies(DetailsChoice::can_move_down));
        self.show.set_sensitive(applies(DetailsChoice::can_show));
        self.hide.set_sensitive(applies(DetailsChoice::can_hide));
        let width = selected
            .and_then(|index| choice.rows().get(index))
            .map(|row| row.width.to_string())
            .unwrap_or_default();
        state.filling.set(true);
        self.width.set_text(&width);
        state.filling.set(false);
        self.width.set_sensitive(selected.is_some());
    }

    /// Checks or clears the selected column, from Show, Hide, Space or
    /// its check box.
    fn set_selected_shown(&self, state: &State, shown: bool) {
        let Some(index) = self.selected_index() else {
            return;
        };
        state.choice.borrow_mut().set_shown(index, shown);
        let checked = state.choice.borrow().rows()[index].shown;
        if let Some(check) = self.list.selected_row().as_ref().and_then(check_of) {
            state.filling.set(true);
            check.set_active(checked);
            state.filling.set(false);
        }
        self.refresh(state);
    }

    /// Moves the selected column with `step` (`DetailsChoice::move_up` or
    /// `move_down`), keeping it selected; the keyboard stays on `button`
    /// while it still applies, else goes to the list.
    fn move_selected(
        &self,
        state: &Rc<State>,
        step: fn(&mut DetailsChoice, usize) -> usize,
        button: &gtk::Button,
    ) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let moved = step(&mut state.choice.borrow_mut(), index);
        if moved == index {
            return;
        }
        // Listing the rows again greys the buttons for a moment, which
        // takes the keyboard off `button`: it is given back.
        self.fill_rows(state, moved);
        self.refresh(state);
        if button.is_sensitive() {
            button.grab_focus();
        } else if let Some(row) = self.list.selected_row() {
            row.grab_focus();
        }
    }

    /// Connects the controls to `state` and lists its rows. The callbacks
    /// reach the controls through [`WeakChooseDetails`]: the controls own
    /// their callbacks, so a callback owning the controls would keep the
    /// closed window alive.
    fn connect(&self, state: &Rc<State>) {
        let handle = self.weak();
        self.list.connect_row_selected(glib::clone!(
            #[strong]
            handle,
            #[strong]
            state,
            move |_, _| {
                if let Some(shown) = handle.upgrade() {
                    shown.refresh(&state);
                }
            }
        ));
        self.show.connect_clicked(glib::clone!(
            #[strong]
            handle,
            #[strong]
            state,
            move |_| {
                if let Some(shown) = handle.upgrade() {
                    shown.set_selected_shown(&state, true);
                }
            }
        ));
        self.hide.connect_clicked(glib::clone!(
            #[strong]
            handle,
            #[strong]
            state,
            move |_| {
                if let Some(shown) = handle.upgrade() {
                    shown.set_selected_shown(&state, false);
                }
            }
        ));
        self.move_up.connect_clicked(glib::clone!(
            #[strong]
            handle,
            #[strong]
            state,
            move |button| {
                if let Some(shown) = handle.upgrade() {
                    shown.move_selected(&state, DetailsChoice::move_up, button);
                }
            }
        ));
        self.move_down.connect_clicked(glib::clone!(
            #[strong]
            handle,
            #[strong]
            state,
            move |button| {
                if let Some(shown) = handle.upgrade() {
                    shown.move_selected(&state, DetailsChoice::move_down, button);
                }
            }
        ));
        self.width.connect_changed(glib::clone!(
            #[strong]
            handle,
            #[strong]
            state,
            move |entry| {
                let Some(shown) = handle.upgrade() else { return };
                if state.filling.get() {
                    return;
                }
                if let Some(index) = shown.selected_index() {
                    state.choice.borrow_mut().set_width_text(index, &entry.text());
                }
            }
        ));
        take_digits_only(&self.width);
        self.list.add_controller(self.list_keys(state));
        self.dialog.add_controller(self.back_to_list_keys());
        self.fill_rows(state, 0);
        self.refresh(state);
    }

    /// Follows clicks on the check box of `row`: a click checks or clears
    /// its column and selects it.
    fn follow_check_box(&self, state: &Rc<State>, row: &gtk::ListBoxRow) {
        let Some(check) = check_of(row) else { return };
        check.connect_toggled(glib::clone!(
            #[strong(rename_to = handle)]
            self.weak(),
            #[strong]
            state,
            #[weak]
            row,
            move |check| {
                let Some(shown) = handle.upgrade() else { return };
                if state.filling.get() {
                    return;
                }
                shown.list.select_row(Some(&row));
                shown.set_selected_shown(&state, check.is_active());
            }
        ));
    }

    /// Space checks or clears the selected column, Enter is OK and Tab
    /// leaves the list, before the list would take them.
    fn list_keys(&self, state: &Rc<State>) -> gtk::EventControllerKey {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[strong(rename_to = handle)]
            self.weak(),
            #[strong]
            state,
            move |_, key, _, modifiers| {
                let Some(shown) = handle.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                    return glib::Propagation::Proceed;
                }
                match key {
                    gdk::Key::space | gdk::Key::KP_Space => {
                        let Some(index) = shown.selected_index() else {
                            return glib::Propagation::Proceed;
                        };
                        let checked = state.choice.borrow().rows()[index].shown;
                        shown.set_selected_shown(&state, !checked);
                        glib::Propagation::Stop
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => {
                        shown.ok.emit_clicked();
                        glib::Propagation::Stop
                    }
                    // Tab leaves the list for the first button that
                    // applies, as from a Windows list; the list would
                    // otherwise keep it among its rows.
                    gdk::Key::Tab | gdk::Key::KP_Tab
                        if !modifiers.contains(gdk::ModifierType::SHIFT_MASK) =>
                    {
                        match shown.after_list() {
                            Some(widget) => {
                                widget.grab_focus();
                                glib::Propagation::Stop
                            }
                            None => glib::Propagation::Proceed,
                        }
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        ));
        keys
    }
}

impl ChooseDetails {
    /// The control Tab goes to from the list: the first that applies of
    /// the side buttons, the width and OK.
    fn after_list(&self) -> Option<gtk::Widget> {
        let next: [&gtk::Widget; 6] = [
            self.move_up.upcast_ref(),
            self.move_down.upcast_ref(),
            self.show.upcast_ref(),
            self.hide.upcast_ref(),
            self.width.upcast_ref(),
            self.ok.upcast_ref(),
        ];
        next.into_iter().find(|widget| widget.is_sensitive()).cloned()
    }

    /// Shift+Tab from the control after the list goes back to the list's
    /// selected row.
    fn back_to_list_keys(&self) -> gtk::EventControllerKey {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[strong(rename_to = handle)]
            self.weak(),
            move |_, key, _, modifiers| {
                let Some(shown) = handle.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let backward = key == gdk::Key::ISO_Left_Tab
                    || (matches!(key, gdk::Key::Tab | gdk::Key::KP_Tab)
                        && modifiers.contains(gdk::ModifierType::SHIFT_MASK));
                let focus = gtk::prelude::GtkWindowExt::focus(&shown.dialog);
                let on_first = focus.is_some_and(|focus| {
                    shown
                        .after_list()
                        .is_some_and(|first| focus == first || focus.is_ancestor(&first))
                });
                match shown.list.selected_row() {
                    Some(row) if backward && on_first => {
                        row.grab_focus();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        ));
        keys
    }
}

/// Makes `entry` take digits only: anything else typed or pasted is left
/// out.
fn take_digits_only(entry: &gtk::Entry) {
    let Some(text) = entry.delegate() else { return };
    text.connect_insert_text(|editable, inserted, position| {
        let digits = digits_only(inserted);
        if digits == inserted {
            return;
        }
        editable.stop_signal_emission_by_name("insert-text");
        if !digits.is_empty() {
            editable.insert_text(&digits, position);
        }
    });
}

impl BrowserWindow {
    /// Opens Choose Details for the active pane's details view
    /// (`win.choose-details`, "More..." in the titles' menu).
    pub(super) fn open_choose_details(&self) {
        // OK applies to the pane the window was opened for, even if the
        // other pane of a split view became active meanwhile.
        let side = self.imp().active_side.get();
        let target = self.choice_target(side);
        let details = self.pane_on(side).details().clone();
        let choice = DetailsChoice::for_listing(&details.chosen_columns(), details.listing(), |column| {
            details.width_on_screen(column)
        });
        let state = Rc::new(State {
            choice: RefCell::new(choice),
            filling: Cell::new(false),
        });
        let shown = ChooseDetails::build(self);
        shown.connect(&state);
        let first = shown.list.row_at_index(0);
        match &first {
            Some(row) => shown.dialog.open_focusing(row),
            None => shown.dialog.open(),
        }
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let answer = shown.dialog.next_response().await;
            if answer.is_some() {
                // The pane's widgets serve every tab: a folder opened from
                // another application while the window was open may show
                // in the pane by now, and the choice was made for another
                // folder and listing. It is then dropped, as Cancel does.
                let window = window.upgrade();
                let same = window
                    .as_ref()
                    .is_some_and(|window| target.is_some() && window.choice_target(side) == target);
                if let (Some(window), true) = (window, same) {
                    window.apply_details_choice(side, &state.choice.borrow());
                }
                shown.dialog.finish();
            }
        });
    }

    /// What the pane on `side` shows now, for Choose Details to check it
    /// still shows it on OK: its tab, the tab's location and what the
    /// details view lists there.
    fn choice_target(&self, side: PaneSide) -> Option<ChoiceTarget> {
        let (tab, uri) = {
            let session = self.imp().session.borrow();
            let active = session.active()?;
            let shown = std::iter::once(active)
                .chain(active.beside())
                .find(|tab| tab.side == side)?;
            (shown.id, shown.uri().to_owned())
        };
        let listing = self.pane_on(side).details().listing();
        Some(ChoiceTarget { tab, uri, listing })
    }

    /// Shows the columns `choice` checks in the pane on `side`, in its
    /// order, with the widths typed, and saves them as the titles' menu
    /// does.
    fn apply_details_choice(&self, side: PaneSide, choice: &DetailsChoice) {
        let pane = self.pane_on(side);
        let details = pane.details();
        for (column, width) in choice.changed_widths() {
            details.set_column_width(column, width);
        }
        details.show_chosen_columns(choice.chosen());
        self.remember_pane_style(pane);
    }
}
