// SPDX-License-Identifier: AGPL-3.0-only
//! The small window that follows a tab dragged out of its strip, as in
//! Chrome (TAB-034): the tab's icon and title over a live picture of the
//! window's contents, a quarter of its size or so. Over another window's
//! strip the tab itself follows the pointer instead ([`carried_tab`]).
//!
//! GTK cannot move a real window with the pointer during a drag on Wayland
//! (it has no `xdg-toplevel-drag-v1`), so the drag icon stands in for it.
//! [`Scaled`] gives the picture its reduced size: a `GtkPicture` asks for
//! its paintable's own size, which a window's contents would make as large
//! as the window.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::{tab_icon, TabView};

/// How wide the window that follows a dragged tab is.
const WIDTH: i32 = 320;

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::gdk;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::Scaled`].
    #[derive(Debug, Default)]
    pub(crate) struct Scaled {
        /// What is drawn, at any size.
        pub(super) inner: RefCell<Option<gdk::Paintable>>,
        /// The size it asks for.
        pub(super) size: Cell<(i32, i32)>,
    }

    #[gtk::glib::object_subclass]
    impl ObjectSubclass for Scaled {
        const NAME: &'static str = "OxScaledPaintable";
        type Type = super::Scaled;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for Scaled {}

    impl gdk::subclass::paintable::PaintableImpl for Scaled {
        fn intrinsic_width(&self) -> i32 {
            self.size.get().0
        }

        fn intrinsic_height(&self) -> i32 {
            self.size.get().1
        }

        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            if let Some(inner) = self.inner.borrow().as_ref() {
                inner.snapshot(snapshot, width, height);
            }
        }
    }
}

glib::wrapper! {
    /// A paintable drawn at a fixed size of its own, scaling another one,
    /// whose changes it passes on.
    pub(crate) struct Scaled(ObjectSubclass<imp::Scaled>)
        @implements gdk::Paintable;
}

impl Scaled {
    /// `inner`, asking for `width` × `height` pixels.
    fn new(inner: &impl IsA<gdk::Paintable>, width: i32, height: i32) -> Self {
        let scaled: Self = glib::Object::new();
        scaled.imp().inner.replace(Some(inner.clone().upcast()));
        scaled.imp().size.set((width, height));
        inner.connect_invalidate_contents(glib::clone!(
            #[weak]
            scaled,
            move |_| scaled.invalidate_contents()
        ));
        scaled
    }
}

/// The tab `tab` itself, `width` wide, as it follows the pointer over
/// another window's strip, which opens a gap for it.
pub(super) fn carried_tab(tab: &TabView, width: i32) -> gtk::Widget {
    let carried = gtk::Box::builder()
        .spacing(super::ICON_TO_TITLE)
        .width_request(width)
        .css_classes(["tab", "active", "tab-drag-tab"])
        .build();
    carried.append(&tab_icon(tab.icon));
    carried.append(
        &gtk::Label::builder()
            .label(&tab.title)
            .xalign(0.0)
            .hexpand(true)
            .max_width_chars(1)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["tab-title"])
            .build(),
    );
    carried.upcast()
}

/// The window that follows tab `tab` out of its strip: its icon and title
/// over a live picture of `content`, the window's contents.
pub(super) fn carried_window(tab: &TabView, content: &gtk::Widget) -> gtk::Widget {
    let window = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["tab-drag-window"])
        .build();
    let header = gtk::Box::builder()
        .spacing(super::ICON_TO_TITLE)
        .css_classes(["tab-drag-header"])
        .build();
    header.append(&tab_icon(tab.icon));
    header.append(
        &gtk::Label::builder()
            .label(&tab.title)
            .xalign(0.0)
            .hexpand(true)
            .max_width_chars(1)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    window.append(&header);
    let (width, height) = (content.width().max(1), content.height().max(1));
    let live = gtk::WidgetPaintable::new(Some(content));
    let picture = gtk::Picture::for_paintable(&Scaled::new(&live, WIDTH, WIDTH * height / width));
    picture.set_can_shrink(false);
    window.append(&picture);
    window.upcast()
}
