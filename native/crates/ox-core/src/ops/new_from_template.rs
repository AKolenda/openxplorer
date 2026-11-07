// SPDX-License-Identifier: AGPL-3.0-only
//! New from template: a new file holding a copy of a template's content.
//!
//! Ports `create_from_template` in `v2.0.0:desktop/file_services.py` and the
//! checks of the `createTemplate` branch of `dispatch` in
//! `v2.0.0:desktop/winspace.py`. The OPS-048 safety rules:
//!
//! - A user template must be in a fresh template list, so only a bounded,
//!   regular, visible, non-link file in the Templates folder is copied.
//! - It is opened without following links and without blocking, one path
//!   component at a time from the Templates folder (a template may be in
//!   a subfolder, OPS-003), and checked on the open descriptor, so a
//!   template or a subfolder swapped for a link, a FIFO or a device after
//!   the list was read is refused.
//! - It is read in 64 KiB blocks that check for cancellation, and refused
//!   past [`MAX_TEMPLATE_BYTES`].
//! - The content goes to a private (0600) hidden stage file in the target
//!   folder, which is published under the new name by a same-folder rename
//!   that never overwrites. A failed stage is deleted. The template's
//!   permissions, execute bits included, are never copied, and a template
//!   is never run.

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use gio::prelude::*;
use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;

use super::context::{on_worker, unless_cancelled, OperationContext};
use super::create::{CreatedItem, MAX_NUMBERED_NAMES};
use super::error::OpsError;
use super::templates::{
    list_templates_blocking, NewDocument, TemplateId, MAX_TEMPLATE_BYTES, TEMPLATE_PATH_SEPARATOR,
};
use crate::gio_node::GioNode;
use crate::location::{is_smb_server, normalise, validate_name, ItemKind};
use crate::random::{random_hex, NAME_BYTES};
use crate::transfer::{already_removed, Cancellation, Node, StagedName};

/// Templates are read in blocks of this many bytes, with a cancellation
/// check before each.
const READ_BLOCK_BYTES: usize = 64 * 1024;

/// The stage file is `.winspace-new-<32 hex>` in the target folder.
const STAGE_PREFIX: &str = ".winspace-new-";

/// The refusal of a taken name, in `create_from_template`'s wording in
/// `v2.0.0:desktop/file_services.py`, whether the name was taken before the file
/// was made or while it was published.
const NAME_TAKEN: &str =
    crate::i18n::message_id("An item with that name already exists. Nothing was overwritten.");

/// What New from template creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFromTemplate {
    /// The folder the new file goes into.
    pub folder_uri: String,
    /// The new file's name.
    pub name: String,
    /// The template to copy.
    pub template: TemplateId,
    /// The user's Templates folder, which user templates must come from.
    pub templates_folder: PathBuf,
}

/// Creates the file `request` describes from its template, never
/// overwriting an existing item.
///
/// # Errors
///
/// An invalid name, a server listing ("Open a share before creating a
/// file."), a protected folder, a taken name ("An item with that name
/// already exists. Nothing was overwritten."), a template that is not in
/// the fresh list, not a regular file or too large, cancellation, or the
/// backend's failure. A failed stage file is deleted.
pub async fn create_from_template(
    request: &NewFromTemplate,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    let request = request.clone();
    let context = context.clone();
    on_worker(move || create_from_template_blocking(&request, &context)).await
}

/// Creates `document` in the folder at `folder_uri` under the first free
/// name of `base_name.<extension>`, `base_name (2).<extension>`, ..., as
/// Windows names a new file ("New Text Document (2).txt"). It is the first
/// half of Explorer's New ▸ Text Document and the menu's other documents,
/// which then rename the file in place. Each attempt publishes without
/// overwriting, as [`create_from_template`] does, so an item that takes a
/// name meanwhile is kept and only moves the new file to the next number.
///
/// # Errors
///
/// As [`create_from_template`], except that a taken name tries the next
/// number; after [`MAX_NUMBERED_NAMES`] taken names the refusal says to
/// rename some of them.
pub async fn create_numbered_document(
    folder_uri: &str,
    base_name: &str,
    document: NewDocument,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    let folder_uri = folder_uri.to_owned();
    let name = format!("{base_name}.{}", document.extension());
    let context = context.clone();
    on_worker(move || {
        first_free_name(&name, |name| {
            create_file_blocking(&folder_uri, name, || Ok(document.contents().to_vec()), &context)
        })
    })
    .await
}

/// Creates a copy of the user template at `path` below `templates_folder`
/// in the folder at `folder_uri`, under the template's file name or the
/// first free numbered one ("Letter (2).odt"), as New does with the
/// templates it lists (OPS-003). The template is read once, under the
/// OPS-048 rules, and each attempt publishes without overwriting.
///
/// # Errors
///
/// As [`create_from_template`], except that a taken name tries the next
/// number, as [`create_numbered_document`] does.
pub async fn create_numbered_from_template(
    folder_uri: &str,
    path: &str,
    templates_folder: &Path,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    let folder_uri = folder_uri.to_owned();
    let path = path.to_owned();
    let templates_folder = templates_folder.to_path_buf();
    let context = context.clone();
    on_worker(move || {
        let name = path
            .rsplit(TEMPLATE_PATH_SEPARATOR)
            .next()
            .unwrap_or(&path)
            .to_owned();
        let contents = read_user_template(&path, &templates_folder, &context.cancel)?;
        first_free_name(&name, |name| {
            create_file_blocking(&folder_uri, name, || Ok(contents.clone()), &context)
        })
    })
    .await
}

/// What `create` makes of the first of `base_name`'s numbered names
/// ([`numbered_file_name`]) it does not find taken.
pub(super) fn first_free_name<Created>(
    base_name: &str,
    mut create: impl FnMut(&str) -> Result<Created, OpsError>,
) -> Result<Created, OpsError> {
    for number in 1..=MAX_NUMBERED_NAMES {
        match create(&numbered_file_name(base_name, number)) {
            Err(OpsError::Exists(_)) => {}
            outcome => return outcome,
        }
    }
    Err(OpsError::failed(crate::i18n::format_message(
        "Too many files are called “{base_name}”. Rename some of them, then try again.",
        &[("base_name", base_name)],
    )))
}

/// `name` for the first attempt, then with ` (number)` before its
/// extension, after the last dot that does not start the name.
fn numbered_file_name(name: &str, number: u32) -> String {
    if number == 1 {
        return name.to_owned();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({number}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({number})"),
    }
}

/// [`create_from_template`] on the calling thread.
fn create_from_template_blocking(
    request: &NewFromTemplate,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    create_file_blocking(
        &request.folder_uri,
        &request.name,
        || template_contents(request, &context.cancel),
        context,
    )
}

/// Creates the file `name` in the folder at `folder_uri` with what
/// `contents` gives, which is asked for only once the name is checked and
/// found free. Nothing is ever overwritten.
pub(super) fn create_file_blocking(
    folder_uri: &str,
    name: &str,
    contents: impl FnOnce() -> Result<Vec<u8>, OpsError>,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    validate_name(name)?;
    let folder_uri = normalise(folder_uri)?;
    if is_smb_server(&folder_uri) {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Open a share before creating a file.",
        )));
    }
    context.protection.check(&folder_uri)?;
    let folder = gio::File::for_uri(&folder_uri);
    let target = GioNode::from_file(folder.child(name));
    if unless_cancelled(&context.cancel, || target.exists(Some(&context.cancel)))? {
        return Err(OpsError::Exists(crate::i18n::gettext(NAME_TAKEN)));
    }
    let contents = contents()?;
    publish_new_file(&folder, &target, &contents, context)?;
    Ok(CreatedItem {
        uri: target.uri(),
        kind: ItemKind::File,
    })
}

/// The content the new file gets.
fn template_contents(request: &NewFromTemplate, cancel: &Cancellation) -> Result<Vec<u8>, OpsError> {
    match &request.template {
        TemplateId::Builtin(template) => Ok(template.contents().to_vec()),
        TemplateId::User(file_name) => read_user_template(file_name, &request.templates_folder, cancel),
    }
}

/// OPS-048: the content of the user template at `path` below the
/// Templates folder, only when a fresh listing still offers it.
fn read_user_template(
    path: &str,
    templates_folder: &Path,
    cancel: &Cancellation,
) -> Result<Vec<u8>, OpsError> {
    // Each folder and the file name are one path component, so the
    // template cannot come from outside the Templates folder.
    for part in path.split(TEMPLATE_PATH_SEPARATOR) {
        validate_name(part)?;
    }
    let id = TemplateId::User(path.to_owned());
    if !list_templates_blocking(templates_folder, cancel)?.contains(&id) {
        return Err(OpsError::failed(crate::i18n::gettext(
            "This template is unavailable, too large, or not a regular template file.",
        )));
    }
    let template = open_template(templates_folder, path)?;
    read_bounded(template, cancel)
}

/// OPS-048: opens the template at `path` below `templates_folder` one
/// component at a time without following a link (`O_NOFOLLOW`), the file
/// without waiting for a FIFO's writer (`O_NONBLOCK`), then checks the
/// open descriptor, closing the race between the listing and the open.
fn open_template(templates_folder: &Path, path: &str) -> Result<File, OpsError> {
    let mut folders: Vec<&str> = path.split(TEMPLATE_PATH_SEPARATOR).collect();
    let file_name = folders.pop().unwrap_or_default();
    let folder_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut folder = rustix::fs::open(templates_folder, folder_flags, Mode::empty()).map_err(io_error)?;
    for name in folders {
        folder = rustix::fs::openat(&folder, name, folder_flags | OFlags::NOFOLLOW, Mode::empty())
            .map_err(refused_link)?;
    }
    let file_flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file =
        File::from(rustix::fs::openat(&folder, file_name, file_flags, Mode::empty()).map_err(refused_link)?);
    if !file.metadata()?.file_type().is_file() {
        return Err(not_regular());
    }
    Ok(file)
}

/// The refusal of a component that is a link (`ELOOP`, or `ENOTDIR` for a
/// folder that is not one), else the system's error.
fn refused_link(errno: Errno) -> OpsError {
    if errno == Errno::LOOP || errno == Errno::NOTDIR {
        not_regular()
    } else {
        io_error(errno)
    }
}

/// The system's error `errno`.
fn io_error(errno: Errno) -> OpsError {
    std::io::Error::from(errno).into()
}

/// The refusal of a template that is a link, FIFO or device.
fn not_regular() -> OpsError {
    OpsError::failed(crate::i18n::gettext(
        "Templates must be regular files, not links or devices.",
    ))
}

/// OPS-048: the whole of `template`, read in [`READ_BLOCK_BYTES`] blocks
/// with a cancellation check before each, and refused past
/// [`MAX_TEMPLATE_BYTES`] even if it grew after it was listed.
fn read_bounded(mut template: File, cancel: &Cancellation) -> Result<Vec<u8>, OpsError> {
    let mut contents = Vec::new();
    let mut block = vec![0u8; READ_BLOCK_BYTES];
    loop {
        cancel.check()?;
        let count = match template.read(&mut block) {
            Ok(0) => return Ok(contents),
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        contents.extend_from_slice(&block[..count]);
        if contents.len() as u64 > MAX_TEMPLATE_BYTES {
            return Err(OpsError::failed(crate::i18n::gettext("Template exceeds 16 MiB.")));
        }
    }
}

/// Writes `contents` to a private stage file in `folder` and publishes it
/// as `target`. The stage file is deleted when anything fails after it was
/// created.
///
/// # Errors
///
/// [`OpsError::Exists`] with [`NAME_TAKEN`] when `target`'s name was taken
/// meanwhile (the item there is left as it was), cancellation, or the
/// backend's failure.
fn publish_new_file(
    folder: &gio::File,
    target: &GioNode,
    contents: &[u8],
    context: &OperationContext,
) -> Result<(), OpsError> {
    let digits = random_hex(NAME_BYTES).map_err(|error| {
        OpsError::failed(crate::i18n::format_message(
            "Could not reserve a private staging name. Nothing was changed. {error}",
            &[("error", &(error).to_string())],
        ))
    })?;
    let stage_name = format!("{STAGE_PREFIX}{digits}");
    // An error names the new file, never its stage.
    let staged = StagedName::new(&stage_name, &[], &target.display_name());
    let stage = folder.child(&stage_name);
    write_private_stage(&stage, contents, context, &staged)?;
    // Same folder, never overwriting: the kernel's no-replace rename
    // locally, `set_display_name` on a phone (XFER-007, XFER-024).
    let published = GioNode::from_file(stage.clone()).publish(target, Some(&context.cancel));
    published.map_err(|error| {
        let cause = match OpsError::from(staged.apply(error)) {
            OpsError::Exists(_) => OpsError::Exists(crate::i18n::gettext(NAME_TAKEN)),
            other => other,
        };
        // OPS-048: a stage that was not published is deleted.
        discard_stage(&GioNode::from_file(stage.clone()), cause)
    })
}

/// `error` naming the new file rather than its stage file `staged`.
fn naming(error: OpsError, staged: &StagedName) -> OpsError {
    match error {
        OpsError::Cancelled => OpsError::Cancelled,
        OpsError::NotMounted(message) => OpsError::NotMounted(staged.reword(&message)),
        OpsError::NotFound(message) => OpsError::NotFound(staged.reword(&message)),
        OpsError::PermissionDenied(message) => OpsError::PermissionDenied(staged.reword(&message)),
        OpsError::Exists(message) => OpsError::Exists(staged.reword(&message)),
        OpsError::NotSupported(message) => OpsError::NotSupported(staged.reword(&message)),
        OpsError::NotDirectory(message) => OpsError::NotDirectory(staged.reword(&message)),
        OpsError::Failed(message) => OpsError::Failed(staged.reword(&message)),
    }
}

/// Creates `stage` exclusively as a private (0600) file and writes
/// `contents` to it. A stage this call created is deleted again when
/// writing fails; a name it could not create is never touched. Errors name
/// the new file that `staged` describes.
fn write_private_stage(
    stage: &gio::File,
    contents: &[u8],
    context: &OperationContext,
    staged: &StagedName,
) -> Result<(), OpsError> {
    let cancellable = Some(context.cancellable());
    let stream = stage
        .create(gio::FileCreateFlags::PRIVATE, cancellable)
        .map_err(|error| naming(error.into(), staged))?;
    let written = match stream.write_all(contents, cancellable) {
        Ok((_, None)) => Ok(()),
        Ok((_, Some(error))) | Err(error) => Err(error),
    };
    // Close even after a failed write, as the Python `finally` does.
    let closed = stream.close(cancellable);
    // OPS-048: a stage that could not be written in full is deleted.
    written
        .and(closed)
        .map_err(|error| discard_stage(&GioNode::from_file(stage.clone()), naming(error.into(), staged)))
}

/// Deletes a stage file this operation created after `cause` stopped it,
/// and returns the error to report: `cause`, or, when the stage cannot be
/// deleted, `cause` followed by the stage's exact location so the user can
/// find what was left behind.
fn discard_stage(stage: &dyn Node, cause: OpsError) -> OpsError {
    match stage.delete() {
        Ok(()) => cause,
        // A device's "not found" counts only once confirmed (XFER-022).
        Err(cleanup) if already_removed(stage, &cleanup) => cause,
        Err(cleanup) => OpsError::failed(crate::i18n::format_message(
            "{cause}\nThe incomplete file remains at {uri}. Inspect it before removing it. {cleanup}",
            &[
                ("cause", &cause.to_string()),
                ("uri", &stage.uri()),
                ("cleanup", &cleanup.to_string()),
            ],
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::test_support::make_fifo;

    /// parity: OPS-048
    #[test]
    fn a_template_swapped_for_a_link_or_a_fifo_is_refused_without_blocking() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let regular = temp.path().join("regular.txt");
        fs::write(&regular, b"template").expect("a template");
        let link = temp.path().join("link.txt");
        symlink(&regular, &link).expect("a link");
        let pipe = temp.path().join("pipe.txt");
        make_fifo(&pipe);

        fs::create_dir(temp.path().join("Office")).expect("a subfolder");
        symlink(temp.path(), temp.path().join("Linked")).expect("a linked folder");

        let through_link = open_template(temp.path(), "link.txt").map(|_| ());
        let from_pipe = open_template(temp.path(), "pipe.txt").map(|_| ());
        let through_linked_folder = open_template(temp.path(), "Linked/regular.txt").map(|_| ());

        assert_eq!(through_link, Err(not_regular()));
        assert_eq!(from_pipe, Err(not_regular()));
        assert_eq!(through_linked_folder, Err(not_regular()));
        assert!(open_template(temp.path(), "regular.txt").is_ok());
    }

    #[test]
    fn a_template_that_grew_past_the_limit_is_refused_and_cancellation_stops_reading() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let grown = temp.path().join("grown.bin");
        let file = File::create(&grown).expect("a template");
        file.set_len(MAX_TEMPLATE_BYTES + 1).expect("a sparse template");
        let cancelled = Cancellation::new();
        cancelled.cancel();

        let too_large = read_bounded(File::open(&grown).expect("readable"), &Cancellation::new());
        let stopped = read_bounded(File::open(&grown).expect("readable"), &cancelled);

        assert_eq!(too_large, Err(OpsError::failed("Template exceeds 16 MiB.")));
        assert_eq!(stopped, Err(OpsError::Cancelled));
    }

    /// The race the check before reading the template cannot close: the
    /// name is taken by the time the stage file is published.
    #[test]
    fn a_name_taken_while_publishing_is_kept_and_the_stage_file_is_deleted() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let existing = temp.path().join("New document.txt");
        fs::write(&existing, b"kept").expect("an existing item");
        let folder = gio::File::for_path(temp.path());
        let target = GioNode::from_file(folder.child("New document.txt"));

        let published = publish_new_file(&folder, &target, b"template", &OperationContext::default());

        assert_eq!(published, Err(OpsError::Exists(NAME_TAKEN.into())));
        assert_eq!(fs::read(&existing).expect("the existing item"), b"kept");
        let names: Vec<_> = fs::read_dir(temp.path())
            .expect("a readable folder")
            .map(|entry| entry.expect("a folder entry").file_name())
            .collect();
        assert_eq!(names, ["New document.txt"], "no {STAGE_PREFIX} file is left");
    }

    #[test]
    fn numbered_file_names_go_before_the_extension() {
        assert_eq!(
            numbered_file_name("New Text Document.txt", 1),
            "New Text Document.txt"
        );
        assert_eq!(
            numbered_file_name("New Text Document.txt", 2),
            "New Text Document (2).txt"
        );
        assert_eq!(numbered_file_name("archive.tar.gz", 3), "archive.tar (3).gz");
        assert_eq!(numbered_file_name("New file", 2), "New file (2)");
        assert_eq!(numbered_file_name(".profile", 2), ".profile (2)");
    }

    /// An item that takes the free name after it was checked, while the new
    /// file is published, is kept, and the new file takes the next number.
    ///
    /// parity: OPS-008
    #[test]
    fn a_name_taken_at_the_last_moment_moves_to_the_next_number() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let folder = gio::File::for_path(temp.path());
        let context = OperationContext::default();
        let mut tried = Vec::new();

        let created = first_free_name("New Text Document.txt", |name| {
            tried.push(name.to_owned());
            let target = GioNode::from_file(folder.child(name));
            if tried.len() == 1 {
                // Appears after the check that the name is free.
                fs::write(temp.path().join(name), b"late").expect("a late item");
            }
            publish_new_file(&folder, &target, b"", &context)?;
            Ok(target.uri())
        });

        assert_eq!(tried, ["New Text Document.txt", "New Text Document (2).txt"]);
        let created = created.expect("the next number is free");
        assert_eq!(
            created,
            folder.child("New Text Document (2).txt").uri().to_string()
        );
        assert_eq!(
            fs::read(temp.path().join("New Text Document.txt")).expect("the late item"),
            b"late"
        );
        let mut names: Vec<_> = fs::read_dir(temp.path())
            .expect("a readable folder")
            .map(|entry| entry.expect("a folder entry").file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["New Text Document (2).txt", "New Text Document.txt"],
            "no {STAGE_PREFIX} file is left"
        );
    }

    /// Any failure but a taken name ends the search at once.
    #[test]
    fn only_a_taken_name_tries_the_next_number() {
        let mut tried = 0;
        let refused: Result<(), _> = first_free_name("New Text Document.txt", |_| {
            tried += 1;
            Err(OpsError::PermissionDenied("Permission denied".into()))
        });

        assert_eq!(
            refused,
            Err(OpsError::PermissionDenied("Permission denied".into()))
        );
        assert_eq!(tried, 1);
    }

    /// A stage file that cannot be deleted after a failure is reported at
    /// its exact location, so the user can find what was left behind; the
    /// failure itself is reported first.
    ///
    /// parity: OPS-048
    #[test]
    fn a_stage_file_left_behind_is_reported_where_it_is() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("a temporary folder");
        let locked = temp.path().join("Locked");
        fs::create_dir(&locked).expect("a folder");
        let stage_path = locked.join(format!("{STAGE_PREFIX}{}", "e".repeat(32)));
        fs::write(&stage_path, b"partial").expect("a stage file");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("chmod");
        // As root, which deletes anywhere, nothing is left behind.
        if fs::write(locked.join("probe"), b"").is_ok() {
            return;
        }
        let stage = GioNode::new(&crate::location::file_uri(&stage_path));

        let error = discard_stage(&stage, OpsError::failed("Disk full")).to_string();

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
        assert!(stage_path.exists());
        let expected = format!(
            "Disk full\nThe incomplete file remains at {}. Inspect it before removing it. ",
            stage.uri()
        );
        assert!(error.starts_with(&expected), "{error}");
        assert!(error.ends_with("Permission denied"), "{error}");
    }

    /// A phone that discarded a cancelled upload answers "not found" for
    /// its stage file; once the folder's listing confirms it is gone,
    /// nothing was left behind and the cancellation stays a cancellation.
    /// A "not found" the listing does not confirm still reports the stage.
    ///
    /// parity: OPS-048, XFER-022
    #[test]
    fn a_stage_file_a_phone_discarded_is_not_reported_as_left_behind() {
        use crate::test_support::RemoteStage;

        let gone = RemoteStage::on_phone(".winspace-new-1", false);
        let still_listed = RemoteStage::on_phone(".winspace-new-2", true);

        assert_eq!(discard_stage(&gone, OpsError::Cancelled), OpsError::Cancelled);
        let reported = discard_stage(&still_listed, OpsError::Cancelled).to_string();
        assert!(
            reported.contains("remains at mtp://phone.example/Folder/.winspace-new-2."),
            "{reported}"
        );
    }

    /// A folder that cannot be written is reported with the new file's
    /// name and the system's reason, never the stage file's name.
    ///
    /// parity: OPS-048
    #[test]
    fn a_read_only_folder_is_reported_with_the_new_files_name() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("a temporary folder");
        let locked = temp.path().join("Locked");
        fs::create_dir(&locked).expect("a folder");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("chmod");
        // As root, which writes anywhere, there is nothing to report.
        if fs::write(locked.join("probe"), b"").is_ok() {
            return;
        }
        let folder = gio::File::for_path(&locked);
        let target = GioNode::from_file(folder.child("New document.txt"));

        let published = publish_new_file(&folder, &target, b"template", &OperationContext::default());

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
        let error = published.expect_err("the folder is read-only").to_string();
        assert!(!error.contains(STAGE_PREFIX), "{error}");
        assert!(
            error.contains(&locked.join("New document.txt").display().to_string()),
            "{error}"
        );
        assert!(error.contains("Permission denied"), "{error}");
    }
}
