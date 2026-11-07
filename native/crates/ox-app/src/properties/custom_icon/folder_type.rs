// SPDX-License-Identifier: AGPL-3.0-only
//! "What kind of folder do you want?" (PROP-036): the template a folder is
//! optimized for, as Windows 11's Customize tab offers it. The list starts
//! on the folder's template (its own, or the one a folder above gave its
//! sub-folders), else General items; "Also apply this template to all
//! subfolders" starts clear, as in Windows. On OK or Apply the template's
//! view is saved for the folder in the app's settings, beside the views
//! it keeps for each folder, and every window showing it follows.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::FolderTemplate;

use super::super::window::{group, ApplyFuture, Pending, PropertiesWindow};

/// The list's names for the templates, in [`FolderTemplate::ALL`]'s order.
fn template_name(template: FolderTemplate) -> String {
    match template {
        FolderTemplate::GeneralItems => ox_core::i18n::gettext("General items"),
        FolderTemplate::Documents => ox_core::i18n::gettext("Documents"),
        FolderTemplate::Pictures => ox_core::i18n::gettext("Pictures"),
        FolderTemplate::Music => ox_core::i18n::gettext("Music"),
        FolderTemplate::Videos => ox_core::i18n::gettext("Videos"),
    }
}

/// The template chosen, until OK or Apply.
struct TemplateChoice {
    window: glib::WeakRef<PropertiesWindow>,
    /// The folder's location, which a rename on General changes.
    uri: Rc<RefCell<String>>,
    /// The template and whether it was applied to the sub-folders, as
    /// last applied or as the folder was when the window opened. Clearing
    /// the box after applying it changes nothing, as in Windows.
    applied: Cell<(FolderTemplate, bool)>,
    list: gtk::DropDown,
    subfolders: gtk::CheckButton,
}

impl TemplateChoice {
    fn chosen(&self) -> (FolderTemplate, bool) {
        let template = usize::try_from(self.list.selected())
            .ok()
            .and_then(|at| FolderTemplate::ALL.get(at).copied())
            .unwrap_or(FolderTemplate::GeneralItems);
        (template, self.subfolders.is_active())
    }
}

impl Pending for TemplateChoice {
    fn has_changes(&self) -> bool {
        let (template, subfolders) = self.chosen();
        let (applied, applied_below) = self.applied.get();
        template != applied || (subfolders && !applied_below)
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(async move {
            let (template, subfolders) = self.chosen();
            let opener = self
                .window
                .upgrade()
                .and_then(|window| window.opener())
                .ok_or_else(|| ox_core::i18n::gettext("The folder's window has closed."))?;
            let uri = self.uri.borrow().clone();
            let (sender, receiver) = async_channel::bounded(1);
            opener.customize_folder(&uri, template, subfolders, move |result| {
                let _ = sender.try_send(result);
            });
            receiver
                .recv()
                .await
                .map_err(|_| ox_core::i18n::gettext("The folder's window has closed."))?
                .map_err(|error| error.to_string())?;
            self.applied.set((template, subfolders));
            Ok(())
        })
    }
}

/// The "What kind of folder do you want?" group of the folder at `uri`.
pub(super) fn folder_type_group(window: &PropertiesWindow, uri: &Rc<RefCell<String>>) -> gtk::Frame {
    let (frame, content) = group(&ox_core::i18n::gettext("What kind of folder do you want?"));
    let caption = gtk::Label::builder()
        .label(ox_core::i18n::gettext("Optimize this folder for:"))
        .xalign(0.0)
        .css_classes(["properties-text"])
        .build();
    let names: Vec<String> = FolderTemplate::ALL.into_iter().map(template_name).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let list = gtk::DropDown::from_strings(&name_refs);
    list.add_css_class("folder-template");
    list.set_hexpand(true);
    list.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
    let subfolders = gtk::CheckButton::with_mnemonic(&ox_core::i18n::gettext(
        "Also apply this template to all subfolders",
    ));
    content.append(&caption);
    content.append(&list);
    content.append(&subfolders);
    let current = window
        .opener()
        .and_then(|opener| opener.folder_template(&uri.borrow()))
        .unwrap_or(FolderTemplate::GeneralItems);
    let at = FolderTemplate::ALL
        .iter()
        .position(|template| *template == current)
        .and_then(|at| u32::try_from(at).ok())
        .unwrap_or_default();
    list.set_selected(at);
    let choice = Rc::new(TemplateChoice {
        window: window.downgrade(),
        uri: Rc::clone(uri),
        applied: Cell::new((current, false)),
        list: list.clone(),
        subfolders: subfolders.clone(),
    });
    let changed = window.downgrade();
    list.connect_selected_notify(move |_| {
        if let Some(window) = changed.upgrade() {
            window.changed();
        }
    });
    let changed = window.downgrade();
    subfolders.connect_toggled(move |_| {
        if let Some(window) = changed.upgrade() {
            window.changed();
        }
    });
    window.add_pending(choice);
    frame
}
