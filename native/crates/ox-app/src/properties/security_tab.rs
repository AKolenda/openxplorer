// SPDX-License-Identifier: AGPL-3.0-only
//! The Security tab, laid out as Windows 11's, over the item's Linux
//! permissions (PROP-006, PROP-007).
//!
//! Object name, then the group or user names (the owner, the group and
//! everyone else, which is all a Linux item has), "To change permissions,
//! click Edit." with Edit..., the permissions of the name selected with
//! Allow and Deny columns, and Advanced. The names are Windows' read from
//! the permission bits ([`super::security_model`]); Deny stays empty, as
//! Linux has no deny entries.
//!
//! Edit... ([`edit`]) changes the bits, and Advanced ([`advanced`]) the
//! owner (for the superuser), the group, the special bits and whether a
//! folder's change reaches everything inside it. Each applies by its own
//! OK or Apply, as Windows' do, and the tab then reads the item again.
//! Both are greyed unless the user owns the item and it is outside
//! previous versions and shares.

mod advanced;
mod edit;

use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use ox_core::permissions::{Account, PermissionClass};
use ox_core::versions::PreviousVersions;

use super::security_model::Level;
use super::window::{push_button, PropertiesWindow, Rows};
use crate::icons::{self, Icon};

/// The glyph of an allowed permission.
const CHECK_GLYPH: i32 = 12;
/// The glyph beside a name in the list.
const NAME_GLYPH: i32 = 16;

/// What the Security tab shows of one item.
#[derive(Debug, Clone)]
pub(super) struct SecurityFacts {
    pub uri: String,
    /// The item's name, for the dialogs' titles.
    pub name: String,
    /// The item's full path, as Windows' Object name.
    pub object_name: String,
    pub owner: Option<Account>,
    pub group: Option<Account>,
    /// The permission bits, with the special ones.
    pub mode: Option<u32>,
    /// The bits as Linux writes them, `0o644`, for Advanced.
    pub mode_text: Option<String>,
    pub is_folder: bool,
    /// Edit... and Advanced can change the permissions.
    pub can_edit: bool,
}

impl SecurityFacts {
    /// The name of `class` in the list: `alex (Owner)`.
    pub(super) fn principal(&self, class: PermissionClass) -> String {
        let name = |account: &Option<Account>| {
            account.as_ref().map_or_else(
                || ox_core::i18n::gettext("Unknown"),
                |account| account.name.clone(),
            )
        };
        match class {
            PermissionClass::Owner => {
                ox_core::i18n::format_message("{name} (Owner)", &[("name", &name(&self.owner))])
            }
            PermissionClass::Group => {
                ox_core::i18n::format_message("{name} (Group)", &[("name", &name(&self.group))])
            }
            PermissionClass::Others => ox_core::i18n::gettext("Everyone else (Others)"),
        }
    }
}

/// What the tab's dialogs need besides the item.
#[derive(Clone)]
pub(super) struct SecurityContext {
    pub versions: Arc<PreviousVersions>,
    /// Reads the item again and refills the tab, after a dialog applied.
    pub reread: Rc<dyn Fn()>,
}

impl std::fmt::Debug for SecurityContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("SecurityContext").finish_non_exhaustive()
    }
}

/// Fills `page` with the Security tab of `facts`, replacing what it
/// showed.
pub(super) fn fill_security(page: &gtk::Box, facts: &SecurityFacts, context: &SecurityContext) {
    while let Some(child) = page.first_child() {
        page.remove(&child);
    }
    let object = Rows::new();
    object.add_line(&ox_core::i18n::gettext("Object name"), &facts.object_name);
    page.append(object.widget());
    let names = NameList::new(facts);
    page.append(&caption(
        &ox_core::i18n::gettext("Group or user names:"),
        names.list.upcast_ref(),
    ));
    page.append(&names.frame);

    let edit_row = gtk::Box::builder().css_classes(["properties-button-row"]).build();
    let edit_text = super::window::paragraph(&ox_core::i18n::gettext("To change permissions, click Edit."));
    edit_text.set_hexpand(true);
    edit_row.append(&edit_text);
    let edit = push_button(&ox_core::i18n::gettext("Edit..."));
    edit.set_sensitive(facts.can_edit && facts.mode.is_some());
    edit_row.append(&edit);
    page.append(&edit_row);

    let grid = PermissionGrid::new(facts.is_folder, false);
    page.append(grid.heading());
    page.append(grid.frame());
    let shown = Rc::new((names.clone(), grid, facts.clone()));
    let show = {
        let shown = Rc::clone(&shown);
        move || {
            let (names, grid, facts) = &*shown;
            let class = names.selected();
            grid.show(facts.mode.unwrap_or_default(), class, &facts.principal(class));
        }
    };
    show();
    names.list.connect_selected_rows_changed(move |_| show());

    let advanced_row = gtk::Box::builder().css_classes(["properties-button-row"]).build();
    let advanced_text = super::window::paragraph(&ox_core::i18n::gettext(
        "For special permissions or advanced settings, click Advanced.",
    ));
    advanced_text.set_hexpand(true);
    advanced_row.append(&advanced_text);
    let advanced = push_button(&ox_core::i18n::gettext("Advanced"));
    advanced.set_valign(gtk::Align::Center);
    advanced_row.append(&advanced);
    page.append(&advanced_row);

    let (facts_edit, context_edit) = (facts.clone(), context.clone());
    let names_edit = names.clone();
    edit.connect_clicked(move |button| {
        if let Some(window) = PropertiesWindow::of(button) {
            edit::open(&window, &facts_edit, &context_edit, names_edit.selected());
        }
    });
    let (facts, context) = (facts.clone(), context.clone());
    advanced.connect_clicked(move |button| {
        if let Some(window) = PropertiesWindow::of(button) {
            advanced::open(&window, &facts, &context);
        }
    });
}

/// A caption over a list, which names it for assistive technologies.
fn caption(text: &str, labelled: &gtk::Widget) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["properties-caption"])
        .build();
    labelled.update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
    label
}

/// The owner, the group and others, one selected.
#[derive(Debug, Clone)]
pub(super) struct NameList {
    pub frame: gtk::Frame,
    pub list: gtk::ListBox,
}

impl NameList {
    pub(super) fn new(facts: &SecurityFacts) -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["properties-names"])
            .build();
        for class in PermissionClass::ALL {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let glyph = match class {
                PermissionClass::Owner => Icon::Home,
                PermissionClass::Group | PermissionClass::Others => Icon::Organization,
            };
            row.append(&icons::image(glyph, NAME_GLYPH));
            row.append(
                &gtk::Label::builder()
                    .label(facts.principal(class))
                    .xalign(0.0)
                    .build(),
            );
            list.append(&row);
        }
        list.select_row(list.row_at_index(0).as_ref());
        let frame = gtk::Frame::builder()
            .child(&list)
            .css_classes(["properties-list-frame"])
            .build();
        Self { frame, list }
    }

    /// The class of the name selected.
    pub(super) fn selected(&self) -> PermissionClass {
        let index = self.list.selected_row().map_or(0, |row| row.index());
        let index = usize::try_from(index).unwrap_or_default();
        PermissionClass::ALL
            .get(index)
            .copied()
            .unwrap_or(PermissionClass::Owner)
    }

    /// Selects `class`.
    pub(super) fn select(&self, class: PermissionClass) {
        let index = PermissionClass::ALL
            .iter()
            .position(|listed| *listed == class)
            .unwrap_or(0);
        let row = self.list.row_at_index(i32::try_from(index).unwrap_or(0));
        self.list.select_row(row.as_ref());
    }
}

/// "Permissions for <name>" with Allow and Deny columns, and one row per
/// permission name: check marks in the tab, check boxes in Edit....
#[derive(Debug, Clone)]
pub(super) struct PermissionGrid {
    heading: gtk::Grid,
    title: gtk::Label,
    frame: gtk::Frame,
    /// Each name, with its Allow mark or check box.
    rows: Vec<(Level, gtk::Widget)>,
    is_folder: bool,
}

impl PermissionGrid {
    /// The grid of a file's or folder's names; with check boxes when
    /// `editable`.
    pub(super) fn new(is_folder: bool, editable: bool) -> Self {
        let heading = gtk::Grid::builder()
            .css_classes(["properties-permission-heading"])
            .build();
        // A long user or group name ends in "…", with the whole heading as
        // the tooltip, rather than widening the window.
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        heading.attach(&title, 0, 0, 1, 1);
        for (column, text) in [
            (1, crate::i18n::message_id("Allow")),
            (2, crate::i18n::message_id("Deny")),
        ] {
            let label = gtk::Label::builder()
                .label(ox_core::i18n::gettext(text))
                .css_classes(["properties-permission-column"])
                .build();
            heading.attach(&label, column, 0, 1, 1);
        }
        let grid = gtk::Grid::builder()
            .css_classes(["properties-permissions"])
            .build();
        let mut rows = Vec::new();
        for (line, level) in (0..).zip(Level::listed(is_folder)) {
            let name = gtk::Label::builder()
                .label(level.label())
                .xalign(0.0)
                .hexpand(true)
                .build();
            grid.attach(&name, 0, line, 1, 1);
            let (allow, deny): (gtk::Widget, gtk::Widget) = if editable {
                let allow = gtk::CheckButton::new();
                allow.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::format_message(
                    "Allow {permission}",
                    &[("permission", level.label())],
                ))]);
                allow.set_sensitive(level != Level::Special);
                let deny = gtk::CheckButton::new();
                deny.set_sensitive(false);
                deny.set_tooltip_text(Some(&ox_core::i18n::gettext(
                    "Linux permissions have no deny entries.",
                )));
                deny.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::format_message(
                    "Deny {permission}",
                    &[("permission", level.label())],
                ))]);
                (allow.upcast(), deny.upcast())
            } else {
                let allow = icons::image(Icon::Checkmark, CHECK_GLYPH);
                allow.add_css_class("properties-allowed");
                (
                    allow.upcast(),
                    gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast(),
                )
            };
            for (column, widget) in [(1, &allow), (2, &deny)] {
                widget.set_halign(gtk::Align::Center);
                widget.add_css_class("properties-permission-cell");
                grid.attach(widget, column, line, 1, 1);
            }
            rows.push((level, allow));
        }
        let frame = gtk::Frame::builder()
            .child(&grid)
            .css_classes(["properties-list-frame"])
            .build();
        frame.update_relation(&[gtk::accessible::Relation::LabelledBy(&[title.upcast_ref()])]);
        Self {
            heading,
            title,
            frame,
            rows,
            is_folder,
        }
    }

    pub(super) fn heading(&self) -> &gtk::Grid {
        &self.heading
    }

    pub(super) fn frame(&self) -> &gtk::Frame {
        &self.frame
    }

    /// Shows what `class`, called `name`, is allowed under `mode`.
    pub(super) fn show(&self, mode: u32, class: PermissionClass, name: &str) {
        let title = ox_core::i18n::format_message("Permissions for {name}", &[("name", name)]);
        self.title.set_text(&title);
        self.title.set_tooltip_text(Some(&title));
        for (level, allow) in &self.rows {
            let allowed = level.is_allowed(mode, class, self.is_folder);
            if let Some(check) = allow.downcast_ref::<gtk::CheckButton>() {
                check.set_active(allowed);
            } else {
                allow.set_opacity(if allowed { 1.0 } else { 0.0 });
                allow.update_property(&[gtk::accessible::Property::Label(&if allowed {
                    ox_core::i18n::format_message("{permission} allowed", &[("permission", level.label())])
                } else {
                    ox_core::i18n::format_message(
                        "{permission} not allowed",
                        &[("permission", level.label())],
                    )
                })]);
            }
        }
    }

    /// The Allow check boxes, with their names, in Edit....
    pub(super) fn checks(&self) -> Vec<(Level, gtk::CheckButton)> {
        self.rows
            .iter()
            .filter_map(|(level, allow)| Some((*level, allow.clone().downcast::<gtk::CheckButton>().ok()?)))
            .collect()
    }
}

/// The change from `before`, the bits a dialog opened on, to `after`, the
/// bits the user left: only those turned on or off, applied to the mode
/// the item has when the change is made.
pub(super) fn changed_bits(before: u32, after: u32) -> ox_core::permissions::ModeChange {
    ox_core::permissions::ModeChange::Bits {
        set: after & !before,
        clear: before & !after,
    }
}

/// Applies `request` to the item of `facts`, refusing a previous version
/// (PROP-024) and holding off formatting while it runs (DEV-012).
pub(super) async fn apply_request(
    window: &PropertiesWindow,
    facts: &SecurityFacts,
    context: &SecurityContext,
    request: ox_core::permissions::PermissionRequest,
) -> Result<(), String> {
    context
        .versions
        .check_writable(&facts.uri)
        .map_err(|refusal| refusal.to_string())?;
    let _writing = match window.opener() {
        Some(opener) => Some(opener.hold_write()?),
        None => None,
    };
    ox_core::permissions::apply_in_background(
        facts.uri.clone(),
        request,
        ox_core::transfer::Cancellation::new(),
    )
    .await
    .map_err(|error| error.to_string())?;
    (context.reread)();
    Ok(())
}
