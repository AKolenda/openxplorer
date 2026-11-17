// SPDX-License-Identifier: AGPL-3.0-only
//! No frame round a whole drop target while a drag is over it (DND-011).
//! GTK's own theme draws a green line round any widget a drag hovers that
//! holds a drop target; Windows Explorer outlines only the row, crumb or
//! folder a drop would go into, as this app's own highlight does.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::icons::pixel_rows;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};

/// How many pixels of the three-pixel band just inside `widget`'s left edge
/// are green, as GTK's drop frame is, in `test`'s window as drawn now.
fn green_on_left_edge(test: &TestWindow, widget: &gtk::Widget) -> usize {
    wait_for_frames(&test.window, 3);
    let window: gtk::Widget = test.window.clone().upcast();
    let place = widget.compute_bounds(&window).expect("the widget is laid out");
    let paintable = gtk::WidgetPaintable::new(Some(&window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(window.width()), f64::from(window.height()));
    let node = snapshot.to_node().expect("the window draws");
    let renderer = window
        .native()
        .and_then(|native| native.renderer())
        .expect("a drawn window");
    let texture = renderer.render_texture(&node, None::<&gtk::graphene::Rect>);
    let rows = pixel_rows(&texture);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "window pixels"
    )]
    let (left, top, height) = {
        // The window draws its frame's margin too, before its own (0, 0).
        let (margin_x, margin_y) = test.window.surface_transform();
        (
            (f64::from(place.x()) + margin_x).max(0.0) as usize,
            (f64::from(place.y()) + margin_y).max(0.0) as usize,
            place.height() as usize,
        )
    };
    // A window bigger than the screen is drawn only as far as it shows.
    let bottom = (top + height).min(rows.len());
    rows[top.min(bottom)..bottom]
        .iter()
        .flat_map(|row| row.iter().skip(left).take(3))
        .filter(|[blue, green, red, _]| *green > red.saturating_add(50) && *green > blue.saturating_add(20))
        .count()
}

/// While a drag is over the navigation pane, the address bar or the folder
/// tree, none of them is framed in green.
///
/// parity: DND-011
#[gtk::test]
fn a_drag_over_the_navigation_pane_or_the_address_bar_draws_no_frame_round_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // The fixture's temporary folder plays the home folder, so the tree
    // starts there and never lists the real disk.
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);
    test.activate("folder-tree", None);
    let tree = test.window.folder_tree();
    wait_until("the tree shows a folder", || tree.selected_uri().is_some());
    let widgets: [(&str, gtk::Widget); 3] = [
        (
            "the navigation pane",
            test.window.sidebar().list().clone().upcast(),
        ),
        ("the address bar", test.window.address_bar().clone().upcast()),
        (
            "the folder tree",
            descendants::<gtk::ListView>(tree)
                .into_iter()
                .next()
                .expect("the tree lists its rows")
                .upcast(),
        ),
    ];

    for (name, widget) in widgets {
        widget.set_state_flags(gtk::StateFlags::DROP_ACTIVE, false);
        let green = green_on_left_edge(&test, &widget);
        widget.unset_state_flags(gtk::StateFlags::DROP_ACTIVE);
        assert_eq!(green, 0, "{name} has no green frame under a drag");
    }
}

/// With `OX_NATIVE_CAPTURE_DIR` set, saves the window while a drag is
/// over the navigation pane and the address bar, with the folder tree
/// shown, in both themes; without it, proves neither is framed in green.
#[gtk::test]
fn the_navigation_pane_under_a_drag_is_captured() {
    use crate::test_support::harness::{capture, ThemeGuard};

    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);
    test.activate("folder-tree", None);
    let tree = test.window.folder_tree();
    wait_until("the tree shows a folder", || tree.selected_uri().is_some());
    let list: gtk::Widget = test.window.sidebar().list().clone().upcast();
    let address: gtk::Widget = test.window.address_bar().clone().upcast();
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        wait_for_frames(&test.window, 3);
        for widget in [&list, &address] {
            widget.set_state_flags(gtk::StateFlags::DROP_ACTIVE, false);
        }
        let green = green_on_left_edge(&test, &list) + green_on_left_edge(&test, &address);
        capture(&test.window, &format!("native-drop-no-frame-{theme}.png"));
        for widget in [&list, &address] {
            widget.unset_state_flags(gtk::StateFlags::DROP_ACTIVE);
        }
        assert_eq!(green, 0);
    }
}
