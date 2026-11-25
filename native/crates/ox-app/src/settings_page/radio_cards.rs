// SPDX-License-Identifier: AGPL-3.0-only
//! A row of cards that wraps in a narrow window, whose radio buttons are
//! one group and one Tab stop, as Windows 11 Settings' theme choice is.
//!
//! The cards sit in a `GtkFlowBox`, which wraps them. Its focus handling
//! claims every Tab while a card follows the focused one, even when that
//! card's radio declines the keyboard, as a radio of a group does when
//! another is chosen (`gtk_flow_box_focus` returns true either way,
//! through GTK 4.20): Tab never left the cards. This widget holds the
//! flow box and takes Tab and Shift+Tab itself: from inside the group
//! they leave it, and from outside they land on the chosen radio, else
//! the first. The flow box's own arrow keys move a cursor between cards
//! that cannot take the keyboard, so they did nothing; the arrow keys now
//! move to the previous or next radio and choose it, as in a radio group.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

mod imp {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    #[derive(Debug, Default)]
    pub(crate) struct RadioCards {
        /// The radio buttons of the cards, one group.
        pub(super) radios: RefCell<Vec<gtk::CheckButton>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RadioCards {
        const NAME: &'static str = "OxRadioCards";
        type Type = super::RadioCards;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_accessible_role(gtk::AccessibleRole::RadioGroup);
        }
    }

    impl ObjectImpl for RadioCards {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for RadioCards {
        fn focus(&self, direction: gtk::DirectionType) -> bool {
            let widget = self.obj();
            match direction {
                gtk::DirectionType::TabForward | gtk::DirectionType::TabBackward => {
                    if widget.focus_child().is_some() {
                        // The group is one stop: Tab leaves it.
                        return false;
                    }
                    let radios = self.radios.borrow();
                    let chosen = radios.iter().find(|radio| radio.is_active()).or(radios.first());
                    chosen.is_some_and(|radio| radio.is_sensitive() && radio.grab_focus())
                }
                _ => widget
                    .first_child()
                    .is_some_and(|child| child.child_focus(direction)),
            }
        }
    }
}

glib::wrapper! {
    /// Cards whose radio buttons are one group and one Tab stop.
    pub(crate) struct RadioCards(ObjectSubclass<imp::RadioCards>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl RadioCards {
    /// Holds `cards`, whose cards show `radios`, one group.
    pub(crate) fn new(cards: &impl IsA<gtk::Widget>, radios: Vec<gtk::CheckButton>) -> Self {
        let widget: Self = glib::Object::new();
        cards.set_parent(&widget);
        widget.imp().radios.replace(radios);
        widget.choose_with_arrow_keys();
        widget
    }

    /// Left and Up choose the previous radio, Right and Down the next,
    /// wrapping round, and move the keyboard to it.
    fn choose_with_arrow_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = widget)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                if !modifiers.is_empty() {
                    return glib::Propagation::Proceed;
                }
                let step: isize = match key {
                    gdk::Key::Left | gdk::Key::KP_Left | gdk::Key::Up | gdk::Key::KP_Up => -1,
                    gdk::Key::Right | gdk::Key::KP_Right | gdk::Key::Down | gdk::Key::KP_Down => 1,
                    _ => return glib::Propagation::Proceed,
                };
                if widget.step_choice(step) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        self.add_controller(keys);
    }

    /// Chooses the radio `step` places from the one with the keyboard,
    /// wrapping round and passing over disabled ones; false when no radio
    /// has the keyboard.
    fn step_choice(&self, step: isize) -> bool {
        let radios = self.imp().radios.borrow().clone();
        let Some(current) = radios.iter().position(gtk::prelude::WidgetExt::has_focus) else {
            return false;
        };
        let count = radios.len().cast_signed();
        let mut index = current.cast_signed();
        for _ in 1..count {
            index = (index + step).rem_euclid(count);
            let radio = &radios[index.cast_unsigned()];
            if radio.is_sensitive() && radio.is_visible() {
                radio.grab_focus();
                radio.activate();
                break;
            }
        }
        true
    }
}
