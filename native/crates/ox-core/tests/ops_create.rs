// SPDX-License-Identifier: AGPL-3.0-only
//! New folder, New file and New from template on temporary local folders.
//!
//! The Python suite covers creation only in
//! `v2.0.0:desktop/tests/gio_integration.py`; its template rules had no desktop
//! test (see OPS-048), so the template cases here are new and follow
//! `v2.0.0:desktop/file_services.py` line by line.

#[path = "common/fifo.rs"]
mod fifo;
mod ops_support;
#[path = "ops_support/snapshots.rs"]
mod snapshots;

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;

use ox_core::location::ItemKind;
use ox_core::ops::{
    create_from_template, create_item, list_templates, BuiltinTemplate, NewFromTemplate, OperationContext,
    OpsError, Template, TemplateId, UndoRecord, MAX_TEMPLATE_BYTES, MAX_USER_TEMPLATES,
};

use fifo::make_fifo;
use ops_support::{block_on, file_uri};
use snapshots::{snapshot_protection, READ_ONLY};

/// The names in `folder`, sorted.
fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("a readable folder")
        .map(|entry| {
            entry
                .expect("a folder entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// The permission bits of `path`.
fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).expect("the item exists").permissions().mode() & 0o7777
}

/// Creates `name` in `folder` from `template`, with the Templates folder
/// `templates`.
fn new_from(folder: &Path, name: &str, template: TemplateId, templates: &Path) -> Result<String, OpsError> {
    let request = NewFromTemplate {
        folder_uri: file_uri(folder),
        name: name.to_owned(),
        template,
        templates_folder: templates.to_path_buf(),
    };
    let created = block_on(create_from_template(&request, &OperationContext::default()))?;
    Ok(created.uri)
}

/// The ids of the user templates listed from `templates`, sorted.
fn user_template_ids(templates: &Path) -> Vec<String> {
    let list =
        block_on(list_templates(templates, &OperationContext::default().cancel)).expect("a template list");
    let mut ids: Vec<String> = list
        .templates
        .iter()
        .filter(|template| !template.is_builtin())
        .map(|template| template.id.to_string())
        .collect();
    ids.sort();
    ids
}

/// Ported from `v2.0.0:desktop/tests/gio_integration.py::GioLocalIntegration::test_enumeration_and_creation`
/// (the creation half; the listing half is in `gio_node.rs`).
///
/// parity: OPS-008
#[test]
fn new_folder_and_new_file_appear_under_their_names() {
    let temp = tempfile::tempdir().unwrap();
    let context = OperationContext::default();

    let folder = block_on(create_item(
        &file_uri(temp.path()),
        "Created folder",
        ItemKind::Folder,
        &context,
    ));
    let file = block_on(create_item(
        &file_uri(temp.path()),
        "Unicode café.txt",
        ItemKind::File,
        &context,
    ));

    assert_eq!(names_in(temp.path()), ["Created folder", "Unicode café.txt"]);
    assert!(temp.path().join("Created folder").is_dir());
    assert_eq!(fs::read(temp.path().join("Unicode café.txt")).unwrap(), b"");
    let folder = folder.unwrap();
    assert_eq!(folder.uri, file_uri(&temp.path().join("Created folder")));
    assert_eq!(file.unwrap().kind, ItemKind::File);
    let undo = UndoRecord::Create {
        uri: folder.uri.clone(),
        kind: ItemKind::Folder,
    };
    assert_eq!(folder.undo_record(), undo);
}

/// parity: OPS-008
#[test]
fn creating_onto_a_taken_name_never_overwrites() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("Reports")).unwrap();
    fs::write(temp.path().join("notes.txt"), b"kept").unwrap();
    let context = OperationContext::default();
    let folder_uri = file_uri(temp.path());

    let folder = block_on(create_item(&folder_uri, "Reports", ItemKind::Folder, &context));
    let file = block_on(create_item(&folder_uri, "notes.txt", ItemKind::File, &context));

    let taken = |name: &str| {
        OpsError::Exists(format!(
            "An item named “{name}” already exists. Nothing was overwritten."
        ))
    };
    assert_eq!(folder, Err(taken("Reports")));
    assert_eq!(file, Err(taken("notes.txt")));
    assert_eq!(fs::read(temp.path().join("notes.txt")).unwrap(), b"kept");
}

/// parity: OPS-006, OPS-008
#[test]
fn invalid_names_are_refused_before_anything_is_created() {
    let temp = tempfile::tempdir().unwrap();
    let folder_uri = file_uri(temp.path());
    let context = OperationContext::default();

    for name in ["", ".", "..", "a/b", "a\\b", "line\nbreak"] {
        let created = block_on(create_item(&folder_uri, name, ItemKind::File, &context));

        assert!(
            matches!(created, Err(OpsError::Failed(_))),
            "{name:?}: {created:?}"
        );
    }
    assert!(names_in(temp.path()).is_empty());
}

/// parity: OPS-008
#[test]
fn a_server_listing_or_a_protected_folder_gets_no_new_items() {
    let temp = tempfile::tempdir().unwrap();
    let snapshot = temp.path().join(".snapshot");
    fs::create_dir(&snapshot).unwrap();
    let protected = OperationContext::new(snapshot_protection());

    let on_server = block_on(create_item(
        "smb://nas/",
        "New folder",
        ItemKind::Folder,
        &protected,
    ));
    let in_snapshot = block_on(create_item(
        &file_uri(&snapshot),
        "New folder",
        ItemKind::Folder,
        &protected,
    ));

    let open_share = "Open a network share before creating files or folders.";
    assert_eq!(on_server, Err(OpsError::Failed(open_share.into())));
    assert_eq!(in_snapshot, Err(OpsError::Failed(READ_ONLY.into())));
    assert!(names_in(&snapshot).is_empty());
}

#[test]
fn a_cancelled_creation_creates_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let context = OperationContext::default();
    context.cancel.cancel();

    let created = block_on(create_item(
        &file_uri(temp.path()),
        "Later",
        ItemKind::Folder,
        &context,
    ));

    assert_eq!(created, Err(OpsError::Cancelled));
    assert!(names_in(temp.path()).is_empty());
}

/// parity: OPS-048
#[test]
fn the_six_starters_come_first_with_their_fixed_names() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("Templates");

    let list = block_on(list_templates(&missing, &OperationContext::default().cancel)).unwrap();

    let starters: Vec<(String, String)> = list
        .templates
        .iter()
        .map(|template| (template.label.clone(), template.suggested_name.clone()))
        .collect();
    let expected = [
        ("Text document", "New document.txt"),
        ("Markdown document", "New document.md"),
        ("CSV file", "New spreadsheet.csv"),
        ("JSON file", "New file.json"),
        ("HTML document", "New page.html"),
        ("Empty file", "New file"),
    ];
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(label, name)| ((*label).to_owned(), (*name).to_owned()))
        .collect();
    assert_eq!(starters, expected);
    assert!(list.templates.iter().all(Template::is_builtin));
    assert_eq!(list.folder, missing);
}

/// parity: OPS-048
#[test]
fn only_visible_regular_files_up_to_sixteen_mebibytes_are_user_templates() {
    let temp = tempfile::tempdir().unwrap();
    let templates = temp.path();
    fs::write(templates.join("Letter.odt"), b"letter").unwrap();
    fs::write(templates.join(".hidden.txt"), b"hidden").unwrap();
    fs::write(templates.join("launcher.desktop"), b"[Desktop Entry]").unwrap();
    symlink(templates.join("Letter.odt"), templates.join("link.odt")).unwrap();
    fs::create_dir(templates.join("Folder")).unwrap();
    make_fifo(&templates.join("pipe"));
    let largest = fs::File::create(templates.join("largest.bin")).unwrap();
    largest.set_len(MAX_TEMPLATE_BYTES).unwrap();
    let too_large = fs::File::create(templates.join("too-large.bin")).unwrap();
    too_large.set_len(MAX_TEMPLATE_BYTES + 1).unwrap();

    let ids = user_template_ids(templates);

    assert_eq!(ids, ["user:Letter.odt", "user:largest.bin"]);
}

#[test]
fn at_most_one_hundred_user_templates_are_listed() {
    let temp = tempfile::tempdir().unwrap();
    for number in 0..MAX_USER_TEMPLATES + 5 {
        fs::write(temp.path().join(format!("template {number}.txt")), b"x").unwrap();
    }

    let ids = user_template_ids(temp.path());

    assert_eq!(ids.len(), MAX_USER_TEMPLATES);
}

#[test]
fn a_starter_writes_its_content_to_a_private_new_file() {
    let temp = tempfile::tempdir().unwrap();
    let templates = temp.path().join("Templates");

    let markdown = new_from(
        temp.path(),
        "Plan.md",
        TemplateId::Builtin(BuiltinTemplate::Markdown),
        &templates,
    );
    let page = new_from(
        temp.path(),
        "index.html",
        TemplateId::Builtin(BuiltinTemplate::Html),
        &templates,
    );

    assert_eq!(markdown.unwrap(), file_uri(&temp.path().join("Plan.md")));
    assert!(page.is_ok());
    assert_eq!(
        fs::read(temp.path().join("Plan.md")).unwrap(),
        b"# New document\n"
    );
    let html = fs::read_to_string(temp.path().join("index.html")).unwrap();
    assert!(html.starts_with("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">"));
    assert_eq!(mode_of(&temp.path().join("Plan.md")), 0o600);
    assert_eq!(names_in(temp.path()), ["Plan.md", "index.html"]);
}

/// parity: OPS-048
#[test]
fn a_user_template_is_copied_without_its_permissions() {
    let temp = tempfile::tempdir().unwrap();
    let templates = temp.path().join("Templates");
    fs::create_dir(&templates).unwrap();
    fs::write(templates.join("script.sh"), b"#!/bin/sh\necho template\n").unwrap();
    fs::set_permissions(templates.join("script.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    let target = temp.path().join("Documents");
    fs::create_dir(&target).unwrap();

    let created = new_from(
        &target,
        "copy.sh",
        TemplateId::User("script.sh".into()),
        &templates,
    );

    assert!(created.is_ok(), "{created:?}");
    assert_eq!(
        fs::read(target.join("copy.sh")).unwrap(),
        b"#!/bin/sh\necho template\n"
    );
    assert_eq!(mode_of(&target.join("copy.sh")), 0o600);
    assert_eq!(mode_of(&templates.join("script.sh")), 0o755);
    assert_eq!(names_in(&target), ["copy.sh"]);
}

#[test]
fn a_template_the_list_does_not_offer_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let templates = temp.path().join("Templates");
    fs::create_dir(&templates).unwrap();
    fs::write(temp.path().join("outside.txt"), b"outside").unwrap();
    symlink(temp.path().join("outside.txt"), templates.join("link.txt")).unwrap();
    fs::write(templates.join(".hidden.txt"), b"hidden").unwrap();
    let target = temp.path().join("Documents");
    fs::create_dir(&target).unwrap();

    let unavailable = "This template is unavailable, too large, or not a regular template file.";
    for file_name in ["link.txt", ".hidden.txt", "missing.txt"] {
        let created = new_from(&target, "new.txt", TemplateId::User(file_name.into()), &templates);

        assert_eq!(created, Err(OpsError::Failed(unavailable.into())), "{file_name}");
    }
    let escaping = new_from(
        &target,
        "new.txt",
        TemplateId::User("../outside.txt".into()),
        &templates,
    );
    assert!(matches!(escaping, Err(OpsError::Failed(_))), "{escaping:?}");
    assert!(names_in(&target).is_empty());
}

#[test]
fn a_template_never_overwrites_and_never_goes_to_a_server_listing() {
    let temp = tempfile::tempdir().unwrap();
    let templates = temp.path().join("Templates");
    fs::write(temp.path().join("New document.txt"), b"kept").unwrap();
    let text = TemplateId::Builtin(BuiltinTemplate::Text);

    let taken = new_from(temp.path(), "New document.txt", text.clone(), &templates);
    let request = NewFromTemplate {
        folder_uri: "smb://nas/".into(),
        name: "New document.txt".into(),
        template: text,
        templates_folder: templates,
    };
    let on_server = block_on(create_from_template(&request, &OperationContext::default()));

    let exists = "An item with that name already exists. Nothing was overwritten.";
    assert_eq!(taken, Err(OpsError::Exists(exists.into())));
    assert_eq!(fs::read(temp.path().join("New document.txt")).unwrap(), b"kept");
    let open_share = "Open a share before creating a file.";
    assert_eq!(
        on_server.map(|created| created.uri),
        Err(OpsError::Failed(open_share.into()))
    );
    assert_eq!(names_in(temp.path()), ["New document.txt"]);
}

/// Templates in subfolders of Templates are listed by their path, and New
/// copies one into the folder under its file name, never overwriting.
///
/// parity: OPS-003
#[test]
fn templates_in_subfolders_are_listed_by_path_and_copied_without_overwriting() {
    let temp = tempfile::tempdir().expect("a temporary folder");
    let templates = temp.path().join("Templates");
    fs::create_dir_all(templates.join("Office/Invoices")).expect("nested template folders");
    fs::create_dir(templates.join(".hidden")).expect("a hidden folder");
    fs::write(templates.join("Letter.odt"), b"letter").expect("a template");
    fs::write(templates.join("Office/Invoices/Invoice.ods"), b"invoice").expect("a nested template");
    fs::write(templates.join(".hidden/Secret.txt"), b"x").expect("a hidden template");
    let target = temp.path().join("target");
    fs::create_dir(&target).expect("a target folder");
    let nested = TemplateId::User("Office/Invoices/Invoice.ods".to_owned());

    let ids = user_template_ids(&templates);
    let created = new_from(&target, "Invoice.ods", nested.clone(), &templates);
    let again = new_from(&target, "Invoice.ods", nested, &templates);

    assert_eq!(ids, ["user:Letter.odt", "user:Office/Invoices/Invoice.ods"]);
    assert!(created.is_ok(), "{created:?}");
    assert_eq!(
        fs::read(target.join("Invoice.ods")).expect("the copy"),
        b"invoice"
    );
    assert!(again.is_err(), "an existing name is never overwritten");
    assert_eq!(
        fs::read(target.join("Invoice.ods")).expect("the copy"),
        b"invoice"
    );
}
