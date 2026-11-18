// SPDX-License-Identifier: AGPL-3.0-only
//! The room a tab gives its title: everything between the icon and the
//! close button (or the tab's end). The title keeps its whole width there
//! and the room cuts it at the last pixel when it runs out, as Windows 11
//! cuts a tab's title, instead of shortening it to an ellipsis. At the
//! tab's narrowest the room keeps [`super::TITLE_MIN_CHARS`] average
//! characters, which is what the tab strip shrinks tabs to.

use gtk::glib;
use gtk::prelude::*;

mod imp {
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::TitleLayout`]: none.
    #[derive(Debug, Default)]
    pub(crate) struct TitleLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for TitleLayout {
        const NAME: &'static str = "OxTabTitleLayout";
        type Type = super::TitleLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for TitleLayout {}

    impl LayoutManagerImpl for TitleLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(
            &self,
            room: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let Some(title) = room.first_child() else {
                return (0, 0, -1, -1);
            };
            let (minimum, natural, baseline, natural_baseline) = title.measure(orientation, for_size);
            if orientation == gtk::Orientation::Vertical {
                return (minimum, natural, baseline, natural_baseline);
            }
            (super::least_width(&title).min(natural), natural, -1, -1)
        }

        fn allocate(&self, room: &gtk::Widget, width: i32, height: i32, baseline: i32) {
            if let Some(title) = room.first_child() {
                // Never narrower than the whole title: the room cuts it.
                let (_, natural, _, _) = title.measure(gtk::Orientation::Horizontal, -1);
                title.allocate(width.max(natural), height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    /// Lays a tab's title out at its whole width in a room that may be
    /// narrower, which cuts it.
    pub(crate) struct TitleLayout(ObjectSubclass<imp::TitleLayout>)
        @extends gtk::LayoutManager;
}

/// `title` in the room its tab gives it: as wide as the tab allows, and
/// cut at the last pixel there.
pub(super) fn clipped(title: &gtk::Label) -> gtk::Box {
    let room = gtk::Box::builder()
        .hexpand(true)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["tab-title-room"])
        .build();
    room.set_layout_manager(Some(glib::Object::new::<TitleLayout>()));
    room.append(title);
    room
}

/// The width of [`super::TITLE_MIN_CHARS`] average characters in
/// `title`'s font.
fn least_width(title: &gtk::Widget) -> i32 {
    let metrics = title.pango_context().metrics(None, None);
    let average = metrics
        .approximate_char_width()
        .max(metrics.approximate_digit_width());
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a few characters' width in pixels"
    )]
    let width = gtk::pango::units_to_double(average.saturating_mul(super::TITLE_MIN_CHARS)).ceil() as i32;
    width
}
