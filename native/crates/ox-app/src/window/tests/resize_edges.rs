// SPDX-License-Identifier: AGPL-3.0-only
//! The handles that resize the window from inside its own edges and
//! corners, as Windows 11 does (`src/resize_edges.rs`).
//!
//! The test display has no compositor, so GTK draws the window's solid
//! frame (`.solid-csd`) and resizes from that; the handles stand aside
//! there, and these tests turn them on to check where they lie.

use gtk::prelude::*;

use super::geometry::laid_out;
use crate::resize_edges::{self, ResizeHandle, INSIDE_EDGE};
use crate::test_support::harness::{descendants, Fixture};

/// The handle at `x`, `y` in `window`, if one is picked there.
fn handle_at(window: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Option<ResizeHandle> {
    window
        .pick(x, y, gtk::PickFlags::DEFAULT)
        .and_then(|widget| widget.downcast::<ResizeHandle>().ok())
}

/// On GTK's solid frame, which the window is resized from, the handles
/// stand aside: what lies at the window's edge is picked there.
#[gtk::test]
fn the_handles_stand_aside_on_a_solid_frame() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let window = &test.window;
    assert!(!window.has_css_class("csd"), "the test display has no compositor");
    let handles = descendants::<ResizeHandle>(window);
    assert_eq!(handles.len(), 8, "one on each edge and corner");
    assert!(handles.iter().all(|handle| !handle.can_target()));
    assert!(handle_at(window, 1.0, 1.0).is_none());
}

/// Every edge and corner of the window resizes it, with the resize cursor
/// of that edge, over the title bar and the panes alike; a few pixels in,
/// the window's own controls are reached, and the close button keeps all
/// but its very corner.
#[gtk::test]
fn the_window_edges_and_corners_resize_with_their_cursor() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let window = &test.window;
    resize_edges::turn_on_for_test(window);
    let width = f64::from(window.width());
    let height = f64::from(window.height());
    let spots = [
        (1.0, 1.0, "nw-resize"),
        (width / 2.0, 1.0, "n-resize"),
        (width - 1.0, 1.0, "ne-resize"),
        (width - 1.0, height / 2.0, "e-resize"),
        (width - 1.0, height - 1.0, "se-resize"),
        (width / 2.0, height - 1.0, "s-resize"),
        (1.0, height - 1.0, "sw-resize"),
        (1.0, height / 2.0, "w-resize"),
        // The rounded corners themselves, not only their outer pixel.
        (10.0, 10.0, "nw-resize"),
        (width - 10.0, height - 10.0, "se-resize"),
    ];
    for (x, y, cursor) in spots {
        let handle = handle_at(window, x, y).unwrap_or_else(|| panic!("a handle at {x}, {y}"));
        let shown = handle.cursor().and_then(|cursor| cursor.name());
        assert_eq!(shown.as_deref(), Some(cursor), "at {x}, {y}");
    }

    let inside = f64::from(INSIDE_EDGE) + 1.0;
    for (x, y) in [
        (width / 2.0, inside),
        (width - inside, height / 2.0),
        (width / 2.0, height - inside),
        (inside, height / 2.0),
    ] {
        assert!(handle_at(window, x, y).is_none(), "nothing resizes at {x}, {y}");
    }

    let close = descendants::<gtk::Button>(window)
        .into_iter()
        .find(|button| button.has_css_class("caption") && button.has_css_class("close"))
        .expect("a close button");
    let bounds = close
        .compute_bounds(window)
        .expect("the close button is laid out");
    let middle = window
        .pick(
            f64::from(bounds.x() + bounds.width() / 2.0),
            f64::from(bounds.y() + bounds.height() / 2.0),
            gtk::PickFlags::DEFAULT,
        )
        .expect("something under the close button's middle");
    assert!(
        middle == close.clone().upcast::<gtk::Widget>() || middle.is_ancestor(&close),
        "the close button is pressed in its middle, not {middle:?}"
    );
}

/// A scroll bar at the window's right edge scrolls: the edge's band
/// stands aside over its thumb and its trough, and the corner still
/// resizes.
#[gtk::test]
fn the_right_edge_leaves_the_file_lists_scroll_bar_to_the_scroll_bar() {
    let fixture = Fixture::with_files(300);
    let test = laid_out(&fixture.uri());
    let window = &test.window;
    resize_edges::turn_on_for_test(window);
    let width = f64::from(window.width());
    let height = f64::from(window.height());
    let thumb = descendants::<gtk::Widget>(window)
        .into_iter()
        .filter(|widget| widget.css_name() == "slider" && widget.is_mapped())
        .find(|slider| {
            let scrollbar = slider.ancestor(gtk::Scrollbar::static_type());
            let vertical = scrollbar
                .and_downcast::<gtk::Scrollbar>()
                .is_some_and(|scrollbar| scrollbar.orientation() == gtk::Orientation::Vertical);
            let right = slider.compute_bounds(window).is_some_and(|bounds| {
                f64::from(bounds.x() + bounds.width()) > width - f64::from(INSIDE_EDGE)
            });
            vertical && right
        })
        .expect("the file list's thumb at the right edge");
    let bounds = thumb.compute_bounds(window).expect("the thumb is laid out");
    // The thumb's last column, which the band covers.
    let thumb_x = f64::from(bounds.x() + bounds.width()) - 0.5;
    let thumb_y = f64::from(bounds.y() + bounds.height() / 2.0);
    assert!(
        thumb_x > width - f64::from(INSIDE_EDGE),
        "the thumb at {bounds:?} reaches into the band of a window {width} wide"
    );
    assert_eq!(
        window.pick(thumb_x, thumb_y, gtk::PickFlags::DEFAULT).as_ref(),
        Some(&thumb),
        "the thumb takes the band's pixels it covers"
    );
    let below_thumb = f64::from(bounds.y() + bounds.height()) + 20.0;
    assert!(below_thumb < height - 20.0, "room below the thumb");
    let trough = window
        .pick(width - 1.0, below_thumb, gtk::PickFlags::DEFAULT)
        .expect("something under the trough");
    assert!(
        trough.ancestor(gtk::Scrollbar::static_type()).is_some(),
        "the trough below the thumb takes the press, not {trough:?}"
    );
    assert!(
        handle_at(window, width - 1.0, height - 1.0).is_some(),
        "the corner resizes"
    );
}

/// A control at a window's edge takes presses all the way to the edge,
/// as in Windows: the band stands aside over a button, a check box and a
/// text field, and still resizes over what takes no press, such as a
/// label, and in the corners.
#[gtk::test]
fn controls_at_the_edge_take_their_own_presses() {
    let window = gtk::Window::new();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let button = gtk::Button::with_label("Browse...");
    let check = gtk::CheckButton::with_label("Read-only");
    let field = gtk::Entry::new();
    let label = gtk::Label::new(Some("Folder:"));
    for widget in [
        button.upcast_ref::<gtk::Widget>(),
        check.upcast_ref(),
        field.upcast_ref(),
        label.upcast_ref(),
    ] {
        widget.set_hexpand(true);
        widget.set_size_request(-1, 40);
        column.append(widget);
    }
    window.set_child(Some(&column));
    window.set_default_size(300, 160);
    resize_edges::install(&window);
    window.present();
    crate::test_support::harness::settle();
    // A plain window does not place the handles itself, as the app's
    // windows do after their own allocation.
    resize_edges::allocate(&window, window.width(), window.height());
    resize_edges::turn_on_for_test(&window);
    let width = f64::from(window.width());
    let left = 1.0;
    let right = width - 1.0;
    for (control, name) in [
        (button.upcast_ref::<gtk::Widget>(), "button"),
        (check.upcast_ref(), "check box"),
        (field.upcast_ref(), "text field"),
    ] {
        let bounds = control.compute_bounds(&window).expect("laid out");
        let y = f64::from(bounds.y() + bounds.height() / 2.0);
        for x in [left, right] {
            let picked = window.pick(x, y, gtk::PickFlags::DEFAULT).expect("a widget");
            assert!(
                picked == *control || picked.is_ancestor(control),
                "the {name} takes the press at {x}, {y}, not {picked:?}"
            );
        }
    }
    let bounds = label.compute_bounds(&window).expect("laid out");
    let y = f64::from(bounds.y() + bounds.height() / 2.0);
    assert!(handle_at(&window, left, y).is_some(), "a label resizes");
    assert!(handle_at(&window, 1.0, 1.0).is_some(), "the corner resizes");
    window.destroy();
}
