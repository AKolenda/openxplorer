// SPDX-License-Identifier: AGPL-3.0-only
//! Advanced: Windows' "Advanced Security Settings" over what Linux has
//! beyond the three entries' bits. Name, Owner (a choice for the
//! superuser, who alone may give an item away) and Group (one of the
//! user's groups), the permission entries as Windows lists them, the
//! special permissions (Set user ID, Set group ID, Sticky), and for a
//! folder "Replace all child object permission entries...", which applies
//! the folder's bits to everything inside it, as `chmod -R` does. OK or
//! Apply makes the change.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::permissions::{
    current_user_is_superuser, group_choices, user_choices, PermissionClass, PermissionRequest,
};

use super::{apply_request, changed_bits, SecurityContext, SecurityFacts};
use crate::properties::permissions_editor::SharedAccountChoice as AccountChoice;
use crate::properties::security_model::{access_summary, special_bit};
use crate::properties::window::{ApplyFuture, Pending, PropertiesWindow, Rows};

/// The special bits and their names.
const SPECIAL: [(PermissionClass, &str); 3] = [
    (PermissionClass::Owner, crate::i18n::message_id("Set user ID")),
    (PermissionClass::Group, crate::i18n::message_id("Set group ID")),
    (
        PermissionClass::Others,
        crate::i18n::message_id("Sticky (only owners rename and delete content)"),
    ),
];

/// What Advanced holds until OK or Apply.
struct AdvancedChanges {
    window: glib::WeakRef<PropertiesWindow>,
    facts: SecurityFacts,
    context: SecurityContext,
    owner: Option<AccountChoice>,
    group: AccountChoice,
    special: Vec<(u32, gtk::CheckButton)>,
    replace: Option<gtk::CheckButton>,
}

impl AdvancedChanges {
    fn mode(&self) -> u32 {
        let mode = self.facts.mode.unwrap_or_default();
        self.special.iter().fold(mode, |mode, (bit, check)| {
            if check.is_active() {
                mode | bit
            } else {
                mode & !bit
            }
        })
    }

    fn replaces(&self) -> bool {
        self.replace.as_ref().is_some_and(gtk::CheckButton::is_active)
    }
}

impl Pending for AdvancedChanges {
    fn has_changes(&self) -> bool {
        Some(self.mode()) != self.facts.mode
            || self.replaces()
            || self.group.changed().is_some()
            || self.owner.as_ref().and_then(AccountChoice::changed).is_some()
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move {
            let Some(window) = self.window.upgrade() else {
                return Ok(());
            };
            let request = PermissionRequest {
                mode: changed_bits(self.facts.mode.unwrap_or_default(), self.mode()),
                owner: self.owner.as_ref().and_then(AccountChoice::changed),
                group: self.group.changed(),
                recursive: self.replaces(),
            };
            apply_request(&window, &self.facts, &self.context, request).await?;
            // The dialog shows the item as it was opened on; after a
            // change it closes, as there is nothing left to apply.
            window.close();
            Ok(())
        })
    }
}

/// Opens Advanced over `parent`.
pub(super) fn open(
    parent: &PropertiesWindow,
    facts: &SecurityFacts,
    context: &SecurityContext,
) -> PropertiesWindow {
    let title =
        ox_core::i18n::format_message("Advanced Security Settings for {name}", &[("name", &facts.name)]);
    let dialog = PropertiesWindow::dialog_of(parent, &title);
    dialog.add_css_class("properties-wide");
    let page = crate::properties::window::page();
    let rows = Rows::new();
    rows.add(&ox_core::i18n::gettext("Name"), &facts.object_name);
    let editable = facts.can_edit;
    let owner = if current_user_is_superuser() && editable {
        let choice = AccountChoice::new(
            ox_core::i18n::gettext_static("Owner"),
            facts.owner.as_ref(),
            user_choices(),
        );
        rows.add_control(&ox_core::i18n::gettext("Owner"), &choice.choice);
        Some(choice)
    } else {
        let name = facts
            .owner
            .as_ref()
            .map(|owner| owner.name.clone())
            .unwrap_or_default();
        rows.add(&ox_core::i18n::gettext("Owner"), &name);
        None
    };
    let group = AccountChoice::new(
        ox_core::i18n::gettext_static("Group"),
        facts.group.as_ref(),
        group_choices(),
    );
    group.choice.set_sensitive(editable);
    rows.add_control(&ox_core::i18n::gettext("Group"), &group.choice);
    if let Some(mode) = &facts.mode_text {
        rows.add(&ox_core::i18n::gettext("Linux mode"), mode);
    }
    page.append(rows.widget());
    page.append(&entries(facts));

    let (special_box, special) = special_checks(facts.mode.unwrap_or_default(), editable);
    let special_rows = Rows::new();
    special_rows.add_control(&ox_core::i18n::gettext("Special permissions"), &special_box);
    page.append(special_rows.widget());
    let replace = facts.is_folder.then(|| {
        let check = gtk::CheckButton::with_label(&ox_core::i18n::gettext(
            "Replace all child object permission entries with this object's permissions",
        ));
        check.set_sensitive(editable);
        page.append(&check);
        check
    });
    dialog.add_tab("advanced", &ox_core::i18n::gettext("Permissions"), &page);
    let changes = Rc::new(AdvancedChanges {
        window: dialog.downgrade(),
        facts: facts.clone(),
        context: context.clone(),
        owner,
        group,
        special,
        replace,
    });
    let notify = {
        let window = dialog.downgrade();
        move || {
            if let Some(window) = window.upgrade() {
                window.changed();
            }
        }
    };
    for (_, check) in &changes.special {
        let notify = notify.clone();
        check.connect_toggled(move |_| notify());
    }
    if let Some(check) = &changes.replace {
        let notify = notify.clone();
        check.connect_toggled(move |_| notify());
    }
    for choice in changes
        .owner
        .iter()
        .map(|owner| &owner.choice)
        .chain([&changes.group.choice])
    {
        let notify = notify.clone();
        choice.connect_selected_notify(move |_| notify());
    }
    dialog.add_pending(changes);
    dialog.present();
    dialog
}

/// Set user ID, Set group ID and Sticky, showing `mode`, each with its bit.
fn special_checks(mode: u32, editable: bool) -> (gtk::Box, Vec<(u32, gtk::CheckButton)>) {
    let special_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["properties-attributes"])
        .build();
    let checks = SPECIAL
        .iter()
        .map(|(class, label)| {
            let bit = special_bit(*class);
            let check = gtk::CheckButton::with_label(&ox_core::i18n::gettext(label));
            check.set_active(mode & bit != 0);
            check.set_sensitive(editable);
            special_box.append(&check);
            (bit, check)
        })
        .collect();
    (special_box, checks)
}

/// "Permission entries:" with Type, Principal and Access columns: Allow,
/// each name and the highest permission it has.
fn entries(facts: &SecurityFacts) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let caption = gtk::Label::builder()
        .label(ox_core::i18n::gettext("Permission entries:"))
        .xalign(0.0)
        .css_classes(["properties-caption"])
        .build();
    column.append(&caption);
    let grid = gtk::Grid::builder().css_classes(["properties-entries"]).build();
    for (line, cells) in (0..).zip(header_and_rows(facts)) {
        for (position, text) in (0..).zip(cells) {
            let label = gtk::Label::builder()
                .label(&text)
                .xalign(0.0)
                .hexpand(position == 1)
                .build();
            if line == 0 {
                label.add_css_class("properties-entries-heading");
            }
            grid.attach(&label, position, line, 1, 1);
        }
    }
    grid.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
    let frame = gtk::Frame::builder()
        .child(&grid)
        .css_classes(["properties-list-frame"])
        .build();
    column.append(&frame);
    column
}

fn header_and_rows(facts: &SecurityFacts) -> Vec<[String; 3]> {
    let mode = facts.mode.unwrap_or_default();
    let mut rows = vec![[
        ox_core::i18n::gettext("Type"),
        ox_core::i18n::gettext("Principal"),
        ox_core::i18n::gettext("Access"),
    ]];
    for class in PermissionClass::ALL {
        rows.push([
            ox_core::i18n::gettext("Allow"),
            facts.principal(class),
            access_summary(mode, class, facts.is_folder).to_owned(),
        ]);
    }
    rows
}
