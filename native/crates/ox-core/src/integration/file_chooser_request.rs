// SPDX-License-Identifier: AGPL-3.0-only
//! Checking the calls of the `org.freedesktop.impl.portal.FileChooser`
//! backend interface and building their answers.
//!
//! New in the native app (INT-032). The desktop portal (xdg-desktop-portal)
//! forwards an application's Open or Save dialog to the backend the user
//! chose; this module turns one call's title and option dictionary into a
//! [`ChooserRequest`] the window can show, and a [`ChooserAnswer`] back
//! into the `(u, a{sv})` reply the portal expects.
//!
//! Safety rule "requests are data, never commands", as for Show in folder:
//! every option is type-checked and bounded, a suggested name may not
//! leave the folder it is saved in, and only absolute local folders are
//! taken from the caller. Options of an unexpected type are ignored, as the
//! portal's own backends do, so a newer caller still gets a dialog.

use std::path::{Path, PathBuf};

use gio::prelude::*;

/// The backend interface the portal calls.
pub const FILE_CHOOSER_INTERFACE: &str = "org.freedesktop.impl.portal.FileChooser";

/// The most filters, patterns per filter, choices or files one call may
/// carry. Real dialogs use a handful; the bound keeps a hostile caller from
/// making the window build thousands of rows.
pub const MAX_LIST_ITEMS: usize = 256;

/// The longest title, label, name or pattern kept, in characters.
pub const MAX_TEXT_CHARS: usize = 1024;

/// The response code of a choice (`response` of the portal's `Request`).
pub const RESPONSE_SUCCESS: u32 = 0;

/// The response code when the user cancelled.
pub const RESPONSE_CANCELLED: u32 = 1;

/// The response code when the dialog ended another way, such as the
/// portal closing it.
pub const RESPONSE_OTHER: u32 = 2;

/// The three methods of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChooserMethod {
    /// `OpenFile`: one or more existing files, or a folder.
    OpenFile,
    /// `SaveFile`: one new or replaced file.
    SaveFile,
    /// `SaveFiles`: a folder to save several named files in.
    SaveFiles,
}

impl ChooserMethod {
    /// The method for a D-Bus method name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "OpenFile" => Some(Self::OpenFile),
            "SaveFile" => Some(Self::SaveFile),
            "SaveFiles" => Some(Self::SaveFiles),
            _ => None,
        }
    }
}

/// What the user is asked to choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChooserMode {
    /// Existing items: several when `multiple`, folders when `directory`.
    Open {
        /// More than one item may be chosen.
        multiple: bool,
        /// Folders are chosen instead of files.
        directory: bool,
    },
    /// One file, with `name` suggested in the name box.
    Save {
        /// The suggested file name; empty when the caller gave none.
        name: String,
    },
    /// A folder for files with these names.
    SaveFiles {
        /// The names, in the caller's order.
        names: Vec<String>,
    },
}

/// One pattern of a filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterPattern {
    /// A shell glob such as `*.png`, matched against the name without
    /// regard to case.
    Glob(String),
    /// A MIME type such as `image/png`, matched against the content type
    /// and its parents.
    MimeType(String),
}

/// A named filter of the type list, such as "Images".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFilter {
    /// The label the type list shows.
    pub name: String,
    /// The patterns, any of which lets a file through.
    pub patterns: Vec<FilterPattern>,
}

impl FileFilter {
    /// Whether a file called `name` with `content_type` passes. Folders are
    /// the caller's business: the window always lists them.
    pub fn matches(&self, name: &str, content_type: Option<&str>) -> bool {
        self.patterns.iter().any(|pattern| match pattern {
            FilterPattern::Glob(glob) => glob_matches(glob, name),
            FilterPattern::MimeType(mime_type) => {
                content_type.is_some_and(|content_type| gio::content_type_is_a(content_type, mime_type))
            }
        })
    }

    /// The filter as the portal writes it, `(sa(us))`.
    fn to_variant(&self) -> glib::Variant {
        let patterns: Vec<(u32, String)> = self
            .patterns
            .iter()
            .map(|pattern| match pattern {
                FilterPattern::Glob(glob) => (0, glob.clone()),
                FilterPattern::MimeType(mime_type) => (1, mime_type.clone()),
            })
            .collect();
        (self.name.clone(), patterns).to_variant()
    }
}

/// An extra choice the caller adds to the dialog, such as an encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The caller's ID, returned with the answer.
    pub id: String,
    /// The label shown beside the control.
    pub label: String,
    /// The options as `(id, label)`; empty for a check box.
    pub options: Vec<(String, String)>,
    /// The option chosen at first: an option ID, or `"true"`/`"false"` for
    /// a check box.
    pub initial: String,
}

impl Choice {
    /// Whether the choice is a check box rather than a list.
    pub fn is_check_box(&self) -> bool {
        self.options.is_empty()
    }
}

/// One checked call: everything the window needs to show the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChooserRequest {
    /// The method that was called.
    pub method: ChooserMethod,
    /// The window title the caller asked for, possibly empty.
    pub title: String,
    /// The label of the accept button, such as "Upload"; `None` for the
    /// method's own word.
    pub accept_label: Option<String>,
    /// What is chosen.
    pub mode: ChooserMode,
    /// The type list, in the caller's order.
    pub filters: Vec<FileFilter>,
    /// The filter selected at first, an index into `filters`.
    pub current_filter: Option<usize>,
    /// The folder to start in, when the caller named an absolute one.
    pub current_folder: Option<PathBuf>,
    /// The caller's extra choices.
    pub choices: Vec<Choice>,
}

/// Why a call was refused. `Display` is the D-Bus error message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChooserRequestError {
    /// The method is not part of the interface.
    #[error("Unknown file chooser method: {0}")]
    UnknownMethod(String),
    /// A list holds more than [`MAX_LIST_ITEMS`] items.
    #[error("The request lists too many {0}.")]
    TooMany(&'static str),
    /// `SaveFiles` named no file, or a suggested name is not a plain name.
    #[error("{0}")]
    BadName(String),
}

impl ChooserRequest {
    /// Checks one call of `method` with its `title` and `options`
    /// (`a{sv}`).
    ///
    /// # Errors
    ///
    /// [`ChooserRequestError`] when the method is unknown, a list is too
    /// long, or a file name to save could leave its folder.
    pub fn from_call(
        method: &str,
        title: &str,
        options: &glib::VariantDict,
    ) -> Result<Self, ChooserRequestError> {
        let method = ChooserMethod::from_name(method)
            .ok_or_else(|| ChooserRequestError::UnknownMethod(bounded(method)))?;
        let mut filters = match method {
            ChooserMethod::SaveFiles => Vec::new(),
            ChooserMethod::OpenFile | ChooserMethod::SaveFile => read_filters(options)?,
        };
        let current_filter = match method {
            ChooserMethod::SaveFiles => None,
            ChooserMethod::OpenFile | ChooserMethod::SaveFile => {
                lookup::<(String, Vec<(u32, String)>)>(options, "current_filter")
                    .and_then(filter_from_parts)
                    .map(|filter| select_filter(&mut filters, filter))
            }
        };
        let mode = match method {
            ChooserMethod::OpenFile => ChooserMode::Open {
                multiple: lookup::<bool>(options, "multiple").unwrap_or(false),
                directory: lookup::<bool>(options, "directory").unwrap_or(false),
            },
            ChooserMethod::SaveFile => ChooserMode::Save {
                name: suggested_name(options)?,
            },
            ChooserMethod::SaveFiles => ChooserMode::SaveFiles {
                names: file_names(options)?,
            },
        };
        let current_folder = match method {
            ChooserMethod::SaveFile => lookup_path(options, "current_file")
                .and_then(|file| file.parent().map(Path::to_path_buf))
                .or_else(|| lookup_path(options, "current_folder")),
            ChooserMethod::OpenFile | ChooserMethod::SaveFiles => lookup_path(options, "current_folder"),
        };
        Ok(Self {
            method,
            title: bounded(title),
            accept_label: lookup::<String>(options, "accept_label")
                .map(|label| bounded(&label))
                .filter(|label| !label.trim().is_empty()),
            mode,
            filters,
            current_filter,
            current_folder,
            choices: read_choices(options)?,
        })
    }

    /// The window title: the caller's, or the method's own words.
    pub fn window_title(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        match &self.mode {
            ChooserMode::Open { directory: true, .. } => "Select folder".to_owned(),
            ChooserMode::Open { multiple: true, .. } => "Open files".to_owned(),
            ChooserMode::Open { .. } => "Open".to_owned(),
            ChooserMode::Save { .. } => "Save as".to_owned(),
            ChooserMode::SaveFiles { .. } => "Save files to".to_owned(),
        }
    }

    /// The accept button's label: the caller's, or "Open", "Select folder"
    /// or "Save". A caller's mnemonic underscore is dropped, since the
    /// button shows plain text.
    pub fn accept_label(&self) -> String {
        if let Some(label) = &self.accept_label {
            return label.replace('_', "");
        }
        match &self.mode {
            ChooserMode::Open { directory: true, .. } => "Select folder".to_owned(),
            ChooserMode::Open { .. } => "Open".to_owned(),
            ChooserMode::Save { .. } | ChooserMode::SaveFiles { .. } => "Save".to_owned(),
        }
    }

    /// Whether the dialog asks for a folder rather than files.
    pub fn chooses_folder(&self) -> bool {
        matches!(
            self.mode,
            ChooserMode::Open { directory: true, .. } | ChooserMode::SaveFiles { .. }
        )
    }

    /// The `(response, results)` reply for `answer`.
    pub fn reply(&self, answer: &ChooserAnswer) -> (u32, glib::Variant) {
        let results = glib::VariantDict::new(None);
        let ChooserAnswer::Chosen {
            locations,
            filter,
            choices,
        } = answer
        else {
            let response = match answer {
                ChooserAnswer::Cancelled => RESPONSE_CANCELLED,
                ChooserAnswer::Chosen { .. } | ChooserAnswer::Ended => RESPONSE_OTHER,
            };
            return (response, results.end());
        };
        let uris: Vec<String> = match &self.mode {
            ChooserMode::SaveFiles { names } => locations
                .first()
                .map(|folder| names.iter().map(|name| file_uri_of(&folder.join(name))).collect())
                .unwrap_or_default(),
            ChooserMode::Open { .. } | ChooserMode::Save { .. } => {
                locations.iter().map(|location| file_uri_of(location)).collect()
            }
        };
        results.insert_value("uris", &uris.to_variant());
        if let Some(filter) = filter.and_then(|index| self.filters.get(index)) {
            results.insert_value("current_filter", &filter.to_variant());
        }
        if !self.choices.is_empty() {
            let chosen: Vec<(String, String)> = self
                .choices
                .iter()
                .map(|choice| {
                    let value = choices
                        .iter()
                        .find(|(id, _)| *id == choice.id)
                        .map_or_else(|| choice.initial.clone(), |(_, value)| value.clone());
                    (choice.id.clone(), value)
                })
                .collect();
            results.insert_value("choices", &chosen.to_variant());
        }
        if let ChooserMode::Open { .. } = self.mode {
            results.insert_value("writable", &false.to_variant());
        }
        (RESPONSE_SUCCESS, results.end())
    }
}

/// How the dialog ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChooserAnswer {
    /// The user chose: the files to open or save, or for `SaveFiles` and
    /// folder dialogs the one folder. `filter` is the type selected at
    /// the end; `choices` are `(id, value)` pairs.
    Chosen {
        /// Absolute local paths.
        locations: Vec<PathBuf>,
        /// The selected type, an index into the request's filters.
        filter: Option<usize>,
        /// The values of the caller's choices.
        choices: Vec<(String, String)>,
    },
    /// The user pressed Cancel or closed the window.
    Cancelled,
    /// The dialog went away for another reason, such as the portal
    /// closing it or the app quitting.
    Ended,
}

/// The `file://` URI of an absolute path.
fn file_uri_of(path: &Path) -> String {
    gio::File::for_path(path).uri().to_string()
}

/// `text` cut to [`MAX_TEXT_CHARS`].
fn bounded(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

/// The option `key` when it has the type `T`.
fn lookup<T: FromVariant + StaticVariantType>(options: &glib::VariantDict, key: &str) -> Option<T> {
    options
        .lookup_value(key, Some(&T::static_variant_type()))?
        .get::<T>()
}

/// A NUL-terminated byte-string option holding an absolute path.
fn lookup_path(options: &glib::VariantDict, key: &str) -> Option<PathBuf> {
    let value = options.lookup_value(key, Some(glib::VariantTy::BYTE_STRING))?;
    path_from_bytes(value.fixed_array::<u8>().ok()?)
}

/// An absolute path from a byte string the portal NUL-terminates.
fn path_from_bytes(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if bytes.is_empty() || bytes.contains(&0) {
        return None;
    }
    let path = Path::new(std::ffi::OsStr::from_bytes(bytes));
    path.is_absolute().then(|| path.to_path_buf())
}

/// The filters of `filters`, `a(sa(us))`.
fn read_filters(options: &glib::VariantDict) -> Result<Vec<FileFilter>, ChooserRequestError> {
    let Some(filters) = lookup::<Vec<(String, Vec<(u32, String)>)>>(options, "filters") else {
        return Ok(Vec::new());
    };
    if filters.len() > MAX_LIST_ITEMS {
        return Err(ChooserRequestError::TooMany("filters"));
    }
    if filters
        .iter()
        .any(|(_, patterns)| patterns.len() > MAX_LIST_ITEMS)
    {
        return Err(ChooserRequestError::TooMany("filter patterns"));
    }
    Ok(filters.into_iter().filter_map(filter_from_parts).collect())
}

/// A filter from its name and `(kind, pattern)` pairs. Unknown kinds are
/// dropped; a filter left without patterns is dropped too.
fn filter_from_parts((name, patterns): (String, Vec<(u32, String)>)) -> Option<FileFilter> {
    let patterns: Vec<FilterPattern> = patterns
        .into_iter()
        .filter(|(_, pattern)| !pattern.is_empty())
        .filter_map(|(kind, pattern)| match kind {
            0 => Some(FilterPattern::Glob(bounded(&pattern))),
            1 => Some(FilterPattern::MimeType(bounded(&pattern))),
            _ => None,
        })
        .collect();
    (!patterns.is_empty()).then(|| FileFilter {
        name: bounded(&name),
        patterns,
    })
}

/// The index of `current` in `filters`, adding it at the end when the
/// caller did not list it (as GTK's dialog shows it).
fn select_filter(filters: &mut Vec<FileFilter>, current: FileFilter) -> usize {
    if let Some(index) = filters.iter().position(|filter| *filter == current) {
        return index;
    }
    filters.push(current);
    filters.len() - 1
}

/// The suggested name of `SaveFile`: `current_name`, else the name of
/// `current_file`.
fn suggested_name(options: &glib::VariantDict) -> Result<String, ChooserRequestError> {
    let name = lookup::<String>(options, "current_name").or_else(|| {
        lookup_path(options, "current_file")
            .and_then(|file| file.file_name().map(|name| name.to_string_lossy().into_owned()))
    });
    match name {
        None => Ok(String::new()),
        Some(name) if name.is_empty() => Ok(String::new()),
        Some(name) => checked_name(&name),
    }
}

/// The names of `SaveFiles`' `files`, `aay`.
fn file_names(options: &glib::VariantDict) -> Result<Vec<String>, ChooserRequestError> {
    let files = options
        .lookup_value("files", Some(glib::VariantTy::BYTE_STRING_ARRAY))
        .map(|files| files.iter().collect::<Vec<_>>())
        .unwrap_or_default();
    if files.is_empty() {
        return Err(ChooserRequestError::BadName(
            "No files to save were named.".to_owned(),
        ));
    }
    if files.len() > MAX_LIST_ITEMS {
        return Err(ChooserRequestError::TooMany("files"));
    }
    files
        .iter()
        .map(|file| {
            let bytes = file.fixed_array::<u8>().unwrap_or_default();
            let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
            checked_name(&String::from_utf8_lossy(bytes))
        })
        .collect()
}

/// `name` when it is a plain file name that stays in its folder.
///
/// # Errors
///
/// [`ChooserRequestError::BadName`] for a name with a slash, `.`, `..` or
/// a NUL.
pub fn checked_name(name: &str) -> Result<String, ChooserRequestError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(ChooserRequestError::BadName(format!(
            "Not a file name: {}",
            bounded(name)
        )));
    }
    Ok(bounded(name))
}

/// The choices of `choices`, `a(ssa(ss)s)`.
fn read_choices(options: &glib::VariantDict) -> Result<Vec<Choice>, ChooserRequestError> {
    type Parts = Vec<(String, String, Vec<(String, String)>, String)>;
    let Some(choices) = lookup::<Parts>(options, "choices") else {
        return Ok(Vec::new());
    };
    if choices.len() > MAX_LIST_ITEMS
        || choices
            .iter()
            .any(|(_, _, options, _)| options.len() > MAX_LIST_ITEMS)
    {
        return Err(ChooserRequestError::TooMany("choices"));
    }
    Ok(choices
        .into_iter()
        .filter(|(id, ..)| !id.is_empty())
        .map(|(id, label, options, initial)| Choice {
            id: bounded(&id),
            label: bounded(&label),
            options: options
                .into_iter()
                .map(|(id, label)| (bounded(&id), bounded(&label)))
                .collect(),
            initial: bounded(&initial),
        })
        .collect())
}

/// Whether the shell glob `pattern` (`*`, `?` and `[...]` classes) matches
/// all of `name`, ignoring case, as GTK's dialogs do for suffixes.
pub fn glob_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    matches_from(&pattern, &name)
}

/// The glob matcher: backtracks over the last `*` only, which is linear
/// in practice and needs no recursion.
fn matches_from(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() {
            match pattern[p] {
                '*' => {
                    star = Some((p, n));
                    p += 1;
                    continue;
                }
                '?' => {
                    p += 1;
                    n += 1;
                    continue;
                }
                '[' => {
                    if let Some((matched, end)) = class_matches(&pattern[p..], name[n]) {
                        if matched {
                            p += end;
                            n += 1;
                            continue;
                        }
                    } else if name[n] == '[' {
                        p += 1;
                        n += 1;
                        continue;
                    }
                }
                literal if literal == name[n] => {
                    p += 1;
                    n += 1;
                    continue;
                }
                _ => {}
            }
        }
        let Some((star_p, star_n)) = star else {
            return false;
        };
        p = star_p + 1;
        n = star_n + 1;
        star = Some((star_p, star_n + 1));
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Whether the class at the start of `pattern` (`[abc]`, `[a-z]`, `[!x]`)
/// holds `c`, and the class's length; `None` when the class is not closed.
fn class_matches(pattern: &[char], c: char) -> Option<(bool, usize)> {
    let mut index = 1;
    let negated = matches!(pattern.get(index), Some('!' | '^'));
    if negated {
        index += 1;
    }
    let mut matched = false;
    let mut first = true;
    loop {
        let current = *pattern.get(index)?;
        if current == ']' && !first {
            return Some((matched != negated, index + 1));
        }
        first = false;
        if pattern.get(index + 1) == Some(&'-') && pattern.get(index + 2).is_some_and(|&end| end != ']') {
            let end = pattern[index + 2];
            matched |= (current..=end).contains(&c);
            index += 3;
        } else {
            matched |= current == c;
            index += 1;
        }
    }
}

/// Lets a test build the option dictionary as the portal sends it.
#[doc(hidden)]
pub fn options_from_entries(entries: &[(&str, glib::Variant)]) -> glib::VariantDict {
    let dict = glib::VariantDict::new(None);
    for (key, value) in entries {
        dict.insert_value(key, value);
    }
    dict
}

/// Lets a test write a byte-string path as the portal does.
#[doc(hidden)]
pub fn path_variant(path: &str) -> glib::Variant {
    let mut bytes = path.as_bytes().to_vec();
    bytes.push(0);
    glib::Variant::array_from_fixed_array(&bytes)
}
