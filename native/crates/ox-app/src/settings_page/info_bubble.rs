// SPDX-License-Identifier: AGPL-3.0-only
//! The ⓘ after a setting's name, and the bubble it opens with the details
//! the row's one line leaves out.
//!
//! Each row of the settings mockup is one short line; what it used to say
//! under its name now shows in a bubble when the pointer rests on the ⓘ,
//! when the keyboard reaches it, or when it is clicked. The bubble is a
//! `GtkPopover`, a surface of its own that the compositor places where it
//! fits, flipping above the ⓘ near the bottom of the screen, so the page's
//! scrolled area or the window's edge never cuts it off. Screen readers
//! read the details as the ⓘ's description, without opening anything.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::icons::{self, Icon};

mod imp {
    use std::cell::OnceCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::InfoButton`].
    #[derive(Debug, Default)]
    pub(crate) struct InfoButton {
        /// The bubble, a child the button places and lets go of itself.
        pub(super) popover: OnceCell<gtk::Popover>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InfoButton {
        const NAME: &'static str = "OxInfoButton";
        type Type = super::InfoButton;
        type ParentType = gtk::Button;
    }

    impl ObjectImpl for InfoButton {
        fn dispose(&self) {
            if let Some(popover) = self.popover.get() {
                popover.unparent();
            }
        }
    }

    impl WidgetImpl for InfoButton {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            // A popover is placed by the widget it belongs to.
            if let Some(popover) = self.popover.get() {
                popover.present();
            }
        }
    }

    impl ButtonImpl for InfoButton {}
}

glib::wrapper! {
    /// The ⓘ: a flat button that owns its bubble.
    pub(crate) struct InfoButton(ObjectSubclass<imp::InfoButton>)
        @extends gtk::Button, gtk::Widget,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

/// The ⓘ glyph.
const INFO_GLYPH: i32 = 14;
/// How wide the bubble's text grows, in characters, before it wraps.
const BUBBLE_CHARS: i32 = 44;

/// An ⓘ named after `title` whose bubble says `details`.
#[derive(Debug, Clone)]
pub(crate) struct InfoBubble {
    /// The ⓘ.
    button: InfoButton,
    /// The bubble.
    popover: gtk::Popover,
    /// The text in the bubble.
    text: gtk::Label,
}

impl InfoBubble {
    /// An ⓘ for the setting called `title`, saying `details`.
    pub(crate) fn new(title: &str, details: &str) -> Self {
        let text = gtk::Label::builder()
            .label(details)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(BUBBLE_CHARS)
            .css_classes(["info-bubble-text"])
            .build();
        // The bubble opens and closes with the pointer and the keyboard,
        // so it never takes the keyboard or a click from the page.
        let popover = gtk::Popover::builder()
            .child(&text)
            .autohide(false)
            .css_classes(["info-bubble"])
            .build();
        let button: InfoButton = glib::Object::new();
        button.set_child(Some(&icons::image(Icon::Info, INFO_GLYPH)));
        button.set_valign(gtk::Align::Center);
        button.add_css_class("info-button");
        popover.set_parent(&button);
        button
            .imp()
            .popover
            .set(popover.clone())
            .expect("a new button has no bubble yet");
        let name = ox_core::i18n::format_message("About {setting}", &[("setting", title)]);
        button.update_property(&[
            gtk::accessible::Property::Label(&name),
            gtk::accessible::Property::Description(details),
        ]);
        let bubble = Self {
            button,
            popover,
            text,
        };
        bubble.open_on_hover_and_focus();
        bubble
    }

    /// The ⓘ, to put after the setting's name.
    pub(crate) fn widget(&self) -> &InfoButton {
        &self.button
    }

    /// What the bubble says.
    pub(crate) fn details(&self) -> String {
        self.text.text().into()
    }

    /// Adds `more` to the bubble as a paragraph of its own.
    pub(crate) fn add_paragraph(&self, more: &str) {
        let details = format!("{}\n\n{more}", self.text.text());
        self.text.set_text(&details);
        self.button
            .update_property(&[gtk::accessible::Property::Description(&details)]);
    }

    /// Whether the bubble is open, for tests.
    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// The bubble, for tests.
    #[cfg(test)]
    pub(crate) fn popover(&self) -> &gtk::Popover {
        &self.popover
    }

    /// Opens the bubble while the pointer is on the ⓘ or the keyboard is,
    /// and when the ⓘ is clicked or tapped; it closes when both have left,
    /// and on Escape.
    fn open_on_hover_and_focus(&self) {
        let popover = &self.popover;
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter(glib::clone!(
            #[weak]
            popover,
            move |_, _, _| popover.popup()
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = button)]
            self.button,
            #[weak]
            popover,
            move |_| {
                if !button.has_focus() {
                    popover.popdown();
                }
            }
        ));
        self.button.add_controller(motion);
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(glib::clone!(
            #[weak]
            popover,
            move |_| popover.popup()
        ));
        focus.connect_leave(glib::clone!(
            #[weak]
            popover,
            move |_| popover.popdown()
        ));
        self.button.add_controller(focus);
        self.button.connect_clicked(glib::clone!(
            #[weak]
            popover,
            move |_| popover.popup()
        ));
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            popover,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == gdk::Key::Escape && popover.is_visible() {
                    popover.popdown();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        self.button.add_controller(keys);
    }
}
