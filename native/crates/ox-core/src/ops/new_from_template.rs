// SPDX-License-Identifier: AGPL-3.0-only
//! New from template: a new file holding a copy of a template's content.
//!
//! Ports `create_from_template` in `desktop/file_services.py` and the
//! checks of the `createTemplate` branch of `dispatch` in
//! `desktop/winspace.py`. The OPS-048 safety rules:
//!
//! - A user template must be in a fresh template list, so only a bounded,
//!   regular, visible, non-link file in the Templates folder is copied.
//! - It is opened without following links and without blocking, and
//!   checked on the open descriptor, so a template swapped for a link, a
//!   FIFO or a device after the list was read is refused.
//! - It is read in 64 KiB blocks that check for cancellation, and refused
//!   past [`MAX_TEMPLATE_BYTES`].
//! - The content goes to a private (0600) hidden stage file in the target
//!   folder, which is published under the new name by a same-folder rename
//!   that never overwrites. A failed stage is deleted. The template's
//!   permissions, execute bits included, are never copied, and a template
//!   is never run.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::create::{name_taken_or, CreatedItem};
use super::error::OpsError;
use super::random::random_hex;
use super::templates::{list_templates_blocking, TemplateId, MAX_TEMPLATE_BYTES};
use crate::gio_node::GioNode;
use crate::location::{is_smb_server, normalise, validate_name, ItemKind};
use crate::transfer::{Cancellation, Node};

/// Templates are read in blocks of this many bytes, with a cancellation
/// check before each.
const READ_BLOCK_BYTES: usize = 64 * 1024;

/// The stage file is `.winspace-new-<32 hex>` in the target folder.
const STAGE_PREFIX: &str = ".winspace-new-";

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

/// [`create_from_template`] on the calling thread.
fn create_from_template_blocking(
    request: &NewFromTemplate,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    validate_name(&request.name)?;
    let folder_uri = normalise(&request.folder_uri)?;
    if is_smb_server(&folder_uri) {
        return Err(OpsError::failed("Open a share before creating a file."));
    }
    context.protection.check(&folder_uri)?;
    let folder = gio::File::for_uri(&folder_uri);
    let target = GioNode::from_file(folder.child(&request.name));
    if target.exists(Some(&context.cancel)) {
        return Err(OpsError::Exists(
            "An item with that name already exists. Nothing was overwritten.".into(),
        ));
    }
    let contents = template_contents(request, &context.cancel)?;
    publish_new_file(&folder, &target, &contents, context)
        .map_err(|error| name_taken_or(error, &request.name))?;
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

/// OPS-048: the content of the user template `file_name`, only when a
/// fresh listing still offers it.
fn read_user_template(
    file_name: &str,
    templates_folder: &Path,
    cancel: &Cancellation,
) -> Result<Vec<u8>, OpsError> {
    // One path component, so the template cannot come from another folder.
    validate_name(file_name)?;
    let id = TemplateId::User(file_name.to_owned());
    if !list_templates_blocking(templates_folder, cancel)?.contains(&id) {
        return Err(OpsError::failed(
            "This template is unavailable, too large, or not a regular template file.",
        ));
    }
    let template = open_regular_file(&templates_folder.join(file_name))?;
    read_bounded(template, cancel)
}

/// OPS-048: opens `path` without following a link (`O_NOFOLLOW`) and
/// without waiting for a FIFO's writer (`O_NONBLOCK`), then checks the
/// open descriptor, closing the race between the listing and the open.
fn open_regular_file(path: &Path) -> Result<File, OpsError> {
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => return Err(not_regular()),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.file_type().is_file() {
        return Err(not_regular());
    }
    Ok(file)
}

/// The refusal of a template that is a link, FIFO or device.
fn not_regular() -> OpsError {
    OpsError::failed("Templates must be regular files, not links or devices.")
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
            return Err(OpsError::failed("Template exceeds 16 MiB."));
        }
    }
}

/// Writes `contents` to a private stage file in `folder` and publishes it
/// as `target`. The stage file is deleted when anything fails after it was
/// created.
fn publish_new_file(
    folder: &gio::File,
    target: &GioNode,
    contents: &[u8],
    context: &OperationContext,
) -> Result<(), OpsError> {
    let digits = random_hex(16).map_err(|error| {
        OpsError::failed(format!(
            "Could not reserve a private staging name. Nothing was changed. {error}"
        ))
    })?;
    let stage = folder.child(format!("{STAGE_PREFIX}{digits}"));
    write_private_stage(&stage, contents, context)?;
    // Same folder, never overwriting: the kernel's no-replace rename
    // locally, `set_display_name` on a phone (XFER-007, XFER-024).
    let published = GioNode::from_file(stage.clone()).publish(target, Some(&context.cancel));
    if published.is_err() {
        discard_stage(&stage);
    }
    published.map_err(OpsError::from)
}

/// Creates `stage` exclusively as a private (0600) file and writes
/// `contents` to it. A stage this call created is deleted again when
/// writing fails; a name it could not create is never touched.
fn write_private_stage(
    stage: &gio::File,
    contents: &[u8],
    context: &OperationContext,
) -> Result<(), OpsError> {
    let cancellable = Some(context.cancellable());
    let stream = stage.create(gio::FileCreateFlags::PRIVATE, cancellable)?;
    let written = match stream.write_all(contents, cancellable) {
        Ok((_, None)) => Ok(()),
        Ok((_, Some(error))) | Err(error) => Err(error),
    };
    // Close even after a failed write, as the Python `finally` does.
    let closed = stream.close(cancellable);
    let outcome = written.and(closed).map_err(OpsError::from);
    if outcome.is_err() {
        discard_stage(stage);
    }
    outcome
}

/// Deletes a stage file this operation created. A stage that cannot be
/// deleted stays hidden under its `.winspace-new-` name; the operation's
/// own error is the one to report.
fn discard_stage(stage: &gio::File) {
    let _ = stage.delete(gio::Cancellable::NONE);
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::test_support::make_fifo;

    #[test]
    fn a_template_swapped_for_a_link_or_a_fifo_is_refused_without_blocking() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let regular = temp.path().join("regular.txt");
        fs::write(&regular, b"template").expect("a template");
        let link = temp.path().join("link.txt");
        symlink(&regular, &link).expect("a link");
        let pipe = temp.path().join("pipe.txt");
        make_fifo(&pipe);

        let through_link = open_regular_file(&link).map(|_| ());
        let from_pipe = open_regular_file(&pipe).map(|_| ());

        assert_eq!(through_link, Err(not_regular()));
        assert_eq!(from_pipe, Err(not_regular()));
        assert!(open_regular_file(&regular).is_ok());
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
}
