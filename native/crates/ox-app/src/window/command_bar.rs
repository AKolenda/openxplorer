// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar under the navigation row.
//!
//! Ports `section.commandbar` in `v2.0.0:desktop/ui/index.html` and its menus in
//! `setup()` and `openNewMenu` of `v2.0.0:desktop/ui/app.js`, in the same order:
//! New ▾ │ Cut, Copy, Paste, Rename, Copy path, Move to Trash │ Sort ▾,
//! View ▾, More options, then at the right the appearance toggle, Settings
//! and the Details toggle. Every control runs a window or application
//! action. New is disabled where nothing can be created, and Delete is named after what
//! it does in the folder: "Move to Trash" or "Delete permanently"
//! (CMD-003).
//!
//! [`CommandBar`] is a `GtkBox` subclass. The template
//! `resources/ui/command-bar.ui` lays out the bar and the three controls at
//! its right; the file commands come from [`EDIT_COMMANDS`] and the menus
//! of [`menus`].

mod menus;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::Appearance;

use crate::icons::{self, Icon};
use crate::theme::AppearanceExt;

use super::breakpoints::WindowWidth;
use super::menu_popover::{name_menu_button, MenuEntry, MenuPopover};
use super::window_action::WindowAction;

use menus::{appearance_items, more_menu};
pub(super) use menus::{new_menu, sort_menu, view_menu};

/// The glyph of an icon-only command: 16 pixels, as Windows 11 draws its
/// command bar (ui-spec.md I01; the web app's were 18).
const ICON_COMMAND_GLYPH: i32 = 16;

/// The glyph of a command with a label, and the chevron of a menu.
const TEXT_COMMAND_GLYPH: i32 = 17;

/// Whether a command stays in a compact window (the 680-pixel rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InCompactWindow {
    /// Shown at every width.
    Kept,
    /// Hidden at 680 pixels or less (`.commandbar #cut{display:none}`).
    Hidden,
}

/// An icon-only command (`button.command` in index.html).
#[derive(Debug)]
struct IconCommand {
    glyph: Icon,
    action: WindowAction,
    /// The accessible name (`aria-label`).
    name: &'static str,
    /// The tooltip (`title`).
    tooltip: &'static str,
    compact: InCompactWindow,
}

/// Cut to Move to Trash, as index.html lists them.
const EDIT_COMMANDS: [IconCommand; 6] = [
    IconCommand {
        glyph: Icon::Cut,
        action: WindowAction::Cut,
        name: crate::i18n::message_id("Cut"),
        tooltip: crate::i18n::message_id("Cut (Ctrl+X)"),
        compact: InCompactWindow::Hidden,
    },
    IconCommand {
        glyph: Icon::Copy,
        action: WindowAction::Copy,
        name: crate::i18n::message_id("Copy"),
        tooltip: crate::i18n::message_id("Copy (Ctrl+C)"),
        compact: InCompactWindow::Kept,
    },
    IconCommand {
        glyph: Icon::ClipboardPaste,
        action: WindowAction::Paste,
        name: crate::i18n::message_id("Paste"),
        tooltip: crate::i18n::message_id("Paste files (Ctrl+V)"),
        compact: InCompactWindow::Kept,
    },
    IconCommand {
        glyph: Icon::Rename,
        action: WindowAction::Rename,
        name: crate::i18n::message_id("Rename"),
        tooltip: crate::i18n::message_id("Rename (F2)"),
        compact: InCompactWindow::Hidden,
    },
    IconCommand {
        glyph: Icon::Share,
        action: WindowAction::CopyPath,
        name: crate::i18n::message_id("Copy path"),
        tooltip: crate::i18n::message_id("Copy path (does not change sharing permissions)"),
        compact: InCompactWindow::Hidden,
    },
    IconCommand {
        glyph: Icon::Delete,
        action: WindowAction::Trash,
        name: crate::i18n::message_id("Move to Trash"),
        tooltip: crate::i18n::message_id("Move to Trash (Delete)"),
        compact: InCompactWindow::Kept,
    },
];

/// The tooltip of Settings.
const SETTINGS_TOOLTIP: &str = crate::i18n::message_id("Settings (Ctrl+,)");

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::CommandBar`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/command-bar.ui")]
    pub(crate) struct CommandBar {
        /// New to More options, filled from the tables.
        #[template_child]
        pub(super) file_commands: TemplateChild<gtk::Box>,
        /// The appearance toggle (`#theme-toggle`).
        #[template_child]
        pub(super) appearance_button: TemplateChild<gtk::MenuButton>,
        /// The sun or moon of the drawn appearance.
        #[template_child]
        pub(super) appearance_glyph: TemplateChild<gtk::Image>,
        /// "Light" or "Dark".
        #[template_child]
        pub(super) appearance_label: TemplateChild<gtk::Label>,
        /// Opens the Settings page.
        #[template_child]
        pub(super) settings_button: TemplateChild<gtk::Button>,
        /// Shows whether the details pane is open.
        #[template_child]
        pub(super) details_toggle: TemplateChild<gtk::ToggleButton>,
        /// The glyph of [`Self::details_toggle`].
        #[template_child]
        pub(super) details_glyph: TemplateChild<gtk::Image>,
        /// Cut, Rename, Copy path and Details, which a compact window
        /// hides.
        pub(super) hidden_when_compact: RefCell<Vec<gtk::Widget>>,
        /// New ▾, disabled where nothing can be created.
        pub(super) new_button: OnceCell<gtk::MenuButton>,
        /// Delete, labelled for the folder.
        pub(super) delete_button: OnceCell<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CommandBar {
        const NAME: &'static str = "OxCommandBar";
        type Type = super::CommandBar;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(bar: &glib::subclass::InitializingObject<Self>) {
            bar.init_template();
        }
    }

    impl ObjectImpl for CommandBar {
        fn constructed(&self) {
            self.parent_constructed();
            crate::i18n::translate_template(&*self.obj(), "command-bar.ui");
            let bar = self.obj();
            bar.add_file_commands();
            bar.finish_right_commands();
        }
    }

    impl WidgetImpl for CommandBar {}
    impl BoxImpl for CommandBar {}
}

glib::wrapper! {
    /// The command bar, showing the light appearance until told otherwise.
    pub(crate) struct CommandBar(ObjectSubclass<imp::CommandBar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl CommandBar {
    /// New ▾ │ Cut … Move to Trash │ Sort ▾, View ▾ and More options,
    /// remembering the commands a compact window hides.
    fn add_file_commands(&self) {
        let imp = self.imp();
        let group = &*imp.file_commands;
        let new_button = text_menu_button(
            &ox_core::i18n::gettext("New"),
            Icon::Add,
            "new-command",
            new_menu(Vec::new()),
        );
        group.append(&new_button);
        imp.new_button
            .set(new_button)
            .expect("constructed runs once per object");
        group.append(&separator());
        for command in &EDIT_COMMANDS {
            let button = icon_button(command);
            if command.compact == InCompactWindow::Hidden {
                imp.hidden_when_compact.borrow_mut().push(button.clone().upcast());
            }
            if command.action == WindowAction::Trash {
                imp.delete_button
                    .set(button.clone())
                    .expect("the bar has one Delete");
            }
            group.append(&button);
        }
        group.append(&separator());
        group.append(&text_menu_button(
            &ox_core::i18n::gettext("Sort"),
            Icon::ArrowSort,
            "sort-command",
            sort_menu(),
        ));
        group.append(&text_menu_button(
            &ox_core::i18n::gettext("View"),
            Icon::Grid,
            "view-command",
            view_menu(),
        ));
        group.append(&more_button());
    }

    /// Gives the appearance toggle, Settings and the Details toggle, which
    /// the template places at the right, what it cannot express.
    fn finish_right_commands(&self) {
        self.finish_appearance_button();
        self.finish_settings_button();
        self.finish_details_toggle();
    }

    /// The appearance menu, and the light appearance until the window
    /// shows the skin's.
    fn finish_appearance_button(&self) {
        let appearance_menu = MenuPopover::new(appearance_items().to_vec());
        self.imp().appearance_button.set_popover(Some(&appearance_menu));
        self.show_appearance_glyph(Appearance::Light);
    }

    /// The gear and its tooltip.
    fn finish_settings_button(&self) {
        let settings = &*self.imp().settings_button;
        settings.set_child(Some(&icons::image(Icon::Settings, ICON_COMMAND_GLYPH)));
        settings.set_tooltip_text(Some(&ox_core::i18n::gettext(ox_core::i18n::gettext_static(
            SETTINGS_TOOLTIP,
        ))));
        WindowAction::Settings.assign_to(settings);
    }

    /// The pane glyph and the `win.details-pane` toggle; a compact window
    /// hides the button.
    fn finish_details_toggle(&self) {
        let imp = self.imp();
        icons::set_icon(&imp.details_glyph, Icon::PanelRight, TEXT_COMMAND_GLYPH);
        WindowAction::DetailsPane.assign_to(&*imp.details_toggle);
        let details_toggle = imp.details_toggle.get().upcast();
        imp.hidden_when_compact.borrow_mut().push(details_toggle);
    }

    /// Shows `appearance`'s sun or moon and its "Light" or "Dark" label.
    fn show_appearance_glyph(&self, appearance: Appearance) {
        let imp = self.imp();
        icons::set_icon(&imp.appearance_glyph, appearance.icon(), TEXT_COMMAND_GLYPH);
        imp.appearance_label
            .set_text(&ox_core::i18n::gettext(appearance.label()));
    }

    /// Shows the drawn appearance on the theme button: a sun and "Light"
    /// or a moon and "Dark", with `tooltip` saying what was chosen
    /// (`applyTheme` in app.js).
    pub(super) fn show_appearance(&self, appearance: Appearance, tooltip: &str) {
        self.show_appearance_glyph(appearance);
        self.imp().appearance_button.set_tooltip_text(Some(tooltip));
    }

    /// Enables or disables New ▾ (`$('new').disabled` in app.js).
    /// The New button's menu.
    pub(super) fn new_menu_popover(&self) -> Option<MenuPopover> {
        let button = self.imp().new_button.get()?;
        button.popover().and_downcast::<MenuPopover>()
    }

    pub(super) fn set_new_enabled(&self, enabled: bool) {
        if let Some(button) = self.imp().new_button.get() {
            button.set_sensitive(enabled);
        }
    }

    /// Names Delete `label`, "Move to Trash" or "Delete permanently", in
    /// its tooltip and for screen readers (`updateToolbar`).
    pub(super) fn show_delete_label(&self, label: &str) {
        let Some(button) = self.imp().delete_button.get() else {
            return;
        };
        button.set_tooltip_text(Some(&ox_core::i18n::format_message(
            "{label} (Delete)",
            &[("label", &(label).to_string())],
        )));
        button.update_property(&[gtk::accessible::Property::Label(label)]);
    }

    /// Hides what the web layout hides in a window of `band`'s width: the
    /// appearance label from 1050 pixels, and Cut, Rename, Copy path and
    /// Details from 680.
    pub(super) fn fit_to_width(&self, band: WindowWidth) {
        let imp = self.imp();
        imp.appearance_label.set_visible(band.shows_appearance_label());
        for control in imp.hidden_when_compact.borrow().iter() {
            control.set_visible(!band.is_compact());
        }
    }

    /// The theme button's tooltip, for tests.
    #[cfg(test)]
    pub(super) fn appearance_tooltip(&self) -> Option<String> {
        let tooltip = self.imp().appearance_button.tooltip_text();
        tooltip.map(|text| text.to_string())
    }
}

fn separator() -> gtk::Separator {
    gtk::Separator::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .build()
}

fn icon_button(command: &IconCommand) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::image(command.glyph, ICON_COMMAND_GLYPH))
        .tooltip_text(ox_core::i18n::gettext(command.tooltip))
        .action_name(command.action.detailed_name())
        .valign(gtk::Align::Center)
        .css_classes(["command"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
        command.name,
    ))]);
    button
}

/// A glyph, a label and the chevron that marks a menu (`setButton` with
/// `arrow`).
fn text_menu_content(label: &str, glyph: Icon) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    content.append(&icons::image(glyph, TEXT_COMMAND_GLYPH));
    content.append(&gtk::Label::new(Some(label)));
    let chevron = icons::image(Icon::ChevronDown, TEXT_COMMAND_GLYPH);
    chevron.add_css_class("chevron");
    content.append(&chevron);
    content
}

/// A command with a label that opens `entries`; `css_class` names it for
/// the stylesheet and the tests.
fn text_menu_button(label: &str, glyph: Icon, css_class: &str, entries: Vec<MenuEntry>) -> gtk::MenuButton {
    gtk::MenuButton::builder()
        .child(&text_menu_content(label, glyph))
        .popover(&MenuPopover::new(entries))
        .valign(gtk::Align::Center)
        .css_classes(["command", "text-command", css_class])
        .build()
}

fn more_button() -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .child(&icons::image(Icon::MoreHorizontal, ICON_COMMAND_GLYPH))
        .tooltip_text(ox_core::i18n::gettext("More options"))
        .popover(&MenuPopover::new(more_menu()))
        .valign(gtk::Align::Center)
        .css_classes(["command", "more-command"])
        .build();
    name_menu_button(&button, &ox_core::i18n::gettext("More options"));
    button
}

#[cfg(test)]
mod translation_tests {
    use ox_core::i18n::{Catalog, DOMAIN};

    use super::*;
    use crate::i18n::translate_properties;

    fn label(widget: &gtk::Widget) -> Option<gtk::Label> {
        if widget.buildable_id().as_deref() == Some("i18n_1") {
            return widget.clone().downcast().ok();
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if let Some(found) = label(&current) {
                return Some(found);
            }
        }
        None
    }

    /// A real composite-template widget uses the selected catalogue for
    /// static text. Metacharacters stay literal, and entry data stays intact.
    ///
    /// parity: INT-031
    #[gtk::test]
    fn marked_template_text_uses_the_selected_catalogue() {
        let temporary = tempfile::tempdir().expect("a private locale folder");
        let locale = temporary.path().join("test/LC_MESSAGES");
        std::fs::create_dir_all(&locale).expect("the test locale");
        let message = "Details";
        let translated = "Locale <&> details";
        let bytes = one_message_catalogue(message, translated);
        std::fs::write(locale.join(format!("{DOMAIN}.mo")), bytes).expect("the test catalogue");
        let catalog = Catalog::find(DOMAIN, &[temporary.path().to_owned()], &["test".to_owned()])
            .expect("the requested locale is found");
        let bar: CommandBar = glib::Object::new();
        let entry = gtk::Entry::new();
        entry.set_text(message);
        bar.append(&entry);
        translate_properties(bar.upcast_ref(), "command-bar.ui", &|message| {
            catalog.gettext(message)
        });
        let label = label(bar.upcast_ref()).expect("the actual template label");
        assert_eq!(label.text(), translated);
        assert_eq!(entry.text(), message, "entry contents are never translated");
    }

    /// A one-message GNU MO fixture, with an obvious sentinel rather
    /// than an invented translation committed as a real language.
    fn one_message_catalogue(message: &str, translated: &str) -> Vec<u8> {
        let length = |text: &str| u32::try_from(text.len()).expect("a tiny test message");
        let mut bytes = Vec::new();
        let header = [
            0x9504_12de,
            0,
            1,
            28,
            36,
            0,
            0,
            length(message),
            44,
            length(translated),
            45 + length(message),
        ];
        for word in header {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(message.as_bytes());
        bytes.push(0);
        bytes.extend(translated.as_bytes());
        bytes.push(0);
        bytes
    }
}
