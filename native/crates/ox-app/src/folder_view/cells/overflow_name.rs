// SPDX-License-Identifier: AGPL-3.0-only
//! Holds an icon tile's name (Extra large, Large and Medium icons) and
//! lets a long one run below its tile, as Windows 11 File Explorer does.
//!
//! Measured on Windows 11's captures (`RefTest`, 100%): an unselected long
//! name shows up to four lines in Large and Medium icons, cut with "..."
//! after the fourth, and a name that fits in three lines shows whole in
//! Extra large, while the rows stay 129 and 289 pixels apart; the lines
//! past the second run into the gap under the row. This frame asks the
//! tile for at most two lines, so the rows keep Windows' pitch, and gives
//! the name its whole height to draw in.

use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Lines of name the tile makes room for; the rest draw below it.
const ROOM_LINES: i32 = 2;

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub(crate) struct OverflowName {
        pub(super) label: RefCell<Option<gtk::Label>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OverflowName {
        const NAME: &'static str = "OxOverflowName";
        type Type = super::OverflowName;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for OverflowName {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_overflow(gtk::Overflow::Visible);
        }

        fn dispose(&self) {
            if let Some(label) = self.label.take() {
                label.unparent();
            }
        }
    }

    impl WidgetImpl for OverflowName {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(label) = self.label.borrow().clone() else {
                return (0, 0, -1, -1);
            };
            let (minimum, natural, _, _) = label.measure(orientation, for_size);
            if orientation == gtk::Orientation::Horizontal {
                return (minimum, natural, -1, -1);
            }
            let room = super::room_for(&label, for_size, natural);
            (minimum.min(room), natural.min(room), -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some(label) = self.label.borrow().clone() else {
                return;
            };
            // The name keeps its whole height, below the frame if need be.
            let (_, natural, _, _) = label.measure(gtk::Orientation::Vertical, width);
            label.allocate(width, natural.max(height), -1, None);
        }
    }
}

/// The height of [`ROOM_LINES`] lines of `label` at `width` pixels,
/// whose name is `natural` pixels tall there: as tall as the name when it
/// takes no more lines.
fn room_for(label: &gtk::Label, width: i32, natural: i32) -> i32 {
    if width <= 0 {
        return natural;
    }
    let layout = label.layout().copy();
    layout.set_width(width.saturating_mul(pango::SCALE));
    let lines = layout.line_count().max(1);
    if lines < ROOM_LINES {
        return natural;
    }
    // Windows' box around two lines runs 2 pixels past its row; the
    // name's second line does here instead, so the rows keep their pitch.
    natural * ROOM_LINES / lines - SPILL
}

/// Pixels of a two-line name's box past the row Windows spaces it in.
const SPILL: i32 = 2;

glib::wrapper! {
    /// An icon tile's name, which may run below the tile.
    pub(crate) struct OverflowName(ObjectSubclass<imp::OverflowName>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl OverflowName {
    /// Holds `label`.
    pub(crate) fn new(label: &gtk::Label) -> Self {
        let frame: Self = glib::Object::new();
        label.set_parent(&frame);
        frame.imp().label.replace(Some(label.clone()));
        frame
    }
}
