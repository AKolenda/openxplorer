// SPDX-License-Identifier: AGPL-3.0-only
//! Edit...: Windows' "Permissions for <item>" dialog over the item's
//! permission bits. Object name, the names, and the selected name's
//! permissions with Allow check boxes; Deny boxes are shown, greyed, as
//! Linux has no deny entries. OK or Apply sets the bits; the special bits
//! stay as they are.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::permissions::{ModeChange, PermissionClass, PermissionRequest};

use super::{apply_request, NameList, PermissionGrid, SecurityContext, SecurityFacts};
use crate::properties::window::{ApplyFuture, Pending, PropertiesWindow, Rows};

/// The bits the dialog holds until OK or Apply.
struct EditedBits {
    window: glib::WeakRef<PropertiesWindow>,
    facts: RefCell<SecurityFacts>,
    context: SecurityContext,
    /// The bits as the user left them.
    mode: Cell<u32>,
}

impl Pending for EditedBits {
    fn has_changes(&self) -> bool {
        Some(self.mode.get()) != self.facts.borrow().mode
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move {
            let Some(window) = self.window.upgrade() else {
                return Ok(());
            };
            let facts = self.facts.borrow().clone();
            let request = PermissionRequest {
                mode: ModeChange::Advanced(self.mode.get()),
                owner: None,
                group: None,
                recursive: false,
            };
            apply_request(&window, &facts, &self.context, request).await?;
            self.facts.borrow_mut().mode = Some(self.mode.get());
            Ok(())
        })
    }
}

/// Opens Edit... over `parent` with `selected` chosen.
pub(super) fn open(
    parent: &PropertiesWindow,
    facts: &SecurityFacts,
    context: &SecurityContext,
    selected: PermissionClass,
) -> PropertiesWindow {
    let title = ox_core::i18n::format_message("Permissions for {name}", &[("name", &facts.name)]);
    let dialog = PropertiesWindow::dialog_of(parent, &title);
    let page = crate::properties::window::page();
    let object = Rows::new();
    object.add(&ox_core::i18n::gettext("Object name"), &facts.object_name);
    page.append(object.widget());
    let names = NameList::new(facts);
    page.append(&super::caption(
        &ox_core::i18n::gettext("Group or user names:"),
        names.list.upcast_ref(),
    ));
    page.append(&names.frame);
    let grid = PermissionGrid::new(facts.is_folder, true);
    page.append(grid.heading());
    page.append(grid.frame());
    dialog.add_tab("security", &ox_core::i18n::gettext("Security"), &page);
    // The single tab shows its name, as Windows' dialog does.
    dialog.show_single_tab();

    let edited = Rc::new(EditedBits {
        window: dialog.downgrade(),
        facts: RefCell::new(facts.clone()),
        context: context.clone(),
        mode: Cell::new(facts.mode.unwrap_or_default()),
    });
    dialog.add_pending(Rc::clone(&edited) as Rc<dyn Pending>);
    let is_showing = Rc::new(Cell::new(false));
    let show = {
        let (names, grid, edited, is_showing) = (
            names.clone(),
            grid.clone(),
            Rc::clone(&edited),
            Rc::clone(&is_showing),
        );
        move || {
            is_showing.set(true);
            let class = names.selected();
            let name = edited.facts.borrow().principal(class);
            grid.show(edited.mode.get(), class, &name);
            is_showing.set(false);
        }
    };
    let show = Rc::new(show);
    for (level, check) in grid.checks() {
        let (names, edited, is_showing, show) = (
            names.clone(),
            Rc::clone(&edited),
            Rc::clone(&is_showing),
            Rc::clone(&show),
        );
        check.connect_toggled(move |check| {
            if is_showing.get() {
                return;
            }
            let is_folder = edited.facts.borrow().is_folder;
            let mode = level.set(edited.mode.get(), names.selected(), is_folder, check.is_active());
            edited.mode.set(mode);
            // Ticking one name can tick others, and clearing one clear
            // those that need it.
            show();
            if let Some(window) = edited.window.upgrade() {
                window.changed();
            }
        });
    }
    let reshow = Rc::clone(&show);
    names.list.connect_selected_rows_changed(move |_| reshow());
    names.select(selected);
    show();
    dialog.present();
    dialog
}
