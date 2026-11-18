// SPDX-License-Identifier: AGPL-3.0-only
//! Tabs: opening in front or behind, closing, switching and their history.

use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::support::click_at;
use crate::folder_view::item::FileItem;
use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow, STANDARD_NAMES};
use crate::window::address_bar::AddressMode;
use crate::window::session::{TabId, TabPlacement};
use ox_core::network::NetworkError;

fn tab_ids(test: &TestWindow) -> Vec<TabId> {
    let session = test.window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.id).collect()
}

fn is_listed(test: &TestWindow, id: TabId) -> bool {
    let session = test.window.imp().session.borrow();
    session.tab(id).is_some_and(|tab| tab.listing_state.is_listed())
}

fn open_three_tabs(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    for uri in [fixture.uri_of("Documents"), fixture.uri()] {
        test.window.add_tab(&uri).expect("valid folder");
        test.wait_for_listing("the new tab");
    }
    test
}

/// parity: TAB-002
#[gtk::test]
fn closing_the_active_tab_shows_the_tab_to_its_right() {
    let fixture = Fixture::standard();
    let test = open_three_tabs(&fixture);
    let [_, middle, right] = tab_ids(&test)[..] else {
        panic!("three tabs are open");
    };
    test.activate_tab(middle);
    test.activate("close-tab", None);
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.active_tab(), Some(right));
}

/// parity: TAB-005
#[gtk::test]
fn ctrl_tab_cycles_through_the_tabs_and_wraps() {
    let fixture = Fixture::standard();
    let test = open_three_tabs(&fixture);
    let [left, middle, _] = tab_ids(&test)[..] else {
        panic!("three tabs are open");
    };
    test.activate("next-tab", None);
    assert_eq!(test.active_tab(), Some(left));
    test.activate("next-tab", None);
    assert_eq!(test.active_tab(), Some(middle));
    test.activate("previous-tab", None);
    test.activate("previous-tab", None);
    assert_eq!(test.active_tab(), Some(tab_ids(&test)[2]));
}

/// parity: TAB-009
#[gtk::test]
fn a_background_tab_leaves_the_current_tab_alone_and_lists_when_first_shown() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_model().select_only(1);
    test.activate("open-tab-background", Some(&fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "the source tab stays in front"
    );
    assert_eq!(
        test.selected_names(),
        ["Notes 2.txt"],
        "the source tab keeps its selection"
    );
    let background = tab_ids(&test)[1];
    wait_for(std::time::Duration::from_millis(50));
    assert!(
        !is_listed(&test, background),
        "a background tab is listed only when shown"
    );
    test.activate_tab(background);
    test.wait_for_listing("the background tab");
    assert!(is_listed(&test, background));
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// parity: TAB-021
#[gtk::test]
fn shift_middle_click_opens_the_tab_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("open-tab", Some(&fixture.uri_of("Documents")));
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// A tab keeps its own scroll position and selection; showing it resets
/// type-to-select and the search box.
///
/// parity: TAB-008, TAB-057
#[gtk::test]
fn switching_back_to_a_tab_restores_its_scroll_position() {
    let fixture = Fixture::with_files(300);
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    let last = pane.model().n_items() - 1;
    pane.reveal(last);
    wait_until("the view to scroll", || pane.scroll_position() > 0.0);
    wait_for(std::time::Duration::from_millis(100));
    let scrolled = pane.scroll_position();
    let first = tab_ids(&test)[0];
    test.window.add_tab(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the second tab");
    assert!(pane.scroll_position() < 1.0, "a new tab starts at the top");
    test.window.search_box().entry().set_text("file");
    test.window.folder_model().select_only(0);
    let second_selection = test.selected_names();
    pane.reveal(1);
    wait_until("the second tab's focused item", || {
        pane.focused_position() == Some(1)
    });
    test.activate_tab(first);
    wait_until("the first tab's scroll position", || {
        (pane.scroll_position() - scrolled).abs() < 1.0
    });
    wait_until("the first tab's focused item", || {
        pane.focused_position() == Some(last)
    });
    assert_eq!(test.window.search_box().entry().text().as_str(), "");
    assert!(
        test.selected_names().is_empty(),
        "the first tab had nothing selected"
    );
    test.activate("next-tab", None);
    assert_eq!(
        test.selected_names(),
        second_selection,
        "each tab keeps its selection"
    );
    wait_until("the second tab's focused item again", || {
        pane.focused_position() == Some(1)
    });
}

/// The tab widget at `index` of the strip.
fn tab_widget(test: &TestWindow, index: usize) -> gtk::Widget {
    crate::window::widget_tree::children(&test.window.tab_strip().tab_list())
        .nth(index)
        .expect("the window shows the tab")
}

/// A click or Enter on a tab shows it, and ends editing the address.
///
/// parity: TAB-004
#[gtk::test]
fn a_click_or_enter_on_a_tab_shows_it() {
    let fixture = Fixture::standard();
    let test = open_three_tabs(&fixture);
    let [left, middle, _] = tab_ids(&test)[..] else {
        panic!("three tabs are open");
    };
    test.activate("location", None);
    assert_eq!(test.window.address_bar().mode(), AddressMode::Entry);

    click_at(&tab_widget(&test, 0), gtk::gdk::BUTTON_PRIMARY, (4.0, 4.0));
    let after_click = test.active_tab();
    let address_mode = test.window.address_bar().mode();
    let middle_tab = tab_widget(&test, 1);
    let keys = middle_tab
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("a tab takes keys");
    let handled = keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Return.into_glib(),
            &0_u32,
            &gtk::gdk::ModifierType::empty(),
        ],
    );

    assert_eq!(after_click, Some(left));
    assert_eq!(
        address_mode,
        AddressMode::Crumbs,
        "switching ends address editing"
    );
    assert!(handled);
    assert_eq!(test.active_tab(), Some(middle));
}

/// parity: NAV-001, NAV-005, NAV-010
#[gtk::test]
fn back_forward_and_up_walk_the_tab_history() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert!(!test.window.is_action_enabled("back"));
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the subfolder");
    test.activate("back", None);
    test.wait_for_listing("the folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(test.window.is_action_enabled("forward"));
    test.activate("forward", None);
    test.wait_for_listing("the subfolder again");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.activate("up", None);
    test.wait_for_listing("the parent folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert_eq!(test.names(), STANDARD_NAMES);
}

#[gtk::test]
fn closing_a_background_tab_keeps_the_active_tab_and_its_filter() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    test.activate("previous-tab", None);
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    test.window.search_box().entry().set_text("Notes 2");
    wait_until("the active filter", || test.names() == ["Notes 2.txt"]);
    let background = tab_ids(&test)[1];
    test.activate_tab_close(background);
    assert_eq!(test.window.tab_count(), 1);
    assert_eq!(test.window.search_box().entry().text().as_str(), "Notes 2");
    assert_eq!(test.names(), ["Notes 2.txt"]);
}

/// parity: TAB-010
#[gtk::test]
fn tabs_are_announced_as_tabs_with_their_selected_state() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");
    let tab_list = test.window.tab_strip().tab_list();
    assert_eq!(tab_list.accessible_role(), gtk::AccessibleRole::TabList);
    assert!(gtk::test_accessible_has_property(
        &tab_list,
        gtk::AccessibleProperty::Label
    ));
    let tabs: Vec<gtk::Box> = descendants::<gtk::Box>(&tab_list)
        .into_iter()
        .filter(|button| button.accessible_role() == gtk::AccessibleRole::Tab)
        .collect();
    assert_eq!(tabs.len(), 2);
    let titles: Vec<Option<String>> = tabs
        .iter()
        .map(|tab| tab.tooltip_text().map(|text| text.to_string()))
        .collect();
    let folder_address = fixture.root().display().to_string();
    assert_eq!(titles[0].as_deref(), Some(folder_address.as_str()));
    for tab in &tabs {
        assert!(gtk::test_accessible_has_state(
            tab,
            gtk::AccessibleState::Selected
        ));
    }
}

/// parity: TAB-001
#[gtk::test]
fn the_new_tab_button_opens_the_home_folder_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let button = &test.window.imp().new_tab_button;
    assert_eq!(button.tooltip_text().as_deref(), Some("New tab (Ctrl+T)"));
    button.emit_clicked();
    test.wait_for_listing("the home folder");
    let home = ox_core::location::file_uri(&gtk::glib::home_dir());
    assert_eq!(test.window.current_uri(), Some(home));
    assert_eq!(test.window.tab_count(), 2);
}

/// Ctrl+Tab stays out of text fields, while Ctrl+T works from the search
/// box, as in Windows 11 File Explorer, and Properties, a window of its
/// own, keeps neither from the browser window.
///
/// parity: TAB-005
#[gtk::test]
fn ctrl_tab_stays_out_of_text_fields_ctrl_t_works_from_search_and_properties_keep_none() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().focus_view();
    assert!(test.window.tab_keys_apply());
    assert!(test.window.new_tab_key_applies());

    test.window.search_box().entry().grab_focus();
    let in_search = (test.window.tab_keys_apply(), test.window.new_tab_key_applies());
    test.window.folder_pane().focus_view();
    test.activate("properties", None);
    test.wait_for_properties("Example projects Properties");
    test.window.folder_pane().focus_view();

    assert_eq!(
        in_search,
        (false, true),
        "the search box keeps Ctrl+Tab, and Ctrl+T opens a tab from it"
    );
    assert!(
        test.window.tab_keys_apply() && test.window.new_tab_key_applies(),
        "Properties is a window of its own and keeps no key from this one"
    );
}

/// The empty part of the title bar between the "+" and the window
/// buttons, which moves the window.
fn drag_area(test: &TestWindow) -> gtk::Widget {
    descendants::<gtk::Widget>(&test.window.titlebar().expect("the title bar"))
        .into_iter()
        .find(|widget| widget.has_css_class("title-drag"))
        .expect("the drag area")
}

/// As Windows 11 lays out its title bar, the tab strip with its "+" grows
/// up to the window buttons, leaving only the drag area (48 pixels) before
/// them: tabs shrink toward their narrowest to fit all of that room, and
/// the strip scrolls with its arrows only once every tab is at its
/// narrowest and they still do not fit. Checked tab by tab up to 40 tabs,
/// at the test display's full 1440 pixels and at 1000. (Windows 11, build
/// 26200, leaves about 40 to 50 pixels at 1x.)
///
/// parity: TAB-019
#[gtk::test]
fn tabs_fill_the_title_bar_up_to_the_drag_area_before_they_scroll() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for width in [1440, 1000] {
        super::narrow_windows::resize(&test, width);
        have_tabs(&test, &fixture, 1);
        let mut overflowed = false;
        for count in 2..=40 {
            have_tabs(&test, &fixture, count);
            let (back, _) = scroll_arrows(&test);
            let tab = tab_widget(&test, 0);
            let narrowest = tab.measure(gtk::Orientation::Horizontal, -1).0.max(100);
            let drag = drag_area(&test).width();
            if back.is_visible() {
                overflowed = true;
                assert!(
                    tab.width() <= narrowest + 1,
                    "{count} tabs at {width} pixels: arrows only once the tabs are at their \
                     narrowest ({} for {narrowest})",
                    tab.width()
                );
                assert!(
                    drag <= 60,
                    "{count} tabs at {width} pixels: the strip reaches the window buttons but \
                     for the drag area, which is {drag} pixels"
                );
            }
            assert!(drag >= 48, "the window can still be grabbed: {drag} pixels");
        }
        assert!(overflowed, "40 tabs overflow at {width} pixels");
    }
}

/// A change to a background tab's folder lists it again behind, so it is
/// current when it comes back, with its selection kept.
///
/// parity: TAB-056
#[gtk::test]
fn a_background_tab_shows_changes_to_its_folder_and_keeps_its_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.select_named("Notes 2.txt");
    let first = tab_ids(&test)[0];
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the second tab");

    std::fs::write(fixture.root().join("Added later.txt"), b"new").expect("a new file");
    wait_until("the background tab to list the new file", || {
        let session = test.window.imp().session.borrow();
        let tab = session.tab(first).expect("the first tab is open");
        let mut items = tab.store.iter::<FileItem>().filter_map(Result::ok);
        items.any(|item| item.entry().name == "Added later.txt")
    });
    assert_ne!(test.active_tab(), Some(first), "it changed behind");
    test.activate_tab(first);

    assert!(test.names().iter().any(|name| name == "Added later.txt"));
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
}

/// A change to a network folder in a background tab only marks the tab;
/// it is listed again when shown, so no sign-in appears behind.
///
/// parity: TAB-056
#[gtk::test]
fn a_background_network_tab_is_listed_again_only_when_shown() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .network()
        .answer_mounts_with(|| Err(NetworkError::NotAFolder));
    test.window
        .open_tab("smb://example.invalid/share", TabPlacement::Background)
        .expect("an SMB share");
    let share = tab_ids(&test)[1];
    let tab_state = |test: &TestWindow| {
        let session = test.window.imp().session.borrow();
        let tab = session.tab(share).expect("the share's tab is open");
        (tab.changed_while_hidden, tab.listing_state.is_listing())
    };
    {
        // As if the share had been listed when it was last shown.
        let mut session = test.window.imp().session.borrow_mut();
        let tab = session.tab_mut(share).expect("the share's tab is open");
        tab.listing_state.begin();
        tab.listing_state.finish();
    }

    test.window.folder_changed(share);
    let behind = tab_state(&test);
    test.activate_tab(share);

    assert_eq!(behind, (true, false), "marked, not listed");
    assert_eq!(tab_state(&test), (false, true), "listed once shown");
}

/// The buttons at the two ends of the tab strip that scroll it, as
/// Windows 11 shows them once the tabs overflow: the one going back, then
/// the one going forward.
fn scroll_arrows(test: &TestWindow) -> (gtk::Button, gtk::Button) {
    let arrows: Vec<gtk::Button> = descendants::<gtk::Button>(test.window.tab_strip())
        .into_iter()
        .filter(|button| button.has_css_class("tab-scroll"))
        .collect();
    let [back, forward] = &arrows[..] else {
        panic!("the strip has its two scroll arrows: {arrows:?}");
    };
    (back.clone(), forward.clone())
}

/// The strip's sideways scroll position.
fn strip_scroll(test: &TestWindow) -> gtk::Adjustment {
    descendants::<gtk::ScrolledWindow>(test.window.tab_strip())
        .remove(0)
        .hadjustment()
}

/// Whether `tab` lies wholly inside the part of the strip shown.
fn is_in_view(test: &TestWindow, tab: &gtk::Widget) -> bool {
    let scroller = descendants::<gtk::ScrolledWindow>(test.window.tab_strip()).remove(0);
    let Some(bounds) = tab.compute_bounds(&scroller) else {
        return false;
    };
    f64::from(bounds.x()) >= -0.5
        && f64::from(bounds.x() + bounds.width()) <= f64::from(scroller.width()) + 0.5
}

/// Opens tabs until `count` are open, or closes the last ones.
fn have_tabs(test: &TestWindow, fixture: &Fixture, count: usize) {
    while tab_ids(test).len() < count {
        test.window.add_tab(&fixture.uri()).expect("valid folder");
    }
    while tab_ids(test).len() > count {
        let last = *tab_ids(test).last().expect("a tab");
        test.window.close_tab(last);
    }
    crate::test_support::harness::wait_for_frames(&test.window, 4);
}

/// Many tabs, as Windows 11 (build 26200) shows 25 in a 1920-pixel
/// window: each shrinks to a minimum that still shows its icon, about
/// three letters of its title and its close button; past that, the strip
/// scrolls, with an arrow at each end that scrolls it, shown only while
/// the tabs overflow, and the "+" still after the strip. The wheel still
/// scrolls it, and a tab shown with Ctrl+Tab comes into view. Closing
/// tabs until they fit hides the arrows again. At 1366 pixels and at the
/// test display's full 1440, with 5, 20 and 40 tabs.
///
/// parity: TAB-019
#[gtk::test]
fn overflowing_tabs_keep_a_few_letters_and_scroll_with_arrows_at_both_ends() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for width in [1366, 1440] {
        super::narrow_windows::resize(&test, width);
        for count in [5, 20, 40] {
            have_tabs(&test, &fixture, count);
            let adjustment = strip_scroll(&test);
            let overflows = adjustment.upper() > adjustment.page_size() + 1.0;
            let (back, forward) = scroll_arrows(&test);
            assert_eq!(
                back.is_visible() && forward.is_visible(),
                overflows,
                "{count} tabs at {width} pixels: arrows only when the tabs overflow"
            );
            assert_eq!(overflows, count >= 20, "{count} tabs at {width} pixels");
            let new_tab = descendants::<gtk::Button>(&test.window)
                .into_iter()
                .find(|button| button.has_css_class("newtab"))
                .expect("the new-tab button");
            let plus = new_tab.compute_bounds(&test.window).expect("a shown + button");
            assert!(
                f64::from(plus.x() + plus.width()) <= f64::from(test.window.width()),
                "the + stays in the window"
            );
            // Each tab shows its icon, about three letters and its close
            // button.
            let tab = tab_widget(&test, 0);
            let title = descendants::<gtk::Label>(&tab)
                .into_iter()
                .find(|label| label.has_css_class("tab-title"))
                .expect("a tab title");
            let three: String = title.text().chars().take(3).collect();
            let letters = title.create_pango_layout(Some(&three)).pixel_size().0;
            let room = title.parent().expect("the title's room").width();
            assert!(
                room >= letters,
                "{count} tabs at {width} pixels: {room} pixels for a title needing {letters}"
            );
        }
    }

    // 40 tabs at 1440 pixels: the tab opened last is in front and in view.
    let (back, forward) = scroll_arrows(&test);
    let adjustment = strip_scroll(&test);
    let last = tab_widget(&test, 39);
    wait_until("the new tab in view", || is_in_view(&test, &last));
    assert!(
        !back.is_focusable() && !forward.is_focusable(),
        "Tab skips the arrows"
    );
    let end = adjustment.value();
    back.emit_clicked();
    wait_until("the strip to scroll back", || adjustment.value() < end - 1.0);
    let after_back = adjustment.value();
    forward.emit_clicked();
    wait_until("the strip to scroll forward", || {
        adjustment.value() > after_back + 1.0
    });
    // Ctrl+Tab from the last tab wraps to the first, off at the far end.
    test.activate("next-tab", None);
    let first = tab_widget(&test, 0);
    wait_until("the first tab in view", || is_in_view(&test, &first));

    have_tabs(&test, &fixture, 5);
    wait_until("the arrows to go", || {
        let (back, forward) = scroll_arrows(&test);
        !back.is_visible() && !forward.is_visible()
    });
}

/// The bounds of `widget` in `tab`, left edge and right edge.
fn span_in(widget: &impl IsA<gtk::Widget>, tab: &gtk::Widget) -> (f64, f64) {
    let bounds = widget.compute_bounds(tab).expect("laid out in its tab");
    (f64::from(bounds.x()), f64::from(bounds.x() + bounds.width()))
}

/// The room `title` is shown in, which cuts it at the last pixel.
fn room_of(title: &gtk::Label) -> gtk::Widget {
    title.parent().expect("the title's room")
}

/// The icon, title and close button of `tab`.
fn tab_parts(tab: &gtk::Widget) -> (gtk::Widget, gtk::Label, gtk::Button) {
    let icon = tab.first_child().expect("an icon");
    let title = descendants::<gtk::Label>(tab)
        .into_iter()
        .find(|label| label.has_css_class("tab-title"))
        .expect("a title");
    let close = descendants::<gtk::Button>(tab)
        .into_iter()
        .find(|button| button.has_css_class("tab-close"))
        .expect("a close button");
    (icon, title, close)
}

/// A tab at its narrowest gives its title all the room after its icon,
/// as Windows 11 does: up to its close button on the tab in front or a
/// hovered tab, and up to the tab's end on the others, whose close button
/// takes no room then, with no spacing before the close button but the
/// few pixels around its glyph. The title is cut at the last pixel only
/// where it runs out of that room, so a wider tab shows more of it, and
/// "Home" fits whole in a tab in front at the narrowest.
///
/// parity: TAB-019
#[gtk::test]
fn a_tab_at_its_narrowest_gives_its_title_all_the_room_after_its_icon() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    super::narrow_windows::resize(&test, 1366);
    have_tabs(&test, &fixture, 5);
    let wide_title = room_of(&tab_parts(&tab_widget(&test, 0)).1).width();
    have_tabs(&test, &fixture, 40);
    let adjustment = strip_scroll(&test);
    assert!(
        adjustment.upper() > adjustment.page_size() + 1.0,
        "40 tabs overflow"
    );

    // A tab behind, at the minimum width: the title runs from after the
    // icon to the tab's end, less its padding.
    let behind = tab_widget(&test, 0);
    let (icon, title, close) = tab_parts(&behind);
    assert!(!close.is_visible(), "a tab behind shows no close button");
    let (_, icon_end) = span_in(&icon, &behind);
    let (title_start, title_end) = span_in(&room_of(&title), &behind);
    let tab_end = f64::from(behind.width());
    assert!(title_start - icon_end <= 10.5, "the title starts after the icon");
    assert!(
        tab_end - title_end <= 10.5,
        "the title reaches the tab's end: it ends at {title_end} of {tab_end}"
    );
    assert!(
        room_of(&title).width() < wide_title,
        "a wider tab gives its title more room"
    );

    // Hovered, it shows its close button, and the title ends before it.
    behind.set_state_flags(gtk::StateFlags::PRELIGHT, false);
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    assert!(close.is_visible(), "a hovered tab shows its close button");
    let (close_start, _) = span_in(&close, &behind);
    let (_, hovered_end) = span_in(&room_of(&title), &behind);
    assert!(
        (close_start - hovered_end).abs() <= 1.0,
        "the title fills the room up to the close button: {hovered_end} then {close_start}"
    );
    behind.unset_state_flags(gtk::StateFlags::PRELIGHT);
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    assert!(!close.is_visible(), "and hides it again");

    // The tab in front, Home, fills the room up to its close button, with
    // only the button's own 5 pixels around its glyph between them, as
    // Windows 11 leaves about 4, and "Home" fits whole beside the icon.
    test.window
        .add_tab(crate::locations::Page::Home.uri())
        .expect("the Home page");
    crate::test_support::harness::wait_for_frames(&test.window, 4);
    let front = tab_widget(&test, 40);
    let (_, title, close) = tab_parts(&front);
    assert_eq!(title.text().as_str(), "Home");
    assert!(close.is_visible(), "the tab in front shows its close button");
    let (close_start, _) = span_in(&close, &front);
    let (_, title_end) = span_in(&room_of(&title), &front);
    assert!(
        (close_start - title_end).abs() <= 1.0,
        "the front tab's title fills the room up to its close button: {title_end} then {close_start}"
    );
    let glyph = close.first_child().expect("the close glyph");
    let (glyph_start, _) = span_in(&glyph, &front);
    assert!(
        glyph_start - title_end <= 6.0,
        "a small gap before the close glyph: {title_end} then {glyph_start}"
    );
    let home = title.create_pango_layout(Some("Home")).pixel_size().0;
    let room = room_of(&title).width();
    assert!(
        room >= home,
        "\"Home\" fits whole at the narrowest: {room} pixels for {home}"
    );
    assert!(
        f64::from(front.width()) <= f64::from(tab_widget(&test, 0).width()) + 0.5,
        "the tab in front is no wider than the others"
    );
}

/// Where each tab starts and ends in the strip's scrolled row.
fn tab_edges(test: &TestWindow) -> Vec<(f64, f64)> {
    let row = test.window.tab_strip().tab_list();
    crate::window::widget_tree::children(&row)
        .filter(|tab| tab.has_css_class("tab"))
        .filter_map(|tab| tab.compute_bounds(&row))
        .map(|bounds| (f64::from(bounds.x()), f64::from(bounds.x() + bounds.width())))
        .collect()
}

/// Each click of a scroll arrow brings exactly the next tab into view,
/// its far edge at the strip's edge, also past a Previous Versions tab,
/// which is wider than the others: forward, a tab's end lands on the
/// strip's right edge; back, a tab's start lands on its left edge.
///
/// parity: TAB-019
#[gtk::test]
fn an_arrow_scrolls_one_whole_tab_also_past_a_wide_snapshot_tab() {
    let fixture = super::item_dialogs::fixture_with_snapshot();
    let snapshot = fixture
        .path("Documents")
        .join(".snapshot")
        .join(super::item_dialogs::SNAPSHOT_NAME);
    let test = TestWindow::open(&fixture.uri());
    super::narrow_windows::resize(&test, 1366);
    have_tabs(&test, &fixture, 10);
    test.window
        .add_tab(&ox_core::location::file_uri(&snapshot))
        .expect("the snapshot's folder");
    test.wait_for_listing("the snapshot's tab");
    have_tabs(&test, &fixture, 30);
    wait_until("the snapshot's tab", || {
        descendants::<gtk::Widget>(test.window.tab_strip())
            .iter()
            .any(|widget| widget.has_css_class("snapshot-tab"))
    });
    crate::test_support::harness::wait_for_frames(&test.window, 4);
    let (back, forward) = scroll_arrows(&test);
    wait_until("the arrows", || back.is_visible());
    let adjustment = strip_scroll(&test);
    // The strip's own animated scroll to the tab in front ends first.
    let settle = || {
        let last = std::cell::Cell::new(f64::NAN);
        wait_until("the strip to stop scrolling", || {
            crate::test_support::harness::wait_for_frames(&test.window, 2);
            let now = adjustment.value();
            (now - last.replace(now)).abs() < 0.01
        });
    };
    settle();
    adjustment.set_value(0.0);
    settle();

    let lands_on = |edge: f64, ends: bool| {
        tab_edges(&test)
            .iter()
            .any(|(start, end)| ((if ends { *end } else { *start }) - edge).abs() <= 1.0)
    };
    let end = adjustment.upper() - adjustment.page_size();
    let mut clicks = 0;
    while adjustment.value() < end - 0.5 {
        forward.emit_clicked();
        settle();
        let right = adjustment.value() + adjustment.page_size();
        assert!(
            lands_on(right, true),
            "click {clicks}: a tab ends at the right edge {right}: {:?}",
            tab_edges(&test)
        );
        clicks += 1;
        assert!(clicks < 40, "the strip reaches its end");
    }
    while adjustment.value() > 0.5 {
        back.emit_clicked();
        settle();
        let left = adjustment.value();
        assert!(
            lands_on(left, false),
            "a tab starts at the left edge {left}: {:?}",
            tab_edges(&test)
        );
        clicks += 1;
        assert!(clicks < 80, "the strip reaches its start");
    }
}

/// With "Reopen my tabs" on, the open tabs are saved while the window is
/// open, soon after they change, not only when the last window closes:
/// an app ended by a signal (SIGTERM at logout) keeps the tabs it had.
///
/// parity: TAB-053
#[gtk::test]
fn the_open_tabs_are_saved_while_the_window_is_open() {
    use ox_core::session::SavedSession;
    use ox_core::settings::{PreferencesUpdate, Settings};

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let update = PreferencesUpdate {
        restore_session: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    // As a window the user opened, which saves its size and tabs.
    test.window.imp().remembers_session.set(true);

    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the new tab");

    let saved_uris = || {
        SavedSession::load(test.settings_directory()).map(|saved| {
            saved
                .tabs
                .iter()
                .map(|tab| tab.panes[tab.active_pane].uri.clone())
                .collect::<Vec<_>>()
        })
    };
    wait_until("the tabs to be saved", || {
        saved_uris() == Some(vec![fixture.uri(), fixture.uri_of("Documents")])
    });
    assert!(test.window.is_visible(), "saved while the window is open");
}

/// A save of the open tabs that fails for a while (a full disk, a folder
/// that may not be written) is tried again at the next save, so the tabs
/// are kept once the storage works again.
///
/// parity: TAB-053
#[gtk::test]
fn a_failed_save_of_the_open_tabs_is_tried_again() {
    use ox_core::session::SavedSession;
    use ox_core::settings::{PreferencesUpdate, Settings};

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let update = PreferencesUpdate {
        restore_session: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    test.window.imp().remembers_session.set(true);
    let directory = test.settings_directory().to_owned();
    // A folder where the session file goes makes every save fail.
    let blocker = directory.join("session.json");
    std::fs::create_dir_all(blocker.join("in the way")).expect("a folder in the way");

    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the new tab");
    wait_for(std::time::Duration::from_secs(4));
    let while_failing = SavedSession::load(&directory);
    std::fs::remove_dir_all(&blocker).expect("the way cleared");

    let saved_uris = || {
        SavedSession::load(&directory).map(|saved| {
            saved
                .tabs
                .iter()
                .map(|tab| tab.panes[tab.active_pane].uri.clone())
                .collect::<Vec<_>>()
        })
    };
    wait_until("the tabs to be saved once the folder may be written", || {
        saved_uris() == Some(vec![fixture.uri(), fixture.uri_of("Documents")])
    });
    assert!(while_failing.is_none(), "nothing could be saved meanwhile");
}

/// When another window saved its tabs, a window whose tabs did not change
/// saves them again once it is in front, so the next start opens the tabs
/// of the window in front, not those of the window that saved last.
///
/// parity: TAB-053
#[gtk::test]
fn a_window_saves_its_unchanged_tabs_again_after_another_window_saved() {
    use ox_core::session::SavedSession;

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let other = test.open_beside(&fixture.uri_of("Documents"));
    let saved_uris = || {
        SavedSession::load(test.settings_directory()).map(|saved| {
            saved
                .tabs
                .iter()
                .map(|tab| tab.panes[tab.active_pane].uri.clone())
                .collect::<Vec<_>>()
        })
    };

    test.window.save_session_in_background();
    wait_until("the first window's tabs to be saved", || {
        saved_uris() == Some(vec![fixture.uri()])
    });
    other.window.save_session_in_background();
    wait_until("the other window's tabs to be saved", || {
        saved_uris() == Some(vec![fixture.uri_of("Documents")])
    });
    test.window.save_session_in_background();
    wait_until("the first window's tabs to be saved again", || {
        saved_uris() == Some(vec![fixture.uri()])
    });
}
