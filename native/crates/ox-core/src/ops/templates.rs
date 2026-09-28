// SPDX-License-Identifier: AGPL-3.0-only
//! The templates New offers: six built-in starters and the files in the
//! user's Templates folder.
//!
//! Ports `PRESETS` and `list_templates` in `desktop/file_services.py`.
//! OPS-048 limits which files count as user templates: at most
//! [`MAX_USER_TEMPLATES`] regular files of at most [`MAX_TEMPLATE_BYTES`]
//! that are neither hidden, links nor `.desktop` launchers, listed without
//! following links. `new_from_template` re-checks a chosen template against
//! a fresh list and reads it under the same limits.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use gio::prelude::*;

use super::context::on_worker;
use super::error::OpsError;
use crate::transfer::Cancellation;

/// How many user templates are listed at most.
pub const MAX_USER_TEMPLATES: usize = 100;

/// The largest user template, in bytes (16 MiB).
pub const MAX_TEMPLATE_BYTES: u64 = 16 * 1024 * 1024;

/// The attributes the Templates folder is listed with.
const LISTING_ATTRIBUTES: &str =
    "standard::name,standard::type,standard::is-hidden,standard::is-symlink,standard::size";

/// The prefix of a user template's protocol id (`user:Letter.odt`).
const USER_ID_PREFIX: &str = "user:";

/// The six starters New always offers, with fixed names and content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinTemplate {
    /// An empty `New document.txt`.
    Text,
    /// `New document.md` with a heading.
    Markdown,
    /// An empty `New spreadsheet.csv`.
    Csv,
    /// `New file.json` holding an empty object.
    Json,
    /// `New page.html`, a minimal UTF-8 HTML5 page.
    Html,
    /// An empty `New file` without an extension.
    Empty,
}

impl BuiltinTemplate {
    /// Every starter, in the order New lists them.
    pub const ALL: [BuiltinTemplate; 6] = [
        BuiltinTemplate::Text,
        BuiltinTemplate::Markdown,
        BuiltinTemplate::Csv,
        BuiltinTemplate::Json,
        BuiltinTemplate::Html,
        BuiltinTemplate::Empty,
    ];

    /// The protocol id (`text`, `markdown`, ...), as `PRESETS` names it.
    pub fn id(self) -> &'static str {
        match self {
            BuiltinTemplate::Text => "text",
            BuiltinTemplate::Markdown => "markdown",
            BuiltinTemplate::Csv => "csv",
            BuiltinTemplate::Json => "json",
            BuiltinTemplate::Html => "html",
            BuiltinTemplate::Empty => "empty",
        }
    }

    /// The name New shows, for example `Text document`.
    pub fn label(self) -> &'static str {
        match self {
            BuiltinTemplate::Text => "Text document",
            BuiltinTemplate::Markdown => "Markdown document",
            BuiltinTemplate::Csv => "CSV file",
            BuiltinTemplate::Json => "JSON file",
            BuiltinTemplate::Html => "HTML document",
            BuiltinTemplate::Empty => "Empty file",
        }
    }

    /// The file name the dialog suggests.
    pub fn suggested_name(self) -> &'static str {
        match self {
            BuiltinTemplate::Text => "New document.txt",
            BuiltinTemplate::Markdown => "New document.md",
            BuiltinTemplate::Csv => "New spreadsheet.csv",
            BuiltinTemplate::Json => "New file.json",
            BuiltinTemplate::Html => "New page.html",
            BuiltinTemplate::Empty => "New file",
        }
    }

    /// The new file's content, byte for byte as `PRESETS` has it.
    pub fn contents(self) -> &'static [u8] {
        match self {
            BuiltinTemplate::Text | BuiltinTemplate::Csv | BuiltinTemplate::Empty => b"",
            BuiltinTemplate::Markdown => b"# New document\n",
            BuiltinTemplate::Json => b"{}\n",
            BuiltinTemplate::Html => concat!(
                "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">",
                "<title>New page</title></head><body></body></html>\n"
            )
            .as_bytes(),
        }
    }
}

/// Which template New from template copies.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TemplateId {
    /// One of the built-in starters.
    Builtin(BuiltinTemplate),
    /// A file in the Templates folder, by its file name.
    User(String),
}

impl FromStr for TemplateId {
    type Err = OpsError;

    /// Parses a protocol id: a starter's id or `user:<file name>`.
    fn from_str(id: &str) -> Result<Self, Self::Err> {
        if let Some(file_name) = id.strip_prefix(USER_ID_PREFIX) {
            return Ok(TemplateId::User(file_name.to_owned()));
        }
        BuiltinTemplate::ALL
            .into_iter()
            .find(|template| template.id() == id)
            .map(TemplateId::Builtin)
            .ok_or_else(|| OpsError::failed("Choose an available template or Empty file."))
    }
}

impl fmt::Display for TemplateId {
    /// The protocol id, as [`TemplateId::from_str`] reads it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateId::Builtin(template) => formatter.write_str(template.id()),
            TemplateId::User(file_name) => write!(formatter, "{USER_ID_PREFIX}{file_name}"),
        }
    }
}

/// One entry of the template list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// What New from template is asked to copy.
    pub id: TemplateId,
    /// The name New shows: a starter's label or the template's file name.
    pub label: String,
    /// The file name the dialog suggests.
    pub suggested_name: String,
}

impl Template {
    /// True for a built-in starter; the dialog marks the others as the
    /// user's own templates.
    pub fn is_builtin(&self) -> bool {
        matches!(self.id, TemplateId::Builtin(_))
    }

    /// The entry for a starter.
    fn builtin(template: BuiltinTemplate) -> Self {
        Self {
            id: TemplateId::Builtin(template),
            label: template.label().to_owned(),
            suggested_name: template.suggested_name().to_owned(),
        }
    }

    /// The entry for the user template `file_name`.
    fn user(file_name: &str) -> Self {
        Self {
            id: TemplateId::User(file_name.to_owned()),
            label: file_name.to_owned(),
            suggested_name: file_name.to_owned(),
        }
    }
}

/// The templates New offers, and the folder the user's own come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateList {
    /// The starters first, then the user's templates in folder order.
    pub templates: Vec<Template>,
    /// The Templates folder, shown under the list.
    pub folder: PathBuf,
}

impl TemplateList {
    /// True when `id` is in this list.
    pub fn contains(&self, id: &TemplateId) -> bool {
        self.templates.iter().any(|template| &template.id == id)
    }
}

/// Lists the starters and the user templates in `folder` (the XDG
/// Templates folder). A missing folder lists only the starters.
///
/// # Errors
///
/// Cancellation, or a failure while the folder is listed.
pub async fn list_templates(folder: &Path, cancel: &Cancellation) -> Result<TemplateList, OpsError> {
    let folder = folder.to_path_buf();
    let cancel = cancel.clone();
    on_worker(move || list_templates_blocking(&folder, &cancel)).await
}

/// [`list_templates`] on the calling thread.
pub(crate) fn list_templates_blocking(
    folder: &Path,
    cancel: &Cancellation,
) -> Result<TemplateList, OpsError> {
    let mut templates: Vec<Template> = BuiltinTemplate::ALL.into_iter().map(Template::builtin).collect();
    let directory = gio::File::for_path(folder);
    if directory.query_exists(Some(cancel.cancellable())) {
        let user_templates = user_templates(&directory, cancel)?;
        templates.extend(user_templates);
    }
    Ok(TemplateList {
        templates,
        folder: folder.to_path_buf(),
    })
}

/// OPS-048: the first [`MAX_USER_TEMPLATES`] files in `directory` that
/// qualify as templates. Links are never followed, so a link cannot make a
/// device, a FIFO or a file outside the folder a template. The listing is
/// closed on success, cancellation and error alike.
fn user_templates(directory: &gio::File, cancel: &Cancellation) -> Result<Vec<Template>, OpsError> {
    let listing = directory.enumerate_children(
        LISTING_ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    let templates = read_templates(&listing, cancel);
    // Closing only releases the listing; what was read stays valid.
    let _ = listing.close(gio::Cancellable::NONE);
    templates
}

/// Reads qualifying templates from an open listing.
fn read_templates(listing: &gio::FileEnumerator, cancel: &Cancellation) -> Result<Vec<Template>, OpsError> {
    let mut templates = Vec::new();
    while templates.len() < MAX_USER_TEMPLATES {
        cancel.check()?;
        let Some(info) = listing.next_file(Some(cancel.cancellable()))? else {
            break;
        };
        if let Some(file_name) = template_name(&info) {
            templates.push(Template::user(&file_name));
        }
    }
    Ok(templates)
}

/// The file name of a qualifying template: a regular file, not hidden and
/// not a link, at most [`MAX_TEMPLATE_BYTES`], and not a `.desktop`
/// launcher, which would describe a program rather than hold a document.
/// A name that is not valid UTF-8 is skipped: a template id is text.
fn template_name(info: &gio::FileInfo) -> Option<String> {
    let is_plain_file = info.file_type() == gio::FileType::Regular && !info.is_symlink() && !info.is_hidden();
    let size = u64::try_from(info.size()).unwrap_or(u64::MAX);
    if !is_plain_file || size > MAX_TEMPLATE_BYTES {
        return None;
    }
    let file_name = info.name().to_str()?.to_owned();
    if file_name.ends_with(".desktop") {
        return None;
    }
    Some(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_ids_round_trip() {
        for template in BuiltinTemplate::ALL {
            let id = TemplateId::Builtin(template);

            assert_eq!(id.to_string().parse::<TemplateId>(), Ok(id));
        }
        let user = TemplateId::User("Letter.odt".into());
        assert_eq!(user.to_string(), "user:Letter.odt");
        assert_eq!("user:Letter.odt".parse::<TemplateId>(), Ok(user));
    }

    #[test]
    fn an_unknown_template_id_is_refused_with_the_python_message() {
        let refused = "spreadsheet".parse::<TemplateId>();

        assert_eq!(
            refused,
            Err(OpsError::Failed(
                "Choose an available template or Empty file.".into()
            ))
        );
    }
}
