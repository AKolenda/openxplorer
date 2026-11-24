// SPDX-License-Identifier: AGPL-3.0-only
//! The breadcrumbs' menus, which app.js lacked, as Dolphin's location bar
//! and Windows 11 File Explorer's crumb chevrons have them.
//!
//! - A click on the divider after a crumb, or Down on a focused crumb,
//!   opens a menu of that folder's subfolders. The folder the address
//!   goes on to is bold and hidden folders show only while hidden files
//!   do. As in Windows, the menu lists the subfolders in one menu,
//!   scrolling when it is taller than the screen (NAV-020). A folder with
//!   more than [`MOST_LISTED_SUBFOLDERS`] lists that many, then "Show all
//!   <n> folders", which opens the folder: the list is sorted off the main
//!   thread and its rows are built a batch at a time (`menu_popover.rs`),
//!   so a folder of 50,000 subfolders never holds up the window.
//! - The chevron after the location icon lists the top-level places, the
//!   one the crumbs start at in bold; the « that takes its place on a
//!   long path lists the crumbs it hides, the nearest last, then those
//!   places (NAV-017, NAV-023).
//!
//! As in Windows Explorer, the wheel over a crumb never switches folders.

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::{enumerate_folder, EntryKind};
use ox_core::location::{same_location, ArchiveLocation, NETWORK_URI, PC_URI, TRASH_URI};

use crate::folder_view::sorting::SortKey;
use crate::icons::Icon;
use crate::locations::Page;

use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The most subfolders a crumb's menu lists; past them, its last item
/// opens the folder, whose file list shows them all.
pub(super) const MOST_LISTED_SUBFOLDERS: usize = 1_000;

/// A folder inside the folder of a crumb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Subfolder {
    /// Its name as listed.
    pub name: String,
    /// Where it opens.
    pub uri: String,
}

/// The subfolders of `uri` in natural order; hidden ones only when
/// `show_hidden`. A folder that cannot be listed has none.
pub(super) async fn list_subfolders(uri: &str, show_hidden: bool) -> Vec<Subfolder> {
    let mut folders = Vec::new();
    if let Some(inside) = ArchiveLocation::from_uri(uri) {
        // Inside a ZIP opened like a folder, the archive reader lists them.
        let browser = ox_core::archive::ArchiveBrowser::new(
            std::sync::Arc::new(ox_core::archive::GioArchiveOpener),
            ox_core::archive::default_preview_root(),
        );
        let listed = browser
            .list_in_background(
                inside.archive_uri.clone(),
                inside.listing_prefix(),
                ox_core::transfer::Cancellation::new(),
            )
            .await;
        let Ok(listing) = listed else {
            return Vec::new();
        };
        folders.extend(
            listing
                .entries
                .into_iter()
                .filter(|member| matches!(member.kind, ox_core::archive::ArchiveEntryKind::Folder))
                .filter(|member| show_hidden || !member.name.starts_with('.'))
                .map(|member| Subfolder {
                    uri: inside.member(&member.member).uri(),
                    name: member.name,
                }),
        );
        return sorted_off_the_main_thread(folders).await;
    }
    let listed = enumerate_folder(uri, |entries| {
        let visible = entries
            .into_iter()
            .filter(|entry| entry.kind == EntryKind::Directory && (show_hidden || !entry.is_hidden));
        folders.extend(visible.map(|entry| Subfolder {
            uri: entry.navigation_uri().to_owned(),
            name: entry.name,
        }));
    })
    .await;
    if listed.is_err() {
        return Vec::new();
    }
    sorted_off_the_main_thread(folders).await
}

/// `folders` in natural order, sorted on a worker thread: tens of
/// thousands of names take long enough to hold up a frame.
async fn sorted_off_the_main_thread(folders: Vec<Subfolder>) -> Vec<Subfolder> {
    gio::spawn_blocking(move || sorted_naturally(folders))
        .await
        .unwrap_or_default()
}

/// `folders` in natural order of their names.
fn sorted_naturally(folders: Vec<Subfolder>) -> Vec<Subfolder> {
    let mut keyed: Vec<(SortKey, Subfolder)> = folders
        .into_iter()
        .map(|folder| (SortKey::new(&folder.name), folder))
        .collect();
    keyed.sort_by(|(a, first), (b, second)| a.natural_cmp(b).then_with(|| first.name.cmp(&second.name)));
    keyed.into_iter().map(|(_, folder)| folder).collect()
}

/// The menu of `folders`, the subfolders of `parent`, the one at `shown`
/// in bold: at most [`MOST_LISTED_SUBFOLDERS`] of them, then, when there
/// are more, "Show all <n> folders", which opens `parent`.
pub(super) fn subfolder_menu(folders: &[Subfolder], parent: &str, shown: &str) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = folders
        .iter()
        .take(MOST_LISTED_SUBFOLDERS)
        .map(|folder| {
            let item =
                MenuItem::with_text_target(&folder.name, Icon::Folder, WindowAction::GoTo, &folder.uri);
            let emphasised = same_location(&folder.uri, shown);
            MenuItem { emphasised, ..item }.into()
        })
        .collect();
    if folders.len() > MOST_LISTED_SUBFOLDERS {
        let count = folders.len().to_string();
        let label = ox_core::i18n::format_message("Show all {count} folders", &[("count", &count)]);
        entries.push(MenuEntry::Divider);
        entries.push(
            MenuItem::with_text_target(&label, Icon::MoreHorizontal, WindowAction::GoTo, parent).into(),
        );
    }
    entries
}

/// The top-level places, as the chevron after the location icon lists
/// them: Home (at `home_uri`), This PC, Network and the Recycle Bin, the
/// place the crumbs start at (`first_crumb`) in bold. Windows lists its
/// own top-level places there; these are the app's, as its navigation
/// pane shows them.
pub(super) fn places_menu(home_uri: &str, first_crumb: Option<&str>) -> Vec<MenuEntry> {
    let places = [
        (ox_core::i18n::gettext("Home"), Icon::Home, home_uri),
        (ox_core::i18n::gettext("This PC"), Page::ThisPc.icon(), PC_URI),
        (
            ox_core::i18n::gettext("Network"),
            Page::Network.icon(),
            NETWORK_URI,
        ),
        (ox_core::i18n::gettext("Recycle Bin"), Icon::Delete, TRASH_URI),
    ];
    places
        .into_iter()
        .map(|(label, glyph, uri)| {
            let item = MenuItem::with_text_target(&label, glyph, WindowAction::GoTo, uri);
            let emphasised = first_crumb.is_some_and(|first| same_location(first, uri));
            MenuItem { emphasised, ..item }.into()
        })
        .collect()
}

/// The menu of the « button: the `hidden` crumbs (label and location),
/// first to last so the nearest is last, then the top-level `places`.
pub(super) fn hidden_crumbs_menu(hidden: &[(String, String)], places: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = hidden
        .iter()
        .map(|(label, uri)| MenuItem::with_text_target(label, Icon::Folder, WindowAction::GoTo, uri).into())
        .collect();
    if !entries.is_empty() {
        entries.push(MenuEntry::Divider);
    }
    entries.extend(places);
    entries
}

/// Opens the subfolder menu of `folder`, with `shown` in bold, on a click
/// on the divider `label` after its crumb.
pub(super) fn open_subfolders_on_click(label: &gtk::Label, folder: &str, shown: &str) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let target = (folder, shown).to_variant();
    click.connect_released(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(label) = gesture.widget() {
            WindowAction::CrumbSubfolders.activate_from(&label, Some(&target));
        }
    });
    label.add_controller(click);
    label.set_cursor_from_name(Some("pointer"));
    label.set_tooltip_text(Some(&ox_core::i18n::gettext("Show subfolders")));
}

/// Gives the crumb `button` of `folder` its subfolder menu on Down, with
/// `next` in bold.
pub(super) fn open_subfolders_on_down(button: &gtk::Button, folder: &str, next: Option<&str>) {
    let keys = gtk::EventControllerKey::new();
    let menu_target = (folder, next.unwrap_or_default()).to_variant();
    keys.connect_key_pressed(move |keys, key, _, modifiers| {
        let down = matches!(key, gdk::Key::Down | gdk::Key::KP_Down);
        if !down || !modifiers.is_empty() {
            return glib::Propagation::Proceed;
        }
        if let Some(button) = keys.widget() {
            WindowAction::CrumbSubfolders.activate_from(&button, Some(&menu_target));
        }
        glib::Propagation::Stop
    });
    button.add_controller(keys);
}

impl BrowserWindow {
    /// Adds the action the crumbs' menus run.
    pub(super) fn install_crumb_actions(&self) {
        self.add_action_entries([gio::ActionEntry::builder(WindowAction::CrumbSubfolders.name())
            .parameter_type(Some(glib::VariantTy::new("(ss)").expect("a valid type")))
            .activate(|window: &BrowserWindow, _, target| {
                let Some((folder, shown)) = target.and_then(glib::Variant::get::<(String, String)>) else {
                    return;
                };
                window.show_subfolder_menu(folder, shown);
            })
            .build()]);
    }

    /// Whether hidden files are shown, so the menus list hidden folders.
    pub(super) fn shows_hidden_files(&self) -> bool {
        let state = self.window_action_state(WindowAction::Hidden);
        state.and_then(|state| state.get::<bool>()).unwrap_or(false)
    }

    /// Lists `folder`'s subfolders and opens their menu under its crumb.
    fn show_subfolder_menu(&self, folder: String, shown: String) {
        let show_hidden = self.shows_hidden_files();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let folders = list_subfolders(&folder, show_hidden).await;
                let entries = subfolder_menu(&folders, &folder, &shown);
                window.popup_crumb_menu(&folder, entries, true);
            }
        ));
    }

    /// The places the chevron after the location icon lists, the one the
    /// crumbs start at in bold.
    fn top_level_places(&self) -> Vec<MenuEntry> {
        let first = self.address_bar().crumb_buttons().into_iter().next();
        let first_uri = first
            .and_then(|crumb| crumb.action_target_value())
            .and_then(|target| target.get::<String>());
        // Home leads where the navigation pane's Home does (HOME-012).
        let home = self.home_location();
        places_menu(&home, first_uri.as_deref())
    }

    /// Where the chevron's top-level places lead, in order, for tests.
    #[cfg(test)]
    pub(crate) fn top_level_places_targets(&self) -> Vec<String> {
        self.top_level_places()
            .into_iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => item.target.and_then(|target| target.get::<String>()),
                MenuEntry::Divider => None,
            })
            .collect()
    }

    /// Opens the top-level places under the chevron `anchor`.
    pub(super) fn show_places_menu(&self, anchor: &gtk::Widget) {
        self.popup_address_menu(anchor, self.top_level_places(), true);
    }

    /// Opens the menu of the « button `anchor`: the `hidden` crumbs, then
    /// the top-level places.
    pub(super) fn show_hidden_crumbs_menu(&self, anchor: &gtk::Widget, hidden: &[gtk::Button]) {
        let hidden: Vec<(String, String)> = hidden
            .iter()
            .filter_map(|crumb| {
                let uri = crumb.action_target_value()?.get::<String>()?;
                Some((crumb.label()?.to_string(), uri))
            })
            .collect();
        let entries = hidden_crumbs_menu(&hidden, self.top_level_places());
        self.popup_address_menu(anchor, entries, true);
    }

    /// Opens `entries` under the crumb of `folder`, closing on a click
    /// elsewhere when `autohide`; nothing when there are none or the crumb
    /// is gone.
    pub(super) fn popup_crumb_menu(
        &self,
        folder: &str,
        entries: Vec<MenuEntry>,
        autohide: bool,
    ) -> Option<MenuPopover> {
        let crumb = self.address_bar().crumb_buttons().into_iter().find(|crumb| {
            let target = crumb.action_target_value();
            target.as_ref().and_then(glib::Variant::str) == Some(folder)
        })?;
        self.popup_address_menu(crumb.upcast_ref(), entries, autohide)
    }

    /// Opens `entries` under `anchor` in the address bar, closing on a
    /// click elsewhere when `autohide`; nothing when there are none or
    /// `anchor` is not shown.
    fn popup_address_menu(
        &self,
        anchor: &gtk::Widget,
        entries: Vec<MenuEntry>,
        autohide: bool,
    ) -> Option<MenuPopover> {
        if entries.is_empty() {
            return None;
        }
        // Parented to the bar, not the crumb: choosing a row navigates,
        // which rebuilds the crumbs while the menu is still attached.
        let bar = self.address_bar();
        let bounds = anchor.compute_bounds(bar)?;
        // The bar keeps one crumb menu and shows it again each time
        // ([`MenuPopover::kept_by`]).
        let popover = MenuPopover::kept_by(bar, "crumb-menu");
        popover.set_entries(entries);
        popover.set_autohide(autohide);
        #[expect(clippy::cast_possible_truncation, reason = "widget bounds are small")]
        let pointing = gdk::Rectangle::new(
            bounds.x() as i32,
            bounds.y() as i32,
            bounds.width() as i32,
            bounds.height() as i32,
        );
        popover.set_pointing_to(Some(&pointing));
        // A menu closed without a choice, by Escape or a click elsewhere,
        // gives the keyboard back to where it was when it opened, such as
        // the crumb Down opened it from; GTK moved it on to the next
        // control, the search box.
        let opener = GtkWindowExt::focus(self).map(|focus| focus.downgrade());
        popover.after_close(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |popover| {
                let chose = popover.chose_an_item();
                let opener = opener.clone();
                glib::idle_add_local_once(move || {
                    let opener = opener.and_then(|opener| opener.upgrade());
                    if let Some(opener) = opener.filter(|opener| !chose && opener.is_drawable()) {
                        if GtkWindowExt::focus(&window).as_ref() != Some(&opener) {
                            opener.grab_focus();
                        }
                    }
                });
            }
        ));
        popover.popup();
        Some(popover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(count: usize) -> Vec<Subfolder> {
        (0..count)
            .map(|number| Subfolder {
                name: format!("Folder {number}"),
                uri: format!("file:///demo/Folder%20{number}"),
            })
            .collect()
    }

    fn items(entries: &[MenuEntry]) -> Vec<&MenuItem> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item),
                MenuEntry::Divider => None,
            })
            .collect()
    }

    /// parity: NAV-020
    #[test]
    fn a_crumb_menu_lists_every_subfolder_with_the_shown_one_bold() {
        let many = folders(450);

        let entries = subfolder_menu(&many, "file:///demo", "file:///demo/Folder%20300");

        let listed = items(&entries);
        assert_eq!(listed.len(), 450, "no page and no More, as in Windows");
        let bold: Vec<&str> = listed
            .iter()
            .filter(|item| item.emphasised)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(bold, ["Folder 300"]);
        assert_eq!(listed[0].target, Some("file:///demo/Folder%200".to_variant()));
        assert_eq!(listed[449].target, Some("file:///demo/Folder%20449".to_variant()));
        assert!(subfolder_menu(&[], "file:///demo", "").is_empty());
    }

    /// A folder of 50,000 subfolders lists the first thousand, then an
    /// item that opens the folder, so its menu is never built row by row
    /// for all of them.
    ///
    /// parity: NAV-020
    #[test]
    fn a_crumb_menu_of_a_huge_folder_stops_at_a_thousand_and_opens_the_folder() {
        let many = folders(50_000);

        let entries = subfolder_menu(&many, "file:///demo", "file:///demo/Folder%20999");

        let listed = items(&entries);
        assert_eq!(listed.len(), MOST_LISTED_SUBFOLDERS + 1);
        assert!(
            listed[MOST_LISTED_SUBFOLDERS - 1].emphasised,
            "the shown one is bold"
        );
        assert_eq!(entries[MOST_LISTED_SUBFOLDERS], MenuEntry::Divider);
        let last = listed[MOST_LISTED_SUBFOLDERS];
        assert_eq!(last.label, "Show all 50000 folders");
        assert_eq!(last.target, Some("file:///demo".to_variant()));
        let at_the_limit = subfolder_menu(&many[..MOST_LISTED_SUBFOLDERS], "file:///demo", "");
        assert_eq!(
            items(&at_the_limit).len(),
            MOST_LISTED_SUBFOLDERS,
            "no extra item"
        );
    }

    /// A crumb menu of a thousand subfolders builds its first hundred rows
    /// at once and the rest a batch at a time while the window is idle,
    /// until every item has its row.
    ///
    /// parity: NAV-020
    #[gtk::test]
    fn a_long_crumb_menu_builds_its_rows_a_batch_at_a_time() {
        let many = folders(5_000);
        let entries = subfolder_menu(&many, "file:///demo", "");
        let menu = MenuPopover::new(entries);
        let rows = || {
            (0..=1_001)
                .take_while(|index| menu.row_list().row_at_index(*index).is_some())
                .count()
        };

        assert_eq!(rows(), 100, "the first batch only");
        assert!(!menu.has_every_row());
        crate::test_support::harness::wait_until("every row", || menu.has_every_row());
        assert_eq!(rows(), 1_001, "a thousand folders and Show all");
        let last = menu.row_list().row_at_index(1_000).expect("a last row");
        assert!(last.has_css_class("after-divider"), "Show all follows a divider");
    }

    /// parity: NAV-017, NAV-023
    #[test]
    fn the_first_chevron_lists_the_places_and_the_overflow_the_hidden_crumbs_first() {
        let home = "file:///home/demo";
        let places = places_menu(home, Some(home));
        let labels: Vec<&str> = items(&places).iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, ["Home", "This PC", "Network", "Recycle Bin"]);
        let bold: Vec<&str> = items(&places)
            .iter()
            .filter(|item| item.emphasised)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(bold, ["Home"]);
        assert!(items(&places_menu(home, Some("file:///")))
            .iter()
            .all(|item| !item.emphasised));

        let hidden = [
            ("Home".to_owned(), home.to_owned()),
            ("Projects".to_owned(), format!("{home}/Projects")),
        ];
        let entries = hidden_crumbs_menu(&hidden, places_menu(home, None));

        let labels: Vec<&str> = items(&entries).iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Home", "Projects", "Home", "This PC", "Network", "Recycle Bin"],
            "the nearest hidden crumb last, then the places"
        );
        assert_eq!(entries[2], MenuEntry::Divider);
        assert_eq!(
            items(&entries)[1].target,
            Some(format!("{home}/Projects").to_variant())
        );
    }
}
