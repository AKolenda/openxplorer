// SPDX-License-Identifier: AGPL-3.0-only
//! A compact drop-down: a bordered button showing the chosen option, whose
//! options open in a list under it.
//!
//! Replaces the `<select>` elements of the Python settings page
//! (`renderSettingsPage` and `textSizeControls` in `desktop/ui/app.js`).
//! GTK's own drop-down draws its arrow and check mark with icons from the
//! desktop theme, which the app does not use (`native/README.md`, Icons),
//! so the button shows the bundled chevron and the list the bundled check
//! mark. A [`ChoiceButton`] is the `GtkMenuButton` a row shows and the
//! [`ChoiceList`] popover it opens, which holds the chosen option.

use std::cell::{Cell, OnceCell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, Icon};

/// The button's chevron (`.select .ico{width:12px}` in the mockup).
const CHEVRON_GLYPH: i32 = 12;
/// The check mark before the chosen option.
const CHECK_GLYPH: i32 = 16;
/// The class of the chosen option's row.
const CHOSEN_CLASS: &str = "chosen";

#[expect(
    unreachable_pub,
    reason = "the glib::Properties derive always makes the selected property's accessors pub"
)]
mod imp {
    use super::{Cell, OnceCell, RefCell};
    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::ChoiceList`].
    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ChoiceList)]
    pub(crate) struct ChoiceList {
        /// The position of the chosen option.
        #[property(get, set = Self::choose, explicit_notify)]
        pub(super) selected: Cell<u32>,
        /// The options' labels.
        pub(super) options: RefCell<Vec<String>>,
        /// The options, one row each, built by `constructed`.
        pub(super) list: OnceCell<gtk::ListBox>,
        /// The chosen option's label on the button.
        pub(super) button_label: OnceCell<gtk::Label>,
        /// The button that opens the list; it holds the list, so the list
        /// holds it weakly.
        pub(super) button: glib::WeakRef<gtk::MenuButton>,
    }

    impl ChoiceList {
        /// Chooses the option at `position`; announces it only when the
        /// choice changed, so showing the saved value saves nothing.
        fn choose(&self, position: u32) {
            if self.selected.replace(position) == position {
                return;
            }
            self.obj().show_choice();
            self.obj().notify_selected();
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ChoiceList {
        const NAME: &'static str = "OxChoiceList";
        type Type = super::ChoiceList;
        type ParentType = gtk::Popover;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ChoiceList {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for ChoiceList {}
    impl PopoverImpl for ChoiceList {}
}

glib::wrapper! {
    /// The options of a compact drop-down, and which one is chosen.
    pub(crate) struct ChoiceList(ObjectSubclass<imp::ChoiceList>)
        @extends gtk::Popover, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::ShortcutManager;
}

/// A compact drop-down: the button a row shows and the list it opens.
#[derive(Debug, Clone)]
pub(crate) struct ChoiceButton {
    /// The bordered button with the chosen option and a chevron.
    pub button: gtk::MenuButton,
    /// The options, and which one is chosen.
    pub choices: ChoiceList,
}

impl ChoiceButton {
    /// A drop-down of the options `labels`, the first chosen.
    pub(crate) fn new(labels: &[String]) -> Self {
        let choices: ChoiceList = glib::Object::new();
        choices.imp().options.replace(labels.to_vec());
        choices.fill_list();
        let button = choices.new_button();
        choices.show_choice();
        Self { button, choices }
    }
}

impl ChoiceList {
    /// The popover's list and look.
    fn build(&self) {
        self.set_has_arrow(false);
        self.set_halign(gtk::Align::End);
        self.set_offset(0, 4);
        self.add_css_class("choice-list");
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(true)
            .accessible_role(gtk::AccessibleRole::ListBox)
            .build();
        list.connect_row_activated(glib::clone!(
            #[weak(rename_to = choices)]
            self,
            move |_, row| {
                let position = u32::try_from(row.index()).unwrap_or_default();
                choices.set_selected(position);
                choices.popdown();
            }
        ));
        self.set_child(Some(&list));
        // The chosen option has keyboard focus when the list opens.
        self.connect_show(ChoiceList::focus_chosen_row);
        self.imp()
            .list
            .set(list)
            .expect("constructed runs once per object");
    }

    /// A bordered button with the chosen option's label and a chevron,
    /// which opens the list.
    fn new_button(&self) -> gtk::MenuButton {
        let label = gtk::Label::builder().xalign(0.0).hexpand(true).build();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.append(&label);
        content.append(&icons::image(Icon::ChevronDown16, CHEVRON_GLYPH));
        self.imp()
            .button_label
            .set(label)
            .expect("a new choice list has no button yet");
        let button = gtk::MenuButton::builder()
            .child(&content)
            .popover(self)
            .valign(gtk::Align::Center)
            .css_classes(["choice-button"])
            .build();
        self.imp().button.set(Some(&button));
        button
    }

    /// One row per option: the check mark's place, then the label.
    fn fill_list(&self) {
        let list = self.list();
        for option in self.imp().options.borrow().iter() {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            content.append(&icons::image(Icon::Checkmark, CHECK_GLYPH));
            let label = gtk::Label::builder().label(option).xalign(0.0).build();
            content.append(&label);
            let row = gtk::ListBoxRow::builder()
                .child(&content)
                .accessible_role(gtk::AccessibleRole::Option)
                .build();
            row.update_property(&[gtk::accessible::Property::Label(option)]);
            list.append(&row);
        }
    }

    fn list(&self) -> &gtk::ListBox {
        self.imp().list.get().expect("constructed builds the list")
    }

    /// The chosen option's label.
    pub(crate) fn chosen_label(&self) -> String {
        let position = self.selected() as usize;
        let options = self.imp().options.borrow();
        options.get(position).cloned().unwrap_or_default()
    }

    /// Shows the chosen option on the button and marks its row. A screen
    /// reader hears it as the button's description, after the row's title
    /// that names the button: "Text size, 100% (default)".
    fn show_choice(&self) {
        let chosen = self.chosen_label();
        if let Some(label) = self.imp().button_label.get() {
            label.set_text(&chosen);
        }
        if let Some(button) = self.imp().button.upgrade() {
            button.update_property(&[gtk::accessible::Property::Description(&chosen)]);
        }
        let selected = self.selected();
        let mut position = 0;
        while let Some(row) = self.list().row_at_index(position) {
            let is_chosen = u32::try_from(position).is_ok_and(|index| index == selected);
            if is_chosen {
                row.add_css_class(CHOSEN_CLASS);
            } else {
                row.remove_css_class(CHOSEN_CLASS);
            }
            let state = gtk::accessible::State::Selected(Some(is_chosen));
            row.update_state(&[state]);
            position += 1;
        }
    }

    /// Chooses the option labelled `label` from the list, as a click on
    /// its row does, for tests.
    ///
    /// # Panics
    ///
    /// When no option is labelled `label`.
    #[cfg(test)]
    pub(crate) fn choose_labelled(&self, label: &str) {
        let options = self.imp().options.borrow().clone();
        let position = options
            .iter()
            .position(|option| option == label)
            .unwrap_or_else(|| panic!("the drop-down offers {label:?}"));
        let row = self
            .list()
            .row_at_index(i32::try_from(position).expect("a few options"))
            .expect("every option has a row");
        row.activate();
    }

    fn focus_chosen_row(&self) {
        let position = i32::try_from(self.selected()).unwrap_or_default();
        if let Some(row) = self.list().row_at_index(position) {
            row.grab_focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GTK cannot read an accessible description back, so the test checks
    /// that the button has one; `show_choice` sets it to the chosen label.
    #[gtk::test]
    fn the_button_describes_the_chosen_option_to_screen_readers() {
        let labels = ["30 seconds".to_owned(), "1 minute".to_owned()];
        let drop_down = ChoiceButton::new(&labels);
        // Choosing closes the list, which needs a window to hand focus to.
        let window = gtk::Window::builder().child(&drop_down.button).build();

        drop_down.choices.choose_labelled("1 minute");

        assert_eq!(drop_down.choices.chosen_label(), "1 minute");
        assert!(gtk::test_accessible_has_property(
            &drop_down.button,
            gtk::AccessibleProperty::Description
        ));
        window.destroy();
    }
}
