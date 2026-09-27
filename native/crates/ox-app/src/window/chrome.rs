// SPDX-License-Identifier: AGPL-3.0-only
//! The window's frame: title bar with tabs, navigation row, command bar,
//! message line, workspace and status bar.
//!
//! Ports the static layout of `desktop/ui/index.html`. Every button runs a
//! window action; behaviour lives in the window controller. The widget tree
//! is built in code because every icon is a native paintable
//! ([`icons::glyph`]) that a `.ui` file cannot describe.

use gtk::prelude::*;
use gtk::{gio, pango};
use ox_core::format;

use crate::folder_view::grid::IconSize;
use crate::folder_view::model::SelectionSummary;
use crate::folder_view::sorting::SortColumn;
use crate::icons::{self, Glyph};
use crate::theme::Appearance;

use super::address_bar::AddressBar;
use super::content::FolderView;
use super::tab_strip::TabStrip;

/// The frame's widgets that the controller updates.
#[derive(Debug)]
pub(super) struct Chrome {
    /// The tab strip in the title bar.
    pub tabs: TabStrip,
    /// Breadcrumbs or the editable address.
    pub address: AddressBar,
    /// The folder filter.
    pub search: gtk::SearchEntry,
    /// Item and selection counts in the status bar.
    pub status: gtk::Label,
    /// The type-to-select hint in the status bar.
    pub hint: gtk::Label,
    /// A message above the workspace, hidden when empty.
    pub message: gtk::Label,
    /// The sidebar beside the folder and details panes.
    pub workspace: gtk::Paned,
    details_view: gtk::Button,
    icons_view: gtk::Button,
    appearance: gtk::MenuButton,
}

fn icon_button(glyph: Glyph, tooltip: &str, action: &str) -> gtk::Button {
    gtk::Button::builder()
        .child(&icons::glyph(glyph, 16))
        .tooltip_text(tooltip)
        .action_name(action)
        .build()
}

fn command_content(label: &str, glyph: Glyph) -> gtk::Box {
    let child = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    child.append(&icons::glyph(glyph, 17));
    child.append(&gtk::Label::new(Some(label)));
    child
}

fn text_button(label: &str, glyph: Glyph, action: &str) -> gtk::Button {
    gtk::Button::builder()
        .child(&command_content(label, glyph))
        .action_name(action)
        .css_classes(["text-command"])
        .build()
}

/// A command's content with the chevron that marks a menu.
fn menu_content(label: &str, glyph: Glyph) -> gtk::Box {
    let content = command_content(label, glyph);
    content.append(&icons::glyph(Glyph::Down, 10));
    content
}

fn menu_button(label: &str, glyph: Glyph, menu: &gio::Menu) -> gtk::MenuButton {
    let child = menu_content(label, glyph);
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.add_css_class("ox-menu");
    gtk::MenuButton::builder()
        .child(&child)
        .popover(&popover)
        .css_classes(["text-command"])
        .build()
}

fn sort_menu() -> gio::Menu {
    let sort = gio::Menu::new();
    for column in SortColumn::ALL {
        sort.append(Some(column.label()), Some(&format!("win.sort::{}", column.key())));
    }
    let direction = gio::Menu::new();
    direction.append(Some("Ascending"), Some("win.direction::ascending"));
    direction.append(Some("Descending"), Some("win.direction::descending"));
    sort.append_section(None, &direction);
    sort
}

fn view_menu() -> gio::Menu {
    let view = gio::Menu::new();
    view.append(Some("Details"), Some("win.view::details"));
    for size in IconSize::ALL {
        view.append(Some(size.label()), Some(&format!("win.view::{}", size.key())));
    }
    let visibility = gio::Menu::new();
    visibility.append(Some("Show hidden files"), Some("win.hidden"));
    visibility.append(Some("Details pane"), Some("win.details-pane"));
    view.append_section(None, &visibility);
    view
}

fn appearance_menu() -> gio::Menu {
    let appearance = gio::Menu::new();
    for (label, value) in [("System", "system"), ("Light", "light"), ("Dark", "dark")] {
        appearance.append(Some(label), Some(&format!("win.theme::{value}")));
    }
    let text = gio::Menu::new();
    text.append(Some("Larger text"), Some("win.text-larger"));
    text.append(Some("Smaller text"), Some("win.text-smaller"));
    text.append(Some("Reset text size"), Some("win.text-reset"));
    appearance.append_section(Some("Text size"), &text);
    appearance
}

/// The Details button shows whether the pane is open: a toggle bound to
/// the boolean `win.details-pane` action.
fn details_pane_toggle() -> gtk::ToggleButton {
    gtk::ToggleButton::builder()
        .child(&command_content("Details", Glyph::Details))
        .action_name("win.details-pane")
        .css_classes(["text-command"])
        .build()
}

/// The Appearance menu button. It shows the appearance drawn now, as
/// `applyTheme` in app.js does; [`Chrome::show_appearance`] updates it.
fn appearance_button() -> gtk::MenuButton {
    let button = menu_button("Light", Glyph::Sun, &appearance_menu());
    button.update_property(&[gtk::accessible::Property::Label("Appearance")]);
    button
}

fn command_bar(appearance: &gtk::MenuButton) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.add_css_class("commandbar");
    bar.append(&text_button("New tab", Glyph::Plus, "win.new-tab"));
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    // File mutation commands are added only when their native workflows exist.
    bar.append(&text_button("Open", Glyph::FolderLine, "win.open"));
    bar.append(&menu_button("Sort", Glyph::Sort, &sort_menu()));
    bar.append(&menu_button("View", Glyph::Grid, &view_menu()));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    bar.append(&details_pane_toggle());
    bar.append(appearance);
    bar
}

/// The title bar: tabs, the new-tab button and the window controls.
fn title_bar(tabs: &TabStrip) -> gtk::WindowHandle {
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    title.add_css_class("ox-titlebar");
    title.append(&tabs.root);
    let new_tab = icon_button(Glyph::Plus, "New tab (Ctrl+T)", "win.new-tab");
    new_tab.add_css_class("newtab");
    title.append(&new_tab);
    title.append(&gtk::WindowControls::new(gtk::PackType::End));
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&title));
    handle
}

fn search_entry() -> gtk::SearchEntry {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Filter this folder")
        .width_request(205)
        .build();
    search.update_property(&[gtk::accessible::Property::Label("Filter this folder")]);
    search.add_css_class("search");
    search
}

/// Back, Forward, Up, Refresh, the address bar and the filter.
fn navigation_row(address: &AddressBar, search: &gtk::SearchEntry) -> gtk::Box {
    let navigation = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    navigation.add_css_class("navrow");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    buttons.add_css_class("nav-buttons");
    for (glyph, label, action) in [
        (Glyph::Back, "Back (Alt+Left)", "win.back"),
        (Glyph::Forward, "Forward (Alt+Right)", "win.forward"),
        (Glyph::Up, "Up (Alt+Up)", "win.up"),
        (Glyph::Refresh, "Refresh (F5)", "win.refresh"),
    ] {
        buttons.append(&icon_button(glyph, label, action));
    }
    navigation.append(&buttons);
    navigation.append(&address.root);
    navigation.append(search);
    navigation
}

fn message_line() -> gtk::Label {
    let message = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .selectable(true)
        .build();
    message.add_css_class("window-message");
    message
}

fn workspace() -> gtk::Paned {
    let workspace = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .vexpand(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .resize_start_child(false)
        .build();
    workspace.add_css_class("workspace");
    workspace
}

/// The status-bar text that shows a type-to-select prefix.
fn typeahead_hint() -> gtk::Label {
    gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .css_classes(["typeahead-hint"])
        .build()
}

/// What the status bar reports about the active tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusSubject {
    /// A landing page, which has nothing to count.
    Page,
    /// A folder listing.
    Folder {
        /// Items shown after filtering.
        shown: u32,
        /// The selection's count and size.
        selected: SelectionSummary,
        /// The folder is still being listed.
        loading: bool,
    },
}

/// The status-bar line (`updateStatus` in `desktop/ui/app.js`): "Ready" on
/// a landing page; otherwise how many items are shown, how many are
/// selected and how large the selected files are, and "Loading…" while
/// the folder is listed.
pub(super) fn status_text(subject: StatusSubject) -> String {
    let StatusSubject::Folder {
        shown,
        selected,
        loading,
    } = subject
    else {
        return "Ready".to_owned();
    };
    let items = if shown == 1 {
        "1 item".to_owned()
    } else {
        format!("{shown} items")
    };
    let mut parts = vec![items];
    if selected.count > 0 {
        parts.push(format!("{} selected", selected.count));
    }
    if selected.count > 0 && selected.has_files {
        parts.push(format::pretty_bytes(selected.bytes));
    }
    if loading {
        parts.push("Loading…".to_owned());
    }
    parts.join("  ·  ")
}

fn view_button(glyph: Glyph, tooltip: &str, view: FolderView) -> gtk::Button {
    let button = icon_button(glyph, tooltip, "win.view");
    button.set_action_target_value(Some(&view.key().to_variant()));
    button
}

impl Chrome {
    /// Builds the frame into `window`; the workspace is empty.
    pub fn new(window: &gtk::ApplicationWindow) -> Self {
        let tabs = TabStrip::new();
        window.set_titlebar(Some(&title_bar(&tabs)));
        let address = AddressBar::new();
        let search = search_entry();
        let message = message_line();
        let workspace = workspace();
        let status = gtk::Label::builder().xalign(0.0).build();
        let hint = typeahead_hint();
        let details_view = view_button(Glyph::List, "Details view", FolderView::Details);
        let icons_view = view_button(Glyph::Grid, "Large icons", FolderView::Icons(IconSize::Large));
        let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        status_bar.add_css_class("statusbar");
        status_bar.append(&status);
        status_bar.append(&hint);
        status_bar.append(&details_view);
        status_bar.append(&icons_view);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&navigation_row(&address, &search));
        let appearance = appearance_button();
        root.append(&command_bar(&appearance));
        root.append(&message);
        root.append(&workspace);
        root.append(&status_bar);
        window.set_child(Some(&root));
        Self {
            tabs,
            address,
            search,
            status,
            hint,
            message,
            workspace,
            details_view,
            icons_view,
            appearance,
        }
    }

    /// Highlights the status-bar button of `view`. Every icon size counts
    /// as the icon view, as the Python app's single grid view did.
    pub fn show_view(&self, view: FolderView) {
        let (on, off) = match view {
            FolderView::Details => (&self.details_view, &self.icons_view),
            FolderView::Icons(_) => (&self.icons_view, &self.details_view),
        };
        on.add_css_class("active");
        off.remove_css_class("active");
    }

    /// Shows the drawn appearance on the Appearance button: a sun and
    /// "Light" or a moon and "Dark", with `tooltip` saying what was chosen.
    pub fn show_appearance(&self, appearance: Appearance, tooltip: &str) {
        let content = menu_content(appearance.label(), appearance.glyph());
        self.appearance.set_child(Some(&content));
        self.appearance.set_tooltip_text(Some(tooltip));
    }

    /// Shows a message above the workspace, or hides it when empty.
    pub fn show_message(&self, message: &str) {
        self.message.set_text(message);
        self.message.set_visible(!message.is_empty());
    }

    /// Clears the folder filter.
    pub fn clear_filter(&self) {
        self.search.set_text("");
    }

    /// The Appearance button's tooltip, for tests.
    #[cfg(test)]
    pub fn appearance_tooltip(&self) -> Option<String> {
        self.appearance.tooltip_text().map(|text| text.to_string())
    }

    /// The status-bar view buttons that show as active, for tests.
    #[cfg(test)]
    pub fn active_view_buttons(&self) -> Vec<String> {
        [&self.details_view, &self.icons_view]
            .into_iter()
            .filter(|button| button.has_css_class("active"))
            .filter_map(|button| button.tooltip_text().map(|text| text.to_string()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(shown: u32, selected: SelectionSummary, loading: bool) -> String {
        status_text(StatusSubject::Folder {
            shown,
            selected,
            loading,
        })
    }

    #[test]
    fn the_status_counts_items_and_the_selection() {
        let nothing = SelectionSummary::default();
        assert_eq!(folder(1, nothing, false), "1 item");
        assert_eq!(folder(4, nothing, true), "4 items  ·  Loading…");
        let two_files = SelectionSummary {
            count: 2,
            bytes: 2048,
            has_files: true,
        };
        let size = format::pretty_bytes(2048);
        assert_eq!(
            folder(4, two_files, false),
            format!("4 items  ·  2 selected  ·  {size}")
        );
    }

    #[test]
    fn a_landing_page_is_ready() {
        assert_eq!(status_text(StatusSubject::Page), "Ready");
    }
}
