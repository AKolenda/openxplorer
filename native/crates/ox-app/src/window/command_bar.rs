// SPDX-License-Identifier: AGPL-3.0-only
//! The command bar under the navigation row.
//!
//! Ports `section.commandbar` in `v2.0.0:desktop/ui/index.html` and its menus in
//! `setup()` and `openNewMenu` of `v2.0.0:desktop/ui/app.js`, in the same order:
//! New ▾ │ Cut, Copy, Paste, Rename, Share, Delete │ Sort ▾,
//! View ▾, See more ("…"), then at the right Update while a release waits
//! (unless Settings › About › "Show the Update button in the command
//! bar" is off), Settings
//! and the Details toggle; the appearance toggle there is hidden,
//! as Settings › Appearance chooses the theme. Every control runs a window or application
//! action. New is disabled where nothing can be created, and Delete is named
//! plainly "Delete", as in Windows Explorer, or "Delete permanently" in the
//! Recycle Bin (CMD-003). Share, which was Copy path in the Python app, shows
//! only where the desktop can share by email (CMD-037, `super::share`).
//! In the Recycle Bin, as in Windows 11 File Explorer, Empty Recycle Bin
//! and Restore all items follow View, Restore all items reading Restore
//! the selected items while items are selected (OPS-041, OPS-042).
//!
//! [`CommandBar`] is a `GtkBox` subclass. The template
//! `resources/ui/command-bar.ui` lays out the bar and the three controls at
//! its right; the file commands come from [`EDIT_COMMANDS`] and the menus
//! of [`menus`]. Its menus work as one group ([`menu_group`]): while one
//! is open, one click on another menu button opens that one.

mod menu_group;
pub(in crate::window) mod menus;
mod overflow;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::Appearance;

use crate::icons::{self, Icon};
use crate::theme::AppearanceExt;

use super::breakpoints::WindowWidth;
use super::menu_popover::{name_menu_button, MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;

use menus::{appearance_items, more_menu};
pub(super) use menus::{disconnect_network_drive, FolderPin, MoreMenuState, MorePlace, NetworkDrive};
pub(super) use menus::{
    gallery_view_menu, group_by_menu, new_menu, search_options_menu, sort_menu, view_menu,
};
use overflow::OverflowLayout;

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

/// Cut to Delete, as index.html lists them.
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
        action: WindowAction::Share,
        name: crate::i18n::message_id("Share"),
        tooltip: crate::i18n::message_id("Share"),
        compact: InCompactWindow::Hidden,
    },
    IconCommand {
        glyph: Icon::Delete,
        action: WindowAction::Trash,
        name: crate::i18n::message_id("Delete"),
        tooltip: crate::i18n::message_id("Delete (Delete)"),
        compact: InCompactWindow::Kept,
    },
];

/// The tooltip of Settings.
const SETTINGS_TOOLTIP: &str = crate::i18n::message_id("Settings (Ctrl+,)");

/// What a command becomes at the top of "…" when it does not fit in
/// the bar: built when the menu is, so it shows the command's label and
/// menu as they are then.
#[derive(Clone)]
struct OverflowEntry(std::rc::Rc<dyn Fn() -> MenuEntry>);

impl std::fmt::Debug for OverflowEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OverflowEntry")
    }
}

impl OverflowEntry {
    /// An item that runs `action`.
    fn item(label: String, glyph: Icon, action: WindowAction) -> Self {
        Self(std::rc::Rc::new(move || {
            MenuItem::new(&label, glyph, action).into()
        }))
    }

    /// An item that opens what `menu` lists now beside "…", enabled while
    /// its first item's action is.
    fn submenu(label: String, glyph: Icon, menu: MenuPopover) -> Self {
        Self(std::rc::Rc::new(move || {
            let entries = menu.entries();
            let action = menu.items().first().map_or(
                super::menu_popover::MenuAction::Window(WindowAction::View),
                |item| item.action,
            );
            MenuItem::submenu(&label, glyph, action, entries).into()
        }))
    }

    /// The entry, as it is now.
    fn entry(&self) -> MenuEntry {
        (self.0)()
    }
}

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::CommandBar`].
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/command-bar.ui")]
    pub(crate) struct CommandBar {
        /// New to See more, filled from the tables.
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
        /// Opens Software updates while a release waits.
        #[template_child]
        pub(super) update_button: TemplateChild<gtk::Button>,
        /// The glyph of [`Self::update_button`].
        #[template_child]
        pub(super) update_glyph: TemplateChild<gtk::Image>,
        /// "Update" or "Restart".
        #[template_child]
        pub(super) update_label: TemplateChild<gtk::Label>,
        /// Shows whether the details pane is open.
        #[template_child]
        pub(super) details_toggle: TemplateChild<gtk::ToggleButton>,
        /// The glyph of [`Self::details_toggle`].
        #[template_child]
        pub(super) details_glyph: TemplateChild<gtk::Image>,
        /// Cut, Rename and Details, which a compact window hides.
        pub(super) hidden_when_compact: RefCell<Vec<gtk::Widget>>,
        /// Share, which a compact window hides too, and which shows only
        /// where the desktop can share.
        pub(super) share_button: OnceCell<gtk::Button>,
        /// Whether the window offers Share.
        pub(super) share_offered: Cell<bool>,
        /// Whether the window is compact (680 pixels or less).
        pub(super) compact: Cell<bool>,
        /// New ▾, disabled where nothing can be created.
        pub(super) new_button: OnceCell<gtk::MenuButton>,
        /// Delete, labelled for the folder.
        pub(super) delete_button: OnceCell<gtk::Button>,
        /// Extract all, shown inside a ZIP opened like a folder (ARC-026).
        pub(super) extract_button: OnceCell<gtk::Button>,
        /// Empty Recycle Bin and Restore, shown in the Recycle Bin.
        pub(super) recycle_bin: OnceCell<super::RecycleBinCommands>,
        /// Search options and Close search, shown while searching.
        pub(super) search_commands: OnceCell<super::SearchCommands>,
        /// Gallery's Collection menu, shown in Gallery (HOME-025).
        pub(super) collection: OnceCell<(gtk::MenuButton, crate::window::menu_popover::MenuPopover)>,
        /// What the Collection menu lists now, to build it only on change.
        pub(super) collection_key: RefCell<String>,
        /// Sort ▾, grayed in Gallery, and View ▾, whose menu Gallery has
        /// its own of (HOME-030, HOME-031).
        pub(super) sort_button: OnceCell<gtk::MenuButton>,
        pub(super) view_popover: OnceCell<crate::window::menu_popover::MenuPopover>,
        /// Whether View shows Gallery's menu now.
        pub(super) gallery_menus: Cell<bool>,
        /// Set as background, Rotate left and Rotate right after a
        /// separator, shown in Gallery while photos are selected.
        pub(super) photo_commands: OnceCell<Vec<gtk::Widget>>,
        /// The "…" menu, which offers Map network drive only in This
        /// PC and Network.
        pub(super) more_menu: OnceCell<crate::window::menu_popover::MenuPopover>,
        /// Lays the commands out, leaving out those that do not fit.
        pub(super) overflow: OnceCell<super::OverflowLayout>,
        /// Each command that can move into "…", in the bar's order, with
        /// what it becomes there.
        pub(super) overflow_entries: RefCell<Vec<(gtk::Widget, super::OverflowEntry)>>,
        /// What the "…" menu shows now: Map network drive or not, Pin
        /// or Unpin.
        pub(super) more_state: RefCell<super::MoreMenuState>,
        /// The menu button whose menu is open (`menu_group.rs`).
        pub(super) open_menu: RefCell<Option<gtk::MenuButton>>,
        /// The window whose going inactive closes the open menu.
        pub(super) watched_window: glib::WeakRef<gtk::Window>,
        /// The surface that last had the pointer's motion.
        pub(super) pointer_surface: glib::WeakRef<gtk::gdk::Surface>,
        /// A press on the open menu taken from the window, until its
        /// release (`menu_group.rs`).
        pub(super) held_press: RefCell<Option<super::menu_group::HeldPress>>,
        /// Whether a pointer button or a touch is down in the window.
        pub(super) pointer_down: Cell<bool>,
        /// The menu button whose menu was open when the last press began
        /// (`menu_group.rs`).
        pub(super) pressed_with_menu_open: RefCell<Option<gtk::MenuButton>>,
        /// Where the keyboard is in the window (`menu_group.rs`).
        pub(super) keyboard_on: glib::WeakRef<gtk::Widget>,
        /// The tab and location the window showed when it last told the
        /// bar.
        pub(super) shown_location: RefCell<Option<(Option<crate::window::session::TabId>, String)>>,
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
            bar.lay_out_commands();
            bar.finish_right_commands();
            bar.group_menus();
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
    /// New ▾ │ Cut … Delete │ Sort ▾, View ▾ and See more ("…"),
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
            self.offer_in_more(
                &button,
                OverflowEntry::item(
                    ox_core::i18n::gettext(command.name),
                    command.glyph,
                    command.action,
                ),
            );
            if command.action == WindowAction::Share {
                button.set_visible(false);
                imp.share_button
                    .set(button.clone())
                    .expect("the bar has one Share");
            } else if command.compact == InCompactWindow::Hidden {
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
        let extract = extract_all_button();
        self.offer_in_more(
            &extract,
            OverflowEntry::item(
                ox_core::i18n::gettext("Extract all"),
                Icon::FolderZip,
                WindowAction::ExtractAll,
            ),
        );
        group.append(&extract);
        imp.extract_button
            .set(extract)
            .expect("constructed runs once per object");
        let sort = text_menu_button(
            &ox_core::i18n::gettext("Sort"),
            Icon::ArrowSort,
            "sort-command",
            sort_menu(),
        );
        self.offer_menu_in_more(&sort, &ox_core::i18n::gettext("Sort"), Icon::ArrowSort);
        group.append(&sort);
        imp.sort_button
            .set(sort)
            .expect("constructed runs once per object");
        let view = text_menu_button(
            &ox_core::i18n::gettext("View"),
            Icon::Grid,
            "view-command",
            view_menu(),
        );
        self.offer_menu_in_more(&view, &ox_core::i18n::gettext("View"), Icon::Grid);
        group.append(&view);
        imp.view_popover
            .set(
                view.popover()
                    .and_downcast::<MenuPopover>()
                    .expect("View has its menu"),
            )
            .expect("constructed runs once per object");
        self.add_place_commands();
    }

    /// The commands shown only in some places, after View: Gallery's
    /// Collection and photo commands, the Recycle Bin's, a search's, then
    /// "…".
    fn add_place_commands(&self) {
        let imp = self.imp();
        let group = &*imp.file_commands;
        let collection_menu = MenuPopover::new(Vec::new());
        let collection_label = ox_core::i18n::gettext("Collection");
        let collection = gtk::MenuButton::builder()
            .child(&text_menu_content(&collection_label, Icon::Image))
            .popover(&collection_menu)
            .valign(gtk::Align::Center)
            .css_classes(["command", "text-command", "collection-command"])
            .visible(false)
            .build();
        name_menu_button(&collection, &collection_label);
        self.offer_menu_in_more(&collection, &collection_label, Icon::Image);
        group.append(&collection);
        imp.collection
            .set((collection, collection_menu))
            .expect("constructed runs once per object");
        let photo_commands = photo_commands();
        for (command, entry) in &photo_commands {
            if let Some(entry) = entry {
                self.offer_in_more(command, entry.clone());
            }
            group.append(command);
        }
        let photo_commands = photo_commands.into_iter().map(|(command, _)| command).collect();
        imp.photo_commands
            .set(photo_commands)
            .expect("constructed runs once per object");
        let recycle_bin = RecycleBinCommands::new();
        self.offer_in_more(
            &recycle_bin.empty,
            OverflowEntry::item(
                ox_core::i18n::gettext("Empty Recycle Bin"),
                Icon::DeleteDismiss,
                WindowAction::EmptyRecycleBin,
            ),
        );
        self.offer_in_more(&recycle_bin.restore, recycle_bin.restore_entry());
        group.append(&recycle_bin.separator);
        group.append(&recycle_bin.empty);
        group.append(&recycle_bin.restore);
        imp.recycle_bin
            .set(recycle_bin)
            .expect("constructed runs once per object");
        let search = SearchCommands::new();
        self.offer_menu_in_more(
            &search.options,
            &ox_core::i18n::gettext("Search options"),
            Icon::SearchSettings,
        );
        self.offer_in_more(
            &search.close,
            OverflowEntry::item(
                ox_core::i18n::gettext("Close search"),
                Icon::Dismiss16,
                WindowAction::CloseSearch,
            ),
        );
        group.append(&search.options);
        group.append(&search.close);
        imp.search_commands
            .set(search)
            .expect("constructed runs once per object");
        let (more, menu) = more_button();
        group.append(&more);
        imp.more_menu.set(menu).expect("constructed runs once per object");
    }

    /// Shows `state` in "…": Map network drive only in This PC and
    /// Network, as Windows 11 File Explorer lists Map network drive, and
    /// Pin to Quick access or Unpin from Quick access for the folder shown.
    pub(super) fn show_more_menu(&self, state: MoreMenuState) {
        let imp = self.imp();
        if *imp.more_state.borrow() == state {
            return;
        }
        imp.more_state.replace(state);
        self.refresh_more_menu();
    }

    /// Lets `command` move into "…" as `entry` when it does not fit.
    fn offer_in_more(&self, command: &impl IsA<gtk::Widget>, entry: OverflowEntry) {
        self.imp()
            .overflow_entries
            .borrow_mut()
            .push((command.clone().upcast(), entry));
    }

    /// Lets the menu button `command` move into "…" as a submenu named
    /// `label` listing what its menu does.
    fn offer_menu_in_more(&self, command: &gtk::MenuButton, label: &str, glyph: Icon) {
        let Some(menu) = command.popover().and_downcast::<MenuPopover>() else {
            return;
        };
        self.offer_in_more(command, OverflowEntry::submenu(label.to_owned(), glyph, menu));
    }

    /// Lays the commands out so those that do not fit move into "…", and
    /// rebuilds "…" whenever other commands moved, or as it opens, when
    /// what the commands it lists show may have changed.
    fn lay_out_commands(&self) {
        let imp = self.imp();
        let layout = OverflowLayout::install(
            &imp.file_commands,
            glib::clone!(
                #[weak(rename_to = bar)]
                self,
                move || bar.refresh_more_menu()
            ),
        );
        imp.overflow
            .set(layout)
            .expect("constructed runs once per object");
        let more = imp.file_commands.last_child().and_downcast::<gtk::MenuButton>();
        if let Some(more) = more {
            more.connect_active_notify(glib::clone!(
                #[weak(rename_to = bar)]
                self,
                move |more| {
                    // Only the commands that moved in can have changed
                    // their labels (Restore's) since the menu was built.
                    if more.is_active() && !bar.commands_in_more().is_empty() {
                        bar.refresh_more_menu();
                    }
                }
            ));
        }
    }

    /// The commands "…" lists first: those the bar left out for lack of
    /// room and those a compact window hides, in the bar's order.
    fn commands_in_more(&self) -> Vec<MenuEntry> {
        let imp = self.imp();
        let left_out = imp
            .overflow
            .get()
            .map(OverflowLayout::left_out)
            .unwrap_or_default();
        let compact = imp.compact.get();
        let hidden_when_compact = imp.hidden_when_compact.borrow();
        let share = imp
            .share_button
            .get()
            .map(|share| share.clone().upcast::<gtk::Widget>());
        imp.overflow_entries
            .borrow()
            .iter()
            .filter(|(command, _)| {
                let hidden_for_room = compact
                    && (hidden_when_compact.contains(command)
                        || (share.as_ref() == Some(command) && imp.share_offered.get()));
                left_out.contains(command) || hidden_for_room
            })
            .map(|(_, entry)| entry.entry())
            .collect()
    }

    /// Builds "…": the commands that did not fit, then its own items.
    fn refresh_more_menu(&self) {
        let imp = self.imp();
        let Some(menu) = imp.more_menu.get() else {
            return;
        };
        let mut entries = self.commands_in_more();
        if !entries.is_empty() {
            entries.push(MenuEntry::Divider);
        }
        entries.extend(more_menu(&imp.more_state.borrow()));
        if menu.entries() != entries {
            menu.set_entries(entries);
        }
    }

    /// The commands left out of the bar for lack of room, for tests.
    #[cfg(test)]
    pub(super) fn commands_left_out(&self) -> Vec<gtk::Widget> {
        self.imp()
            .overflow
            .get()
            .map(OverflowLayout::left_out)
            .unwrap_or_default()
    }

    /// Gives Update, the appearance toggle, Settings and the Details
    /// toggle, which the template places at the right, what it cannot
    /// express.
    ///
    /// Settings › Appearance chooses the theme, so the appearance toggle
    /// stays built but hidden; Update shows only while a release waits
    /// and Settings › About leaves it on ([`Self::show_update`]).
    fn finish_right_commands(&self) {
        self.finish_update_button();
        self.finish_appearance_button();
        self.finish_settings_button();
        self.finish_details_toggle();
        self.imp().appearance_button.set_visible(false);
    }

    /// The download glyph and Software updates.
    fn finish_update_button(&self) {
        let imp = self.imp();
        icons::set_icon(&imp.update_glyph, Icon::ArrowDownload, TEXT_COMMAND_GLYPH);
        WindowAction::CheckUpdates.assign_to(&*imp.update_button);
    }

    /// Shows Update with `label`, "Update" or "Restart", and `tooltip`
    /// saying why, or hides it for `None`.
    pub(super) fn show_update(&self, label: Option<&str>, tooltip: Option<&str>) {
        let imp = self.imp();
        let Some(label) = label else {
            imp.update_button.set_visible(false);
            return;
        };
        imp.update_label.set_text(label);
        imp.update_button.set_tooltip_text(tooltip);
        imp.update_button
            .update_property(&[gtk::accessible::Property::Label(label)]);
        imp.update_button.set_visible(true);
    }

    /// The label of Update while it shows, for tests.
    #[cfg(test)]
    pub(super) fn shown_update(&self) -> Option<String> {
        let imp = self.imp();
        imp.update_button
            .is_visible()
            .then(|| imp.update_label.text().to_string())
    }

    /// The appearance menu, and the light appearance until the window
    /// shows the skin's.
    fn finish_appearance_button(&self) {
        let appearance_menu = MenuPopover::new(appearance_items().to_vec());
        self.imp().appearance_button.set_popover(Some(&appearance_menu));
        crate::accessible_names::describe_menu_button(&self.imp().appearance_button, None);
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
        let label = ox_core::i18n::gettext(appearance.label());
        imp.appearance_label.set_text(&label);
        crate::accessible_names::name_menu_button(&imp.appearance_button, &label);
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

    /// Names Delete `label`, "Delete" or "Delete permanently", in
    /// its tooltip and for screen readers (`updateToolbar`).
    pub(super) fn show_delete_label(&self, label: &str) {
        let Some(button) = self.imp().delete_button.get() else {
            return;
        };
        button.set_tooltip_text(Some(&ox_core::i18n::format_message(
            "{label} (Delete)",
            &[("label", label)],
        )));
        button.update_property(&[gtk::accessible::Property::Label(label)]);
    }

    /// Hides what the web layout hides in a window of `band`'s width: the
    /// appearance label from 1050 pixels, and Cut, Rename, Share,
    /// Details and Update's label from 680.
    pub(super) fn fit_to_width(&self, band: WindowWidth) {
        let imp = self.imp();
        imp.appearance_label.set_visible(band.shows_appearance_label());
        imp.update_label.set_visible(!band.is_compact());
        for control in imp.hidden_when_compact.borrow().iter() {
            control.set_visible(!band.is_compact());
        }
        imp.compact.set(band.is_compact());
        self.show_share();
        if let Some(layout) = imp.overflow.get() {
            let gap = if band >= WindowWidth::Medium {
                overflow::NARROW_GAP
            } else {
                overflow::GAP
            };
            layout.set_gap(gap);
        }
        self.refresh_more_menu();
    }

    /// Offers Share, or hides it where the desktop cannot share.
    pub(super) fn offer_share(&self, offered: bool) {
        self.imp().share_offered.set(offered);
        self.show_share();
    }

    /// Shows Share when it is offered and the window is not compact.
    fn show_share(&self) {
        let imp = self.imp();
        if let Some(button) = imp.share_button.get() {
            button.set_visible(imp.share_offered.get() && !imp.compact.get());
        }
    }

    /// Shows Extract all while an archive is selected or the window shows
    /// the inside of a ZIP.
    pub(super) fn show_extract_all(&self, shown: bool) {
        if let Some(button) = self.imp().extract_button.get() {
            button.set_visible(shown);
        }
    }

    /// The Search options button, shown while a search shows in Windows
    /// 11's layout.
    pub(super) fn search_options_button(&self) -> Option<gtk::MenuButton> {
        self.imp()
            .search_commands
            .get()
            .map(|commands| commands.options.clone())
    }

    /// Shows Search options and Close search while a search shows in
    /// Windows 11's layout (SRCH-046), or hides them.
    pub(super) fn show_search_commands(&self, shown: bool) {
        if let Some(commands) = self.imp().search_commands.get() {
            commands.options.set_visible(shown);
            commands.close.set_visible(shown);
            if !shown {
                commands.options.popdown();
            }
        }
    }

    /// Shows Gallery's Collection menu with `entries` while the window
    /// shows Gallery, or hides it for `None` (HOME-025).
    /// `key` names what `entries` list; the menu is rebuilt only when it
    /// changes.
    pub(super) fn show_collection(&self, shown: Option<(String, Vec<MenuEntry>)>) {
        let Some((button, menu)) = self.imp().collection.get() else {
            return;
        };
        button.set_visible(shown.is_some());
        match shown {
            Some((key, entries)) => {
                if *self.imp().collection_key.borrow() != key {
                    menu.set_entries(entries);
                    self.imp().collection_key.replace(key);
                }
            }
            None => button.popdown(),
        }
    }

    /// The commands shown in the bar's file group, in order, each named
    /// by its label or, for an icon command, its tooltip without the
    /// shortcut, and whether it can be used now; for tests.
    #[cfg(test)]
    pub(super) fn shown_commands(&self) -> Vec<(String, bool)> {
        crate::window::widget_tree::children(&*self.imp().file_commands)
            .filter(|widget| widget.has_css_class("command") && widget.is_visible())
            .filter_map(|widget| {
                // A menu button's own content, not its menu's.
                let content = widget
                    .downcast_ref::<gtk::MenuButton>()
                    .and_then(gtk::MenuButton::child)
                    .or_else(|| widget.downcast_ref::<gtk::Button>().and_then(ButtonExt::child));
                let tooltip = widget.tooltip_text().map(|tooltip| tooltip.to_string());
                let named = EDIT_COMMANDS
                    .iter()
                    .find(|command| {
                        tooltip.as_deref() == Some(ox_core::i18n::gettext(command.tooltip).as_str())
                    })
                    .map(|command| ox_core::i18n::gettext(command.name));
                let label = named
                    .or_else(|| {
                        content
                            .as_ref()
                            .and_then(|content| {
                                crate::window::widget_tree::descendants::<gtk::Label>(content)
                                    .first()
                                    .cloned()
                            })
                            .map(|label| label.text().to_string())
                    })
                    .or(tooltip)?;
                let usable = widget.is_sensitive()
                    && widget
                        .downcast_ref::<gtk::Actionable>()
                        .and_then(gtk::prelude::ActionableExt::action_name)
                        .is_none_or(|name| {
                            let (group, action) = name.split_once('.').unwrap_or(("win", &name));
                            widget
                                .root()
                                .and_then(|root| root.downcast::<gtk::ApplicationWindow>().ok())
                                .filter(|_| group == "win")
                                .and_then(|window| window.lookup_action(action))
                                .is_none_or(|action| action.is_enabled())
                        });
                Some((label, usable))
            })
            .collect()
    }

    /// Shows Gallery's View menu and grays Sort while the window shows
    /// Gallery (`in_gallery`), or the folders' (HOME-030, HOME-031).
    pub(super) fn show_gallery_menus(&self, in_gallery: bool) {
        let imp = self.imp();
        if let Some(sort) = imp.sort_button.get() {
            sort.set_sensitive(!in_gallery);
            if in_gallery {
                sort.popdown();
            }
        }
        if imp.gallery_menus.replace(in_gallery) == in_gallery {
            return;
        }
        if let Some(view) = imp.view_popover.get() {
            view.set_entries(if in_gallery {
                gallery_view_menu()
            } else {
                view_menu()
            });
        }
    }

    /// Shows Set as background, Rotate left and Rotate right, or hides
    /// them (HOME-030).
    pub(super) fn show_photo_commands(&self, shown: bool) {
        for command in self.imp().photo_commands.get().into_iter().flatten() {
            command.set_visible(shown);
        }
    }

    /// The Collection button, for the snapshot hook's scenes and tests.
    pub(super) fn collection_button(&self) -> Option<gtk::MenuButton> {
        self.imp().collection.get().map(|(button, _)| button.clone())
    }

    /// Shows Empty Recycle Bin and Restore with what Restore restores
    /// while the window shows the Recycle Bin, or hides them for `None`.
    pub(super) fn show_recycle_bin_commands(&self, restore: Option<RestoreTarget>) {
        if let Some(commands) = self.imp().recycle_bin.get() {
            commands.show(restore);
        }
    }
}

/// What Restore in the Recycle Bin's command bar restores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RestoreTarget {
    /// Nothing is selected: "Restore all items".
    AllItems,
    /// "Restore the selected items".
    SelectedItems,
}

impl RestoreTarget {
    /// The command's label, as Windows 11 File Explorer words it.
    fn label(self) -> &'static str {
        match self {
            RestoreTarget::AllItems => ox_core::i18n::gettext_static("Restore all items"),
            RestoreTarget::SelectedItems => ox_core::i18n::gettext_static("Restore the selected items"),
        }
    }

    /// The window action the command runs.
    fn action(self) -> WindowAction {
        match self {
            RestoreTarget::AllItems => WindowAction::RestoreAllItems,
            RestoreTarget::SelectedItems => WindowAction::Restore,
        }
    }
}

/// The Recycle Bin's commands after View, as Windows 11 File Explorer
/// shows them there: a separator, Empty Recycle Bin and Restore.
#[derive(Debug)]
pub(super) struct RecycleBinCommands {
    separator: gtk::Separator,
    empty: gtk::Button,
    restore: gtk::Button,
    restore_label: gtk::Label,
    /// What Restore restores now.
    target: std::rc::Rc<std::cell::Cell<RestoreTarget>>,
}

impl RecycleBinCommands {
    /// The commands, hidden until the window shows the Recycle Bin.
    fn new() -> Self {
        let separator = separator();
        separator.set_visible(false);
        let (empty, _) = text_button(
            &ox_core::i18n::gettext("Empty Recycle Bin"),
            Icon::DeleteDismiss,
            WindowAction::EmptyRecycleBin,
            "empty-recycle-bin-command",
        );
        empty.set_visible(false);
        let target = RestoreTarget::AllItems;
        let (restore, restore_label) = text_button(
            target.label(),
            Icon::ArrowCounterclockwise,
            target.action(),
            "restore-command",
        );
        restore.set_visible(false);
        Self {
            separator,
            empty,
            restore,
            restore_label,
            target: std::rc::Rc::new(std::cell::Cell::new(target)),
        }
    }

    /// Restore as "…" lists it when it does not fit: named for what it
    /// restores now.
    fn restore_entry(&self) -> OverflowEntry {
        let target = std::rc::Rc::clone(&self.target);
        OverflowEntry(std::rc::Rc::new(move || {
            let target = target.get();
            MenuItem::new(target.label(), Icon::ArrowCounterclockwise, target.action()).into()
        }))
    }

    /// Shows the commands with Restore restoring `restore`, or hides them
    /// for `None`.
    fn show(&self, restore: Option<RestoreTarget>) {
        let shown = restore.is_some();
        self.separator.set_visible(shown);
        self.empty.set_visible(shown);
        self.restore.set_visible(shown);
        let Some(target) = restore else {
            return;
        };
        self.target.set(target);
        let label = target.label();
        if self.restore_label.text() != label {
            self.restore_label.set_text(label);
            self.restore
                .update_property(&[gtk::accessible::Property::Label(label)]);
        }
        self.restore
            .set_action_name(Some(&target.action().detailed_name()));
    }
}

/// Windows 11's commands while a search shows (SRCH-046): Search
/// options, a menu of what the search looks at, and Close search.
#[derive(Debug)]
pub(super) struct SearchCommands {
    options: gtk::MenuButton,
    close: gtk::Button,
}

impl SearchCommands {
    /// The commands, hidden until a search shows.
    fn new() -> Self {
        let label = ox_core::i18n::gettext("Search options");
        let options = text_menu_button(
            &label,
            Icon::SearchSettings,
            "search-options-command",
            search_options_menu(),
        );
        name_menu_button(&options, &label);
        options.set_visible(false);
        let (close, _) = text_button(
            &ox_core::i18n::gettext("Close search"),
            Icon::Dismiss16,
            WindowAction::CloseSearch,
            "close-search-command",
        );
        close.set_visible(false);
        Self { options, close }
    }
}

/// Set as background (grayed: not done), Rotate left and Rotate right
/// after a separator, as Windows 11's Gallery shows them while photos are
/// selected; hidden until then (HOME-030).
fn photo_commands() -> Vec<(gtk::Widget, Option<OverflowEntry>)> {
    let photo_separator = separator();
    let photo_command = |label: String, glyph: Icon, action: WindowAction, css_class: &str| {
        let (button, _) = text_button(&label, glyph, action, css_class);
        (button, OverflowEntry::item(label, glyph, action))
    };
    let (background, background_entry) = photo_command(
        ox_core::i18n::gettext("Set as background"),
        Icon::Image,
        WindowAction::SetAsBackground,
        "set-background-command",
    );
    background.set_tooltip_text(Some(&ox_core::i18n::gettext(
        "Setting the desktop background is not available yet.",
    )));
    let (rotate_left, rotate_left_entry) = photo_command(
        ox_core::i18n::gettext("Rotate left"),
        Icon::ArrowCounterclockwise,
        WindowAction::RotateLeft,
        "rotate-left-command",
    );
    let (rotate_right, rotate_right_entry) = photo_command(
        ox_core::i18n::gettext("Rotate right"),
        Icon::ArrowClockwise,
        WindowAction::RotateRight,
        "rotate-right-command",
    );
    let photo_commands: Vec<(gtk::Widget, Option<OverflowEntry>)> = vec![
        (photo_separator.upcast(), None),
        (background.upcast(), Some(background_entry)),
        (rotate_left.upcast(), Some(rotate_left_entry)),
        (rotate_right.upcast(), Some(rotate_right_entry)),
    ];
    for (command, _) in &photo_commands {
        command.set_visible(false);
    }
    photo_commands
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
    let button = gtk::MenuButton::builder()
        .child(&text_menu_content(label, glyph))
        .popover(&MenuPopover::new(entries))
        .valign(gtk::Align::Center)
        .css_classes(["command", "text-command", css_class])
        .build();
    crate::accessible_names::describe_menu_button(&button, Some(label));
    button
}

/// A command with a glyph and `label` that runs `action`; `css_class`
/// names it for the stylesheet and the tests. Returns the label too, for
/// a command whose label changes.
fn text_button(label: &str, glyph: Icon, action: WindowAction, css_class: &str) -> (gtk::Button, gtk::Label) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    content.append(&icons::image(glyph, TEXT_COMMAND_GLYPH));
    let text = gtk::Label::new(Some(label));
    content.append(&text);
    let button = gtk::Button::builder()
        .child(&content)
        .action_name(action.detailed_name())
        .valign(gtk::Align::Center)
        .css_classes(["command", "text-command", css_class])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    (button, text)
}

/// Extract all, as Windows Explorer's command bar shows it while a ZIP is
/// selected or open; hidden otherwise. A selected TAR archive shows it
/// too, as Extract all… in its menu does.
fn extract_all_button() -> gtk::Button {
    let (button, _) = text_button(
        &ox_core::i18n::gettext("Extract all"),
        Icon::FolderZip,
        WindowAction::ExtractAll,
        "extract-command",
    );
    button.set_tooltip_text(Some(&ox_core::i18n::gettext(
        "Extract all files from this archive",
    )));
    button.set_visible(false);
    button
}

/// The "…" button and its menu.
fn more_button() -> (gtk::MenuButton, MenuPopover) {
    let menu = MenuPopover::new(more_menu(&MoreMenuState::default()));
    let button = gtk::MenuButton::builder()
        .child(&icons::image(Icon::MoreHorizontal, ICON_COMMAND_GLYPH))
        .tooltip_text(ox_core::i18n::gettext("See more"))
        .popover(&menu)
        .valign(gtk::Align::Center)
        .css_classes(["command", "more-command"])
        .build();
    name_menu_button(&button, &ox_core::i18n::gettext("See more"));
    (button, menu)
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
