// SPDX-License-Identifier: AGPL-3.0-only
//! The New file and New from template dialog (OPS-002, OPS-048).
//!
//! Ports `newTemplateDialog` in `v2.0.0:desktop/ui/app.js`. The dialog lists the
//! six built-in starters and the user's templates (" · Your template"),
//! read from the XDG Templates folder when it opens; choosing another
//! template puts its suggested name in the File name field. Create checks
//! the name and makes the file from a fresh list, never overwriting an
//! existing item; a refusal stays in the dialog. The new file is selected
//! afterwards and Undo moves it to the Trash.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::ops::{
    create_from_template, list_templates, BuiltinTemplate, CreatedItem, NewFromTemplate, OperationContext,
    Template, TemplateId, TemplateList,
};
use ox_core::places::{FolderLocations, KnownFolder};
use ox_core::transfer::Cancellation;

use super::names::check_typed_name;
use super::new_items::NewFileKind;
use super::FileCommand;
use crate::window::dialog::Dialog;
use crate::window::BrowserWindow;
use crate::window::ButtonStyle;

/// Why an empty office document is useless, and that templates are safe
/// (`.modal-note` in `newTemplateDialog`).
const TEMPLATE_NOTE: &str = "An empty .docx, .xlsx, .pdf, or .odt file is not a valid document. For those \
                             formats, place a real starter document in your Templates folder and choose it \
                             here. Templates are copied, never executed.";

/// The suffix of a template from the Templates folder in the list.
const USER_TEMPLATE_SUFFIX: &str = " · Your template";

/// The title and the line under it, for `kind`.
fn dialog_text(kind: NewFileKind) -> (&'static str, &'static str) {
    match kind {
        NewFileKind::Empty => (
            "New file",
            "Create an empty file with any filename and extension.",
        ),
        NewFileKind::Starter(_) | NewFileKind::AnyTemplate => (
            "New from template",
            "Create a new copy without changing the template.",
        ),
    }
}

/// The template `kind` starts with: its own, or the first of the list.
fn initial_position(kind: NewFileKind, list: &TemplateList) -> usize {
    let wanted = match kind {
        NewFileKind::Empty => Some(TemplateId::Builtin(BuiltinTemplate::Empty)),
        NewFileKind::Starter(template) => Some(TemplateId::Builtin(template)),
        NewFileKind::AnyTemplate => None,
    };
    wanted
        .and_then(|id| list.templates.iter().position(|template| template.id == id))
        .unwrap_or(0)
}

/// How the list names `template`.
fn list_label(template: &Template) -> String {
    if template.is_builtin() {
        template.label.clone()
    } else {
        format!("{}{USER_TEMPLATE_SUFFIX}", template.label)
    }
}

/// The user's Templates folder, read from `user-dirs.dirs` as the Python
/// bridge reads it for every request.
async fn templates_folder() -> Option<PathBuf> {
    let reading = gio::spawn_blocking(|| {
        let paths = FolderLocations::from_environment().read_paths();
        paths.path(KnownFolder::Templates).to_path_buf()
    });
    reading.await.ok()
}

/// The dialog's two controls.
#[derive(Debug)]
struct TemplateFields {
    name: gtk::Entry,
    choice: gtk::DropDown,
}

impl BrowserWindow {
    /// A New menu item: lists the templates, then asks for the file's
    /// name and template and creates it.
    pub(crate) async fn create_file(&self, kind: NewFileKind) {
        if !self.allows(FileCommand::New) || self.refuses_writes_during_update() {
            return;
        }
        let Some(folder_uri) = self.current_uri() else {
            return;
        };
        let Some(templates_folder) = templates_folder().await else {
            return;
        };
        let list = match list_templates(&templates_folder, &Cancellation::new()).await {
            Ok(list) => list,
            Err(error) => {
                self.show_message(&error.to_string());
                return;
            }
        };
        let created = self.ask_for_template_file(kind, &list, &folder_uri).await;
        if let Some(created) = created {
            self.finish_creation(created);
        }
    }

    /// Shows the dialog until a file is created or the user cancels.
    async fn ask_for_template_file(
        &self,
        kind: NewFileKind,
        list: &TemplateList,
        folder_uri: &str,
    ) -> Option<CreatedItem> {
        let (title, description) = dialog_text(kind);
        let dialog = Dialog::new(self, title, description);
        let fields = add_template_fields(&dialog, list, initial_position(kind, list));
        dialog.add_cancel_button();
        dialog.add_button("Create", ButtonStyle::Accent);
        dialog.open();
        loop {
            dialog.next_response().await?;
            let Some(request) = template_request(&dialog, &fields, list, folder_uri) else {
                continue;
            };
            let context = OperationContext::new(self.context().write_protection());
            dialog.set_busy(Some(&context.cancel));
            let outcome = create_from_template(&request, &context).await;
            dialog.set_busy(None);
            match outcome {
                Ok(created) => {
                    dialog.finish();
                    return Some(created);
                }
                Err(error) => dialog.show_error(&error.to_string()),
            }
        }
    }
}

/// Adds the File name field, the template list, the note and the
/// Templates folder line, `initial` chosen.
fn add_template_fields(dialog: &Dialog, list: &TemplateList, initial: usize) -> TemplateFields {
    let suggested = list
        .templates
        .get(initial)
        .map_or("", |template| template.suggested_name.as_str());
    let name = dialog.add_text_field("File name", suggested);
    let labels: Vec<String> = list.templates.iter().map(list_label).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let choice = gtk::DropDown::from_strings(&label_refs);
    choice.set_selected(u32::try_from(initial).unwrap_or(0));
    choice.add_css_class("template-select");
    dialog.add_labelled("File type / template", &choice);
    let suggestions: Vec<String> = list
        .templates
        .iter()
        .map(|template| template.suggested_name.clone())
        .collect();
    choice.connect_selected_notify(glib::clone!(
        #[weak]
        name,
        move |choice| {
            let chosen = usize::try_from(choice.selected()).ok();
            if let Some(suggestion) = chosen.and_then(|index| suggestions.get(index)) {
                name.set_text(suggestion);
            }
        }
    ));
    dialog.add_note(TEMPLATE_NOTE);
    dialog.add_hint(&format!("Templates folder: {}", list.folder.display()));
    TemplateFields { name, choice }
}

/// The creation the dialog asks for, or `None` after showing why the
/// typed name is refused.
fn template_request(
    dialog: &Dialog,
    fields: &TemplateFields,
    list: &TemplateList,
    folder_uri: &str,
) -> Option<NewFromTemplate> {
    let typed = fields.name.text();
    let name = match check_typed_name(&typed) {
        Ok(name) => name.to_owned(),
        Err(invalid) => {
            dialog.show_error(&invalid.to_string());
            return None;
        }
    };
    let chosen = usize::try_from(fields.choice.selected()).ok();
    let template = chosen.and_then(|index| list.templates.get(index))?;
    Some(NewFromTemplate {
        folder_uri: folder_uri.to_owned(),
        name,
        template: template.id.clone(),
        templates_folder: list.folder.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starters() -> TemplateList {
        let templates = BuiltinTemplate::ALL
            .into_iter()
            .map(|template| Template {
                id: TemplateId::Builtin(template),
                label: template.label().to_owned(),
                suggested_name: template.suggested_name().to_owned(),
            })
            .collect();
        TemplateList {
            templates,
            folder: PathBuf::from("/home/user/Templates"),
        }
    }

    /// parity: OPS-002
    #[test]
    fn each_new_menu_item_starts_with_its_own_template_and_title() {
        let list = starters();
        let markdown = NewFileKind::Starter(BuiltinTemplate::Markdown);

        assert_eq!(initial_position(NewFileKind::Empty, &list), 5);
        assert_eq!(initial_position(markdown, &list), 1);
        assert_eq!(initial_position(NewFileKind::AnyTemplate, &list), 0);
        assert_eq!(dialog_text(NewFileKind::Empty).0, "New file");
        assert_eq!(dialog_text(markdown).0, "New from template");
        assert_eq!(
            dialog_text(NewFileKind::AnyTemplate).1,
            "Create a new copy without changing the template."
        );
    }

    /// parity: OPS-002
    #[test]
    fn user_templates_are_marked_in_the_list() {
        let user = Template {
            id: TemplateId::User("Letter.odt".to_owned()),
            label: "Letter.odt".to_owned(),
            suggested_name: "Letter.odt".to_owned(),
        };

        assert_eq!(list_label(&user), "Letter.odt · Your template");
        assert_eq!(list_label(&starters().templates[0]), "Text document");
    }
}
