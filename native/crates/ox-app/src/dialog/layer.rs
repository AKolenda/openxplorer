// SPDX-License-Identifier: AGPL-3.0-only
//! The in-window host of a dialog: a dimmed layer over everything under
//! the title bar, with one dialog centred on it.
//!
//! Ports `#modal-layer`, `showModal` and `closeModal` of
//! `v2.0.0:desktop/ui/app.js`, drawn as `native/docs/ui-spec.md` §4.11 says:
//! the scrim covers the window's content, as `positionTabDialog` places
//! the layer of a tab's Properties under the title bar, and the dialog is
//! at most the window's width less 30 pixels (`calc(100vw - 30px)`).
//!
//! While a dialog is shown, the content under the layer takes neither
//! clicks nor keyboard focus, and Escape dismisses the dialog (ACC-004).
//! The title bar stays usable, so a tab's Properties can be left for
//! another tab and found again (PROP-008); the window decides which
//! dialog belongs to which tab, and asks the layer to show or withdraw it.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::DialogFrame;

/// The space the layer keeps around a dialog on each side
/// (`.modal-layer{padding:25px}`, and `calc(100vw - 30px)` for the width).
const LAYER_MARGIN: i32 = 15;

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{DialogFrame, LAYER_MARGIN};

    /// Private state of [`super::DialogLayer`].
    #[derive(Debug, Default)]
    pub(crate) struct DialogLayer {
        /// The content under the layer, which must not take focus while a
        /// dialog is shown.
        pub(super) content: glib::WeakRef<gtk::Widget>,
        /// The dialog shown, if any.
        pub(super) shown: RefCell<Option<DialogFrame>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DialogLayer {
        const NAME: &'static str = "OxDialogLayer";
        type Type = super::DialogLayer;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("ox-dialog-layer");
        }
    }

    impl ObjectImpl for DialogLayer {
        fn constructed(&self) {
            self.parent_constructed();
            let layer = self.obj();
            layer.set_visible(false);
            layer.add_controller(super::escape_dismisses());
        }

        fn dispose(&self) {
            if let Some(frame) = self.shown.take() {
                frame.unparent();
            }
        }
    }

    impl WidgetImpl for DialogLayer {
        /// The layer never asks for room: it lies over the window's
        /// content and takes the size of it.
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        /// Centres the dialog, at most its preferred width and the room
        /// the layer has, and at most as tall as that room.
        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some(frame) = self.shown.borrow().clone() else {
                return;
            };
            let room_width = (width - 2 * LAYER_MARGIN).max(1);
            let room_height = (height - 2 * LAYER_MARGIN).max(1);
            let minimum_width = frame.measure(gtk::Orientation::Horizontal, -1).0;
            let frame_width = frame.width().pixels().min(room_width).max(minimum_width);
            let natural_height = frame.measure(gtk::Orientation::Vertical, frame_width).1;
            let frame_height = natural_height.min(room_height);
            let x = (width - frame_width) / 2;
            let y = (height - frame_height) / 2;
            let area = gtk::Allocation::new(x, y, frame_width, frame_height);
            frame.size_allocate(&area, -1);
        }
    }
}

glib::wrapper! {
    /// The dimmed layer that shows one in-window dialog at a time.
    pub(crate) struct DialogLayer(ObjectSubclass<imp::DialogLayer>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DialogLayer {
    /// Puts a new layer over the content of `window`, under its title bar.
    ///
    /// The window's child moves into an overlay whose overlay child is the
    /// layer, so the layer covers exactly the window's content.
    pub(crate) fn install_over(window: &impl IsA<gtk::Window>) -> Self {
        let layer: Self = glib::Object::new();
        let overlay = gtk::Overlay::new();
        if let Some(content) = window.child() {
            window.set_child(None::<&gtk::Widget>);
            overlay.set_child(Some(&content));
            layer.imp().content.set(Some(&content));
        }
        overlay.add_overlay(&layer);
        window.set_child(Some(&overlay));
        layer
    }

    /// Shows `frame` in place of the dialog shown now, which is withdrawn
    /// without closing, and moves keyboard focus into it.
    pub(crate) fn present(&self, frame: &DialogFrame) {
        self.withdraw();
        frame.set_parent(self);
        self.imp().shown.replace(Some(frame.clone()));
        self.set_content_available(false);
        self.set_visible(true);
        frame.child_focus(gtk::DirectionType::TabForward);
    }

    /// Takes the dialog shown off the layer without closing it, so it can
    /// be shown again, and hides the layer. `None` when none is shown.
    pub(crate) fn withdraw(&self) -> Option<DialogFrame> {
        let frame = self.imp().shown.take()?;
        frame.unparent();
        self.set_visible(false);
        self.set_content_available(true);
        Some(frame)
    }

    /// The dialog shown, if any.
    pub(crate) fn shown(&self) -> Option<DialogFrame> {
        self.imp().shown.borrow().clone()
    }

    /// Lets the content under the layer take focus again, or not.
    fn set_content_available(&self, available: bool) {
        if let Some(content) = self.imp().content.upgrade() {
            content.set_can_focus(available);
            content.set_can_target(available);
        }
    }
}

/// Escape dismisses the dialog shown (`Escape` in `showModal`'s keydown).
fn escape_dismisses() -> gtk::ShortcutController {
    crate::modal::on_escape(|widget| {
        let layer = widget
            .downcast_ref::<DialogLayer>()
            .expect("the Escape shortcut belongs to a dialog layer");
        let Some(frame) = layer.shown() else {
            return glib::Propagation::Proceed;
        };
        frame.close();
        glib::Propagation::Stop
    })
}
