// SPDX-License-Identifier: AGPL-3.0-only
//! The Details tab of a file, laid out as Windows 11's (PROP-034).
//!
//! A sunken list under a Property and Value header: blue headings with a
//! thin line after them (Description, Origin, Image, ...), and under each
//! the property's name and value ([`super::details_rows`]). The values are
//! read only.
//!
//! The list moves with the arrow keys, Home, End, Page Up and Page Down;
//! each row reads as "Camera model: …" to a screen reader. Ctrl+C copies
//! the value of the row with the cursor, and Enter, F2 or a click in a
//! value lets its text be selected and copied, as clicking a value does in
//! Windows; Escape goes back to the list.
//!
//! The File rows come from the item's properties as soon as they are read.
//! A picture's, video's or audio file's own rows are read the first time
//! the tab is shown, in a reader process ([`crate::media_details`]) that
//! the window stops when it closes; a reader that fails or takes too long
//! leaves the File rows alone.
//!
//! Windows' "Remove Properties and Personal Information" is left out: see
//! PROP-035 in `native/parity/features.toml`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::LocationContext;

use super::details_rows::{detail_sections, DetailSection, DetailValue};
use super::metadata::ItemProperties;
use super::window::PropertiesWindow;
use crate::icons::{self, Icon};
use crate::media_details::{read_in_reader, MediaDetails, MediaKind, ReaderProcess};

/// The list's height, as Windows' Details list fills its tab.
const LIST_HEIGHT: i32 = 320;
/// The size of a Rating star.
const STAR_SIZE: i32 = 14;

/// The Details tab of one file.
#[derive(Clone)]
pub(super) struct DetailsPanel {
    inner: Rc<Inner>,
}

struct Inner {
    page: gtk::Box,
    rows: gtk::ListBox,
    locations: LocationContext,
    properties: RefCell<Option<ItemProperties>>,
    media: RefCell<Option<MediaDetails>>,
    kind: Cell<Option<MediaKind>>,
    shown: Cell<bool>,
    reading: Cell<bool>,
    process: ReaderProcess,
    /// Each property row and the text its value copies as.
    values: RefCell<Vec<(gtk::ListBoxRow, String)>>,
}

impl Inner {
    /// The text the value of `row` copies as.
    fn value_text(&self, row: &gtk::ListBoxRow) -> Option<String> {
        self.values
            .borrow()
            .iter()
            .find(|(kept, _)| kept == row)
            .map(|(_, text)| text.clone())
    }
}

impl DetailsPanel {
    /// An empty Details tab, filled by [`Self::show_properties`].
    pub(super) fn new(locations: LocationContext) -> Self {
        let page = super::window::page();
        page.add_css_class("properties-details");
        let rows = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Browse)
            .activate_on_single_click(false)
            .css_classes(["details-rows"])
            .build();
        rows.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
            "Details",
        ))]);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&rows)
            .min_content_height(LIST_HEIGHT)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&column_titles());
        column.append(&scroller);
        let frame = gtk::Frame::builder()
            .child(&column)
            .css_classes(["properties-list-frame", "details-frame"])
            .build();
        page.append(&frame);
        let panel = Self {
            inner: Rc::new(Inner {
                page,
                rows,
                locations,
                properties: RefCell::default(),
                media: RefCell::default(),
                kind: Cell::new(None),
                shown: Cell::new(false),
                reading: Cell::new(false),
                process: ReaderProcess::default(),
                values: RefCell::default(),
            }),
        };
        panel.connect_keys();
        panel.inner.rows.connect_row_activated(|_, row| edit_value(row));
        panel
    }

    /// The tab's page.
    pub(super) fn page(&self) -> &gtk::Box {
        &self.inner.page
    }

    /// Stops the reader when `window` closes.
    pub(super) fn follow_window(&self, window: &PropertiesWindow) {
        let process = self.inner.process.clone();
        window.connect_closed_once(move || process.stop());
    }

    /// Shows the File rows of `properties`, and reads the media rows if
    /// the tab is shown.
    pub(super) fn show_properties(&self, properties: &ItemProperties) {
        let kind = properties
            .entry
            .content_type
            .as_deref()
            .and_then(MediaKind::for_content_type);
        self.inner.kind.set(kind);
        self.inner.properties.replace(Some(properties.clone()));
        self.fill();
        self.read_media();
    }

    /// The tab was shown: its media rows are read the first time.
    pub(super) fn shown(&self) {
        self.inner.shown.set(true);
        self.read_media();
    }

    fn read_media(&self) {
        let inner = &self.inner;
        let Some(kind) = inner.kind.get() else {
            return;
        };
        if !inner.shown.get() || inner.reading.replace(true) {
            return;
        }
        let Some((uri, size)) = inner
            .properties
            .borrow()
            .as_ref()
            .map(|properties| (properties.entry.uri.clone(), properties.entry.size))
        else {
            return;
        };
        let panel = Rc::downgrade(&self.inner);
        glib::spawn_future_local(async move {
            let process = match panel.upgrade() {
                Some(inner) => inner.process.clone(),
                None => return,
            };
            let media = read_in_reader(&uri, kind, size, &process).await;
            if let Some(inner) = panel.upgrade() {
                let panel = DetailsPanel { inner };
                if media.is_some() {
                    panel.inner.media.replace(media);
                    panel.fill();
                }
            }
        });
    }

    /// Fills the list from what is known, keeping the cursor's row.
    fn fill(&self) {
        let inner = &self.inner;
        let Some(properties) = inner.properties.borrow().clone() else {
            return;
        };
        let sections = detail_sections(
            &properties,
            inner.kind.get(),
            inner.media.borrow().as_ref(),
            &inner.locations,
        );
        let had_focus = inner.rows.focus_child().is_some();
        let cursor = inner.rows.selected_row().map(|row| row.widget_name().to_string());
        while let Some(child) = inner.rows.first_child() {
            inner.rows.remove(&child);
        }
        let mut first = None;
        let mut values = Vec::new();
        for section in &sections {
            inner.rows.append(&heading_row(section));
            for row in &section.rows {
                let widget = property_row(&row.name, &row.value);
                first.get_or_insert_with(|| widget.clone());
                inner.rows.append(&widget);
                values.push((widget, row.value.text()));
            }
        }
        inner.values.replace(values);
        let again = cursor.and_then(|name| find_row(&inner.rows, &name)).or(first);
        if let Some(row) = again {
            inner.rows.select_row(Some(&row));
            if had_focus {
                row.grab_focus();
            }
        }
    }

    /// Ctrl+C copies the cursor's value; F2 lets its text be selected.
    fn connect_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        let panel = Rc::downgrade(&self.inner);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(inner) = panel.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let rows = &inner.rows;
            let Some(row) = rows.selected_row().filter(|row| row.is_focus()) else {
                return glib::Propagation::Proceed;
            };
            let copy = modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gdk::Key::c | gdk::Key::C | gdk::Key::Insert);
            if copy {
                if let Some(text) = inner.value_text(&row) {
                    rows.clipboard().set_text(&text);
                }
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::F2 && modifiers.is_empty() {
                edit_value(&row);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.inner.rows.add_controller(keys);
    }
}

/// "Property" and "Value" above the list, hidden from screen readers:
/// each row names its property.
fn column_titles() -> gtk::Box {
    let titles = gtk::Box::builder().css_classes(["details-columns"]).build();
    let property = gtk::Label::builder()
        .label(ox_core::i18n::gettext("Property"))
        .xalign(0.0)
        .css_classes(["details-name"])
        .build();
    let rule = gtk::Separator::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["details-column-rule"])
        .build();
    let value = gtk::Label::builder()
        .label(ox_core::i18n::gettext("Value"))
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["details-value-title"])
        .build();
    titles.append(&property);
    titles.append(&rule);
    titles.append(&value);
    titles.set_accessible_role(gtk::AccessibleRole::Presentation);
    titles
}

/// A heading: its blue name and a thin line to the right edge.
fn heading_row(section: &DetailSection) -> gtk::ListBoxRow {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let label = gtk::Label::builder()
        .label(&section.heading)
        .xalign(0.0)
        .css_classes(["details-heading"])
        .build();
    let rule = gtk::Separator::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["details-heading-rule"])
        .build();
    line.append(&label);
    line.append(&rule);
    let row = gtk::ListBoxRow::builder()
        .child(&line)
        .selectable(false)
        .activatable(false)
        .css_classes(["details-heading-row"])
        .build();
    row.update_property(&[gtk::accessible::Property::Label(&section.heading)]);
    row
}

/// A property's row: its name and its value, read as "Name: value".
fn property_row(name: &str, value: &DetailValue) -> gtk::ListBoxRow {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let name_label = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["details-name"])
        .build();
    line.append(&name_label);
    let text = value.text();
    match value {
        DetailValue::Text(shown) => {
            let label = gtk::Label::builder()
                .label(shown)
                .xalign(0.0)
                .hexpand(true)
                .selectable(true)
                .focusable(false)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["details-value"])
                .build();
            if shown.chars().count() > 32 {
                label.set_tooltip_text(Some(shown));
            }
            follow_value_focus(&label);
            line.append(&label);
        }
        DetailValue::Stars(stars) => {
            let holder = gtk::Box::builder()
                .spacing(2)
                .hexpand(true)
                .css_classes(["details-value", "details-stars"])
                .build();
            for star in 0..5 {
                let glyph = if star < *stars {
                    Icon::StarFilled
                } else {
                    Icon::Star
                };
                let image = icons::image(glyph, STAR_SIZE);
                image.set_accessible_role(gtk::AccessibleRole::Presentation);
                holder.append(&image);
            }
            line.append(&holder);
        }
    }
    let label = ox_core::i18n::format_message("{name}: {value}", &[("name", name), ("value", &text)]);
    let row = gtk::ListBoxRow::builder().child(&line).name(name).build();
    row.update_property(&[gtk::accessible::Property::Label(&label)]);
    row
}

/// Puts the keyboard in the value of `row`, all of it selected, so it can
/// be copied; a Rating has no text to select.
fn edit_value(row: &gtk::ListBoxRow) {
    let Some(label) = row
        .child()
        .and_then(|line| line.last_child())
        .and_downcast::<gtk::Label>()
    else {
        return;
    };
    label.set_focusable(true);
    label.grab_focus();
    label.select_region(0, -1);
}

/// A value takes the keyboard only while it is being selected: Escape
/// gives it back to the list, and so does leaving it.
fn follow_value_focus(label: &gtk::Label) {
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(|controller| {
        // Another window coming to the front leaves the value as it is;
        // only another widget of this window taking the keyboard ends it.
        let label = controller
            .widget()
            .and_downcast::<gtk::Label>()
            .filter(|label| !label.is_focus());
        if let Some(label) = label {
            label.select_region(0, 0);
            label.set_focusable(false);
        }
    });
    label.add_controller(focus);
    let escape = crate::modal::on_escape(|widget| {
        let row = widget.ancestor(gtk::ListBoxRow::static_type());
        match row {
            Some(row) if widget.is_focus() => {
                row.grab_focus();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    label.add_controller(escape);
}

/// The property row named `name`.
fn find_row(rows: &gtk::ListBox, name: &str) -> Option<gtk::ListBoxRow> {
    let mut child = rows.first_child();
    while let Some(widget) = child {
        if let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>() {
            if row.is_selectable() && row.widget_name() == name {
                return Some(row.clone());
            }
        }
        child = widget.next_sibling();
    }
    None
}
