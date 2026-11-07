// SPDX-License-Identifier: AGPL-3.0-only
//! The This PC page, laid out as Windows 11 lays it out (owner decision
//! 2026-11-06): tiles under collapsible group headers, no heading and no
//! Quick access cards, and the network locations' art. Network is listed
//! like a folder (`network.rs`).

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::places::NetworkKind;

use super::file_ops_support::{is_enabled, is_triggered_by, shortcuts_of};
use super::geometry::{bounds, laid_out, Bounds};
use super::support::{art_image_showing, arts_in, menu_button_with_class};
use crate::icons::{Art, Connection, Icon};
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::mounted_volume;
use crate::volumes::VolumeKind;
use crate::window::landing;
use crate::window::menu_popover::MenuPopover;

/// The labels `widget` shows, in order.
fn texts_in(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    descendants::<gtk::Label>(widget)
        .iter()
        .map(|label| label.text().to_string())
        .collect()
}

/// This PC with two drives besides Local Disk: Backup on the fixture
/// folder and USB on its Documents, as the volume monitor would list them.
fn this_pc_with_drives(fixture: &Fixture) -> TestWindow {
    let test = laid_out(Page::ThisPc.uri());
    test.window.imp().volumes.replace(vec![
        mounted_volume("Backup", &fixture.uri(), VolumeKind::Drive),
        mounted_volume("USB", &fixture.uri_of("Documents"), VolumeKind::Drive),
    ]);
    test.window.render_places();
    wait_for_frames(&test.window, 2);
    test
}

/// The drive tiles' box and the tile named `name` in it.
fn drive_tile(test: &TestWindow, name: &str) -> (gtk::FlowBox, gtk::FlowBoxChild) {
    let landing = test.window.folder_pane().landing();
    landing::tile_groups(landing)
        .into_iter()
        .find_map(|(flow, tiles)| {
            let tile = tiles
                .into_iter()
                .find(|tile| texts_in(tile).first().map(String::as_str) == Some(name))?;
            Some((flow, tile))
        })
        .unwrap_or_else(|| panic!("This PC has a tile named {name}"))
}

/// The first group header This PC shows.
fn first_header(test: &TestWindow) -> gtk::Button {
    let headers = landing::group_headers(test.window.folder_pane().landing());
    headers.into_iter().next().expect("This PC heads its groups")
}

/// Emits `key-pressed` with `key` on the key controller of `widget`;
/// whether a handler took it.
fn press_key(widget: &impl IsA<gtk::Widget>, key: gtk::gdk::Key) -> bool {
    let controller = widget
        .observe_controllers()
        .into_iter()
        .find_map(|controller| controller.ok().and_downcast::<gtk::EventControllerKey>())
        .expect("the widget has a key controller");
    let no_keycode = 0_u32;
    controller.emit_by_name::<bool>(
        "key-pressed",
        &[&key.into_glib(), &no_keycode, &gtk::gdk::ModifierType::empty()],
    )
}

/// Runs the shortcut of `widget` for `accelerator`, as GTK runs it for the
/// focused tile; whether one ran.
fn press_shortcut(widget: &impl IsA<gtk::Widget>, accelerator: &str) -> bool {
    let (key, modifiers) = gtk::accelerator_parse(accelerator).expect("a valid accelerator");
    let widget = widget.upcast_ref::<gtk::Widget>();
    shortcuts_of(widget, gtk::PropagationPhase::Bubble)
        .into_iter()
        .filter(|shortcut| {
            shortcut
                .trigger()
                .is_some_and(|trigger| is_triggered_by(&trigger, key, modifiers))
        })
        .any(|shortcut| {
            shortcut
                .action()
                .is_some_and(|action| action.activate(gtk::ShortcutActionFlags::empty(), widget, None))
        })
}

/// Pinned folders stay in the navigation pane: This PC shows no heading,
/// no Quick access cards, only its groups; and nothing on it can be
/// searched or created.
///
/// parity: HOME-001
#[gtk::test]
fn this_pc_shows_its_groups_with_no_heading_or_quick_access_and_nothing_to_create() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("pin-folder", None);
    wait_until("the pin", || !test.context.settings_data().pins.is_empty());

    test.window.navigate(Page::ThisPc.uri()).expect("This PC");
    test.wait_for_listing("This PC");

    let landing = test.window.folder_pane().landing();
    let texts = texts_in(landing);
    assert!(
        !texts.iter().any(|text| text == "This PC"),
        "no heading: {texts:?}"
    );
    assert!(
        !texts.iter().any(|text| text == "Example projects"),
        "no Quick access cards: {texts:?}"
    );
    assert!(test
        .window
        .sidebar()
        .labels()
        .contains(&"Example projects".to_owned()));
    assert_eq!(landing::section_titles(landing), ["Devices and drives"]);
    test.save_share("smb://nas/media", "Media (M:)");
    wait_until("the Network locations group", || {
        landing::section_titles(landing) == ["Devices and drives", "Network locations"]
    });
    assert_eq!(test.window.status_bar().texts().0, "Ready");
    assert!(!test.window.search_box().is_sensitive(), "Search");
    assert!(!is_enabled(&test, "up"), "Up");
    assert!(
        !menu_button_with_class(&test, "new-command").is_sensitive(),
        "New"
    );
}

/// Drive tiles are the Tiles view's: at least 256 pixels a column with
/// their margins, all as wide, beside each other while they fit.
#[gtk::test]
fn drive_tiles_are_laid_out_as_the_tiles_view() {
    let fixture = Fixture::standard();
    let test = this_pc_with_drives(&fixture);
    let landing = test.window.folder_pane().landing();
    let (_, tiles) = landing::tile_groups(landing)
        .into_iter()
        .next()
        .expect("the drives' group");
    let places: Vec<Bounds> = tiles.iter().map(|tile| bounds(&test, tile)).collect();

    assert_eq!(places.len(), 3, "Local Disk, Backup and USB");
    assert!(
        places.iter().all(|place| place.width == places[0].width),
        "{places:?}"
    );
    assert!(places[0].width >= 256 - 4, "{places:?}");
    assert!(
        places.iter().all(|place| place.y == places[0].y),
        "one row: {places:?}"
    );
}

/// Up from a group's first row reaches its header; Left collapses the
/// group and Right expands it, and Down goes back to its first tile, where
/// the arrows carry on. A collapsed group stays collapsed when the page is
/// drawn again.
///
/// parity: HOME-001
#[gtk::test]
fn the_keyboard_collapses_and_expands_a_group_from_its_header() {
    let fixture = Fixture::standard();
    let test = this_pc_with_drives(&fixture);
    let (flow, backup) = drive_tile(&test, "Backup");
    let header = first_header(&test);
    backup.child_focus(gtk::DirectionType::TabForward);

    assert!(press_key(&flow, gtk::gdk::Key::Up), "Up from the first row");
    assert!(header.has_focus(), "the header has the keyboard");
    assert!(press_key(&header, gtk::gdk::Key::Left));
    assert!(!flow.is_visible(), "Left collapses the group");
    gtk::prelude::GtkWindowExt::set_focus(&test.window, None::<&gtk::Widget>);
    assert!(
        header.is_focusable(),
        "a collapsed group's header stays a Tab stop"
    );
    test.window.folder_pane().focus_view();
    assert!(header.has_focus(), "the keyboard comes back to the page");
    test.window.render_places();
    let (flow, _) = landing::tile_groups(test.window.folder_pane().landing())
        .into_iter()
        .next()
        .expect("the drives' group");
    assert!(!flow.is_visible(), "a redraw keeps the group collapsed");
    let header = first_header(&test);
    assert!(header.has_focus(), "and the keyboard on its header");
    assert!(press_key(&header, gtk::gdk::Key::Right));
    assert!(flow.is_visible(), "Right expands it");

    assert!(press_key(&header, gtk::gdk::Key::Down));
    let first = flow.child_at_index(0).expect("Local Disk");
    assert!(
        first.has_focus() && first.is_selected(),
        "Down goes to the first tile"
    );
    assert!(!header.is_focusable(), "the header leaves the Tab order");
    flow.emit_by_name::<bool>(
        "move-cursor",
        &[&gtk::MovementStep::VisualPositions, &1_i32, &false, &false],
    );
    let second = flow.child_at_index(1).expect("Backup");
    assert!(
        second.has_focus(),
        "the arrows carry on from the tile Down reached"
    );
}

/// A drive coming or going draws the page again; the selected tile stays
/// selected and keeps the keyboard.
///
/// parity: HOME-003
#[gtk::test]
fn a_redraw_keeps_the_selected_tile_and_the_keyboard() {
    let fixture = Fixture::standard();
    let test = this_pc_with_drives(&fixture);
    let (_, usb) = drive_tile(&test, "USB");
    usb.child_focus(gtk::DirectionType::TabForward);

    test.window.imp().volumes.borrow_mut().insert(
        0,
        mounted_volume("Camera", &fixture.uri_of("Pictures"), VolumeKind::Device),
    );
    test.window.render_places();

    let (flow, usb) = drive_tile(&test, "USB");
    assert_eq!(
        texts_in(&flow.child_at_index(1).expect("a second tile"))[0],
        "Camera"
    );
    assert_eq!(flow.selected_children(), std::slice::from_ref(&usb));
    assert!(usb.has_focus());
}

/// Opening a drive's tile, by a double-click or Enter, goes to the drive;
/// selecting a tile in one group clears the other.
///
/// parity: HOME-003
#[gtk::test]
fn a_drive_tile_opens_the_drive_and_one_tile_is_selected() {
    let fixture = Fixture::standard();
    let test = this_pc_with_drives(&fixture);
    test.save_share("smb://nas/media", "Media (M:)");
    wait_until("the Network locations group", || {
        landing::tile_groups(test.window.folder_pane().landing()).len() == 2
    });
    let (drives, backup) = drive_tile(&test, "Backup");
    let (shares, media) = drive_tile(&test, "Media (\\\\nas) (M:)");
    drives.select_child(&backup);
    shares.select_child(&media);
    assert!(drives.selected_children().is_empty(), "one tile is selected");

    drives.emit_by_name::<()>("child-activated", &[&backup]);

    test.wait_for_listing("Backup");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// Right-clicks `tile` and returns the menu it opens in the folder pane.
fn open_tile_menu(test: &TestWindow, tile: &gtk::FlowBoxChild) -> MenuPopover {
    let right_click = tile
        .observe_controllers()
        .into_iter()
        .filter_map(|controller| controller.ok().and_downcast::<gtk::GestureClick>())
        .find(|click| click.button() == gtk::gdk::BUTTON_SECONDARY)
        .expect("the tile opens its menu on a right-click");
    right_click.emit_by_name::<()>("pressed", &[&1_i32, &5.0_f64, &5.0_f64]);
    let menu = descendants::<MenuPopover>(test.window.folder_pane())
        .into_iter()
        .find(gtk::prelude::WidgetExt::is_visible)
        .expect("the tile's menu is open in the folder pane");
    wait_for_frames(&test.window, 2);
    menu
}

/// Closes `menu` and waits for it to let go of the pane.
fn close_menu(menu: &MenuPopover) {
    menu.popdown();
    wait_until("the menu to let go of the pane", || menu.parent().is_none());
}

/// A right-click selects the drive and opens the drive's menu, with
/// Properties; the Menu key opens it too, and Alt+Enter opens the drive's
/// Properties.
///
/// parity: HOME-003
#[gtk::test]
fn a_drive_tile_has_the_drive_menu_and_its_properties() {
    let fixture = Fixture::standard();
    let test = this_pc_with_drives(&fixture);
    let (flow, backup) = drive_tile(&test, "Backup");

    let menu = open_tile_menu(&test, &backup);
    let labels = texts_in(&menu);
    close_menu(&menu);
    assert_eq!(
        flow.selected_children(),
        std::slice::from_ref(&backup),
        "a right-click selects"
    );
    for label in ["Open", "Open in new window", "Disconnect", "Properties"] {
        assert!(labels.iter().any(|shown| shown == label), "{label} in {labels:?}");
    }
    assert!(press_shortcut(&flow, "Menu"), "the Menu key opens the menu");
    let menu = descendants::<MenuPopover>(test.window.folder_pane())
        .into_iter()
        .find(gtk::prelude::WidgetExt::is_visible)
        .expect("the Menu key's menu");
    close_menu(&menu);

    assert!(press_shortcut(&flow, "<Alt>Return"));
    wait_until("the drive's Properties", || {
        !test.window.properties_windows().is_empty()
    });
    let windows = test.window.properties_windows();
    for window in &windows {
        window.close();
    }
    wait_until("Properties to close", || {
        test.window.properties_windows().is_empty()
    });
    assert_eq!(windows.len(), 1);
}

/// app.js drew a blue server on every saved share's card; the owner's icon
/// mapping (2026-09-28) shows a network location the same way everywhere,
/// so a mapped drive that is not mounted is the crossed-out drive on the
/// network bar on its tile and in the sidebar alike.
///
/// parity: LOOK-016
#[gtk::test]
fn a_saved_share_tile_shows_the_art_of_its_sidebar_row() {
    let test = laid_out(Page::ThisPc.uri());
    test.save_share("smb://nas/media", "Media (M:)");
    let crossed_out_drive =
        Art::for_network_location(NetworkKind::Share, "Media (M:)", Connection::Disconnected);
    let landing = test.window.folder_pane().landing();
    wait_until("the saved share's tile", || {
        arts_in(landing).contains(&crossed_out_drive)
    });
    let sidebar_arts = arts_in(test.window.sidebar());
    assert!(sidebar_arts.contains(&crossed_out_drive), "{sidebar_arts:?}");
}

/// A server keeps the blue app.js gave a saved share's server glyph
/// (`im.style.color='#4b96c0'`), now on the network bar; the stylesheet
/// colours it, not code. Its row under Network shows it.
///
/// parity: LOOK-016
#[gtk::test]
fn a_server_row_shows_the_server_in_the_share_blue_on_the_network_bar() {
    let test = laid_out(Page::ThisPc.uri());
    // Browsing a server lists it under Network for the session; recording
    // it directly mounts nothing.
    test.window.context().remember_network("smb://nas/");
    let server = Art::for_network_location(NetworkKind::Server, "nas", Connection::Disconnected);
    let sidebar = test.window.sidebar();
    wait_until("the server's row", || arts_in(sidebar).contains(&server));
    wait_for_frames(&test.window, 2);
    let row_art = art_image_showing(sidebar, server).expect("the row is drawn");
    let glyph = descendants::<gtk::Image>(&row_art)
        .into_iter()
        .find(|image| image.has_css_class("server"))
        .expect("the row shows the server glyph");
    assert_eq!(glyph.icon_name().as_deref(), Some(Icon::Server.name()));
    assert_eq!(glyph.color().to_str(), "rgb(75,150,192)");
}

/// Drawing the page again while a tile's menu is open, as a drive or a
/// found server does, destroys the tile; the menu belongs to the pane, so
/// it stays usable and closes cleanly.
///
/// parity: SIDE-017
#[gtk::test]
fn a_this_pc_tile_menu_survives_the_page_being_drawn_again() {
    let test = laid_out(Page::ThisPc.uri());
    test.save_share("smb://nas/media", "Media (M:)");
    let share = Art::for_network_location(NetworkKind::Share, "Media (M:)", Connection::Disconnected);
    let landing = test.window.folder_pane().landing();
    wait_until("the tile", || art_image_showing(landing, share).is_some());
    wait_for_frames(&test.window, 2);
    let tile = art_image_showing(landing, share)
        .and_then(|image| image.ancestor(gtk::FlowBoxChild::static_type()))
        .and_downcast::<gtk::FlowBoxChild>()
        .expect("the art is on a tile");
    let menu = open_tile_menu(&test, &tile);
    let pane = test.window.folder_pane().clone();
    assert_eq!(menu.parent().as_ref(), Some(pane.upcast_ref::<gtk::Widget>()));

    test.window.render_places();
    wait_for_frames(&test.window, 2);
    let still_open = menu.is_visible();
    close_menu(&menu);

    assert!(still_open, "the redraw does not close the menu");
}
