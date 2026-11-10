// SPDX-License-Identifier: AGPL-3.0-only
//! Resize handles on a window's own edges and corners, as Windows 11 has.
//!
//! GTK 4.14 resizes a window with client-side decorations only from
//! outside its visible edge: from a band in its shadow at most 12 pixels
//! wide (`RESIZE_HANDLE_SIZE` in gtkwindow.c, both in
//! `get_edge_for_coordinates` and in the input region it gives the
//! surface). Nothing inside the window resizes it, so the pointer has to
//! find a thin invisible strip beyond the window's edge or its rounded
//! corner, which few people ever do.
//!
//! [`install`] adds a handle for each edge and corner, laid over
//! everything else in the window: a band [`INSIDE_EDGE`] pixels wide just
//! inside each edge, which leaves a scroll bar's thumb under it to the
//! scroll bar, and a square [`INSIDE_CORNER`] pixels wide in each
//! corner, which covers the rounded corner. Each shows its resize cursor
//! (`nw-resize`, `n-resize`, …, the names GTK itself uses) and starts the
//! compositor's resize from its edge on a primary press; the window does
//! the same for the rounded-off pixels of its corners, where GTK picks
//! no child. [`allocate`]
//! places them after GTK's own allocation, and widens the surface's input
//! region to [`OUTSIDE`] pixels around the window, where GTK's own edge
//! detection still resizes: the skin keeps the window's shadow at least
//! that wide (`window.ox.ox-edge-resize` in `resources/skin/base.css`), since GTK
//! accepts a press only within the shadow.
//!
//! The handles stand aside, as GTK's own do, while the window cannot be
//! resized from that edge: when it is not resizable, maximized or full
//! screen, on an edge the compositor tiled (its edge constraints), and
//! without a compositor's shadow (`.solid-csd`, where GTK resizes from the
//! frame's border and padding instead, or a frame the window manager
//! draws). They work alike on X11 and Wayland, since GTK's
//! `gdk_toplevel_begin_resize` starts either system's resize.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
// `input_shape` is Cairo's region and rectangle, the only types GDK takes a
// surface's input region in. Nothing here draws: the region says where
// presses reach the window (`widen_input_region`).
use gtk::{cairo as input_shape, gdk, glib};

/// The width of the band just inside each edge that resizes the window.
pub(crate) const INSIDE_EDGE: i32 = 4;

/// The side of the square inside each corner that resizes the window
/// from that corner. It covers the window's 8-pixel rounded corner.
pub(crate) const INSIDE_CORNER: i32 = 12;

/// How far outside its edge the window takes a press to resize, past
/// GTK's own 12 pixels.
pub(crate) const OUTSIDE: i32 = 16;

/// The edges and corners, clockwise from the top left.
const EDGES: [gdk::SurfaceEdge; 8] = [
    gdk::SurfaceEdge::NorthWest,
    gdk::SurfaceEdge::North,
    gdk::SurfaceEdge::NorthEast,
    gdk::SurfaceEdge::East,
    gdk::SurfaceEdge::SouthEast,
    gdk::SurfaceEdge::South,
    gdk::SurfaceEdge::SouthWest,
    gdk::SurfaceEdge::West,
];

mod imp {
    use std::cell::Cell;

    use gtk::subclass::prelude::*;
    use gtk::{gdk, glib};

    /// Private state of [`super::ResizeHandle`].
    #[derive(Debug)]
    pub(crate) struct ResizeHandle {
        /// The edge or corner the handle resizes from.
        pub(super) edge: Cell<gdk::SurfaceEdge>,
        /// Set while the handle looks at what lies under it, which it
        /// stands aside for meanwhile.
        pub(super) looking_under: Cell<bool>,
    }

    impl Default for ResizeHandle {
        fn default() -> Self {
            Self {
                edge: Cell::new(gdk::SurfaceEdge::North),
                looking_under: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResizeHandle {
        const NAME: &'static str = "OxResizeHandle";
        type Type = super::ResizeHandle;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("ox-resize-handle");
            // Screen readers skip it: the keyboard resizes a window
            // through the desktop's own window menu.
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for ResizeHandle {}

    impl WidgetImpl for ResizeHandle {
        /// An edge's band leaves a scroll bar's thumb under it to the
        /// scroll bar: the file list's thumb lies at the window's right
        /// edge, 3 pixels wide 2 in from it, and dragging it scrolls.
        /// The corners resize whatever lies under them.
        fn contains(&self, x: f64, y: f64) -> bool {
            if self.looking_under.get() || !self.parent_contains(x, y) {
                return false;
            }
            if super::is_corner(self.edge.get()) {
                return true;
            }
            self.looking_under.set(true);
            let under = super::under(&self.obj(), x, y);
            self.looking_under.set(false);
            !under.is_some_and(|widget| super::is_scroll_thumb(&widget))
        }
    }
}

glib::wrapper! {
    /// An invisible handle on one edge or corner of a window that resizes
    /// the window from there.
    pub(crate) struct ResizeHandle(ObjectSubclass<imp::ResizeHandle>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ResizeHandle {
    /// A handle that resizes its window from `edge`, with that edge's
    /// cursor.
    fn new(edge: gdk::SurfaceEdge) -> Self {
        let handle: Self = glib::Object::builder()
            .property("focusable", false)
            .property("cursor", gdk::Cursor::from_name(cursor_name(edge), None))
            .build();
        handle.imp().edge.set(edge);
        let press = gtk::GestureClick::new();
        press.set_button(gdk::BUTTON_PRIMARY);
        press.connect_pressed(move |press, _, _, _| begin_resize(press, edge));
        handle.add_controller(press);
        handle
    }

    /// The edge or corner the handle resizes from.
    pub(crate) fn edge(&self) -> gdk::SurfaceEdge {
        self.imp().edge.get()
    }
}

/// The cursor GTK shows over its own handle on `edge`.
pub(crate) fn cursor_name(edge: gdk::SurfaceEdge) -> &'static str {
    match edge {
        gdk::SurfaceEdge::NorthWest => "nw-resize",
        gdk::SurfaceEdge::North => "n-resize",
        gdk::SurfaceEdge::NorthEast => "ne-resize",
        gdk::SurfaceEdge::East => "e-resize",
        gdk::SurfaceEdge::SouthEast => "se-resize",
        gdk::SurfaceEdge::South => "s-resize",
        gdk::SurfaceEdge::SouthWest => "sw-resize",
        _ => "w-resize",
    }
}

/// Starts the compositor's resize of the pressed handle's window from
/// `edge`, as GTK's own `click_gesture_pressed_cb` does.
fn begin_resize(press: &gtk::GestureClick, edge: gdk::SurfaceEdge) {
    let sequence = press.current_sequence();
    let Some(event) = press.last_event(sequence.as_ref()) else {
        return;
    };
    let Some((x, y)) = event.position() else {
        return;
    };
    let toplevel = press
        .widget()
        .and_then(|handle| handle.native())
        .and_then(|native| native.surface())
        .and_downcast::<gdk::Toplevel>();
    let Some(toplevel) = toplevel else {
        return;
    };
    press.set_state(gtk::EventSequenceState::Claimed);
    toplevel.begin_resize(
        edge,
        press.device().as_ref(),
        // BUTTON_PRIMARY is 1.
        1,
        x,
        y,
        event.time(),
    );
    press.reset();
}

/// Adds the handles to `window`, over everything in it. A window that
/// calls this calls [`allocate`] after GTK's own `size_allocate`.
pub(crate) fn install(window: &impl IsA<gtk::Window>) {
    let window = window.upcast_ref::<gtk::Window>();
    // The skin widens the window's shadow to OUTSIDE for it.
    window.add_css_class("ox-edge-resize");
    for edge in EDGES {
        ResizeHandle::new(edge).set_parent(window);
    }
    // GTK picks a window's last children first. The window's content and
    // a title bar set later go last, and so does the title bar GTK makes
    // for a window without one when it realizes it (without telling), so
    // the handles go back over them each time.
    for property in ["child", "titlebar"] {
        window.connect_notify_local(Some(property), |window, _| raise(window));
    }
    window.connect_realize(raise);
    for property in ["resizable", "maximized", "fullscreened"] {
        window.connect_notify_local(Some(property), |window, _| refresh(window));
    }
    // Tiling and the compositor's edge constraints arrive as surface
    // state, which may not change the window's size.
    window.connect_realize(|window| {
        let Some(surface) = window.surface() else {
            return;
        };
        let window = window.downgrade();
        surface.connect_notify_local(Some("state"), move |_, _| {
            if let Some(window) = window.upgrade() {
                refresh(&window);
            }
        });
    });
    follow_cut_corners(window);
    // GTK's window unparents only its own title bar and child.
    window.connect_destroy(|window| {
        for handle in handles(window) {
            handle.unparent();
        }
    });
}

/// Resizes from the rounded-off pixels of `window`'s corners too. GTK
/// picks the window's children only inside its rounded border
/// (`overflow: hidden`), so there the window itself is under the pointer,
/// not the corner's handle: the window shows the corner's cursor and
/// starts the resize itself.
fn follow_cut_corners(window: &gtk::Window) {
    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion(|motion, x, y| {
        let Some(window) = motion.widget().and_downcast::<gtk::Window>() else {
            return;
        };
        let corner = cut_corner(&window, x, y);
        window.set_cursor_from_name(corner.map(cursor_name));
    });
    motion.connect_leave(|motion| {
        if let Some(window) = motion.widget() {
            window.set_cursor(None);
        }
    });
    window.add_controller(motion);
    let press = gtk::GestureClick::new();
    press.set_button(gdk::BUTTON_PRIMARY);
    press.connect_pressed(|press, _, x, y| {
        let window = press.widget().and_downcast::<gtk::Window>();
        if let Some(edge) = window.and_then(|window| cut_corner(&window, x, y)) {
            begin_resize(press, edge);
        }
    });
    window.add_controller(press);
}

/// The corner whose handle is on and covers `x`, `y` in `window`, when
/// no child of the window is picked there: in the rounded-off part of
/// the corner.
#[expect(clippy::cast_possible_truncation, reason = "window coordinates are small")]
fn cut_corner(window: &gtk::Window, x: f64, y: f64) -> Option<gdk::SurfaceEdge> {
    let corner = handles(window).into_iter().find(|handle| {
        is_corner(handle.edge())
            && handle.can_target()
            && handle
                .compute_bounds(window)
                .is_some_and(|bounds| bounds.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)))
    })?;
    // Outside its own rounded border the window picks nothing, and GTK
    // sends the event to the window.
    let picked = window.pick(x, y, gtk::PickFlags::DEFAULT);
    picked
        .is_none_or(|picked| &picked == window.upcast_ref::<gtk::Widget>())
        .then(|| corner.edge())
}

/// Whether `edge` is a corner.
fn is_corner(edge: gdk::SurfaceEdge) -> bool {
    matches!(
        edge,
        gdk::SurfaceEdge::NorthWest
            | gdk::SurfaceEdge::NorthEast
            | gdk::SurfaceEdge::SouthEast
            | gdk::SurfaceEdge::SouthWest
    )
}

/// Puts `window`'s handles back over everything else in it, unless they
/// are there.
fn raise(window: &gtk::Window) {
    if window
        .last_child()
        .is_some_and(|child| child.is::<ResizeHandle>())
    {
        return;
    }
    for handle in handles(window) {
        handle.insert_before(window, None::<&gtk::Widget>);
    }
}

/// What `handle`'s window picks at `x`, `y` in the handle, while the
/// handle stands aside.
#[expect(clippy::cast_possible_truncation, reason = "window coordinates are small")]
fn under(handle: &ResizeHandle, x: f64, y: f64) -> Option<gtk::Widget> {
    let window = handle.parent()?;
    let point = handle.compute_point(&window, &gtk::graphene::Point::new(x as f32, y as f32))?;
    window.pick(
        f64::from(point.x()),
        f64::from(point.y()),
        gtk::PickFlags::DEFAULT,
    )
}

/// Whether `widget` is a scroll bar's thumb.
fn is_scroll_thumb(widget: &gtk::Widget) -> bool {
    widget.css_name() == "slider" && widget.ancestor(gtk::Scrollbar::static_type()).is_some()
}

/// The handles [`install`] added to `window`.
fn handles(window: &gtk::Window) -> Vec<ResizeHandle> {
    std::iter::successors(window.first_child(), WidgetExt::next_sibling)
        .filter_map(|child| child.downcast::<ResizeHandle>().ok())
        .collect()
}

/// Places `window`'s handles in its `width` by `height` content, turns
/// them on or off for its state, and widens the surface's input region.
/// Call it after GTK's own `size_allocate`, which resets that region.
pub(crate) fn allocate(window: &impl IsA<gtk::Window>, width: i32, height: i32) {
    let window = window.upcast_ref::<gtk::Window>();
    let handles = handles(window);
    if handles.is_empty() {
        return;
    }
    let frame = Frame::of(window);
    for handle in &handles {
        let (x, y, handle_width, handle_height) = handle_rect(handle.edge(), width, height);
        handle.size_allocate(&gtk::Allocation::new(x, y, handle_width, handle_height), -1);
        handle.set_can_target(frame.resizes_from(handle.edge()));
    }
    if frame.client_shadow {
        widen_input_region(window);
    }
}

/// Turns `window`'s handles on or off for its present state.
fn refresh(window: &gtk::Window) {
    let frame = Frame::of(window);
    for handle in handles(window) {
        handle.set_can_target(frame.resizes_from(handle.edge()));
    }
}

/// Where the handle on `edge` lies in a window `width` by `height`: its
/// x, y, width and height.
pub(crate) fn handle_rect(edge: gdk::SurfaceEdge, width: i32, height: i32) -> (i32, i32, i32, i32) {
    let corner = INSIDE_CORNER.min(width / 2).min(height / 2).max(0);
    let band = INSIDE_EDGE.min(width / 2).min(height / 2).max(0);
    let across = (width - 2 * corner).max(0);
    let down = (height - 2 * corner).max(0);
    match edge {
        gdk::SurfaceEdge::NorthWest => (0, 0, corner, corner),
        gdk::SurfaceEdge::North => (corner, 0, across, band),
        gdk::SurfaceEdge::NorthEast => (width - corner, 0, corner, corner),
        gdk::SurfaceEdge::East => (width - band, corner, band, down),
        gdk::SurfaceEdge::SouthEast => (width - corner, height - corner, corner, corner),
        gdk::SurfaceEdge::South => (corner, height - band, across, band),
        gdk::SurfaceEdge::SouthWest => (0, height - corner, corner, corner),
        _ => (0, corner, band, down),
    }
}

/// Lets presses up to [`OUTSIDE`] pixels beyond `window`'s edge reach it,
/// where GTK gave it 12 (`update_realized_window_properties`).
fn widen_input_region(window: &gtk::Window) {
    let Some(surface) = window.surface() else {
        return;
    };
    let Some(bounds) = window.compute_bounds(window) else {
        return;
    };
    let (offset_x, offset_y) = window.surface_transform();
    let x = whole_pixels(offset_x + f64::from(bounds.x())) - OUTSIDE;
    let y = whole_pixels(offset_y + f64::from(bounds.y())) - OUTSIDE;
    let width = whole_pixels(f64::from(bounds.width())) + 2 * OUTSIDE;
    let height = whole_pixels(f64::from(bounds.height())) + 2 * OUTSIDE;
    let rectangle = input_shape::RectangleInt::new(x, y, width, height);
    surface.set_input_region(Some(&input_shape::Region::create_rectangle(&rectangle)));
}

/// `value` rounded to whole pixels.
#[expect(clippy::cast_possible_truncation, reason = "window coordinates are small")]
fn whole_pixels(value: f64) -> i32 {
    value.round() as i32
}

/// What decides whether a window resizes from its edges: GTK's own rules
/// in `get_edge_for_coordinates`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Frame {
    /// The window can be resized, and is neither maximized nor full
    /// screen.
    pub(crate) free: bool,
    /// GTK draws the window's frame with a compositor's shadow (`.csd`),
    /// not as a solid border (`.solid-csd`) or by the window manager.
    pub(crate) client_shadow: bool,
    /// The surface's state: tiling and the compositor's edge constraints.
    pub(crate) state: gdk::ToplevelState,
    /// The compositor reports which edges may be resized.
    pub(crate) edge_constraints: bool,
}

impl Frame {
    /// The frame of `window` as it is now.
    fn of(window: &gtk::Window) -> Self {
        let toplevel = window.surface().and_downcast::<gdk::Toplevel>();
        Self {
            free: window.is_resizable() && !window.is_maximized() && !window.is_fullscreen(),
            client_shadow: window.has_css_class("csd"),
            state: toplevel
                .as_ref()
                .map_or_else(gdk::ToplevelState::empty, gdk::prelude::ToplevelExt::state),
            edge_constraints: toplevel
                .as_ref()
                .is_some_and(gdk::prelude::ToplevelExt::supports_edge_constraints),
        }
    }

    /// Whether the window resizes from `edge` now.
    pub(crate) fn resizes_from(self, edge: gdk::SurfaceEdge) -> bool {
        if !self.free || !self.client_shadow {
            return false;
        }
        if !self.edge_constraints {
            return !self.state.contains(gdk::ToplevelState::TILED);
        }
        let needed = resizable_flags(edge);
        self.state.contains(needed)
    }
}

/// The state flags that let the window resize from `edge`.
fn resizable_flags(edge: gdk::SurfaceEdge) -> gdk::ToplevelState {
    let top = gdk::ToplevelState::TOP_RESIZABLE;
    let right = gdk::ToplevelState::RIGHT_RESIZABLE;
    let bottom = gdk::ToplevelState::BOTTOM_RESIZABLE;
    let left = gdk::ToplevelState::LEFT_RESIZABLE;
    match edge {
        gdk::SurfaceEdge::NorthWest => top | left,
        gdk::SurfaceEdge::North => top,
        gdk::SurfaceEdge::NorthEast => top | right,
        gdk::SurfaceEdge::East => right,
        gdk::SurfaceEdge::SouthEast => bottom | right,
        gdk::SurfaceEdge::South => bottom,
        gdk::SurfaceEdge::SouthWest => bottom | left,
        _ => left,
    }
}

/// Turns `window`'s handles on as if it had a compositor's shadow and
/// could be resized from every edge, which no compositor-less test
/// display gives it.
#[cfg(test)]
pub(crate) fn turn_on_for_test(window: &impl IsA<gtk::Window>) {
    for handle in handles(window.upcast_ref()) {
        handle.set_can_target(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_RESIZABLE: gdk::ToplevelState = gdk::ToplevelState::TOP_RESIZABLE
        .union(gdk::ToplevelState::RIGHT_RESIZABLE)
        .union(gdk::ToplevelState::BOTTOM_RESIZABLE)
        .union(gdk::ToplevelState::LEFT_RESIZABLE);

    fn floating() -> Frame {
        Frame {
            free: true,
            client_shadow: true,
            state: ALL_RESIZABLE,
            edge_constraints: true,
        }
    }

    /// A floating window resizes from every edge and corner.
    #[test]
    fn a_floating_window_resizes_from_every_edge() {
        for edge in EDGES {
            assert!(floating().resizes_from(edge), "{edge:?}");
        }
    }

    /// No handle works on a window that cannot be resized, is maximized
    /// or full screen, or has GTK's solid frame or the window manager's.
    #[test]
    fn handles_stand_aside_where_gtk_resizes_otherwise_or_not_at_all() {
        let frames = [
            Frame {
                free: false,
                ..floating()
            },
            Frame {
                client_shadow: false,
                ..floating()
            },
        ];
        for frame in frames {
            for edge in EDGES {
                assert!(!frame.resizes_from(edge), "{frame:?} {edge:?}");
            }
        }
    }

    /// A window tiled to the left half resizes only from its free right
    /// edge, as the compositor's constraints say.
    #[test]
    fn a_tiled_window_resizes_only_from_its_free_edges() {
        let tiled = Frame {
            state: gdk::ToplevelState::TILED
                | gdk::ToplevelState::LEFT_TILED
                | gdk::ToplevelState::TOP_TILED
                | gdk::ToplevelState::BOTTOM_TILED
                | gdk::ToplevelState::RIGHT_RESIZABLE,
            ..floating()
        };
        let free: Vec<_> = EDGES
            .into_iter()
            .filter(|edge| tiled.resizes_from(*edge))
            .collect();
        assert_eq!(free, [gdk::SurfaceEdge::East]);

        let without_constraints = Frame {
            edge_constraints: false,
            ..tiled
        };
        for edge in EDGES {
            assert!(!without_constraints.resizes_from(edge), "{edge:?}");
        }
    }

    /// The bands are 4 pixels wide inside each edge, the corners 12-pixel
    /// squares, and together they ring the window without overlapping.
    #[test]
    fn the_handles_ring_the_window_inside_its_edge() {
        let (width, height) = (800, 600);
        assert_eq!(
            handle_rect(gdk::SurfaceEdge::NorthWest, width, height),
            (0, 0, 12, 12)
        );
        assert_eq!(
            handle_rect(gdk::SurfaceEdge::North, width, height),
            (12, 0, 776, 4)
        );
        assert_eq!(
            handle_rect(gdk::SurfaceEdge::NorthEast, width, height),
            (788, 0, 12, 12)
        );
        assert_eq!(
            handle_rect(gdk::SurfaceEdge::East, width, height),
            (796, 12, 4, 576)
        );
        assert_eq!(
            handle_rect(gdk::SurfaceEdge::SouthEast, width, height),
            (788, 588, 12, 12)
        );
        assert_eq!(
            handle_rect(gdk::SurfaceEdge::South, width, height),
            (12, 596, 776, 4)
        );
        assert_eq!(
            handle_rect(gdk::SurfaceEdge::SouthWest, width, height),
            (0, 588, 12, 12)
        );
        assert_eq!(
            handle_rect(gdk::SurfaceEdge::West, width, height),
            (0, 12, 4, 576)
        );
    }

    /// Each handle shows the resize cursor GTK shows for its edge.
    #[test]
    fn each_edge_has_its_resize_cursor() {
        let names: Vec<_> = EDGES.into_iter().map(cursor_name).collect();
        assert_eq!(
            names,
            [
                "nw-resize",
                "n-resize",
                "ne-resize",
                "e-resize",
                "se-resize",
                "s-resize",
                "sw-resize",
                "w-resize"
            ]
        );
    }
}
