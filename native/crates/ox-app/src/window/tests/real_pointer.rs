// SPDX-License-Identifier: AGPL-3.0-only
//! Menus driven by a real pointer, moved with `xdotool` on X11 or with a
//! virtual pointer on a wlroots Wayland compositor such as sway: hovering, unlike a test's direct calls, sends GTK's own motion
//! and crossing events through every popover's grab. Skipped when neither
//! tool can move the pointer here.

use std::process::Command;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::test_support::harness::{wait_until, Fixture, TestWindow};
use crate::window::menu_popover::MenuPopover;

/// How the test moves the pointer.
enum Mover {
    /// `xdotool mousemove`, to a place on the X screen.
    Xdotool,
    /// A virtual pointer that stays plugged in on a wlroots compositor:
    /// the program `OX_VIRTUAL_POINTER` names reads "x y" lines and
    /// answers "ok" once the pointer is there.
    Virtual(std::process::Child, std::io::BufReader<std::process::ChildStdout>),
}

/// The tool that can move the pointer here, if one can.
fn pointer_mover() -> Option<Mover> {
    use std::io::BufRead;
    let display = gdk::Display::default()?;
    let backend = display.type_().name();
    if backend.contains("X11") {
        let works = Command::new("xdotool")
            .arg("version")
            .output()
            .is_ok_and(|output| output.status.success());
        return works.then_some(Mover::Xdotool);
    }
    let program = std::env::var("OX_VIRTUAL_POINTER").ok()?;
    let mut child = Command::new(program)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let mut reader = std::io::BufReader::new(child.stdout.take()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    (line.trim() == "ready").then_some(Mover::Virtual(child, reader))
}

/// Moves the pointer to `(x, y)` on the screen and lets GTK see it.
fn move_pointer(mover: &mut Mover, x: f64, y: f64) {
    use std::io::{BufRead, Write};
    #[expect(clippy::cast_possible_truncation, reason = "screen coordinates are small")]
    let (x, y) = (x.round().max(0.0) as i32, y.round().max(0.0) as i32);
    match mover {
        Mover::Xdotool => {
            let status = Command::new("xdotool")
                .args(["mousemove", &x.to_string(), &y.to_string()])
                .status()
                .expect("xdotool runs");
            assert!(status.success());
        }
        Mover::Virtual(child, reader) => {
            let input = child.stdin.as_mut().expect("piped");
            writeln!(input, "{x} {y}").expect("the pointer listens");
            input.flush().expect("sent");
            let mut line = String::new();
            reader.read_line(&mut line).expect("the pointer answers");
        }
    }
    settle_for(Duration::from_millis(80));
}

/// Clicks the left button where the pointer is.
fn click(mover: &mut Mover) {
    use std::io::{BufRead, Write};
    match mover {
        Mover::Xdotool => {
            let status = Command::new("xdotool").args(["click", "1"]).status().expect("xdotool runs");
            assert!(status.success());
        }
        Mover::Virtual(child, reader) => {
            let input = child.stdin.as_mut().expect("piped");
            writeln!(input, "click").expect("the pointer listens");
            input.flush().expect("sent");
            let mut line = String::new();
            reader.read_line(&mut line).expect("the pointer answers");
        }
    }
    settle_for(Duration::from_millis(150));
}

/// Runs the main loop for `time`.
fn settle_for(time: Duration) {
    let until = std::time::Instant::now() + time;
    let context = glib::MainContext::default();
    while std::time::Instant::now() < until {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Where the surface of `native` starts on the screen.
fn surface_origin(native: &impl IsA<gtk::Native>, toplevel_origin: (f64, f64)) -> (f64, f64) {
    let surface = native.surface().expect("a mapped native has a surface");
    match surface.downcast_ref::<gdk::Popup>() {
        Some(popup) => {
            let parent = popup.parent().expect("a popup has a parent surface");
            let parent_native = gtk::Native::for_surface(&parent).expect("the parent is a native");
            let (px, py) = surface_origin(&parent_native, toplevel_origin);
            (px + f64::from(popup.position_x()), py + f64::from(popup.position_y()))
        }
        None => toplevel_origin,
    }
}

/// Where the toplevel's surface starts on the screen: the pointer is put
/// at a known place and the surface says where it sees it.
fn toplevel_origin(mover: &mut Mover, window: &gtk::Window) -> (f64, f64) {
    move_pointer(mover, 300.0, 300.0);
    let surface = window.surface().expect("a mapped window has a surface");
    let display = surface.display();
    let pointer = display.default_seat().and_then(|seat| seat.pointer()).expect("a pointer");
    let (x, y, _) = surface.device_position(&pointer).expect("the pointer is over the window");
    (300.0 - x, 300.0 - y)
}

/// The middle of `row` of `menu` on the screen.
fn row_middle(menu: &MenuPopover, label: &str, toplevel: (f64, f64)) -> (f64, f64) {
    let row = menu.row(label);
    let (sx, sy) = surface_origin(menu, toplevel);
    let (tx, ty) = menu.surface_transform();
    let point = row
        .compute_point(menu, &gtk::graphene::Point::new(row.width() as f32 / 2.0, row.height() as f32 / 2.0))
        .expect("the row is in the menu");
    (sx + tx + f64::from(point.x()), sy + ty + f64::from(point.y()))
}

/// With Group by's submenu open, resting a real pointer on More opens
/// More's in its place; on Ascending, the submenu closes; moving into a
/// submenu keeps it open.
#[gtk::test]
fn a_real_pointer_swaps_and_keeps_submenus() {
    let Some(mut mover) = pointer_mover() else {
        eprintln!("skipped: no tool moves the pointer here");
        return;
    };
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let toplevel = toplevel_origin(&mut mover, test.window.upcast_ref());
    test.window.right_click(None);
    let menu = test.window.context_menu();
    menu.row("Sort by").emit_activate();
    wait_until("the Sort menu", || menu.is_visible() && menu.row_labels().contains(&"Group by".to_owned()));
    settle_for(Duration::from_millis(200));

    let (x, y) = row_middle(&menu, "Group by", toplevel);
    move_pointer(&mut mover, x, y);
    settle_for(Duration::from_millis(500));
    let open = menu.open_submenu_menu().map(|submenu| submenu.row_labels());
    eprintln!("after Group by: {open:?}");
    assert!(open.is_some_and(|labels| labels.contains(&"Same as sort".to_owned())), "Group by opens");

    // Into the submenu, across nothing else: it stays open.
    let submenu = menu.open_submenu_menu().expect("open");
    let (sx, sy) = row_middle(&submenu, "Type", toplevel);
    move_pointer(&mut mover, sx, sy);
    settle_for(Duration::from_millis(500));
    assert!(menu.open_submenu_menu().is_some(), "moving into the submenu keeps it");

    // Back to the menu, onto More.
    let (mx, my) = row_middle(&menu, "More", toplevel);
    move_pointer(&mut mover, mx, my);
    settle_for(Duration::from_millis(500));
    let open = menu.open_submenu_menu().map(|submenu| submenu.row_labels());
    eprintln!("after More: {open:?}");
    assert!(
        open.is_some_and(|labels| labels.first().map(String::as_str) == Some("Size")),
        "More's submenu takes Group by's place"
    );

    let (ax, ay) = row_middle(&menu, "Ascending", toplevel);
    move_pointer(&mut mover, ax, ay);
    settle_for(Duration::from_millis(500));
    assert!(menu.open_submenu_menu().is_none(), "Ascending closes the submenu");
    assert!(menu.is_visible());

    // Back to Group by, into its submenu, and a click on Date modified:
    // both menus close and the folder is grouped.
    let (x, y) = row_middle(&menu, "Group by", toplevel);
    move_pointer(&mut mover, x, y);
    settle_for(Duration::from_millis(500));
    let submenu = menu.open_submenu_menu().expect("Group by opens again");
    let (dx, dy) = row_middle(&submenu, "Date modified", toplevel);
    move_pointer(&mut mover, dx, dy);
    settle_for(Duration::from_millis(300));
    click(&mut mover);
    wait_until("the choice", || test.action_state("group-by").as_deref() == Some("modified"));
    wait_until("both menus to close", || !menu.is_visible() && !submenu.is_visible());
}
