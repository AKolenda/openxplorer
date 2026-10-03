// SPDX-License-Identifier: AGPL-3.0-only
//! The user's templates in the New menu: each file of the Templates folder
//! as an item, each subfolder as a submenu (OPS-003).
//!
//! As Nautilus's New Document menu and Dolphin's Create New, the menu
//! names a template by its file name without the extension, and choosing
//! one opens the New from template dialog with it chosen, so the name can
//! be changed before the file is made; nothing is ever overwritten
//! (OPS-048). The list is read on GIO's worker threads when the window
//! opens and again whenever a New menu opens, so a template added
//! meanwhile appears.

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::ops::{list_templates, Template, TemplateList};
use ox_core::transfer::Cancellation;

use super::template_dialog::templates_folder;
use crate::icons::Icon;
use crate::window::menu_popover::{MenuEntry, MenuItem};
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// How the New menu names a template: its file name without the
/// extension, unless that would leave nothing.
fn menu_label(template: &Template) -> &str {
    let file_name = template.suggested_name.as_str();
    match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
    }
}

/// The New menu's entries for the user templates of `list` whose path
/// below Templates starts with the folders `within`: the files there, then
/// a submenu per subfolder, in list order.
fn entries_within(list: &TemplateList, within: &[&str]) -> Vec<MenuEntry> {
    let mut files = Vec::new();
    let mut folders: Vec<&str> = Vec::new();
    for template in list.templates.iter().filter(|template| !template.is_builtin()) {
        let path = template.folders();
        let Some(rest) = path.strip_prefix(within) else {
            continue;
        };
        match rest.first() {
            None => {
                let target = template.id.to_string();
                let item = MenuItem::with_text_target(
                    menu_label(template),
                    Icon::Document,
                    WindowAction::NewFromUserTemplate,
                    &target,
                );
                files.push(item.into());
            }
            Some(folder) if !folders.contains(folder) => folders.push(folder),
            Some(_) => {}
        }
    }
    for folder in folders {
        let inner: Vec<&str> = within.iter().copied().chain([folder]).collect();
        let submenu = entries_within(list, &inner);
        files.push(MenuItem::submenu(folder, Icon::Folder, WindowAction::ShowNewMenu, submenu).into());
    }
    files
}

/// The New menu's entries for the user templates of `list`.
pub(in crate::window) fn template_entries(list: &TemplateList) -> Vec<MenuEntry> {
    entries_within(list, &[])
}

impl BrowserWindow {
    /// The New menu's entries for the templates last read.
    pub(in crate::window) fn template_menu_entries(&self) -> Vec<MenuEntry> {
        let operations = self.imp().file_operations.borrow();
        operations
            .templates
            .as_ref()
            .map(template_entries)
            .unwrap_or_default()
    }

    /// Reads the Templates folder again, then gives the command bar's New
    /// menu the templates it lists.
    pub(in crate::window) fn refresh_template_menu(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let Some(folder) = templates_folder().await else {
                    return;
                };
                let Ok(list) = list_templates(&folder, &Cancellation::new()).await else {
                    return;
                };
                let changed = window.imp().file_operations.borrow().templates.as_ref() != Some(&list);
                if !changed {
                    return;
                }
                window.imp().file_operations.borrow_mut().templates = Some(list);
                if let Some(menu) = window.command_bar().new_menu_popover() {
                    menu.set_entries(crate::window::command_bar::new_menu(
                        window.template_menu_entries(),
                    ));
                }
            }
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ox_core::ops::TemplateId;

    use super::*;

    fn user(path: &str) -> Template {
        let file_name = path.rsplit('/').next().unwrap_or(path);
        Template {
            id: TemplateId::User(path.to_owned()),
            label: path.to_owned(),
            suggested_name: file_name.to_owned(),
        }
    }

    /// The labels of `entries`, a submenu with its entries in brackets.
    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) if item.submenu.is_empty() => item.label.clone(),
                MenuEntry::Item(item) => format!("{} {:?}", item.label, labels(&item.submenu)),
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect()
    }

    /// parity: OPS-003
    #[test]
    fn templates_are_listed_directly_with_subfolders_as_submenus() {
        let list = TemplateList {
            templates: vec![
                user("Letter.odt"),
                user("Office/Invoice.ods"),
                user("Office/Legal/Contract.odt"),
                user(".bashrc"),
            ],
            folder: PathBuf::from("/home/user/Templates"),
        };

        let entries = template_entries(&list);

        assert_eq!(
            labels(&entries),
            [
                "Letter",
                ".bashrc",
                r#"Office ["Invoice", "Legal [\"Contract\"]"]"#
            ]
        );
        let MenuEntry::Item(letter) = &entries[0] else {
            panic!("a template is an item");
        };
        assert_eq!(
            letter.target.as_ref().and_then(glib::Variant::str),
            Some("user:Letter.odt")
        );
    }
}
