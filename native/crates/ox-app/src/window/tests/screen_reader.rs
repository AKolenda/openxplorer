// SPDX-License-Identifier: AGPL-3.0-only
//! What a screen reader hears, read back through the accessibility bus as
//! Orca reads it ([`AtSpi`]).
//!
//! GTK keeps roles, properties and states that its AT-SPI bridge never
//! sends (GTK 4.14 sends no column-header role, no sort order and no
//! level), so these tests ask the bus, not GTK. Where no accessibility
//! bus runs they return early, unless `OX_REQUIRE_ATSPI=1` makes that a
//! failure.

use gtk::prelude::*;

use crate::properties::PropertiesWindow;
use crate::test_support::atspi::{AtSpi, Heard};
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};

/// The focusable nodes heard with `role` and `name`.
fn focusable(bus: &AtSpi, role: &str, name: &str) -> Vec<Heard> {
    bus.find(role, name)
        .into_iter()
        .filter(|heard| heard.has("focusable"))
        .collect()
}

/// The menu button of the window named `name`.
fn menu_button_named(test: &TestWindow, name: &str) -> gtk::MenuButton {
    descendants::<gtk::MenuButton>(&test.window)
        .into_iter()
        .find(|button| {
            button.is_mapped()
                && button
                    .first_child()
                    .is_some_and(|toggle| toggle.accessible_label_text() == name)
        })
        .unwrap_or_else(|| panic!("{name} is a menu button"))
}

/// The label a widget is named with, for finding a widget by its name.
trait AccessibleLabelText {
    fn accessible_label_text(&self) -> String;
}

impl<T: IsA<gtk::Widget>> AccessibleLabelText for T {
    fn accessible_label_text(&self) -> String {
        let widget = self.upcast_ref::<gtk::Widget>();
        let labels = descendants::<gtk::Label>(widget);
        let shown = labels.first().map(|label| label.text().to_string());
        shown
            .filter(|text| !text.is_empty())
            .or_else(|| widget.tooltip_text().map(|text| text.to_string()))
            .unwrap_or_default()
    }
}

/// The command bar's menus are heard as buttons that open a menu,
/// collapsed until the menu opens and expanded while it is open, as
/// Narrator hears Windows File Explorer's "New", "Sort", "View" and
/// "See more": not as toggle buttons that are "not pressed".
///
/// parity: ACC-001
#[gtk::test]
fn the_command_bar_menus_are_heard_as_menu_buttons_with_their_state() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let Some(bus) = AtSpi::connect(&test.window) else {
        return;
    };
    for name in ["New", "Sort", "View", "See more"] {
        let heard = focusable(&bus, "push button", name);
        assert_eq!(
            heard.len(),
            1,
            "{name} is one focusable push button: {:?}",
            bus.showing()
                .into_iter()
                .filter(|heard| heard.name == name)
                .collect::<Vec<_>>()
        );
        let heard = &heard[0];
        assert!(heard.has("has-popup"), "{name} opens a menu: {heard:?}");
        assert!(
            heard.has("expandable") && !heard.has("expanded"),
            "{name} is collapsed: {heard:?}"
        );
        assert!(!heard.has("pressed"), "{name} is no pressed toggle: {heard:?}");
    }
    let view = menu_button_named(&test, "View");
    view.popup();
    wait_until("the View menu opens", || view.is_active());
    let heard = focusable(&bus, "push button", "View");
    assert!(
        heard.first().is_some_and(|heard| heard.has("expanded")),
        "View is expanded: {:?}",
        bus.everything()
            .into_iter()
            .filter(|heard| heard.has("has-popup") || heard.has("expanded"))
            .collect::<Vec<_>>()
    );
    view.popdown();
    wait_until("the View menu closes", || !view.is_active());
    let heard = focusable(&bus, "push button", "View");
    assert!(
        heard.first().is_some_and(|heard| !heard.has("expanded")),
        "View collapses: {heard:?}"
    );
}

/// Each read-only value of Properties is heard with its name, as Narrator
/// reads "Type of file: Text Document (.txt)": Orca heard only the name
/// twice ("Type of file: Type of file:"), never the value.
///
/// parity: ACC-001, PROP-001
#[gtk::test]
fn properties_values_are_heard_with_their_names() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("properties", None);
    let properties: PropertiesWindow = test.wait_for_properties("Notes 2 Properties");
    let Some(bus) = AtSpi::connect(&properties) else {
        return;
    };
    let values: Vec<Heard> = bus
        .showing()
        .into_iter()
        .filter(|heard| heard.role == "label" && heard.has("focusable"))
        .collect();
    let named = |start: &str| {
        values
            .iter()
            .find(|heard| heard.name.starts_with(start))
            .unwrap_or_else(|| panic!("a value named {start}: {values:#?}"))
    };
    assert_eq!(named("Type of file:").name, "Type of file: Text Document (.txt)");
    assert!(
        named("Size:")
            .name
            .replace('\u{a0}', " ")
            .starts_with("Size: 20 bytes"),
        "{:?}",
        named("Size:")
    );
    assert!(named("Location:").name.len() > "Location: ".len());
    // The caption is in the value's name, so it is not heard again
    // beside it: Orca read the page as "Type of file: Type of file: ...".
    let captions: Vec<Heard> = bus
        .showing()
        .into_iter()
        .filter(|heard| {
            heard.role == "label" && ["Type of file:", "Size:", "Location:"].contains(&heard.name.as_str())
        })
        .collect();
    assert!(captions.is_empty(), "{captions:?}");
    for heard in &values {
        let (name, value) = heard
            .name
            .split_once(": ")
            .unwrap_or_else(|| panic!("{heard:?} names its value"));
        assert!(!value.is_empty() && value != name, "{heard:?}");
    }
    properties.close();
}

/// The status bar's Details view and Large icons buttons are heard as
/// toggle buttons, the one of the view shown pressed: Orca heard two plain
/// buttons and could not tell which view was on.
///
/// parity: ACC-001
#[gtk::test]
fn the_status_bar_view_buttons_are_heard_pressed_for_the_view_shown() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    let Some(bus) = AtSpi::connect(&test.window) else {
        return;
    };
    let pressed = |name: &str| {
        let heard = focusable(&bus, "toggle button", name);
        assert_eq!(heard.len(), 1, "{name} is a toggle button: {heard:?}");
        heard[0].has("pressed")
    };
    assert!(pressed("Details view") && !pressed("Large icons"));
    test.activate("view", Some("large"));
    wait_until("the icon view shows", || pressed("Large icons"));
    assert!(!pressed("Details view"));
}

/// The navigation pane is heard as a tree of tree items, as Narrator reads
/// Windows 11's "Navigation pane tree": This PC and Network expand and
/// collapse, and the drives under This PC are a level down where GTK
/// sends levels (4.16 and later).
///
/// parity: ACC-001, SIDE-033
#[gtk::test]
fn the_navigation_pane_is_heard_as_a_tree() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let Some(bus) = AtSpi::connect(&test.window) else {
        return;
    };
    let heard = bus.showing();
    let tree: Vec<&Heard> = heard
        .iter()
        .filter(|heard| heard.name == "Navigation pane")
        .collect();
    assert_eq!(tree.len(), 1, "{tree:?}");
    assert_eq!(tree[0].role, "tree");
    let items: Vec<&Heard> = heard.iter().filter(|heard| heard.role == "tree item").collect();
    let item = |name: &str| {
        items
            .iter()
            .find(|heard| heard.name == name)
            .copied()
            .unwrap_or_else(|| panic!("{name} is a tree item: {items:#?}"))
    };
    assert!(item("Home").has("focusable") && !item("Home").has("expandable"));
    let this_pc = item("This PC");
    assert!(
        this_pc.has("expandable") && this_pc.has("expanded"),
        "{this_pc:?}"
    );
    assert!(heard
        .iter()
        .all(|heard| heard.role != "list item" || heard.name != "Home"));
    let sends_levels = gtk::minor_version() >= 16;
    if sends_levels {
        assert_eq!(item("Home").attribute("level"), Some("1"));
        assert_eq!(item("This PC").attribute("level"), Some("1"));
        let drive = items
            .iter()
            .find(|heard| heard.description == "/")
            .expect("the root drive is listed");
        assert_eq!(drive.attribute("level"), Some("2"));
    }
}

/// The column titles are heard as buttons that sort, the sorted one
/// saying which way: GTK's column-header role reached Orca as a "filler"
/// and its sort order not at all.
///
/// parity: ACC-001, VIEW-002
#[gtk::test]
fn the_column_titles_are_heard_as_buttons_with_the_sort_order() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    let Some(bus) = AtSpi::connect(&test.window) else {
        return;
    };
    let title = |name: &str| {
        let heard = focusable(&bus, "push button", name);
        assert_eq!(
            heard.len(),
            1,
            "{name} is a button: {:?}",
            bus.find("filler", name)
        );
        heard[0].clone()
    };
    assert!(
        title("Name").description.starts_with("Sorted ascending."),
        "{:?}",
        title("Name")
    );
    assert!(
        !title("Size").description.starts_with("Sorted"),
        "{:?}",
        title("Size")
    );
    test.activate("sort", Some("size"));
    wait_until("Size sorts", || title("Size").description.starts_with("Sorted"));
    assert!(
        !title("Name").description.starts_with("Sorted"),
        "{:?}",
        title("Name")
    );
}

/// A Details row is heard with its columns as they are now: hiding a
/// column, or writing dates another way (as a new day does at midnight
/// with relative dates), redraws the cells without binding the rows
/// again, and the rows' descriptions used to keep the old columns and
/// dates until a row was reused. A group's heading is heard with no
/// file's details.
///
/// parity: ACC-002, VIEW-004
#[gtk::test]
fn a_details_rows_description_follows_its_columns_and_dates() {
    use crate::folder_view::sorting::SortColumn;
    use ox_core::format::DateStyle;
    let fixture = Fixture::standard();
    // Changed now, so its date reads "Today at …" written relatively.
    fixture.write("Fresh notes.txt");
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    let details = test.window.folder_pane().details();
    let description = || {
        details
            .bound_row_descriptions()
            .into_iter()
            .find(|(name, _)| name.as_deref() == Some("Fresh notes.txt"))
            .map(|(_, description)| description)
            .unwrap_or_default()
    };
    wait_until("the row's description", || description().contains("Size: "));

    details
        .column(SortColumn::Size)
        .expect("a Size column")
        .set_visible(false);
    let without_size = description();
    let before = details.date_style();
    let other = match before {
        DateStyle::Relative => DateStyle::Absolute,
        DateStyle::Absolute => DateStyle::Relative,
    };
    details.set_date_style(other);
    let dated_otherwise = description();
    details.set_date_style(before);
    test.activate("group-by", Some("type"));
    wait_until("a heading's row", || {
        details
            .bound_row_descriptions()
            .iter()
            .any(|(name, _)| name.is_none())
    });
    let headings: Vec<String> = details
        .bound_row_descriptions()
        .into_iter()
        .filter(|(name, _)| name.is_none())
        .map(|(_, description)| description)
        .collect();

    assert!(!without_size.contains("Size: "), "{without_size}");
    assert!(without_size.contains("Type: Text Document"), "{without_size}");
    assert_ne!(dated_otherwise, without_size, "dates written the other way");
    assert!(
        headings.iter().all(String::is_empty),
        "headings have no file's details: {headings:?}"
    );
}

/// A row of the Details view is heard with the other columns shown after
/// its name, each after its title, as the row shows them: Orca heard only
/// "Notes 2", never that it is a text document, its size or its date.
///
/// parity: ACC-002
#[gtk::test]
fn a_details_row_is_heard_with_its_type_size_and_date() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    let Some(bus) = AtSpi::connect(&test.window) else {
        return;
    };
    let rows = bus.find("table row", "Notes 2");
    assert_eq!(rows.len(), 1, "{rows:?}");
    // Sizes keep their number and unit together with a no-break space.
    let description = rows[0].description.replace('\u{a0}', " ");
    assert!(description.starts_with("Date modified: "), "{description}");
    assert!(description.contains(", Type: Text Document"), "{description}");
    assert!(description.contains(", Size: 1 KB"), "{description}");
    let folder = bus.find("table row", "Documents");
    assert!(folder[0].description.contains("Type: File folder"), "{folder:?}");
    assert!(
        !folder[0].description.contains("Size:"),
        "a folder's empty Size is left out: {folder:?}"
    );
}
