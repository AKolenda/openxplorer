// SPDX-License-Identifier: AGPL-3.0-only
//! The frame's geometry and controls, measured against the current app.
//!
//! The current app has a 42-pixel title bar whose active tab starts 9
//! pixels in and runs into the navigation row, the "+" right after the
//! last tab, 46-pixel caption buttons, and 34-pixel address and search
//! boxes (see [`super::geometry`] for where the numbers come from).
//!
//! The frame's templates (`resources/ui/`) leave two things to Rust: the
//! window actions of their buttons and their glyphs, which name bundled
//! icons. The last tests prove that no control is left without either.

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::geometry::{bounds, button_for, laid_out, Bounds};
use super::icons::{
    assert_same_colour, css_colour, is_same_colour, painted, pixel_rows, unpremultiplied_rgb, Pixel,
    TRANSITION_TIME,
};
use super::support::{app_menu, menu_button_with_class};
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for, wait_for_frames, Fixture, TestWindow, ThemeGuard};
use crate::window::widget_tree::children;

/// parity: TAB-010, LOOK-009
#[gtk::test]
fn the_active_tab_starts_9_pixels_in_and_reaches_the_bottom_of_the_42_pixel_title_bar() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let title_bar = test
        .window
        .titlebar()
        .expect("the window has the tab strip as its title bar");
    let title = bounds(&test, &title_bar);
    assert_eq!(
        (title.y, title.height),
        (0, 42),
        "the title bar is 42 pixels tall"
    );
    let tabs = test.window.tab_strip().tab_list();
    let first_tab = tabs.first_child().expect("one tab");
    assert_eq!(
        bounds(&test, &first_tab),
        Bounds::new(9, 7, 215, 35),
        "215 x 35 at (9, 7)"
    );
    assert!(first_tab.has_css_class("active"));
}

/// parity: LOOK-009
#[gtk::test]
fn the_new_tab_button_follows_the_last_tab() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    wait_for_frames(&test.window, 3);
    let tabs = test.window.tab_strip().tab_list();
    let last_tab = tabs.last_child().expect("two tabs");
    let tab = bounds(&test, &last_tab);
    assert_eq!(tab.x, 9 + 215 + 2, "tabs are 2 pixels apart");
    let plus = bounds(&test, &button_for(&test, "win.new-tab"));
    assert_eq!(plus.x, tab.right() + 5, "5 pixels after the last tab");
    assert_eq!((plus.y, plus.width, plus.height), (8, 38, 34));
}

/// parity: LOOK-009
#[gtk::test]
fn the_caption_buttons_are_46_pixels_wide_and_as_tall_as_the_title_bar() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let captions: Vec<gtk::Button> = descendants::<gtk::Button>(&test.window)
        .into_iter()
        .filter(|button| button.has_css_class("caption"))
        .collect();
    assert!(!captions.is_empty(), "the desktop's layout shows caption buttons");
    let window_width = test.window.width();
    for caption in &captions {
        let place = bounds(&test, caption);
        assert_eq!(
            (place.y, place.width, place.height),
            (0, 46, 42),
            "{:?}",
            caption.tooltip_text()
        );
    }
    let close = captions.iter().find(|button| button.has_css_class("close"));
    if let Some(close) = close {
        let place = bounds(&test, close);
        assert_eq!(place.right(), window_width, "Close sits in the corner");
    }
}

/// As in Windows 11's File Explorer, a separator stands in the gap between
/// two inactive tabs but not beside the tab in front or a hovered tab, the
/// tab in front curves out at its foot into the navigation row, and only
/// the tab in front and a hovered tab show their close button. Measured
/// in the dark appearance, whose strip, bars and separator differ most.
///
/// parity: LOOK-009
#[gtk::test]
fn tabs_are_separated_flared_and_show_close_as_in_windows_11() {
    /// The colour painted at (`x`, `y`) of `rows`.
    fn colour_at(rows: &[Vec<Pixel>], x: i32, y: i32) -> gdk::RGBA {
        let row = usize::try_from(y).expect("a point in the title bar");
        let column = usize::try_from(x).expect("a point in the title bar");
        let [red, green, blue] = unpremultiplied_rgb(rows[row][column]);
        gdk::RGBA::new(red, green, blue, 1.0)
    }
    /// Whether the close button at `close` shows its glyph: some pixel
    /// differs from the button's corner.
    fn shows_glyph(rows: &[Vec<Pixel>], close: Bounds) -> bool {
        let corner = colour_at(rows, close.x + 1, close.y + 1);
        (close.y..close.y + close.height)
            .any(|y| (close.x..close.right()).any(|x| !is_same_colour(colour_at(rows, x, y), corner)))
    }
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    for _ in 0..2 {
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("valid folder");
        test.wait_for_listing("another tab");
    }
    test.activate("theme", Some("dark"));
    wait_for(TRANSITION_TIME);
    wait_for_frames(&test.window, 3);
    let bar = test.window.titlebar().expect("the window has its title bar");
    let tabs: Vec<gtk::Widget> = children(&test.window.tab_strip().tab_list()).collect();
    let place =
        |widget: &gtk::Widget| Bounds::from_rect(&widget.compute_bounds(&bar).expect("a laid out tab"));
    let [first, second, front] = [&tabs[0], &tabs[1], &tabs[2]];
    assert!(front.has_css_class("active"), "the tab opened last is in front");
    let (first_place, second_place, front_place) = (place(first), place(second), place(front));
    let close_of = |tab: &gtk::Widget| {
        let close = descendants::<gtk::Button>(tab)
            .into_iter()
            .find(|button| button.has_css_class("tab-close"))
            .expect("a tab has a close button");
        Bounds::from_rect(&close.compute_bounds(&bar).expect("a laid out button"))
    };
    let middle = first_place.y + first_place.height / 2;
    let bottom = front_place.y + front_place.height;
    let strip = css_colour("#202020");
    let gap_colours = |rows: &[Vec<Pixel>], left: Bounds, right: Bounds| -> Vec<gdk::RGBA> {
        (left.right()..right.x)
            .map(|x| colour_at(rows, x, middle))
            .collect()
    };

    let rows = pixel_rows(&painted(&bar));
    let between_inactive = gap_colours(&rows, first_place, second_place);
    let beside_front = gap_colours(&rows, second_place, front_place);
    let start_flare = colour_at(&rows, front_place.x - 1, bottom - 1);
    let end_flare = colour_at(&rows, front_place.right(), bottom - 1);
    let past_start_flare = colour_at(&rows, front_place.x - 4, bottom - 4);
    let front_close = shows_glyph(&rows, close_of(front));
    let inactive_close = shows_glyph(&rows, close_of(second));
    second.set_state_flags(gtk::StateFlags::PRELIGHT, false);
    wait_for(TRANSITION_TIME);
    wait_for_frames(&test.window, 2);
    let hovered_rows = pixel_rows(&painted(&bar));
    second.unset_state_flags(gtk::StateFlags::PRELIGHT);
    let beside_hovered = gap_colours(&hovered_rows, first_place, second_place);
    let hovered_close = shows_glyph(&hovered_rows, close_of(second));

    // alpha(white, .08) over #202020.
    let separator = css_colour("#323232");
    assert!(
        between_inactive
            .iter()
            .any(|&colour| is_same_colour(colour, separator)),
        "a separator between two inactive tabs: {between_inactive:?}"
    );
    for (gap, what) in [
        (&beside_front, "the tab in front"),
        (&beside_hovered, "a hovered tab"),
    ] {
        assert!(
            gap.iter().all(|&colour| is_same_colour(colour, strip)),
            "no separator beside {what}: {gap:?}"
        );
    }
    let chrome = css_colour("#282828");
    assert_same_colour(start_flare, chrome, "the curve at the start of the tab in front");
    assert_same_colour(end_flare, chrome, "the curve at its end");
    assert_same_colour(past_start_flare, strip, "the strip above the curve");
    assert!(front_close, "the tab in front shows its close button");
    assert!(!inactive_close, "an inactive tab does not");
    assert!(hovered_close, "a hovered tab does");
}

/// parity: LOOK-012
#[gtk::test]
fn the_address_and_search_boxes_are_34_pixels_tall_7_pixels_below_the_title_bar() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let address = bounds(&test, test.window.address_bar());
    assert_eq!((address.x, address.y, address.height), (178, 49, 34));
    let search = bounds(&test, test.window.search_box());
    assert_eq!((search.y, search.width, search.height), (49, 235, 34));
    assert_eq!(
        search.right(),
        test.window.width() - 16,
        "the search box ends 16 pixels from the edge"
    );
}

/// A tab's close button is 22 pixels square, and Close turns red with a
/// white glyph under the pointer, as in Windows.
///
/// parity: LOOK-009
#[gtk::test]
fn the_tab_close_is_22_pixels_and_close_hovers_red() {
    /// Puts the display's decoration layout back, also when a check fails.
    struct LayoutGuard(gtk::Settings, Option<glib::GString>);
    impl Drop for LayoutGuard {
        fn drop(&mut self) {
            self.0.set_gtk_decoration_layout(self.1.as_deref());
        }
    }
    let settings = gtk::Settings::default().expect("GTK tests run on a private display");
    let _layout = LayoutGuard(settings.clone(), settings.gtk_decoration_layout());
    settings.set_gtk_decoration_layout(Some(":minimize,maximize,close"));
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let buttons = descendants::<gtk::Button>(&test.window);
    let tab_close = buttons
        .iter()
        .find(|button| button.has_css_class("tab-close"))
        .expect("the tab has a close button");
    let place = bounds(&test, tab_close);
    assert_eq!((place.width, place.height), (22, 22));
    let close = buttons
        .iter()
        .find(|button| button.has_css_class("caption") && button.has_css_class("close"))
        .expect("the layout shows Close");
    close.set_state_flags(gtk::StateFlags::PRELIGHT, false);
    wait_for(TRANSITION_TIME);
    wait_for_frames(&test.window, 2);
    let rows = pixel_rows(&painted(close));
    let [red, green, blue] = unpremultiplied_rgb(rows[3][3]);
    let background = gdk::RGBA::new(red, green, blue, 1.0);
    assert_same_colour(background, css_colour("#c42b1c"), "Close under the pointer");
    assert_same_colour(close.color(), css_colour("#ffffff"), "its glyph");
    close.unset_state_flags(gtk::StateFlags::PRELIGHT);
}

#[gtk::test]
fn the_search_box_names_the_folder_it_searches() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri_of("Documents"));
    let placeholder = test.window.search_box().entry().placeholder_text();
    assert_eq!(placeholder.as_deref(), Some("Search Documents"));
}

/// Windows 11's title bar has no open-windows button, so neither has
/// this one: the tab menu's "Open windows…" lists every window below the
/// tab in front, then New window and Quit.
///
/// parity: TAB-044
#[gtk::test]
fn the_open_windows_menu_lists_every_window_then_new_window_and_quit() {
    let fixture = Fixture::standard();
    let first = laid_out(&fixture.uri());
    let second = first.open_beside(&fixture.uri_of("Documents"));
    let bar = first.window.titlebar().expect("the window has its title bar");
    assert!(
        descendants::<gtk::MenuButton>(&bar).is_empty(),
        "the title bar has no open-windows button"
    );
    first.activate("open-windows", None);
    wait_for_frames(&first.window, 2);
    let strip = first.window.tab_strip();
    let menu = strip.windows_menu();
    let shown = menu.is_visible();
    let (has_target, pointing_to) = menu.pointing_to();
    let labels = menu.row_labels();
    let checked = menu.checked_labels();
    menu.popdown();
    let front = strip
        .tab_list()
        .first_child()
        .and_then(|tab| tab.compute_bounds(strip))
        .expect("a laid out tab");
    assert!(shown, "the menu opens");
    #[expect(clippy::cast_possible_truncation, reason = "tab bounds are small")]
    let below_front = gdk::Rectangle::new(
        front.x() as i32,
        front.y() as i32,
        front.width() as i32,
        front.height() as i32,
    );
    assert!(has_target);
    assert_eq!(pointing_to, below_front, "it opens from the tab in front");
    let title_of = |test: &TestWindow| test.window.title().map(|title| title.to_string());
    let first_title = title_of(&first).expect("a titled window");
    let second_title = title_of(&second).expect("a titled window");
    assert!(
        labels.contains(&first_title) && labels.contains(&second_title),
        "{labels:?}"
    );
    assert_eq!(
        &labels[labels.len() - 3..],
        ["-", "New window", "Quit OpenXplorer"]
    );
    assert_eq!(checked, [first_title], "this window is checked");
    second.window.close();
}

/// Every More menu command works in a folder, and a row shows no tooltip
/// that would only repeat its name, as Windows 11's menus show none.
/// License & source is in Settings > About now (CMD-007).
///
/// parity: CMD-031
#[gtk::test]
fn the_more_menu_items_work_and_show_no_tooltip_repeating_their_name() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let more_button = menu_button_with_class(&test, "more-command");
    let more = app_menu(&more_button);
    more_button.popup();
    for label in ["Pin to Quick access", "Select all", "Options"] {
        let row = more.row(label);
        assert!(row.is_sensitive(), "{label}");
        assert_eq!(row.tooltip_text(), None, "{label}");
    }
    more_button.popdown();
}

/// A row whose label the menu cuts short shows its whole label as a
/// tooltip, which then adds what the row cannot show; a row shown whole
/// shows none.
///
/// parity: CMD-005
#[gtk::test]
fn a_menu_row_cut_short_shows_its_whole_label_as_a_tooltip() {
    use crate::icons::Icon;
    use crate::window::menu_popover::{row_tooltip, MenuItem};
    use crate::window::window_action::WindowAction;

    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let more_button = menu_button_with_class(&test, "more-command");
    let more = app_menu(&more_button);
    let long = format!("{}final", "Quarterly planning notes ".repeat(30));
    more.set_entries(vec![
        MenuItem::new(&long, Icon::Document, WindowAction::SelectAll).into(),
        MenuItem::new("Select all", Icon::Document, WindowAction::SelectAll).into(),
    ]);
    more_button.popup();
    wait_for_frames(&test.window, 3);
    assert_eq!(row_tooltip(&more.row(&long)), Some(long.clone()));
    assert_eq!(row_tooltip(&more.row("Select all")), None);
    more_button.popdown();
}

/// A new window on a landing page has no file list to focus; GTK's first
/// focusable widget was a crumb, which drew the address bar's editing
/// line.
#[gtk::test]
fn a_new_window_on_a_landing_page_leaves_the_address_bar_alone() {
    let test = laid_out(Page::ThisPc.uri());
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window);
    let address = test.window.address_bar();
    let in_address = focus.is_some_and(|widget| widget.is_ancestor(address));
    assert!(!in_address, "the address bar does not hold focus");
}

#[gtk::test]
fn a_new_window_types_into_its_file_list() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window).expect("a focused widget");
    let view = test.window.folder_pane().view_widget();
    assert!(
        focus.is_ancestor(&view) || focus == view,
        "focus is in the file list, not {focus:?}"
    );
}

/// Every window action a control of the frame names, its menus included,
/// without the `win.` prefix.
fn window_actions_named(test: &TestWindow) -> Vec<String> {
    let widgets = descendants::<gtk::Widget>(&test.window);
    let controls = widgets
        .iter()
        .filter_map(|widget| widget.dynamic_cast_ref::<gtk::Actionable>());
    let detailed_names = controls.filter_map(ActionableExt::action_name);
    detailed_names
        .filter_map(|name| name.strip_prefix("win.").map(str::to_owned))
        .collect()
}

#[gtk::test]
fn every_control_of_the_frame_runs_an_action_the_window_has() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let named = window_actions_named(&test);
    let from_templates = [
        "new-tab",
        "address-history",
        "details-pane",
        "view",
        "check-updates",
        "settings",
    ];
    for action in from_templates {
        assert!(
            named.iter().any(|name| name == action),
            "a control runs win.{action}"
        );
    }
    let missing: Vec<&String> = named
        .iter()
        .filter(|name| test.window.lookup_action(name).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "controls name actions the window lacks: {missing:?}"
    );
}

/// Every image the frame can show is a bundled icon, GTK's own included:
/// the search box's clear button shows the bundled close glyph, not the
/// desktop theme's. The frame shows no theme images at all.
///
/// parity: LOOK-015
#[gtk::test]
fn every_image_of_the_frame_shows_a_bundled_icon() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let images: Vec<gtk::Image> = descendants::<gtk::Image>(&test.window)
        .into_iter()
        .filter(|image| image.is_visible() && image.storage_type() != gtk::ImageType::Empty)
        .collect();
    let glyph_count = images.iter().filter(|image| image.has_css_class("glyph")).count();
    assert!(glyph_count > 0, "the frame shows glyphs");
    let theme = gtk::IconTheme::for_display(&WidgetExt::display(&test.window));
    let missing: Vec<String> = images
        .iter()
        .map(|image| image.icon_name().map(String::from).unwrap_or_default())
        .filter(|name| !(name.starts_with("ox-") && theme.has_icon(name)))
        .collect();
    assert!(missing.is_empty(), "images without a bundled icon: {missing:?}");
    let clear = test
        .window
        .search_box()
        .entry()
        .last_child()
        .and_downcast::<gtk::Image>();
    let clear_icon = clear.and_then(|image| image.icon_name());
    assert_eq!(clear_icon.as_deref(), Some("ox-dismiss-16-symbolic"));
}
