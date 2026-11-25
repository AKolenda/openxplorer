// SPDX-License-Identifier: AGPL-3.0-only
//! Office documents (Word, Excel and `PowerPoint` files, and `OpenDocument`
//! text, spreadsheets and presentations) turned into PDF by `LibreOffice`,
//! for the Preview pane's pages and the views' thumbnails, as Windows'
//! Preview pane shows them through Office's preview handlers (VIEW-070,
//! VIEW-057).
//!
//! `LibreOffice` reads a file the user may have downloaded from anywhere,
//! so it runs only in the thumbnailers' sandbox ([`Sandbox`]): no
//! network, no session bus and no other process, the system read-only,
//! the document alone of the user's files, read-only and under a name of
//! the sandbox's own, within the thumbnailers' memory, file-size and
//! processor limits, for at most [`TIMEOUT`], and stopped as soon as the
//! selection changes. Its profile is a new, private one in the sandbox's
//! own folder each time, written before it starts so that no macro of a
//! document ever runs ([`PROFILE`]): nothing it keeps outlives the
//! document. It never sees the folder the document is in, so it can
//! leave no lock file (`.~lock.*#`) or other file beside it. A computer
//! without `LibreOffice`, without bubblewrap or without poppler's
//! `pdftoppm` (which draws the PDF's pages, in the same sandbox) converts
//! nothing: the Preview pane says "No preview available." under the
//! file's icon, and the item keeps its icon.
//!
//! One document is converted at a time ([`Lane`]): the Preview pane's
//! selection goes first, and stops a thumbnail's conversion of another
//! document, which waits for its turn again; only a few thumbnails wait
//! behind it. A PDF
//! made is kept in the app's cache ([`Cache`]), named after the
//! document's real path, size, modification time and inode
//! ([`Version`]), so selecting it again
//! shows it at once and a changed document is converted again; a
//! document that could not be converted (damaged, protected by a
//! password, or too slow twice) is not tried again in this run.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gio, glib};

use super::{
    extension_of, installed_program, pdf_pages_in, program_folders, read_picture, run_within,
    sandbox_arguments, writer, Job, Sandbox, Source, Thumbnailer, UserFolders, Workspace, INPUT, INSIDE,
    PDF_PROGRAM,
};

/// The longest a conversion may take before it is stopped: a document
/// that takes longer keeps its icon, and says "No preview available.".
pub(crate) const TIMEOUT: Duration = Duration::from_secs(15);

/// The largest document converted, in bytes; a larger one keeps its icon.
const SIZE_LIMIT: u64 = 100 * 1024 * 1024;

/// The largest PDF read back from `LibreOffice`, in bytes.
const PDF_SIZE_LIMIT: u64 = 64 * 1024 * 1024;

/// How many thumbnails may wait for their turn ([`Lane`]); one more is
/// asked again the next time it is shown.
const QUEUE_LIMIT: usize = 4;

/// How often a request waiting for its turn looks whether it was
/// cancelled.
const WAIT_POLL: Duration = Duration::from_millis(50);

/// The most PDFs the cache keeps, and the most bytes; the ones used
/// longest ago go first.
const CACHE_COUNT_LIMIT: usize = 256;
const CACHE_BYTE_LIMIT: u64 = 512 * 1024 * 1024;

/// The cache's folder, in the app's own folder of the user's cache.
#[cfg(not(test))]
const CACHE_FOLDER: &str = "document-previews";

/// The end of the name a PDF is written under before it takes its own.
const TEMPORARY_SUFFIX: &str = ".openxplorer.tmp";

/// How old a PDF left under its temporary name must be to be removed: a
/// conversion that still writes it takes seconds.
const LEFTOVER_AGE: Duration = Duration::from_hours(1);

/// What the cache's names start with, so a cache of another version's
/// conversions is never taken for this one's.
const CACHE_VERSION: &str = "1";

/// The folders inside the sandbox's own: `LibreOffice`'s private profile
/// and the folder it writes its PDF in.
const PROFILE_FOLDER: &str = "profile";
const OUTPUT_FOLDER: &str = "out";

/// The PDF's name: the document's inside the sandbox, as a PDF.
const OUTPUT_PDF: &str = "openxplorer-item.pdf";

/// The settings `LibreOffice`'s private profile starts with, in the
/// profile's own format (`user/registrymodifications.xcu`):
///
/// - no macro runs, whatever the document asks (`DisableMacrosExecution`
///   turns Basic, JavaScript, `BeanShell` and Python off) and whoever signed
///   it (`MacroSecurityLevel` 3, "Very high", with no trusted location);
/// - links to other files and to the network are never updated on load
///   (Writer's `Update/Link` 0 and Calc's 1 mean "Never": Writer's values
///   are `sw/inc/linkenum.hxx`'s, Calc's `ScLkUpdMode`'s), and a link
///   from an untrusted document is blocked;
/// - no `OpenCL`, which would reach the graphics card.
///
/// The sandbox has no network and none of the user's files anyway; these
/// keep `LibreOffice` from even trying.
const PROFILE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="BlockUntrustedRefererLinks" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Writer/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>0</value></prop></item>
<item oor:path="/org.openoffice.Office.Calc/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>1</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Misc"><prop oor:name="UseOpenCL" oor:op="fuse"><value>false</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Misc"><prop oor:name="FirstRun" oor:op="fuse"><value>false</value></prop></item>
</oor:items>
"#;

/// The types converted: Word, Excel and `PowerPoint` documents (with and
/// without macros, which never run) and `OpenDocument` text, spreadsheets
/// and presentations.
const DOCUMENT_TYPES: [&str; 15] = [
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-word.document.macroenabled.12",
    "application/msword",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-excel.sheet.macroenabled.12",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.ms-powerpoint.presentation.macroenabled.12",
    "application/vnd.ms-powerpoint",
    "application/vnd.oasis.opendocument.text",
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.oasis.opendocument.presentation",
    "application/vnd.oasis.opendocument.text-template",
    "application/vnd.oasis.opendocument.spreadsheet-template",
    "application/vnd.oasis.opendocument.presentation-template",
];

/// The extensions converted whatever type the system names for them: a
/// computer without the Office types in its MIME database calls a `.docx`
/// a ZIP archive.
const DOCUMENT_EXTENSIONS: [&str; 15] = [
    "docx", "docm", "doc", "xlsx", "xlsm", "xls", "pptx", "pptm", "ppt", "odt", "ods", "odp", "ott", "ots",
    "otp",
];

/// How many threads `LibreOffice`'s pool may run (`MAX_CONCURRENCY`, which
/// it reads at start): its own count, the processors', would exceed the
/// thumbnailers' task limit on a computer of many processors.
const THREADS: &str = "4";

/// Whether a file of `content_type` named `name` is a document
/// `LibreOffice` converts here.
pub(crate) fn is_document(content_type: &str, name: &str) -> bool {
    let content_type = content_type.to_ascii_lowercase();
    if DOCUMENT_TYPES.contains(&content_type.as_str()) {
        return true;
    }
    let extension = name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    DOCUMENT_EXTENSIONS.contains(&extension.as_str())
}

/// Why a document has no PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Nothing converts documents here: no `LibreOffice`, no bubblewrap
    /// that works, or no `pdftoppm` to draw the pages.
    Unavailable,
    /// The item is no plain file of this computer, or it is larger than
    /// [`SIZE_LIMIT`].
    Refused,
    /// `LibreOffice` could not convert it: it is damaged, needs a password,
    /// or is no document after all. Not tried again in this run.
    Failed,
    /// It took longer than [`TIMEOUT`]. Tried once more, as a computer
    /// busy just then may have slowed it, then not again in this run.
    TimedOut,
    /// The request was dropped, too many wait already, or the app could
    /// not prepare the conversion or keep its PDF (a full disk); it is
    /// asked again the next time it is shown.
    Cancelled,
}

/// Who asks for a conversion: the Preview pane's selection goes before
/// the views' thumbnails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Priority {
    /// The Preview pane, for the selection.
    Preview,
    /// A view, for an item's thumbnail.
    Thumbnail,
}

/// The `LibreOffice` installed on this computer, with the system or in
/// `/opt`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Program {
    /// `…/program/soffice`, as it really is.
    soffice: PathBuf,
    /// The folders the sandbox shows read-only besides the system's: the
    /// install folder when it is outside `/usr`, and what its links lead
    /// to outside it (Debian's `/etc/libreoffice`).
    folders: Vec<PathBuf>,
}

impl Program {
    /// The `LibreOffice` of this computer: the `soffice` or `libreoffice`
    /// command (or `libreoffice24.8` and the like, as The Document
    /// Foundation's own packages name it) on `PATH` that leads to a
    /// `program/soffice`. Flatpak's `LibreOffice` is not used: Flatpak
    /// always gives an app its own data folder, where the user's
    /// `LibreOffice` keeps its profile, so a document it converted could
    /// change the profile the user opens next.
    fn find() -> Option<Self> {
        let path = std::env::var_os("PATH")?;
        let mut commands = vec![
            glib::find_program_in_path("soffice"),
            glib::find_program_in_path("libreoffice"),
        ];
        for folder in std::env::split_paths(&path) {
            let Ok(listing) = fs::read_dir(&folder) else {
                continue;
            };
            let mut named: Vec<PathBuf> = listing
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("libreoffice"))
                })
                .map(|entry| entry.path())
                .collect();
            named.sort();
            commands.extend(named.into_iter().map(Some));
        }
        commands
            .into_iter()
            .flatten()
            .find_map(|command| Self::of_command(&command))
    }

    /// The installed `LibreOffice` the command at `command` starts, when it
    /// is, or links to, a `program/soffice`; Debian's `libreoffice`
    /// script starts `/usr/lib/libreoffice/program/soffice`, which is
    /// looked for beside the system's libraries.
    fn of_command(command: &Path) -> Option<Self> {
        let real = fs::canonicalize(command).ok()?;
        let soffice = if is_soffice(&real) {
            real
        } else {
            ["/usr/lib/libreoffice", "/usr/lib64/libreoffice"]
                .iter()
                .map(|root| Path::new(root).join("program/soffice"))
                .find(|soffice| is_soffice(soffice))?
        };
        let root = soffice.parent()?.parent()?.to_path_buf();
        Some(Self {
            folders: install_folders(&root, &UserFolders::of_session()),
            soffice,
        })
    }

    /// The words that convert the document at `inside_item` into a PDF
    /// in `output`, with the private profile at `profile`: `LibreOffice`
    /// without a window, without its first-start wizard, restore or lock
    /// check, its macros off by [`PROFILE`].
    fn conversion(profile: &Path, output: &Path, inside_item: &Path) -> Vec<OsString> {
        let mut words: Vec<OsString> = [
            "--headless",
            "--invisible",
            "--norestore",
            "--nolockcheck",
            "--nodefault",
            "--nologo",
            "--nofirststartwizard",
        ]
        .map(OsString::from)
        .into();
        let mut profile_url = OsString::from("-env:UserInstallation=");
        profile_url.push(gio::File::for_path(profile).uri().as_str());
        words.push(profile_url);
        words.extend(["--convert-to", "pdf", "--outdir"].map(OsString::from));
        words.push(output.as_os_str().to_owned());
        words.push(inside_item.as_os_str().to_owned());
        words
    }
}

/// Whether `path` is `LibreOffice`'s `soffice` in its `program` folder.
fn is_soffice(path: &Path) -> bool {
    path.file_name() == Some(OsStr::new("soffice"))
        && path.parent().and_then(Path::file_name) == Some(OsStr::new("program"))
        && path.is_file()
}

/// The most links of an install folder followed out of it.
const LINK_LIMIT: usize = 64;

/// How deep in the install folder links are looked for.
const LINK_DEPTH: usize = 3;

/// The folders `LibreOffice` installed at `root` needs to see: `root`
/// itself when it is outside the system's folders (`/opt/libreoffice24.8`),
/// and what its links lead to outside the system's `/usr` (Debian's
/// `/etc/libreoffice` and `/var/lib/libreoffice`), each only when it is
/// none of the user's own folders ([`UserFolders::may_show`]).
fn install_folders(root: &Path, user: &UserFolders) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if !root.starts_with("/usr") && user.may_show(root) {
        folders.push(root.to_path_buf());
    }
    let mut pending = vec![(root.to_path_buf(), 0)];
    while let Some((folder, depth)) = pending.pop() {
        let Ok(listing) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in listing.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && depth < LINK_DEPTH {
                pending.push((path, depth + 1));
            } else if kind.is_symlink() {
                let Ok(target) = fs::canonicalize(&path) else {
                    continue;
                };
                let outside = !target.starts_with("/usr") && !target.starts_with(root);
                let shown = folders.iter().any(|shown: &PathBuf| target.starts_with(shown));
                if outside && !shown && folders.len() < LINK_LIMIT && user.may_show(&target) {
                    folders.push(target);
                }
            }
        }
    }
    folders
}

/// `LibreOffice` and the sandbox it runs in, found once per run.
#[derive(Debug)]
pub(crate) struct Converter {
    program: Program,
    /// The thumbnailers' sandbox.
    sandbox: Sandbox,
    cache: Cache,
    /// Document versions that could not be converted in this run, by
    /// their cache names, with why.
    failed: Mutex<HashMap<String, Failure>>,
    /// Document versions that ran out of time once in this run, which are
    /// remembered as [`Failure::TimedOut`] the second time.
    timed_out_once: Mutex<std::collections::HashSet<String>>,
    lane: Lane,
}

/// The converter of this run, or `None` when nothing converts documents
/// here (see [`Failure::Unavailable`]). Looked for once, off the main
/// thread: blocks.
#[cfg(not(test))]
pub(crate) fn converter() -> Option<Arc<Converter>> {
    static FOUND: std::sync::OnceLock<Option<Arc<Converter>>> = std::sync::OnceLock::new();
    FOUND.get_or_init(|| Converter::find().map(Arc::new)).clone()
}

/// The converter a test chose with [`use_converter`]: none unless it
/// chose one, as a test never starts the programs of the computer it
/// runs on by itself.
#[cfg(test)]
pub(crate) fn converter() -> Option<Arc<Converter>> {
    TEST_CONVERTER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

impl Converter {
    /// `LibreOffice` in the thumbnailers' sandbox, with `pdftoppm` to draw
    /// its pages, or `None` with a note in the log.
    #[cfg(not(test))]
    fn find() -> Option<Self> {
        let Some(program) = Program::find() else {
            glib::g_debug!(
                ox_core::LOG_DOMAIN,
                "LibreOffice is not installed: documents get no previews"
            );
            return None;
        };
        if !installed_program(PDF_PROGRAM) {
            glib::g_message!(
                ox_core::LOG_DOMAIN,
                "poppler's pdftoppm is not installed: documents get no previews"
            );
            return None;
        }
        let sandbox = Sandbox::find()?;
        // `LibreOffice` refuses to start without a `/proc`, which some
        // containers will not mount in the sandbox.
        if matches!(sandbox, Sandbox::Bubblewrap { proc: false, .. }) {
            glib::g_message!(
                ox_core::LOG_DOMAIN,
                "The sandbox has no /proc here, which LibreOffice needs: documents get no previews"
            );
            return None;
        }
        let cache = Cache::new(glib::user_cache_dir().join("winspace").join(CACHE_FOLDER));
        Some(Self::new(program, sandbox, cache))
    }

    /// The converter of `program` in `sandbox`, keeping its PDFs in
    /// `cache`.
    fn new(program: Program, sandbox: Sandbox, cache: Cache) -> Self {
        Self {
            program,
            sandbox,
            cache,
            failed: Mutex::new(HashMap::new()),
            timed_out_once: Mutex::new(std::collections::HashSet::new()),
            lane: Lane::default(),
        }
    }

    /// The PDF of the document at `path`, from the cache, or converted
    /// when it is not there yet, in its turn for `priority` ([`Lane`]).
    /// Blocks: run it off the main thread.
    pub(crate) fn pdf(
        &self,
        path: &Path,
        priority: Priority,
        cancellation: &gio::Cancellable,
    ) -> Result<PathBuf, Failure> {
        let version = Version::of(path).ok_or(Failure::Refused)?;
        let name = version.cache_name();
        if let Some(pdf) = self.cache.find(&name) {
            return Ok(pdf);
        }
        if let Some(failure) = self.failure_of(&name) {
            return Err(failure);
        }
        // Noise under a document's name would be read as text, page after
        // page, until the time limit: only a document's own container is
        // converted.
        if !has_document_signature(path) {
            self.remember(name, Failure::Failed);
            return Err(Failure::Failed);
        }
        loop {
            // The conversion has a cancellation of its own, which the
            // request's cancels and which the Preview pane's request may
            // cancel too (see [`Lane::wait`]): a thumbnail's conversion
            // stopped for the selection waits for its turn again, rather
            // than leaving its item without a thumbnail until it is shown
            // again.
            let conversion = gio::Cancellable::new();
            let link = {
                let conversion = conversion.clone();
                cancellation.connect_cancelled(move |_| conversion.cancel())
            };
            let outcome = self.convert_in_turn(path, name.clone(), priority, &conversion);
            if let Some(link) = link {
                cancellation.disconnect_cancelled(link);
            }
            let stopped_for_the_selection = outcome == Err(Failure::Cancelled)
                && conversion.is_cancelled()
                && !cancellation.is_cancelled();
            if !stopped_for_the_selection {
                return outcome;
            }
        }
    }

    /// Converts the document at `path`, whose cache name is `name`, in its
    /// turn for `priority`, and remembers a failure that is the
    /// document's.
    fn convert_in_turn(
        &self,
        path: &Path,
        name: String,
        priority: Priority,
        cancellation: &gio::Cancellable,
    ) -> Result<PathBuf, Failure> {
        let _turn = self.lane.wait(priority, &name, cancellation)?;
        // Another request may have converted it while this one waited.
        if let Some(pdf) = self.cache.find(&name) {
            return Ok(pdf);
        }
        match self.convert(path, &name, cancellation) {
            Err(Failure::TimedOut) => {
                let again = !self
                    .timed_out_once
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(name.clone());
                if again {
                    self.remember(name, Failure::TimedOut);
                }
                Err(Failure::TimedOut)
            }
            Err(Failure::Failed) => {
                self.remember(name, Failure::Failed);
                Err(Failure::Failed)
            }
            outcome => outcome,
        }
    }

    /// The pages `first` to `last` of the PDF at `pdf`, one this
    /// converter made, drawn `width` pixels wide by `pdftoppm` in its
    /// sandbox, as [`super::pdf_pages`] draws a PDF's ([`twice`]).
    /// Blocks.
    pub(crate) fn pages(
        &self,
        pdf: &Path,
        first: u32,
        last: u32,
        width: i32,
        cancellation: &gio::Cancellable,
    ) -> Option<Vec<Pixbuf>> {
        twice(cancellation, || {
            pdf_pages_in(&self.sandbox, pdf, first, last, width, cancellation)
        })
    }

    /// Why the document version named `name` could not be converted
    /// earlier in this run, if it could not.
    fn failure_of(&self, name: &str) -> Option<Failure> {
        self.failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .copied()
    }

    /// Remembers that the document version named `name` could not be
    /// converted, and why, for the rest of this run.
    fn remember(&self, name: String, failure: Failure) {
        self.failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name, failure);
    }

    /// Converts the document at `path` in the sandbox, and keeps its PDF
    /// in the cache as `name`.
    fn convert(&self, path: &Path, name: &str, cancellation: &gio::Cancellable) -> Result<PathBuf, Failure> {
        let workspace = Workspace::new().ok_or(Failure::Cancelled)?;
        let profile = workspace.path().join(PROFILE_FOLDER).join("user");
        let output = workspace.path().join(OUTPUT_FOLDER);
        let made = fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&profile)
            .is_ok()
            && fs::DirBuilder::new().mode(0o700).create(&output).is_ok()
            && fs::write(profile.join("registrymodifications.xcu"), PROFILE).is_ok();
        if !made {
            return Err(Failure::Cancelled);
        }
        let command = self.sandbox.office_command(
            &self.program.soffice,
            &self.program.folders,
            path,
            workspace.path(),
        );
        let started = std::time::Instant::now();
        let status = run_within(&command, self.sandbox.needs_runtime_dir(), cancellation, TIMEOUT);
        if cancellation.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        if status.is_none() {
            // Stopped at its time limit, or never started at all, which
            // is no fault of the document's.
            return Err(if started.elapsed() >= TIMEOUT {
                Failure::TimedOut
            } else {
                Failure::Cancelled
            });
        }
        // LibreOffice says nothing useful through its status: the PDF
        // alone decides.
        let pdf = read_pdf(&output.join(OUTPUT_PDF)).ok_or(Failure::Failed)?;
        self.cache.keep(name, &pdf).ok_or(Failure::Cancelled)
    }
}

/// The first bytes of the documents converted: a ZIP archive (Office's
/// and `OpenDocument`'s files), a Compound File (Office 97 to 2003's, and
/// Office's files protected by a password) and Rich Text, which Word
/// also saves as `.doc`.
const DOCUMENT_SIGNATURES: [&[u8]; 3] = [b"PK\x03\x04", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1", b"{\\rtf"];

/// How many of a file's first bytes [`has_document_signature`] reads.
const SIGNATURE_LENGTH: u64 = 64;

/// Whether the plain file at `path` starts as a document does
/// ([`DOCUMENT_SIGNATURES`]), or as markup: the `.xls` and `.doc` files
/// many sites export are HTML, or Office 2003's XML, which `LibreOffice`
/// reads too.
fn has_document_signature(path: &Path) -> bool {
    let Some(file) = open_plain(path) else {
        return false;
    };
    let mut start = Vec::new();
    if file.take(SIGNATURE_LENGTH).read_to_end(&mut start).is_err() {
        return false;
    }
    let markup = start
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(&start)
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        == Some(&b'<');
    markup
        || DOCUMENT_SIGNATURES
            .iter()
            .any(|signature| start.starts_with(signature))
}

/// The plain file at `path`, opened for reading without waiting on a
/// pipe or a device swapped in for it.
fn open_plain(path: &Path) -> Option<fs::File> {
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits().cast_signed())
        .open(path)
        .ok()?;
    file.metadata().ok()?.is_file().then_some(file)
}

/// The PDF `LibreOffice` wrote at `path`, read with care: a link, a pipe
/// or a device is not followed or read, and a file larger than
/// [`PDF_SIZE_LIMIT`] or one that is no PDF is refused.
fn read_pdf(path: &Path) -> Option<Vec<u8>> {
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags.bits().cast_signed())
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > PDF_SIZE_LIMIT {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(PDF_SIZE_LIMIT).read_to_end(&mut bytes).ok()?;
    bytes.starts_with(b"%PDF-").then_some(bytes)
}

/// A document as it is now: where it really is, its size and when it
/// was changed, which name its PDF in the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    path: PathBuf,
    size: u64,
    modified: (i64, i64),
    inode: (u64, u64),
}

impl Version {
    /// The version of the plain file at `path` within [`SIZE_LIMIT`];
    /// `None` for anything else.
    fn of(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > SIZE_LIMIT {
            return None;
        }
        Some(Self {
            path: fs::canonicalize(path).ok()?,
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            inode: (metadata.dev(), metadata.ino()),
        })
    }

    /// The name of its PDF in the cache: a digest of all of the above,
    /// so a changed or replaced document gets a name of its own.
    fn cache_name(&self) -> String {
        let mut key = Vec::new();
        key.extend_from_slice(CACHE_VERSION.as_bytes());
        key.push(0);
        key.extend_from_slice(self.path.as_os_str().as_encoded_bytes());
        key.push(0);
        key.extend_from_slice(
            format!(
                "{}:{}.{}:{}:{}",
                self.size, self.modified.0, self.modified.1, self.inode.0, self.inode.1
            )
            .as_bytes(),
        );
        let digest = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &key)
            .map(|digest| digest.to_string())
            .unwrap_or_default();
        format!("{digest}.pdf")
    }
}

/// The PDFs made, in a private folder of the user's cache.
#[derive(Debug)]
struct Cache {
    folder: PathBuf,
}

impl Cache {
    /// The cache in `folder`, made when the first PDF is kept.
    fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// The kept PDF named `name`, marked as just used; `None` when there
    /// is none, or something else is there.
    fn find(&self, name: &str) -> Option<PathBuf> {
        let path = self.folder.join(name);
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() > PDF_SIZE_LIMIT {
            return None;
        }
        if let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(path)
    }

    /// Keeps `pdf` as `name`, written whole before it takes the name, and
    /// removes the PDFs used longest ago beyond the cache's limits.
    fn keep(&self, name: &str, pdf: &[u8]) -> Option<PathBuf> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.folder)
            .ok()?;
        let target = self.folder.join(name);
        let temporary = self
            .folder
            .join(format!(".{:08x}{TEMPORARY_SUFFIX}", glib::random_int()));
        let written = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .and_then(|mut file| file.write_all(pdf))
            .and_then(|()| fs::rename(&temporary, &target));
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
            return None;
        }
        self.trim(name);
        Some(target)
    }

    /// Removes the PDFs used longest ago until at most
    /// [`CACHE_COUNT_LIMIT`] of them, and [`CACHE_BYTE_LIMIT`] bytes, are
    /// left; `kept`, just made, stays.
    fn trim(&self, kept: &str) {
        let Ok(listing) = fs::read_dir(&self.folder) else {
            return;
        };
        let entries: Vec<fs::DirEntry> = listing.filter_map(Result::ok).collect();
        // A PDF being written when the app ended was left under its
        // temporary name, which no later run takes up.
        for entry in &entries {
            let is_leftover = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with('.') && name.ends_with(TEMPORARY_SUFFIX));
            let is_old = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age >= LEFTOVER_AGE);
            if is_leftover && is_old {
                let _ = fs::remove_file(entry.path());
            }
        }
        let mut pdfs: Vec<(SystemTime, u64, PathBuf)> = entries
            .iter()
            .filter(|entry| Path::new(&entry.file_name()).extension() == Some(OsStr::new("pdf")))
            .filter_map(|entry| {
                let metadata = entry.metadata().ok()?;
                let used = metadata.modified().ok()?;
                Some((used, metadata.len(), entry.path()))
            })
            .collect();
        pdfs.sort_unstable();
        let mut count = pdfs.len();
        let mut bytes: u64 = pdfs.iter().map(|(_, size, _)| size).sum();
        for (_, size, path) in pdfs {
            if count <= CACHE_COUNT_LIMIT && bytes <= CACHE_BYTE_LIMIT {
                break;
            }
            if path.file_name() == Some(OsStr::new(kept)) {
                continue;
            }
            if fs::remove_file(&path).is_ok() {
                count -= 1;
                bytes = bytes.saturating_sub(size);
            }
        }
    }
}

/// The turns of the conversions: one runs at a time, the Preview pane's
/// before the thumbnails, in the order they came, and at most
/// [`QUEUE_LIMIT`] thumbnails wait.
#[derive(Debug, Default)]
struct Lane {
    state: Mutex<LaneState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct LaneState {
    /// The conversion that runs, if one does: whom for, the cache name of
    /// the document it converts, and what stops it.
    running: Option<(Priority, String, gio::Cancellable)>,
    /// The next request's number.
    next: u64,
    /// The requests waiting, by priority and number.
    waiting: Vec<(Priority, u64)>,
}

/// A conversion's turn; the next one's starts when it is dropped.
#[derive(Debug)]
struct Turn<'lane>(&'lane Lane);

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.0.lock().running = None;
        self.0.changed.notify_all();
    }
}

impl Lane {
    fn lock(&self) -> std::sync::MutexGuard<'_, LaneState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits for the turn of a request of `priority` for the document whose
    /// cache name is `name`, or
    /// [`Failure::Cancelled`] when it is cancelled first or too many
    /// thumbnails wait already. The Preview pane's request stops a
    /// thumbnail's conversion of another document that runs, through its
    /// `cancellation`, so a slow document scrolled past never keeps the
    /// selection waiting; that thumbnail is asked again the next time it
    /// is shown. A conversion of the selected document itself goes on, as
    /// its PDF is what the Preview pane waits for. Blocks.
    fn wait(
        &self,
        priority: Priority,
        name: &str,
        cancellation: &gio::Cancellable,
    ) -> Result<Turn<'_>, Failure> {
        let mut state = self.lock();
        let thumbnails = state
            .waiting
            .iter()
            .filter(|(waiting, _)| *waiting == Priority::Thumbnail)
            .count();
        if priority == Priority::Thumbnail && thumbnails >= QUEUE_LIMIT {
            return Err(Failure::Cancelled);
        }
        let ticket = (priority, state.next);
        state.next += 1;
        state.waiting.push(ticket);
        loop {
            if cancellation.is_cancelled() {
                state.waiting.retain(|waiting| *waiting != ticket);
                drop(state);
                self.changed.notify_all();
                return Err(Failure::Cancelled);
            }
            if priority == Priority::Preview {
                if let Some((Priority::Thumbnail, converting, running)) = &state.running {
                    if converting != name {
                        running.cancel();
                    }
                }
            }
            let first = state.waiting.iter().min().copied();
            if state.running.is_none() && first == Some(ticket) {
                state.waiting.retain(|waiting| *waiting != ticket);
                state.running = Some((priority, name.to_owned(), cancellation.clone()));
                return Ok(Turn(self));
            }
            state = self
                .changed
                .wait_timeout(state, WAIT_POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Sandbox {
    /// The command that runs `LibreOffice`'s `soffice` (with the `folders`
    /// it needs, read-only) on the document at `item`, in the sandbox
    /// whose own folder is `workspace` ([`INSIDE`] in it), writing its
    /// PDF to the sandbox's [`OUTPUT_FOLDER`] with its profile in
    /// [`PROFILE_FOLDER`].
    fn office_command(
        &self,
        soffice: &Path,
        folders: &[PathBuf],
        item: &Path,
        workspace: &Path,
    ) -> Vec<OsString> {
        let inside = Path::new(INSIDE);
        let inside_item = inside.join(format!("{INPUT}{}", extension_of(item)));
        let mut program = vec![soffice.as_os_str().to_owned()];
        program.extend(Program::conversion(
            &inside.join(PROFILE_FOLDER),
            &inside.join(OUTPUT_FOLDER),
            &inside_item,
        ));
        match self {
            Self::Bubblewrap {
                program: bwrap,
                proc,
                limits,
            } => {
                let mut shown = program_folders(soffice, &UserFolders::of_session());
                shown.extend(folders.iter().cloned());
                let mut command = limits.words();
                command.push(bwrap.as_os_str().to_owned());
                command.extend(sandbox_arguments(workspace, item, &inside_item, &shown, *proc));
                command.extend(["--setenv", "MAX_CONCURRENCY", THREADS].map(OsString::from));
                command.push("--".into());
                command.extend(program);
                command
            }
            #[cfg(test)]
            Self::Off => unreachable!("documents are converted in a sandbox only"),
        }
    }
}

/// The thumbnail of the document `job` names: the first page of its PDF
/// ([`Converter::pdf`]) drawn to fit the thumbnail's size, written into
/// the shared cache as the Thumbnail Managing Standard asks. Blocks.
pub(in crate::thumbnails) fn write_thumbnail(
    job: &Job,
    cancellation: &gio::Cancellable,
) -> Result<PathBuf, Failure> {
    let converter = converter().ok_or(Failure::Unavailable)?;
    let path = gio::File::for_uri(&job.uri).path().ok_or(Failure::Refused)?;
    let pdf = converter.pdf(&path, Priority::Thumbnail, cancellation)?;
    let pixels = job.flavor.pixels();
    let picture = twice(cancellation, || {
        first_page(&converter.sandbox, &pdf, pixels, cancellation)
    });
    if cancellation.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let picture = framed(&picture.ok_or(Failure::Failed)?).ok_or(Failure::Failed)?;
    let source = Source {
        uri: job.uri.clone(),
        modified: job.modified,
    };
    // A cache that refused the picture is the app's trouble, not the
    // document's: it is asked again the next time it is shown.
    writer::save_thumbnail(&job.cache_dir, &source, job.flavor, &picture).map_err(|_| Failure::Cancelled)
}

/// The color of the line around a document's thumbnail: a light gray
/// that shows a white page on a white background, as Explorer frames
/// one.
const FRAME: u32 = 0xc8c8_c8ff;

/// `page` with a line of [`FRAME`] one pixel wide around it, so a page
/// with little on it still shows as a page and not as an empty tile.
fn framed(page: &Pixbuf) -> Option<Pixbuf> {
    let framed = page.copy()?;
    let (width, height) = (framed.width(), framed.height());
    if width < 3 || height < 3 {
        return Some(framed);
    }
    for (x, y, line_width, line_height) in [
        (0, 0, width, 1),
        (0, height - 1, width, 1),
        (0, 0, 1, height),
        (width - 1, 0, 1, height),
    ] {
        // A part of the picture shares its pixels.
        framed.new_subpixbuf(x, y, line_width, line_height).fill(FRAME);
    }
    Some(framed)
}

/// What `draw` drew, asked a second time when the first gave nothing and
/// the request still stands. The PDF is one this converter made, which
/// `pdftoppm` has drawn already when it was kept, so nothing is lost on a
/// damaged one; but the app's picture decoder, which reads the pages
/// back, can end without an answer on the first picture after the app
/// starts, and the document's first view would say "No preview
/// available." for it.
fn twice<T>(cancellation: &gio::Cancellable, draw: impl Fn() -> Option<T>) -> Option<T> {
    draw().or_else(|| (!cancellation.is_cancelled()).then(draw).flatten())
}

/// The first page of the PDF at `pdf`, drawn by `pdftoppm` in `sandbox`
/// with its longer side `pixels` long.
fn first_page(sandbox: &Sandbox, pdf: &Path, pixels: i32, cancellation: &gio::Cancellable) -> Option<Pixbuf> {
    let workspace = Workspace::new()?;
    let output = format!("{INSIDE}/first-page");
    let pixels_word = pixels.to_string();
    let words = [
        PDF_PROGRAM,
        "-png",
        "-f",
        "1",
        "-l",
        "1",
        "-singlefile",
        "-scale-to",
        &pixels_word,
        "%i",
        &output,
    ];
    let program = Thumbnailer {
        command: words.iter().map(|word| (*word).to_owned()).collect(),
        types: Vec::new(),
    };
    let command = sandbox.command(&program, pdf, workspace.path(), 0);
    run_within(&command, sandbox.needs_runtime_dir(), cancellation, TIMEOUT)?;
    read_picture(&workspace.path().join("first-page.png"), pixels)
}

/// The converter of the tests, chosen by [`use_converter`].
#[cfg(test)]
static TEST_CONVERTER: Mutex<Option<Arc<Converter>>> = Mutex::new(None);

/// Makes [`converter`] answer `converter` until it is chosen again:
/// `None` for a computer without `LibreOffice`, [`test_office`] for the
/// `LibreOffice` of the computer the tests run on.
#[cfg(test)]
pub(crate) fn use_converter(converter: Option<Arc<Converter>>) {
    *TEST_CONVERTER.lock().unwrap_or_else(PoisonError::into_inner) = converter;
}

/// The `LibreOffice` of the computer the tests run on, in its sandbox,
/// keeping its PDFs in `cache`; `None` (with a note) where it, a
/// working bubblewrap or `pdftoppm` is missing.
#[cfg(test)]
pub(crate) fn test_office(cache: &Path) -> Option<Arc<Converter>> {
    let Some(program) = Program::find() else {
        eprintln!("skipped: LibreOffice is not installed");
        return None;
    };
    if !installed_program(PDF_PROGRAM) {
        eprintln!("skipped: pdftoppm is not installed");
        return None;
    }
    let Some(sandbox) = Sandbox::find() else {
        eprintln!("skipped: bubblewrap cannot make its sandbox here");
        return None;
    };
    Some(Arc::new(Converter::new(
        program,
        sandbox,
        Cache::new(cache.to_path_buf()),
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    use super::*;
    use crate::test_support::office_documents::{self, Kind};

    /// The private profile `LibreOffice` starts with turns every macro off
    /// and keeps links from updating.
    ///
    /// parity: VIEW-070
    #[test]
    fn the_profile_turns_macros_and_links_off() {
        let item = |path: &str, name: &str, value: &str| {
            format!(
                "<item oor:path=\"{path}\"><prop oor:name=\"{name}\" oor:op=\"fuse\"><value>{value}</value></prop></item>"
            )
        };
        let scripting = "/org.openoffice.Office.Common/Security/Scripting";
        for wanted in [
            item(scripting, "DisableMacrosExecution", "true"),
            item(scripting, "MacroSecurityLevel", "3"),
            item(scripting, "BlockUntrustedRefererLinks", "true"),
            item("/org.openoffice.Office.Writer/Content/Update", "Link", "0"),
            item("/org.openoffice.Office.Calc/Content/Update", "Link", "1"),
        ] {
            assert!(PROFILE.contains(&wanted), "{wanted}");
        }
    }

    /// `LibreOffice` is started without a window, restore or lock check,
    /// with the sandbox's private profile, and writes its PDF in the
    /// sandbox's own folder; bubblewrap shows it the document alone,
    /// read-only, under a name of its own, and none of the user's
    /// folders.
    ///
    /// parity: VIEW-070
    #[test]
    fn libreoffice_runs_in_the_sandbox_with_a_private_profile() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let item = folder.path().join("Quarterly plan.docx");
        fs::write(&item, b"a document").expect("the document");
        let sandbox = Sandbox::Bubblewrap {
            program: PathBuf::from("/usr/bin/bwrap"),
            proc: true,
            limits: super::super::Limits::Unlimited,
        };
        let soffice = Path::new("/usr/lib/libreoffice/program/soffice");
        let extra = PathBuf::from("/etc/libreoffice");
        let command =
            sandbox.office_command(soffice, std::slice::from_ref(&extra), &item, Path::new("/tmp/ws"));
        let words: Vec<&str> = command.iter().map(|word| word.to_str().expect("UTF-8")).collect();
        let joined = words.join(" ");
        assert_eq!(words[0], "/usr/bin/bwrap");
        assert!(joined.contains("--unshare-all"), "{joined}");
        assert!(joined.contains("--die-with-parent"), "{joined}");
        assert!(joined.contains("--bind /tmp/ws /tmp"), "{joined}");
        assert!(
            joined.contains(&format!(
                "--ro-bind {} /tmp/openxplorer-item.docx",
                item.display()
            )),
            "{joined}"
        );
        assert!(
            joined.contains("--ro-bind /etc/libreoffice /etc/libreoffice"),
            "{joined}"
        );
        assert!(joined.contains("--setenv MAX_CONCURRENCY 4"), "{joined}");
        let program = words
            .iter()
            .position(|word| *word == "--")
            .map(|at| &words[at + 1..])
            .expect("the program after the sandbox's words");
        assert_eq!(
            program,
            [
                "/usr/lib/libreoffice/program/soffice",
                "--headless",
                "--invisible",
                "--norestore",
                "--nolockcheck",
                "--nodefault",
                "--nologo",
                "--nofirststartwizard",
                "-env:UserInstallation=file:///tmp/profile",
                "--convert-to",
                "pdf",
                "--outdir",
                "/tmp/out",
                "/tmp/openxplorer-item.docx",
            ]
        );
        let home = glib::home_dir();
        assert!(
            !words
                .iter()
                .any(|word| Path::new(word) == home || Path::new(word) == folder.path()),
            "{joined}"
        );
    }

    /// `LibreOffice`'s install folder is shown when it is outside `/usr`,
    /// with what its links lead to outside it; a link into the user's
    /// own folders is not followed.
    ///
    /// parity: VIEW-070
    #[test]
    fn the_install_folder_and_its_links_out_are_shown() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let base = folder.path().join("opt");
        let root = base.join("libreoffice9.9");
        let settings = base.join("etc-libreoffice");
        let home = base.join("home");
        fs::create_dir_all(root.join("program")).expect("the program folder");
        fs::create_dir_all(root.join("share")).expect("the share folder");
        fs::create_dir_all(&settings).expect("its settings");
        fs::create_dir_all(home.join("Documents")).expect("a home");
        std::os::unix::fs::symlink(&settings, root.join("share/registry")).expect("a link out");
        std::os::unix::fs::symlink(home.join("Documents"), root.join("share/docs")).expect("a link home");
        let user = UserFolders::of(&home, &[]);

        let shown = install_folders(&root, &user);

        let settings = fs::canonicalize(&settings).expect("real");
        let root = fs::canonicalize(&root).unwrap_or(root);
        assert!(
            shown
                .iter()
                .any(|shown| *shown == root || shown.ends_with("libreoffice9.9")),
            "{shown:?}"
        );
        assert!(shown.contains(&settings), "{shown:?}");
        assert!(
            !shown.iter().any(|shown| shown.ends_with("Documents")),
            "{shown:?}"
        );
    }

    /// One conversion runs at a time; the Preview pane's request goes
    /// before thumbnails that came earlier; a cancelled request leaves the
    /// queue; too many thumbnails waiting are turned away.
    ///
    /// parity: VIEW-070
    #[test]
    fn conversions_take_turns_and_the_preview_goes_first() {
        let lane = Arc::new(Lane::default());
        let first = lane
            .wait(Priority::Thumbnail, "thumbnail.pdf", &gio::Cancellable::new())
            .expect("the first turn");
        let (done, order) = mpsc::channel();
        let mut waiting = Vec::new();
        for (name, priority) in [("thumbnail", Priority::Thumbnail), ("preview", Priority::Preview)] {
            let (lane, done) = (Arc::clone(&lane), done.clone());
            waiting.push(thread::spawn(move || {
                let turn = lane
                    .wait(priority, name, &gio::Cancellable::new())
                    .expect("a turn");
                done.send(name).expect("the test waits");
                thread::sleep(Duration::from_millis(20));
                drop(turn);
            }));
            thread::sleep(Duration::from_millis(100));
        }
        let cancelled = gio::Cancellable::new();
        let leaving = {
            let (lane, cancelled) = (Arc::clone(&lane), cancelled.clone());
            thread::spawn(move || {
                lane.wait(Priority::Thumbnail, "thumbnail.pdf", &cancelled)
                    .map(|_| ())
            })
        };
        thread::sleep(Duration::from_millis(100));
        assert!(order.try_recv().is_err(), "nothing runs beside the first");
        cancelled.cancel();
        assert_eq!(leaving.join().expect("joined"), Err(Failure::Cancelled));
        drop(first);
        let seen: Vec<&str> = (0..2)
            .map(|_| order.recv_timeout(Duration::from_secs(5)).expect("a turn"))
            .collect();
        assert_eq!(seen, ["preview", "thumbnail"]);
        for thread in waiting {
            thread.join().expect("joined");
        }

        let busy = lane
            .wait(Priority::Preview, "preview.pdf", &gio::Cancellable::new())
            .expect("a turn");
        let stop = gio::Cancellable::new();
        let queued: Vec<_> = (0..QUEUE_LIMIT)
            .map(|_| {
                let (lane, stop) = (Arc::clone(&lane), stop.clone());
                thread::spawn(move || lane.wait(Priority::Thumbnail, "thumbnail.pdf", &stop).map(|_| ()))
            })
            .collect();
        thread::sleep(Duration::from_millis(200));
        assert_eq!(
            lane.wait(Priority::Thumbnail, "thumbnail.pdf", &gio::Cancellable::new())
                .map(|_| ()),
            Err(Failure::Cancelled),
            "a thumbnail beyond the queue's limit is asked again later"
        );
        stop.cancel();
        for thread in queued {
            assert_eq!(thread.join().expect("joined"), Err(Failure::Cancelled));
        }
        drop(busy);
    }

    /// The cache keeps a PDF whole under its name, readable by its owner
    /// only, and drops the ones used longest ago beyond its limit.
    ///
    /// parity: VIEW-070
    #[test]
    fn the_cache_keeps_the_newest_pdfs() {
        use std::os::unix::fs::PermissionsExt;

        let folder = tempfile::tempdir().expect("a temporary folder");
        let cache = Cache::new(folder.path().join("previews"));
        let kept = cache.keep("a.pdf", b"%PDF-1.7 a").expect("kept");
        assert_eq!(fs::read(&kept).expect("read"), b"%PDF-1.7 a");
        assert_eq!(
            fs::metadata(&kept).expect("meta").permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(folder.path().join("previews"))
                .expect("meta")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(cache.find("a.pdf"), Some(kept));
        assert_eq!(cache.find("b.pdf"), None);

        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        for index in 0..CACHE_COUNT_LIMIT + 2 {
            let path = cache.folder.join(format!("{index:04}.pdf"));
            fs::write(&path, b"%PDF-").expect("a PDF");
            fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(old + Duration::from_secs(index as u64)))
                .expect("mtime");
        }
        let leftover = cache.folder.join(".0badf00d.openxplorer.tmp");
        let fresh = cache.folder.join(".0000beef.openxplorer.tmp");
        for (path, time) in [(&leftover, old), (&fresh, SystemTime::now())] {
            fs::write(path, b"%PDF-half").expect("a leftover");
            fs::File::options()
                .write(true)
                .open(path)
                .and_then(|file| file.set_modified(time))
                .expect("mtime");
        }
        cache.keep("new.pdf", b"%PDF-new").expect("kept");
        assert!(!leftover.exists(), "an old leftover went");
        assert!(fresh.exists(), "a PDF still being written stays");
        fs::remove_file(&fresh).expect("removed");
        let left = fs::read_dir(&cache.folder).expect("listing").count();
        assert_eq!(left, CACHE_COUNT_LIMIT);
        assert!(!cache.folder.join("0000.pdf").exists(), "the oldest went first");
        assert!(cache.folder.join("new.pdf").exists());
    }

    /// A document changed after its PDF was made is converted again and
    /// shows its new text, even when it keeps its size and was changed
    /// within the same second: the old PDF is never shown for it.
    ///
    /// parity: VIEW-070
    #[test]
    fn a_changed_document_is_never_shown_from_its_old_pdf() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let Some(converter) = test_office(&folder.path().join("cache")) else {
            return;
        };
        let item = folder.path().join("Minutes.odt");
        office_documents::write(&item, Kind::Odt, "First version AAAA");
        let first = converter
            .pdf(&item, Priority::Preview, &gio::Cancellable::new())
            .expect("converted");
        let size = fs::metadata(&item).expect("meta").len();
        let modified = fs::metadata(&item).expect("meta").modified().expect("mtime");

        office_documents::write(&item, Kind::Odt, "Second version BBB");
        assert_eq!(fs::metadata(&item).expect("meta").len(), size, "the same size");
        let file = fs::File::options().write(true).open(&item).expect("open");
        file.set_modified(modified + Duration::from_millis(1))
            .expect("mtime");
        let second = converter
            .pdf(&item, Priority::Preview, &gio::Cancellable::new())
            .expect("converted again");

        assert_ne!(first, second);
        if let Some(text) = office_documents::pdf_text(&second) {
            assert!(text.contains("Second version BBB"), "{text}");
            assert!(!text.contains("First version"), "{text}");
        }
    }

    /// A thumbnail's conversion that the Preview pane stops for another
    /// document is done again once the selected document's is: the item
    /// still gets its thumbnail.
    ///
    /// parity: VIEW-070, VIEW-057
    #[test]
    fn a_thumbnail_stopped_for_the_selection_is_converted_after_it() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let Some(converter) = test_office(&folder.path().join("cache")) else {
            return;
        };
        let thumbnail = folder.path().join("Long report.odt");
        office_documents::write_long(&thumbnail, Kind::Odt, 400);
        let selected = folder.path().join("Agenda.docx");
        office_documents::write(&selected, Kind::Docx, "Agenda");
        let request = gio::Cancellable::new();
        let made = {
            let (converter, thumbnail, request) =
                (Arc::clone(&converter), thumbnail.clone(), request.clone());
            thread::spawn(move || converter.pdf(&thumbnail, Priority::Thumbnail, &request))
        };
        // Once the thumbnail's conversion runs, the selection asks for its
        // own.
        let start = Instant::now();
        while converter.lane.lock().running.is_none() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the thumbnail's conversion started"
            );
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(100));
        let preview = converter.pdf(&selected, Priority::Preview, &gio::Cancellable::new());

        assert!(preview.is_ok(), "{preview:?}");
        let made = made.join().expect("joined");
        assert!(
            made.is_ok(),
            "the thumbnail's document was converted after all: {made:?}"
        );
        assert!(!request.is_cancelled());
    }

    /// A document's macros never run: the test document's macro, which
    /// would replace its text when it is opened, leaves it as it is. A
    /// damaged document, one that needs a password and a file that only
    /// looks like a document convert to nothing, quickly, and are not
    /// tried again; nothing is written beside any of them.
    ///
    /// parity: VIEW-070
    #[test]
    fn macros_never_run_and_bad_documents_fail_quietly() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let Some(converter) = test_office(&folder.path().join("cache")) else {
            return;
        };
        let documents = folder.path().join("Documents");
        fs::create_dir(&documents).expect("a folder");
        let macro_document = documents.join("Form with a macro.odt");
        office_documents::write_with_macro(&macro_document, "Text the macro would replace");
        let pdf = converter
            .pdf(&macro_document, Priority::Preview, &gio::Cancellable::new())
            .expect("the document converts");
        if let Some(text) = office_documents::pdf_text(&pdf) {
            assert!(text.contains("Text the macro would replace"), "{text}");
            assert!(!text.contains(office_documents::MACRO_MARK), "{text}");
        }

        for (name, write) in [
            ("Damaged.docx", office_documents::write_damaged as fn(&Path)),
            ("Locked.odt", office_documents::write_password_protected),
            ("Not a document.xlsx", office_documents::write_noise),
        ] {
            let item = documents.join(name);
            write(&item);
            let before = Instant::now();
            let outcome = converter.pdf(&item, Priority::Preview, &gio::Cancellable::new());
            eprintln!("{name}: {outcome:?} in {:?}", before.elapsed());
            assert_eq!(outcome, Err(Failure::Failed), "{name}");
            assert!(
                before.elapsed() < Duration::from_secs(5),
                "{name}: {:?}",
                before.elapsed()
            );
            let again = Instant::now();
            assert_eq!(
                converter.pdf(&item, Priority::Preview, &gio::Cancellable::new()),
                outcome
            );
            assert!(again.elapsed() < Duration::from_millis(100), "not tried again");
        }
        let left: Vec<_> = fs::read_dir(&documents)
            .expect("listing")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(left.len(), 4, "nothing was written beside them: {left:?}");
    }

    /// A cancelled conversion stops `LibreOffice` at once and is not
    /// remembered as failed.
    ///
    /// parity: VIEW-070
    #[test]
    fn a_cancelled_conversion_stops_at_once() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let Some(converter) = test_office(&folder.path().join("cache")) else {
            return;
        };
        let item = folder.path().join("Long report.odt");
        office_documents::write_long(&item, Kind::Odt, 400);
        let cancellation = gio::Cancellable::new();
        let stopper = {
            let cancellation = cancellation.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(300));
                cancellation.cancel();
            })
        };
        let before = Instant::now();
        let outcome = converter.pdf(&item, Priority::Preview, &cancellation);
        stopper.join().expect("joined");
        if outcome.is_ok() {
            eprintln!(
                "converted before it could be cancelled, in {:?}",
                before.elapsed()
            );
            return;
        }
        assert_eq!(outcome, Err(Failure::Cancelled));
        assert!(
            before.elapsed() < Duration::from_secs(2),
            "{:?}",
            before.elapsed()
        );
        assert_eq!(
            converter.failure_of(&Version::of(&item).expect("version").cache_name()),
            None
        );
    }

    /// A document that takes longer than the time limit (a workbook of a
    /// few kilobytes whose formulas take half a minute) is stopped there,
    /// with every process `LibreOffice` started, tried once more, and then
    /// not again;
    /// the next document converts at once after it.
    ///
    /// parity: VIEW-070
    #[test]
    fn a_slow_document_is_stopped_at_the_time_limit() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let Some(converter) = test_office(&folder.path().join("cache")) else {
            return;
        };
        let slow = folder.path().join("Forecast.xlsx");
        office_documents::write_slow_spreadsheet(&slow, 600);
        let quick = folder.path().join("Notes.odt");
        office_documents::write(&quick, Kind::Odt, "Notes");

        for attempt in ["first", "second"] {
            let before = Instant::now();
            let outcome = converter.pdf(&slow, Priority::Preview, &gio::Cancellable::new());
            let took = before.elapsed();
            eprintln!("the slow workbook, {attempt} time: {outcome:?} after {took:?}");
            assert_eq!(outcome, Err(Failure::TimedOut));
            assert!(
                took >= TIMEOUT && took < TIMEOUT + Duration::from_secs(3),
                "{took:?}"
            );
        }
        let again = Instant::now();
        assert_eq!(
            converter.pdf(&slow, Priority::Preview, &gio::Cancellable::new()),
            Err(Failure::TimedOut)
        );
        assert!(
            again.elapsed() < Duration::from_millis(100),
            "not tried a third time"
        );
        assert!(converter
            .pdf(&quick, Priority::Preview, &gio::Cancellable::new())
            .is_ok());
        assert_eq!(fs::read_dir(folder.path()).expect("listing").count(), 3);
    }

    /// A decompression bomb, a Word document of two megabytes whose text
    /// unpacks to 256 MiB, four times the sandbox's file-size limit, is stopped within
    /// the time limit, and nothing is left running or written beside it.
    ///
    /// parity: VIEW-070
    #[test]
    fn a_decompression_bomb_is_stopped_by_the_limits() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let Some(converter) = test_office(&folder.path().join("cache")) else {
            return;
        };
        let bomb = folder.path().join("Bomb.docx");
        office_documents::write_bomb(&bomb, 256);
        assert!(fs::metadata(&bomb).expect("written").len() < 4 * 1024 * 1024);

        let before = Instant::now();
        let outcome = converter.pdf(&bomb, Priority::Preview, &gio::Cancellable::new());
        let took = before.elapsed();

        eprintln!("the bomb: {outcome:?} after {took:?}");
        assert!(
            matches!(outcome, Err(Failure::Failed | Failure::TimedOut)),
            "{outcome:?}"
        );
        assert!(took < TIMEOUT + Duration::from_secs(3), "{took:?}");
        assert_eq!(fs::read_dir(folder.path()).expect("listing").count(), 1);
    }

    /// Only a file that starts as a document is converted: a ZIP, a
    /// Compound File, Rich Text or markup (an `.xls` that is an HTML
    /// table); noise and plain text under a document's name are not.
    ///
    /// parity: VIEW-070
    #[test]
    fn only_a_documents_container_is_converted() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        for (bytes, wanted) in [
            (&b"PK\x03\x04rest"[..], true),
            (b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1rest", true),
            (b"{\\rtf1\\ansi", true),
            (b"\xef\xbb\xbf\r\n  <html><table>", true),
            (b"<?xml version=\"1.0\"?><Workbook>", true),
            (b"Name,Amount\nRent,700\n", false),
            (b"\x00\x01\x02\x03", false),
            (b"", false),
        ] {
            let item = folder.path().join("Export.xls");
            fs::write(&item, bytes).expect("the file");
            assert_eq!(has_document_signature(&item), wanted, "{bytes:?}");
        }
    }

    /// A document larger than the size limit, a folder and a pipe are
    /// refused without starting `LibreOffice`.
    ///
    /// parity: VIEW-070
    #[test]
    fn huge_documents_folders_and_pipes_are_refused() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let sandbox = Sandbox::Bubblewrap {
            program: PathBuf::from("/nonexistent/bwrap"),
            proc: true,
            limits: super::super::Limits::Unlimited,
        };
        let converter = Converter::new(
            Program {
                soffice: PathBuf::from("/nonexistent/program/soffice"),
                folders: Vec::new(),
            },
            sandbox,
            Cache::new(folder.path().join("cache")),
        );
        let huge = folder.path().join("Huge.docx");
        fs::File::create(&huge)
            .and_then(|file| file.set_len(SIZE_LIMIT + 1))
            .expect("a sparse file");
        let pipe = folder.path().join("Pipe.docx");
        let made = std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .expect("mkfifo");
        assert!(made.success());
        for item in [&huge, &pipe, &folder.path().to_path_buf()] {
            assert_eq!(
                converter.pdf(item, Priority::Preview, &gio::Cancellable::new()),
                Err(Failure::Refused),
                "{}",
                item.display()
            );
        }
    }
}
