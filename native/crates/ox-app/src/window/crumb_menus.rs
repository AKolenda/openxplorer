// SPDX-License-Identifier: AGPL-3.0-only
//! The breadcrumbs' subfolder menus and wheel, which app.js lacked, as
//! Dolphin's location bar and Explorer's crumb chevrons have them.
//!
//! - A click on the divider after a crumb, or Down on a focused crumb,
//!   opens a menu of that folder's subfolders. The folder the address
//!   goes on to is bold, hidden folders show only while hidden files do,
//!   and past 30 folders a "More" item lists the next ones (NAV-020).
//! - The wheel over a crumb replaces it with the previous or next folder
//!   beside it and goes there, while the crumbs fit the bar; when they
//!   overflow, the wheel scrolls them as before (NAV-022).

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::{enumerate_folder, EntryKind};
use ox_core::location::{parent_location, same_location};

use crate::folder_view::sorting::SortKey;
use crate::icons::Icon;

use super::address_bar::AddressBar;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The most subfolders one menu lists before "More" (Dolphin's limit).
const MENU_PAGE: usize = 30;

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
    let mut keyed: Vec<(SortKey, Subfolder)> = folders
        .into_iter()
        .map(|folder| (SortKey::new(&folder.name), folder))
        .collect();
    keyed.sort_by(|(a, first), (b, second)| a.natural_cmp(b).then_with(|| first.name.cmp(&second.name)));
    keyed.into_iter().map(|(_, folder)| folder).collect()
}

/// The menu of `folders` from `first` on: up to [`MENU_PAGE`] folders,
/// the one at `shown` in bold, then "More" for the rest of `parent`'s.
pub(super) fn subfolder_menu(
    folders: &[Subfolder],
    parent: &str,
    shown: &str,
    first: usize,
) -> Vec<MenuEntry> {
    let page = folders.iter().skip(first).take(MENU_PAGE);
    let mut entries: Vec<MenuEntry> = page
        .map(|folder| {
            let item =
                MenuItem::with_text_target(&folder.name, Icon::Folder, WindowAction::GoTo, &folder.uri);
            let emphasised = same_location(&folder.uri, shown);
            MenuItem { emphasised, ..item }.into()
        })
        .collect();
    let next = first + MENU_PAGE;
    if folders.len() > next {
        let next = u32::try_from(next).unwrap_or(u32::MAX);
        let target = (parent, shown, next).to_variant();
        entries.push(MenuEntry::Divider);
        entries.push(
            MenuItem::with_target(
                "More",
                Icon::MoreHorizontal,
                WindowAction::CrumbSubfolders,
                target,
            )
            .into(),
        );
    }
    entries
}

/// The folder `step` places from `current` among `folders`, if any.
fn sibling<'a>(folders: &'a [Subfolder], current: &str, step: i32) -> Option<&'a Subfolder> {
    let position = folders
        .iter()
        .position(|folder| same_location(&folder.uri, current))?;
    let target = position.checked_add_signed(isize::try_from(step).ok()?)?;
    folders.get(target)
}

/// Opens the subfolder menu of `folder`, with `shown` in bold, on a click
/// on the divider `label` after its crumb.
pub(super) fn open_subfolders_on_click(label: &gtk::Label, folder: &str, shown: &str) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let target = (folder, shown, 0_u32).to_variant();
    click.connect_released(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(label) = gesture.widget() {
            WindowAction::CrumbSubfolders.activate_from(&label, Some(&target));
        }
    });
    label.add_controller(click);
    label.set_cursor_from_name(Some("pointer"));
    label.set_tooltip_text(Some("Show subfolders"));
}

impl AddressBar {
    /// Gives the crumb `button` of `folder` its subfolder menu on Down,
    /// with `next` in bold, and its sibling switching on the wheel.
    pub(super) fn add_crumb_menu_and_wheel(&self, button: &gtk::Button, folder: &str, next: Option<&str>) {
        let keys = gtk::EventControllerKey::new();
        let menu_target = (folder, next.unwrap_or_default(), 0_u32).to_variant();
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
        // Discrete, so GTK adds up a high-resolution wheel's fractions to
        // whole notches; a touchpad's pixel deltas are ignored, as Dolphin
        // counts only whole wheel notches, so one swipe cannot skip
        // through several folders.
        let wheel = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
        );
        let folder = folder.to_owned();
        wheel.connect_scroll(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |wheel, _, dy| {
                let notches = dy.trunc();
                let touchpad = wheel.unit() == gdk::ScrollUnit::Surface;
                if bar.crumbs_overflow() || touchpad || notches == 0.0 {
                    return glib::Propagation::Proceed;
                }
                let step: i32 = if notches > 0.0 { 1 } else { -1 };
                if let Some(button) = wheel.widget() {
                    WindowAction::CrumbSibling.activate_from(&button, Some(&(&folder, step).to_variant()));
                }
                glib::Propagation::Stop
            }
        ));
        button.add_controller(wheel);
    }
}

impl BrowserWindow {
    /// Adds the actions the crumbs' menus and wheel run.
    pub(super) fn install_crumb_actions(&self) {
        self.add_action_entries([
            gio::ActionEntry::builder(WindowAction::CrumbSubfolders.name())
                .parameter_type(Some(glib::VariantTy::new("(ssu)").expect("a valid type")))
                .activate(|window: &BrowserWindow, _, target| {
                    let Some((folder, shown, first)) =
                        target.and_then(glib::Variant::get::<(String, String, u32)>)
                    else {
                        return;
                    };
                    window.show_subfolder_menu(folder, shown, first);
                })
                .build(),
            gio::ActionEntry::builder(WindowAction::CrumbSibling.name())
                .parameter_type(Some(glib::VariantTy::new("(si)").expect("a valid type")))
                .activate(|window: &BrowserWindow, _, target| {
                    if let Some((folder, step)) = target.and_then(glib::Variant::get::<(String, i32)>) {
                        window.go_to_sibling(folder, step);
                    }
                })
                .build(),
        ]);
    }

    /// Whether hidden files are shown, so the menus list hidden folders.
    pub(super) fn shows_hidden_files(&self) -> bool {
        let state = self.window_action_state(WindowAction::Hidden);
        state.and_then(|state| state.get::<bool>()).unwrap_or(false)
    }

    /// Lists `folder`'s subfolders and opens their menu under its crumb.
    fn show_subfolder_menu(&self, folder: String, shown: String, first: u32) {
        let show_hidden = self.shows_hidden_files();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let folders = list_subfolders(&folder, show_hidden).await;
                let first = usize::try_from(first).unwrap_or(usize::MAX);
                let entries = subfolder_menu(&folders, &folder, &shown, first);
                window.popup_crumb_menu(&folder, entries, true);
            }
        ));
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
        if entries.is_empty() {
            return None;
        }
        let crumb = self.address_bar().crumb_buttons().into_iter().find(|crumb| {
            let target = crumb.action_target_value();
            target.as_ref().and_then(glib::Variant::str) == Some(folder)
        })?;
        // Parented to the bar, not the crumb: choosing a row navigates,
        // which rebuilds the crumbs while the menu is still attached.
        let bar = self.address_bar();
        let bounds = crumb.compute_bounds(bar)?;
        let popover = MenuPopover::new(entries);
        popover.set_autohide(autohide);
        popover.set_parent(bar);
        #[expect(clippy::cast_possible_truncation, reason = "widget bounds are small")]
        let pointing = gdk::Rectangle::new(
            bounds.x() as i32,
            bounds.y() as i32,
            bounds.width() as i32,
            bounds.height() as i32,
        );
        popover.set_pointing_to(Some(&pointing));
        popover.connect_closed(|popover| {
            let closed = popover.clone();
            glib::idle_add_local_once(move || closed.unparent());
        });
        popover.popup();
        Some(popover)
    }

    /// Goes to the folder `step` places from `folder` among its parent's
    /// subfolders.
    pub(super) fn go_to_sibling(&self, folder: String, step: i32) {
        let Some(parent) = parent_location(&folder) else {
            return;
        };
        let show_hidden = self.shows_hidden_files();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let folders = list_subfolders(&parent, show_hidden).await;
                if let Some(next) = sibling(&folders, &folder, step) {
                    window.navigate_or_report(&next.uri);
                }
            }
        ));
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
    fn a_crumb_menu_lists_thirty_subfolders_with_the_shown_one_bold_then_more() {
        let many = folders(45);

        let first_page = subfolder_menu(&many, "file:///demo", "file:///demo/Folder%203", 0);
        let second_page = subfolder_menu(&many, "file:///demo", "file:///demo/Folder%203", 30);

        let first = items(&first_page);
        assert_eq!(first.len(), 31);
        let bold: Vec<&str> = first
            .iter()
            .filter(|item| item.emphasised)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(bold, ["Folder 3"]);
        assert_eq!(first[0].target, Some("file:///demo/Folder%200".to_variant()));
        let more = first[30];
        assert_eq!(more.label, "More");
        assert_eq!(
            more.target,
            Some(("file:///demo", "file:///demo/Folder%203", 30_u32).to_variant())
        );
        assert_eq!(items(&second_page).len(), 15);
        assert!(subfolder_menu(&[], "file:///demo", "", 0).is_empty());
    }

    /// parity: NAV-022
    #[test]
    fn the_wheel_steps_to_the_folder_beside_and_stops_at_either_end() {
        let three = folders(3);

        assert_eq!(sibling(&three, "file:///demo/Folder%201", 1), Some(&three[2]));
        assert_eq!(sibling(&three, "file:///demo/Folder%201", -1), Some(&three[0]));
        assert_eq!(sibling(&three, "file:///demo/Folder%202", 1), None);
        assert_eq!(sibling(&three, "file:///demo/Folder%200", -1), None);
    }
}
