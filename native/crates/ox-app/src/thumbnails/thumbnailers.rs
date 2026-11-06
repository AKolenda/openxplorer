// SPDX-License-Identifier: AGPL-3.0-only
//! The thumbnailers the desktop installs, `thumbnailers/*.thumbnailer` in
//! the user's data folder and the system ones (GNOME's convention, which
//! Files reads): one program each, with the MIME types it makes pictures
//! of. They are the only way to a preview of types neither the
//! thumbnailer service nor gdk-pixbuf knows, such as Unity asset bundles,
//! spreadsheets, and videos on a session without tumbler.
//!
//! [`super::make_thumbnail`] asks them after the service: a forked program
//! costs more than gdk-pixbuf decoding a picture in the app, so a picture
//! the app reads is decoded there, and only one it cannot read (a type
//! without a gdk-pixbuf loader) or one of another kind comes here. The
//! user's `org.gnome.desktop.thumbnailers` settings are honoured, as Files
//! honours them: `disable` names MIME types no program is started for, and
//! `disable-all` turns every one of them off. An entry whose `TryExec`
//! program this computer lacks is skipped too, failing on every item
//! otherwise.
//!
//! The program reads a file the user may have downloaded from anywhere,
//! with a parser of its own, so it runs in a bubblewrap sandbox, as Files
//! runs it ([`Sandbox`]): no network and no session bus, the system's
//! programs and libraries read-only, the item alone of the user's files,
//! and a private folder for its picture. A computer without bubblewrap,
//! or where it cannot make its namespaces, starts none of these programs
//! at all; the service and the app's own decoding still preview what
//! they know.
//!
//! A thumbnailer writes a plain PNG, without the keys that keep a
//! thumbnail valid; the app reads it with care (it was made from an
//! untrusted file) and writes it into the shared cache itself, as Files
//! does, through [`super::writer`].

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Read;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::ThumbnailFlavor;
use rustix::process::{kill_process_group, Pid, Signal};

use super::writer::{self, Source};

/// The group every thumbnailer file keeps its keys in.
const GROUP: &str = "Thumbnailer Entry";

/// The folder of the `.thumbnailer` files in each data folder.
const FOLDER: &str = "thumbnailers";

/// The largest entry read; a real one names a command and a page of types.
const ENTRY_SIZE_LIMIT: u64 = 64 * 1024;

/// How long a thumbnailer may run before it is killed: a video needs a
/// while, an unresponsive program must not hold a slot for ever.
const TIMEOUT: Duration = Duration::from_secs(30);

/// How long a running thumbnailer is left alone before its request's
/// cancellation and the timeout are looked at again.
const POLL: Duration = Duration::from_millis(50);

/// The name of the private folder a thumbnailer writes its picture in,
/// with `XXXXXX` for the random part.
const WORKSPACE: &str = "openxplorer-thumbnailer-";

/// Where the sandbox shows the private folder to the program.
const INSIDE: &str = "/tmp";

/// The name the item has inside the sandbox, before its extension.
const INPUT: &str = "openxplorer-item";

/// The name of the picture the program writes.
const OUTPUT: &str = "openxplorer-thumbnail.png";

/// The largest picture read back from a thumbnailer; a thumbnail is a few
/// hundred pixels square.
const OUTPUT_SIZE_LIMIT: u64 = 32 * 1024 * 1024;

/// The longest side of a thumbnailer's picture the app decodes.
const OUTPUT_SIDE_LIMIT: u32 = 16_384;

/// The most pixels of a thumbnailer's picture the app decodes: a frame of
/// an 8K video, which a program that ignores `%s` may hand back. A small
/// PNG may declare far more, and gdk-pixbuf would allocate them all.
const OUTPUT_PIXEL_LIMIT: u64 = 7680 * 4320;

/// The start of every PNG file.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The chunks of a thumbnailer's PNG the app keeps: those that draw the
/// picture. The others, text and colour profiles among them, may hold
/// compressed data that libpng would expand without the pixel limit.
const DRAWN_CHUNKS: [&[u8; 4]; 5] = [b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"];

/// The `PATH` a thumbnailer runs with.
const PROGRAM_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// The system folders every program needs, shown read-only in the
/// sandbox: each is a folder of its own, or a link into `/usr`.
const SYSTEM_FOLDERS: [&str; 5] = ["/bin", "/sbin", "/lib", "/lib32", "/lib64"];

/// The system files and folders a program may need besides `/usr`, shown
/// read-only when this computer has them: the linker's cache, Debian's
/// alternatives and the fonts.
const SYSTEM_EXTRAS: [&str; 4] = [
    "/etc/ld.so.cache",
    "/etc/alternatives",
    "/etc/fonts",
    "/var/cache/fontconfig",
];

/// GNOME's settings for the desktop's thumbnailers, which Files honours:
/// the MIME types no external thumbnailer is started for, and the switch
/// that turns every one of them off.
const DISABLE_SCHEMA: &str = "org.gnome.desktop.thumbnailers";
const DISABLE_KEY: &str = "disable";
const DISABLE_ALL_KEY: &str = "disable-all";

/// The desktop's thumbnailers, read once per run by [`installed`], which a
/// worker thread may also read.
static INSTALLED: OnceLock<Arc<Catalogue>> = OnceLock::new();

/// What a thumbnail is made of: the item, its type and time, and where the
/// cache expects the picture.
#[derive(Debug, Clone)]
pub(super) struct Job {
    /// The cache folder, [`glib::user_cache_dir`].
    pub cache_dir: PathBuf,
    /// The item's URI, as the thumbnail names it.
    pub uri: String,
    /// Its content type, which picks the thumbnailer.
    pub content_type: String,
    /// Its modification time in seconds, which keeps the thumbnail valid.
    pub modified: u64,
    /// The size of the thumbnail wanted.
    pub flavor: ThumbnailFlavor,
}

/// What the user turned external thumbnailers off for in
/// `org.gnome.desktop.thumbnailers`, read for each request so that a
/// change takes effect at once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Disabled {
    /// The MIME types `disable` names no program is started for.
    pub types: Vec<String>,
    /// `disable-all` is set: no program is started for any type.
    pub all: bool,
}

impl Disabled {
    /// What this session's settings turn off. A session without the
    /// schema turns none off.
    pub(super) fn of_session() -> Self {
        let Some(schema) =
            gio::SettingsSchemaSource::default().and_then(|source| source.lookup(DISABLE_SCHEMA, true))
        else {
            return Self::default();
        };
        let settings = gio::Settings::new(DISABLE_SCHEMA);
        let types = if schema.has_key(DISABLE_KEY) {
            settings
                .strv(DISABLE_KEY)
                .iter()
                .map(ToString::to_string)
                .collect()
        } else {
            Vec::new()
        };
        let all = schema.has_key(DISABLE_ALL_KEY) && settings.boolean(DISABLE_ALL_KEY);
        Self { types, all }
    }

    /// Whether no program may be started for `content_type`: `disable`
    /// names types, and a type it names takes the kinds of it along, as an
    /// entry naming a parent type does.
    pub(super) fn covers(&self, content_type: &str) -> bool {
        self.all || self.types.iter().any(|declared| covers(content_type, declared))
    }
}

/// The thumbnailers a desktop offers, and the ones its user turned off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Catalogue {
    /// The thumbnailers installed, in the order they are tried.
    thumbnailers: Vec<Thumbnailer>,
    /// What the user turned off.
    disabled: Disabled,
    /// How the programs run; `None` runs none.
    sandbox: Option<Sandbox>,
}

impl Catalogue {
    /// The thumbnailers the data folders `folders` hold, run in the
    /// sandbox `sandbox` finds: one without a working sandbox keeps none,
    /// and the sandbox is looked for only when there is a program to run
    /// in it.
    fn of(folders: &[PathBuf], sandbox: impl FnOnce() -> Option<Sandbox>) -> Self {
        let mut thumbnailers = read(folders);
        let sandbox = if thumbnailers.is_empty() { None } else { sandbox() };
        if sandbox.is_none() {
            thumbnailers.clear();
        }
        Self {
            thumbnailers,
            disabled: Disabled::default(),
            sandbox,
        }
    }

    /// The thumbnailers of a test, which read no folder of the desktop's
    /// and run without the sandbox, so a test's program sees its files.
    #[cfg(test)]
    pub(super) fn for_test(thumbnailers: Vec<Thumbnailer>) -> Self {
        Self {
            thumbnailers,
            disabled: Disabled::default(),
            sandbox: Some(Sandbox::Off),
        }
    }

    /// These thumbnailers with what `disabled` turns off.
    pub(super) fn with_disabled(&self, disabled: Disabled) -> Self {
        Self {
            disabled,
            ..self.clone()
        }
    }

    /// The thumbnailer that makes `content_type`, unless the user turned
    /// external thumbnailers off for the type or for every type.
    fn for_type(&self, content_type: &str) -> Option<&Thumbnailer> {
        if self.disabled.covers(content_type) {
            return None;
        }
        self.thumbnailers
            .iter()
            .find(|thumbnailer| thumbnailer.handles(content_type))
    }
}

/// One `.thumbnailer` file: what it makes thumbnails of and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Thumbnailer {
    /// The `Exec` line, split into words, with `%i`, `%u`, `%o` and `%s`
    /// still to be replaced.
    command: Vec<String>,
    /// The MIME types it makes thumbnails of.
    types: Vec<String>,
}

impl Thumbnailer {
    /// The thumbnailer a test needs, without a `.thumbnailer` file of the
    /// desktop's: `command` as `Exec` would be split, and the types it
    /// takes.
    #[cfg(test)]
    pub(super) fn for_test(command: &[&str], types: &[&str]) -> Self {
        Self {
            command: command.iter().map(|word| (*word).to_owned()).collect(),
            types: types.iter().map(|mime| (*mime).to_owned()).collect(),
        }
    }

    /// The thumbnailer the entry `text` describes, or `None` when it is not
    /// a usable one: a file of another kind, an entry naming no command or
    /// no type, or one whose `TryExec` program this computer lacks. Blocks.
    fn parse(text: &str) -> Option<Self> {
        let entry = glib::KeyFile::new();
        entry.load_from_data(text, glib::KeyFileFlags::NONE).ok()?;
        // An entry whose program is not installed only says what another
        // desktop can make; asking it would fail on every item.
        if let Ok(program) = entry.string(GROUP, "TryExec") {
            if !installed_program(&program) {
                return None;
            }
        }
        let exec = entry.string(GROUP, "Exec").ok()?;
        let command: Vec<String> = glib::shell_parse_argv(exec.as_str())
            .ok()?
            .into_iter()
            .map(|word| word.into_string().ok())
            .collect::<Option<_>>()?;
        if command.is_empty() {
            return None;
        }
        // The list ends with a `;` and real entries pad it with spaces.
        let types: Vec<String> = entry
            .string(GROUP, "MimeType")
            .ok()?
            .split(';')
            .map(str::trim)
            .filter(|mime| !mime.is_empty())
            .map(str::to_owned)
            .collect();
        if types.is_empty() {
            return None;
        }
        Some(Self { command, types })
    }

    /// Whether this thumbnailer makes thumbnails of `content_type`, a type
    /// of its own or a kind of one (`image/x-cbz` of an entry naming
    /// only a parent): GIO's type hierarchy decides, as Files lets it.
    fn handles(&self, content_type: &str) -> bool {
        self.types.iter().any(|declared| covers(content_type, declared))
    }

    /// The command to run for the file at `path` (also named by `uri`),
    /// writing its picture to `output` with `pixels` as its longest edge.
    ///
    /// Both paths are handed over as the bytes the system knows them by: a
    /// file whose name is no UTF-8 still has a thumbnail made of it.
    fn command(&self, path: &Path, uri: &str, output: &Path, pixels: i32) -> Vec<OsString> {
        let pixels = pixels.to_string();
        let values = [
            (b'i', path.as_os_str()),
            (b'u', OsStr::new(uri)),
            (b'o', output.as_os_str()),
            (b's', OsStr::new(pixels.as_str())),
        ];
        self.command.iter().map(|word| expand(word, &values)).collect()
    }
}

/// Whether `declared` names `content_type`, itself or a kind of it.
fn covers(content_type: &str, declared: &str) -> bool {
    declared == content_type
        || gio::content_type_is_mime_type(content_type, declared)
        || gio::content_type_is_a(content_type, declared)
}

/// `word` with each `%i`, `%u`, `%o` and `%s` replaced by the value
/// `values` names, in one pass: a path that itself holds one of them is
/// taken as it is, and a `%` before another letter stays one.
fn expand(word: &str, values: &[(u8, &OsStr)]) -> OsString {
    let mut expanded = Vec::with_capacity(word.len());
    let mut rest = word.as_bytes();
    while let Some(percent) = rest.iter().position(|byte| *byte == b'%') {
        expanded.extend_from_slice(&rest[..percent]);
        let after = &rest[percent + 1..];
        let value = after
            .first()
            .and_then(|letter| values.iter().find(|(replaced, _)| replaced == letter));
        if let Some((_, value)) = value {
            expanded.extend_from_slice(value.as_bytes());
            rest = &after[1..];
        } else {
            expanded.push(b'%');
            rest = after;
        }
    }
    expanded.extend_from_slice(rest);
    OsString::from_vec(expanded)
}

/// The thumbnailers this desktop installs, read the first time a
/// thumbnail has to be made and once per run (`thumbnailers/` holds a
/// handful of small files); what the user turns off is not kept here but
/// read for each request ([`Disabled::of_session`]).
pub(super) fn installed() -> Arc<Catalogue> {
    Arc::clone(INSTALLED.get_or_init(|| Arc::new(session_catalogue())))
}

/// The desktop's thumbnailers, in the sandbox this computer offers.
#[cfg(not(test))]
fn session_catalogue() -> Catalogue {
    let mut folders = vec![glib::user_data_dir()];
    folders.extend(glib::system_data_dirs());
    Catalogue::of(&folders, Sandbox::find)
}

/// None in a test: a window test never starts the programs of the
/// computer it runs on, whose thumbnailers differ from one to the next.
#[cfg(test)]
fn session_catalogue() -> Catalogue {
    Catalogue::for_test(Vec::new())
}

/// The thumbnailers the data folders `folders` hold, the user's folders
/// before the system ones and each folder's files by name, which is the
/// order Files tries them in.
fn read(folders: &[PathBuf]) -> Vec<Thumbnailer> {
    let mut thumbnailers = Vec::new();
    for folder in folders {
        for file in entries(&folder.join(FOLDER)) {
            let Some(text) = read_entry(&file) else {
                continue;
            };
            if let Some(thumbnailer) = Thumbnailer::parse(&text) {
                thumbnailers.push(thumbnailer);
            }
        }
    }
    thumbnailers
}

/// The text of the entry at `path`, or `None` when it is missing, of
/// another kind, or too large to be one.
fn read_entry(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > ENTRY_SIZE_LIMIT {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// The `.thumbnailer` files of `folder` by name; an absent folder holds
/// none.
fn entries(folder: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "thumbnailer")
        })
        .collect();
    files.sort();
    files
}

/// Whether the program an entry names is installed: a path is checked
/// directly, a bare name looked for in `PATH`.
fn installed_program(program: &str) -> bool {
    if program.contains('/') {
        let Ok(metadata) = fs::metadata(program) else {
            return false;
        };
        return metadata.is_file() && metadata.permissions().mode() & 0o111 != 0;
    }
    glib::find_program_in_path(program).is_some()
}

/// Makes the thumbnail of `job` with the thumbnailer of `catalogue` that
/// makes its type, and writes it into the cache as the Thumbnail Managing
/// Standard asks; `None` when none makes the type, the user turned them
/// off for it, it failed, or the request was cancelled. Blocks: run it off
/// the main thread.
pub(super) fn write_thumbnail(
    job: &Job,
    catalogue: &Catalogue,
    cancellation: &gio::Cancellable,
) -> Option<PathBuf> {
    let thumbnailer = catalogue.for_type(&job.content_type)?;
    let sandbox = catalogue.sandbox.as_ref()?;
    // A thumbnailer takes a file of this computer, its `%i` being a path: a
    // file on a share, a phone or in an archive is left to the service.
    let path = gio::File::for_uri(&job.uri).path()?;
    // The program writes its picture in a private folder of its own, never
    // in the cache: the app copies a good picture there itself.
    let workspace = Workspace::new()?;
    let pixels = job.flavor.pixels();
    let command = sandbox.command(thumbnailer, &path, workspace.path(), pixels);
    // One that failed on its own may still have written a good picture, so
    // the picture alone decides; a stopped one left none worth reading.
    run(&command, cancellation)?;
    let picture = read_picture(&workspace.path().join(OUTPUT), pixels)?;
    drop(workspace);
    if cancellation.is_cancelled() {
        return None;
    }
    let source = Source {
        uri: job.uri.clone(),
        modified: job.modified,
    };
    // Nothing follows this program, so why the cache refused the picture is
    // of no use here: no thumbnail was made.
    writer::save_thumbnail(&job.cache_dir, &source, job.flavor, &picture).ok()
}

/// How a desktop thumbnailer is run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Sandbox {
    /// In bubblewrap, as Files runs them.
    Bubblewrap {
        /// bubblewrap itself.
        program: PathBuf,
        /// Whether the sandbox has a `/proc` of its own, which a container
        /// may refuse to mount; without one it is tighter still, and only
        /// a program that needs it fails.
        proc: bool,
    },
    /// Without a sandbox: only a test's own program, which must see the
    /// files of its test.
    #[cfg(test)]
    Off,
}

impl Sandbox {
    /// The bubblewrap of this computer, if it is installed and can make
    /// the sandbox here: it is tried with `true`, since a container or a
    /// strict kernel may refuse the namespaces it needs, first with a
    /// `/proc` of its own, as Files gives it, then without.
    fn find() -> Option<Self> {
        let program = glib::find_program_in_path("bwrap")?;
        [true, false]
            .into_iter()
            .map(|proc| Self::Bubblewrap {
                program: program.clone(),
                proc,
            })
            .find(Self::works)
    }

    /// Whether this sandbox runs `true` here.
    fn works(&self) -> bool {
        let Some(workspace) = Workspace::new() else {
            return false;
        };
        let item = workspace.path().join("probe");
        if fs::write(&item, b"").is_err() {
            return false;
        }
        let probe = Thumbnailer {
            command: vec!["true".to_owned()],
            types: Vec::new(),
        };
        let command = self.command(&probe, &item, workspace.path(), 0);
        run(&command, &gio::Cancellable::new()).is_some_and(|status| status.success())
    }

    /// The command that runs `thumbnailer` for the file at `item`, writing
    /// its picture to [`OUTPUT`] in `workspace` with `pixels` as its
    /// longest edge.
    ///
    /// In the sandbox the program sees `workspace` as [`INSIDE`], and the
    /// item there under a name of the sandbox's own, read-only; it is
    /// given that path and URI.
    fn command(
        &self,
        thumbnailer: &Thumbnailer,
        item: &Path,
        workspace: &Path,
        pixels: i32,
    ) -> Vec<OsString> {
        match self {
            Self::Bubblewrap { program: bwrap, proc } => {
                let inside = Path::new(INSIDE);
                let inside_item = inside.join(format!("{INPUT}{}", extension_of(item)));
                let inside_uri = format!("file://{}", inside_item.display());
                let mut program =
                    thumbnailer.command(&inside_item, &inside_uri, &inside.join(OUTPUT), pixels);
                // The program is named by its full path, since the sandbox's
                // `PATH` holds only the system's folders.
                let resolved = program.first().and_then(|name| program_path(name));
                if let (Some(first), Some(resolved)) = (program.first_mut(), &resolved) {
                    resolved.as_os_str().clone_into(first);
                }
                let mut command = vec![bwrap.as_os_str().to_owned()];
                command.extend(sandbox_arguments(
                    workspace,
                    item,
                    &inside_item,
                    resolved.as_deref(),
                    *proc,
                ));
                command.push("--".into());
                command.extend(program);
                command
            }
            #[cfg(test)]
            Self::Off => {
                let uri = gio::File::for_path(item).uri();
                thumbnailer.command(item, &uri, &workspace.join(OUTPUT), pixels)
            }
        }
    }
}

/// The extension the item keeps inside the sandbox, with its dot, which
/// some thumbnailers go by; one of other characters than letters and
/// digits, or a long one, is left out.
fn extension_of(item: &Path) -> String {
    item.extension()
        .and_then(OsStr::to_str)
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default()
}

/// bubblewrap's arguments for a thumbnailer: the system read-only, the
/// program itself when it lives elsewhere, `workspace` as [`INSIDE`] and
/// the item at `inside_item`, in fresh namespaces (no network, no other
/// process, no session bus) that end with the app; `proc` gives it a
/// `/proc` of its own.
fn sandbox_arguments(
    workspace: &Path,
    item: &Path,
    inside_item: &Path,
    program: Option<&Path>,
    proc: bool,
) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = Vec::new();
    let mut add = |words: &[&OsStr]| arguments.extend(words.iter().map(|word| (*word).to_owned()));
    let word = OsStr::new;
    add(&[word("--ro-bind"), word("/usr"), word("/usr")]);
    for extra in SYSTEM_EXTRAS {
        add(&[word("--ro-bind-try"), word(extra), word(extra)]);
    }
    for folder in SYSTEM_FOLDERS {
        if let Ok(target) = fs::read_link(folder) {
            add(&[word("--symlink"), target.as_os_str(), word(folder)]);
        } else if Path::new(folder).is_dir() {
            add(&[word("--ro-bind"), word(folder), word(folder)]);
        }
    }
    // A program installed outside the system's folders, such as one in
    // the user's own `bin` folder, is shown read-only where it is.
    if let Some(program) = program {
        let in_system = std::iter::once("/usr")
            .chain(SYSTEM_FOLDERS)
            .any(|folder| program.starts_with(folder));
        if !in_system {
            add(&[word("--ro-bind"), program.as_os_str(), program.as_os_str()]);
        }
    }
    if proc {
        add(&[word("--proc"), word("/proc")]);
    }
    add(&[
        word("--dev"),
        word("/dev"),
        word("--chdir"),
        word("/"),
        word("--setenv"),
        word("GIO_USE_VFS"),
        word("local"),
        word("--setenv"),
        word("HOME"),
        word(INSIDE),
        word("--setenv"),
        word("TMPDIR"),
        word(INSIDE),
        word("--unshare-all"),
        word("--die-with-parent"),
        word("--new-session"),
        word("--bind"),
        workspace.as_os_str(),
        word(INSIDE),
        word("--ro-bind"),
        item.as_os_str(),
        inside_item.as_os_str(),
    ]);
    arguments
}

/// Where the program an `Exec` line starts with lives: a path as it is,
/// a bare name looked for in `PATH`.
fn program_path(program: &OsStr) -> Option<PathBuf> {
    let path = Path::new(program);
    if program.as_bytes().contains(&b'/') {
        return path.is_absolute().then(|| path.to_path_buf());
    }
    glib::find_program_in_path(path)
}

/// A private folder for one thumbnailer's picture, readable by its owner
/// only and removed with everything in it when dropped.
struct Workspace(PathBuf);

impl Workspace {
    /// A new folder under the system's temporary folder.
    fn new() -> Option<Self> {
        let parent = glib::tmp_dir();
        for _ in 0..8 {
            let path = parent.join(format!("{WORKSPACE}{:08x}", glib::random_int()));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Some(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return None,
            }
        }
        None
    }

    /// The folder.
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // The program may have taken its folders' permissions away, or
        // nested them deeper than a path may name, to keep what it left:
        // the folder is emptied through the folders' own descriptors, each
        // given back to its owner first, and links are removed, never
        // followed. Its sandbox is gone by now, so nothing changes the
        // folder meanwhile.
        if !remove_tree(&self.0) {
            glib::g_warning!(
                ox_core::LOG_DOMAIN,
                "A thumbnailer's folder could not be removed: {}",
                self.0.display()
            );
        }
    }
}

/// How deep a workspace's folders are emptied; a thumbnailer has no need
/// of more, and a deeper one is left, as a deeper walk could exhaust the
/// stack or the descriptors.
const CLEANUP_DEPTH: usize = 32;

/// Removes the folder `folder` with everything in it, as [`Workspace`]'s
/// drop describes; whether it is gone.
fn remove_tree(folder: &Path) -> bool {
    use rustix::fs::{Mode, OFlags};

    if !fs::symlink_metadata(folder).is_ok_and(|metadata| metadata.is_dir()) {
        return fs::remove_file(folder).is_ok() || !folder.exists();
    }
    let _ = fs::set_permissions(folder, fs::Permissions::from_mode(0o700));
    let flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC;
    if let Ok(descriptor) = rustix::fs::open(folder, flags, Mode::empty()) {
        empty_folder(&descriptor, 0);
    }
    fs::remove_dir(folder).is_ok()
}

/// Removes what it can of everything in the open folder `folder`, `depth`
/// folders below the workspace; the folder's own removal tells whether it
/// all went.
fn empty_folder(folder: &std::os::fd::OwnedFd, depth: usize) {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags};

    let Ok(listing) = rustix::fs::Dir::read_from(folder) else {
        return;
    };
    // The names are taken first, so removing them does not disturb the
    // listing.
    let names: Vec<_> = listing
        .map_while(Result::ok)
        .map(|entry| (entry.file_name().to_owned(), entry.file_type()))
        .filter(|(name, _)| name.as_bytes() != b"." && name.as_bytes() != b"..")
        .collect();
    for (name, kind) in names {
        let kind = if kind == FileType::Unknown {
            rustix::fs::statat(folder, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
                .map_or(FileType::Unknown, |stat| FileType::from_raw_mode(stat.st_mode))
        } else {
            kind
        };
        if kind != FileType::Directory {
            let _ = rustix::fs::unlinkat(folder, name.as_c_str(), AtFlags::empty());
            continue;
        }
        if depth < CLEANUP_DEPTH {
            let _ = rustix::fs::chmodat(folder, name.as_c_str(), Mode::RWXU, AtFlags::empty());
            let flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC;
            if let Ok(inner) = rustix::fs::openat(folder, name.as_c_str(), flags, Mode::empty()) {
                empty_folder(&inner, depth + 1);
            }
        }
        // An inner folder that could not be opened or emptied may be empty
        // all the same: its removal is tried whatever happened above.
        let _ = rustix::fs::unlinkat(folder, name.as_c_str(), AtFlags::REMOVEDIR);
    }
}

/// The picture a thumbnailer wrote at `path`, decoded to fit `edge`
/// pixels square. The program read an untrusted file, so what it left is
/// taken with care: a link, a pipe or a device is not followed or read,
/// and a file larger than [`OUTPUT_SIZE_LIMIT`] is refused.
fn read_picture(path: &Path, edge: i32) -> Option<Pixbuf> {
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags.bits().cast_signed())
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > OUTPUT_SIZE_LIMIT {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(OUTPUT_SIZE_LIMIT).read_to_end(&mut bytes).ok()?;
    let png = drawn_png(&bytes)?;
    writer::scaled_png(&png, edge).filter(|picture| picture.width() > 0 && picture.height() > 0)
}

/// The PNG in `bytes` with only the chunks that draw it ([`DRAWN_CHUNKS`]),
/// or `None` when `bytes` are no well-formed PNG or it declares a picture
/// larger than [`OUTPUT_SIDE_LIMIT`] or [`OUTPUT_PIXEL_LIMIT`]: a
/// thumbnailer writes PNG, and only a PNG's size can be checked before
/// gdk-pixbuf allocates it.
fn drawn_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut rest = bytes.strip_prefix(PNG_SIGNATURE)?;
    let mut png = PNG_SIGNATURE.to_vec();
    let mut first = true;
    loop {
        let length = usize::try_from(u32::from_be_bytes(rest.get(..4)?.try_into().ok()?)).ok()?;
        let kind = rest.get(4..8)?;
        // The length, the kind, the data and the checksum.
        let chunk = rest.get(..length.checked_add(12)?)?;
        if first {
            // The header comes first, and says how large the picture is.
            if kind != b"IHDR" || length != 13 {
                return None;
            }
            let side = |at: usize| chunk.get(at..at + 4)?.try_into().ok().map(u32::from_be_bytes);
            let (width, height) = (side(8)?, side(12)?);
            let fits = (1..=OUTPUT_SIDE_LIMIT).contains(&width)
                && (1..=OUTPUT_SIDE_LIMIT).contains(&height)
                && u64::from(width) * u64::from(height) <= OUTPUT_PIXEL_LIMIT;
            if !fits {
                return None;
            }
            first = false;
        } else if kind == b"IHDR" {
            return None;
        }
        if DRAWN_CHUNKS.iter().any(|drawn| kind == drawn.as_slice()) {
            png.extend_from_slice(chunk);
        }
        if kind == b"IEND" {
            return Some(png);
        }
        rest = &rest[chunk.len()..];
    }
}

/// Runs `command` with nothing of its output shown, and returns how it
/// ended; `None` when it could not start or had to be killed, because the
/// request was cancelled or it outlasted [`TIMEOUT`].
///
/// The wait polls instead of blocking on the child so that a cancelled
/// request stops the program within [`POLL`]: a request scrolled away must
/// not keep a slot for the whole timeout.
fn run(command: &[OsString], cancellation: &gio::Cancellable) -> Option<ExitStatus> {
    let (program, arguments) = command.split_first()?;
    let mut child = std::process::Command::new(program)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // The program reads an untrusted file, so it is handed none of the
        // app's environment, which may hold tokens or the addresses of the
        // display and the session bus: only the system's `PATH` and the
        // user's language.
        .env_clear()
        .env("PATH", PROGRAM_PATH)
        .envs(std::env::vars_os().filter(|(name, _)| {
            name.to_str()
                .is_some_and(|name| name == "LANG" || name == "LANGUAGE" || name.starts_with("LC_"))
        }))
        // The thumbnailer gets a process group of its own, so stopping it
        // stops the programs it started too: a wrapper script's `cp`, or
        // ffmpeg, would otherwise keep writing the picture the app deletes.
        .process_group(0)
        .spawn()
        .ok()?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            stop(&mut child);
            return None;
        }
        thread::sleep(POLL);
    }
}

/// Kills the process group `child` leads, and reaps the child.
///
/// The group is the child's own, [`run`] having put it in one: its number
/// is the child's. A group that is gone already is no failure — the
/// program may have exited between the last look and this kill — so the
/// outcome is not reported. In the sandbox, killing bubblewrap ends every
/// program inside it too (`--die-with-parent` and its own process
/// namespace).
fn stop(child: &mut std::process::Child) {
    let group = i32::try_from(child.id())
        .ok()
        .and_then(Pid::from_raw)
        .is_some_and(|group| kill_process_group(group, Signal::KILL).is_ok());
    if !group {
        // Without a group of its own the child is all the app can end.
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use ox_core::entry::thumbnail_file;

    use super::*;

    /// The type of the Unity asset bundles the app is asked to preview.
    const BUNDLE: &str = "application/x-unity-assetbundle";

    /// A data folder with `entries` written as `(file name, text)`.
    fn data_dir(entries: &[(&str, &str)]) -> (tempfile::TempDir, Vec<PathBuf>) {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let data = folder.path().join("share");
        for (name, text) in entries {
            let path = data.join(FOLDER).join(name);
            fs::create_dir_all(path.parent().expect("a folder")).expect("the folder");
            fs::write(&path, text).expect("the entry");
        }
        (folder, vec![data])
    }

    /// The entry a desktop installs names the types it makes thumbnails of
    /// and the command it runs for them; one whose program is missing, or
    /// which names no command or type, is of no use and is skipped.
    ///
    /// parity: VIEW-057
    #[test]
    fn an_entry_names_its_types_and_its_command() {
        let entry = Thumbnailer::parse(
            "[Thumbnailer Entry]\n\
             TryExec=/bin/sh\n\
             Exec=/bin/sh -c %i %o %s\n\
             MimeType=application/x-unity-assetbundle; video/mp4 ;\n",
        )
        .expect("an entry");
        assert_eq!(entry.types, [BUNDLE, "video/mp4"]);
        assert!(entry.handles(BUNDLE));
        assert!(!entry.handles("image/png"));

        // TryExec is optional; a program this computer lacks makes the
        // entry unusable, as does one it may not run.
        let no_try_exec =
            Thumbnailer::parse("[Thumbnailer Entry]\nExec=/bin/sh %i %o %s\nMimeType=video/mp4;\n")
                .expect("an entry without TryExec");
        assert!(no_try_exec.handles("video/mp4"));
        assert!(Thumbnailer::parse(
            "[Thumbnailer Entry]\nTryExec=/nonexistent/ox-thumbnailer\n\
             Exec=/nonexistent/ox-thumbnailer %i %o %s\nMimeType=video/mp4;\n"
        )
        .is_none());
        assert!(Thumbnailer::parse(
            "[Thumbnailer Entry]\nTryExec=/etc/hostname\nExec=/etc/hostname %i %o %s\nMimeType=video/mp4;\n"
        )
        .is_none());

        // A file of another kind, and an entry without a command or a type.
        assert!(Thumbnailer::parse("this is not an entry").is_none());
        assert!(Thumbnailer::parse("[Thumbnailer Entry]\nMimeType=video/mp4;\n").is_none());
        assert!(Thumbnailer::parse("[Thumbnailer Entry]\nExec=/bin/sh %i %o %s\n").is_none());
        assert!(Thumbnailer::parse("[Thumbnailer Entry]\nExec=\nMimeType=video/mp4;\n").is_none());

        // Only .thumbnailer files are read.
        let (_folder, dirs) = data_dir(&[
            (
                "unity.thumbnailer",
                "[Thumbnailer Entry]\nExec=/bin/sh %i %o %s\nMimeType=video/mp4;\n",
            ),
            (
                "notes.txt",
                "[Thumbnailer Entry]\nExec=/bin/sh %i %o %s\nMimeType=video/mp4;\n",
            ),
        ]);
        assert_eq!(read(&dirs).len(), 1);
    }

    /// A type of an entry's own, or a kind of one, matches it: GIO's type
    /// hierarchy says that `text/x-python` is a `text/plain`.
    ///
    /// parity: VIEW-057
    #[test]
    fn the_types_of_an_entry_cover_the_kinds_of_them() {
        let entry = Thumbnailer::parse("[Thumbnailer Entry]\nExec=/bin/sh %i %o %s\nMimeType=text/plain;\n")
            .expect("an entry");
        assert!(entry.handles("text/plain"), "the type itself");
        assert!(entry.handles("text/x-python"), "a kind of it");
        assert!(!entry.handles(BUNDLE), "another type");
    }

    /// `%i` is the item's path, `%u` its URI, `%o` where the picture goes
    /// and `%s` the edge wanted, also more than once in a word; a path that
    /// holds one of them itself is still taken as it is.
    ///
    /// parity: VIEW-057
    #[test]
    fn the_parameters_of_an_exec_line_are_replaced() {
        let entry = Thumbnailer::parse(
            "[Thumbnailer Entry]\n\
             Exec=/usr/bin/f3d --output=%o --resolution=%s,%s %i %u plain 100%%\n\
             MimeType=model/3mf;\n",
        )
        .expect("an entry");
        let words = |command: Vec<OsString>| -> Vec<String> {
            command
                .into_iter()
                .map(|word| word.to_string_lossy().into_owned())
                .collect()
        };
        let command = words(entry.command(
            Path::new("/home/demo/Sprite.ab"),
            "file:///home/demo/Sprite.ab",
            Path::new("/home/demo/cache/.tmp.png"),
            128,
        ));
        assert_eq!(
            command,
            [
                "/usr/bin/f3d",
                "--output=/home/demo/cache/.tmp.png",
                "--resolution=128,128",
                "/home/demo/Sprite.ab",
                "file:///home/demo/Sprite.ab",
                "plain",
                "100%%",
            ]
        );

        // A file whose name holds a parameter is passed once, not
        // substituted a second time.
        let odd = words(entry.command(
            Path::new("/home/demo/a%s%b.ab"),
            "file:///home/demo/a%s%25b.ab",
            Path::new("/home/demo/cache/.tmp.png"),
            128,
        ));
        assert_eq!(odd[3], "/home/demo/a%s%b.ab");
    }

    /// A thumbnailer the desktop installs makes the thumbnail of a type
    /// the app cannot read, and the app writes it into the cache with the
    /// keys that keep it valid, as it writes its own.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_desktop_thumbnailer_makes_a_thumbnail_in_the_cache() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let picture = folder.path().join("picture.png");
        let pixbuf = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 300, 200).expect("a picture");
        pixbuf.fill(0x3366_99ff);
        pixbuf.savev(&picture, "png", &[]).expect("the picture");
        // A thumbnailer of the test's own: it hands back the picture above,
        // refusing an item it was not given a path to and a size it was not
        // asked for, so the parameters of the command are proven too.
        let script = folder.path().join("ox-test-thumbnailer");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\ntest -f \"$1\" || exit 1\ntest \"$3\" = 128 || exit 1\ncp '{}' \"$2\"\n",
                picture.display()
            ),
        )
        .expect("the script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("the mode");
        // Its entry lives in a data folder of its own, never in the
        // desktop's: no test depends on what this computer installs.
        let (_data, dirs) = data_dir(&[(
            "ox-test.thumbnailer",
            &format!(
                "[Thumbnailer Entry]\nTryExec={script}\nExec={script} %i %o %s\nMimeType={BUNDLE};\n",
                script = script.display()
            ),
        )]);
        let item = folder.path().join("Sprite.ab");
        fs::write(&item, b"an asset bundle of no interest to the test").expect("the item");
        let cache = folder.path().join("cache");
        let job = Job {
            cache_dir: cache.clone(),
            uri: gio::File::for_path(&item).uri().to_string(),
            content_type: BUNDLE.to_owned(),
            modified: 1_700_000_000,
            flavor: ThumbnailFlavor::Normal,
        };
        let catalogue = Catalogue::of(&dirs, || Some(Sandbox::Off));
        // Without a sandbox that works here no program is kept at all, and
        // without an entry no sandbox is looked for.
        assert!(Catalogue::of(&dirs, || None).thumbnailers.is_empty());
        let (_none, no_entries) = data_dir(&[]);
        let empty = Catalogue::of(&no_entries, || panic!("no sandbox is looked for"));
        assert!(empty.thumbnailers.is_empty());

        let written = write_thumbnail(&job, &catalogue, &gio::Cancellable::new()).expect("a thumbnail");

        assert_eq!(written, thumbnail_file(&cache, &job.uri, ThumbnailFlavor::Normal));
        let thumbnail = Pixbuf::from_file(&written).expect("a PNG");
        assert_eq!((thumbnail.width(), thumbnail.height()), (128, 85));
        assert_eq!(
            thumbnail.option("tEXt::Thumb::URI").as_deref(),
            Some(job.uri.as_str())
        );
        assert_eq!(
            thumbnail.option("tEXt::Thumb::MTime").as_deref(),
            Some("1700000000")
        );
        let mode = fs::metadata(&written).expect("the file").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            fs::read_dir(written.parent().expect("a folder"))
                .expect("the cache folder")
                .count(),
            1,
            "the thumbnailer's own file is gone"
        );

        // A type no entry names is left to the app, and a thumbnailer that
        // writes nothing makes no thumbnail either.
        let other = Job {
            content_type: "image/png".to_owned(),
            ..job.clone()
        };
        assert!(write_thumbnail(&other, &catalogue, &gio::Cancellable::new()).is_none());
        let silent = Catalogue::for_test(vec![Thumbnailer {
            command: vec!["true".to_owned()],
            types: vec![BUNDLE.to_owned()],
        }]);
        let empty_cache = folder.path().join("empty-cache");
        let job = Job {
            cache_dir: empty_cache,
            ..job
        };
        assert!(write_thumbnail(&job, &silent, &gio::Cancellable::new()).is_none());
    }

    /// A request the user scrolled away from stops its thumbnailer at once
    /// and leaves no picture behind.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_cancelled_request_stops_its_thumbnailer() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let script = folder.path().join("ox-slow-thumbnailer");
        fs::write(&script, "#!/bin/sh\nsleep 5\n").expect("the script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("the mode");
        let item = folder.path().join("Sprite.ab");
        fs::write(&item, b"an asset bundle").expect("the item");
        let cache = folder.path().join("cache");
        let job = Job {
            cache_dir: cache.clone(),
            uri: gio::File::for_path(&item).uri().to_string(),
            content_type: BUNDLE.to_owned(),
            modified: 1_700_000_000,
            flavor: ThumbnailFlavor::Normal,
        };
        let slow = Catalogue::for_test(vec![Thumbnailer::for_test(
            &[script.to_str().expect("a path"), "%i", "%o", "%s"],
            &[BUNDLE],
        )]);
        let cancellation = gio::Cancellable::new();
        cancellation.cancel();

        let written = write_thumbnail(&job, &slow, &cancellation);

        assert!(written.is_none(), "a cancelled request makes no thumbnail");
        assert!(!cache.exists(), "and leaves nothing in the cache");
    }

    /// A request the user scrolled away from stops the programs its
    /// thumbnailer started too: none of them keeps writing the picture the
    /// app deletes.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_stopped_request_stops_the_programs_its_thumbnailer_started() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let started = folder.path().join("grandchild");
        let script = folder.path().join("ox-forking-thumbnailer");
        // A thumbnailer that starts a program of its own, writes its number
        // down and waits for it, as a wrapper script or ffmpeg does.
        fs::write(
            &script,
            format!("#!/bin/sh\nsleep 60 &\necho $! > '{}'\nwait\n", started.display()),
        )
        .expect("the script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("the mode");
        let catalogue = Catalogue::for_test(vec![Thumbnailer::for_test(
            &[script.to_str().expect("a path"), "%i", "%o", "%s"],
            &[BUNDLE],
        )]);
        let item = folder.path().join("Sprite.ab");
        fs::write(&item, b"an asset bundle").expect("the item");
        let job = Job {
            cache_dir: folder.path().join("cache"),
            uri: gio::File::for_path(&item).uri().to_string(),
            content_type: BUNDLE.to_owned(),
            modified: 1_700_000_000,
            flavor: ThumbnailFlavor::Normal,
        };
        let cancellation = gio::Cancellable::new();

        let (grandchild, written) = thread::scope(|scope| {
            let making = scope.spawn(|| write_thumbnail(&job, &catalogue, &cancellation));
            let grandchild = pid_in(&started);
            cancellation.cancel();
            (grandchild, making.join().expect("the worker"))
        });

        assert!(written.is_none(), "a cancelled request makes no thumbnail");
        assert!(
            !still_runs(grandchild),
            "the program the thumbnailer started went with it"
        );
    }

    /// A type the user turned external thumbnailers off for starts none,
    /// while another type still gets its thumbnailer.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_type_the_user_turned_off_starts_no_thumbnailer() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let (script, started, _argument) = fake_program(folder.path());
        let script = script.to_str().expect("a path");
        let picture = Thumbnailer::for_test(&[script, "%i", "%o", "%s"], &["image/png"]);
        let video = Thumbnailer::for_test(&[script, "%i", "%o", "%s"], &["video/mp4"]);
        let mut catalogue = Catalogue::for_test(vec![picture, video]);
        // The user's `disable` names MIME types, not files.
        catalogue.disabled.types = vec!["image/png".to_owned()];
        let (png, mp4) = (folder.path().join("photo.png"), folder.path().join("clip.mp4"));
        fs::write(&png, b"not a picture").expect("the item");
        fs::write(&mp4, b"not a video").expect("the item");
        let job_of = |path: &Path, content_type: &str| Job {
            cache_dir: folder.path().join("cache"),
            uri: gio::File::for_path(path).uri().to_string(),
            content_type: content_type.to_owned(),
            modified: 1_700_000_000,
            flavor: ThumbnailFlavor::Normal,
        };

        assert!(write_thumbnail(&job_of(&png, "image/png"), &catalogue, &gio::Cancellable::new()).is_none());
        assert!(!started.exists(), "no program runs for a type turned off");
        assert!(write_thumbnail(&job_of(&mp4, "video/mp4"), &catalogue, &gio::Cancellable::new()).is_some());
        assert!(started.exists(), "another type still gets its thumbnailer");
    }

    /// A desktop with `disable-all` set starts no program, and one where
    /// the user turned it back off again starts them at once: the setting
    /// is read for each request, the entries once.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_desktop_with_every_thumbnailer_off_starts_none() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let (script, started, _argument) = fake_program(folder.path());
        let (_data, dirs) = data_dir(&[(
            "fake.thumbnailer",
            &format!(
                "[Thumbnailer Entry]\nTryExec={script}\nExec={script} %i %o %s\nMimeType={BUNDLE};\n",
                script = script.display()
            ),
        )]);
        let item = folder.path().join("Sprite.ab");
        fs::write(&item, b"an asset bundle").expect("the item");
        let job = Job {
            cache_dir: folder.path().join("cache"),
            uri: gio::File::for_path(&item).uri().to_string(),
            content_type: BUNDLE.to_owned(),
            modified: 1_700_000_000,
            flavor: ThumbnailFlavor::Normal,
        };
        let on = Catalogue::of(&dirs, || Some(Sandbox::Off));
        let off = on.with_disabled(Disabled {
            types: Vec::new(),
            all: true,
        });

        assert_eq!(on.thumbnailers.len(), 1, "the entry a desktop installs");
        assert!(write_thumbnail(&job, &off, &gio::Cancellable::new()).is_none());
        assert!(!started.exists(), "no program ran");
        assert!(write_thumbnail(&job, &on, &gio::Cancellable::new()).is_some());
        assert!(started.exists(), "and one does when the user allows them");
    }

    /// A file whose name is no UTF-8 gets its thumbnail all the same, from
    /// an entry taking its URI and from one taking its path: both are
    /// handed over as the bytes the system knows them by.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_name_that_is_no_utf8_is_still_previewed() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let name = OsStr::from_bytes(b"Sprite\xff.ab");
        let item = folder.path().join(name);
        fs::write(&item, b"an asset bundle").expect("the item");
        let (script, started, argument) = fake_program(folder.path());
        let script = script.to_str().expect("a path");
        let uri = gio::File::for_path(&item).uri().to_string();

        for (index, (placeholder, expected)) in [("%u", OsStr::new(uri.as_str())), ("%i", item.as_os_str())]
            .into_iter()
            .enumerate()
        {
            let cache = folder.path().join(format!("cache-{index}"));
            let entry = Thumbnailer::for_test(&[script, placeholder, "%o"], &[BUNDLE]);
            let job = Job {
                cache_dir: cache.clone(),
                uri: uri.clone(),
                content_type: BUNDLE.to_owned(),
                modified: 1_700_000_000,
                flavor: ThumbnailFlavor::Normal,
            };

            let written = write_thumbnail(&job, &Catalogue::for_test(vec![entry]), &gio::Cancellable::new())
                .unwrap_or_else(|| panic!("a thumbnail from an entry taking {placeholder}"));

            assert_eq!(written, thumbnail_file(&cache, &uri, ThumbnailFlavor::Normal));
            assert_eq!(
                fs::read(&argument).expect("the argument the program was given"),
                expected.as_bytes(),
                "{placeholder} is handed over as it is"
            );
            assert!(started.exists(), "the program ran for {placeholder}");
        }
    }

    /// A program for a test's thumbnailer: it touches `started`, writes its
    /// first argument to `argument`, and copies `handed.png` where it is
    /// asked.
    fn fake_program(folder: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let started = folder.join("started");
        let argument = folder.join("argument");
        let handed = folder.join("handed.png");
        let picture = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 64).expect("a picture");
        picture.fill(0xcc_3333ff);
        picture.savev(&handed, "png", &[]).expect("the picture");
        let script = folder.join("ox-fake-thumbnailer");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\ntouch '{}'\nprintf '%s' \"$1\" > '{}'\ncp '{}' \"$2\"\n",
                started.display(),
                argument.display(),
                handed.display()
            ),
        )
        .expect("the script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("the mode");
        (script, started, argument)
    }

    /// The number `path` holds, waited for while the worker starts its
    /// program.
    fn pid_in(path: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(pid) = fs::read_to_string(path)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            assert!(
                Instant::now() < deadline,
                "the thumbnailer never started its program"
            );
            thread::sleep(POLL);
        }
    }

    /// Whether `pid` still runs; a zombie that nobody reaps runs nothing,
    /// as in a container whose PID 1 does not.
    fn still_runs(pid: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            let state = stat
                .rsplit_once(')')
                .map(|(_, rest)| rest.trim().to_owned())
                .unwrap_or_default();
            let runs = !state.is_empty() && !state.starts_with('Z');
            if !runs || Instant::now() >= deadline {
                return runs;
            }
            thread::sleep(POLL);
        }
    }

    /// In bubblewrap a thumbnailer is given the item, read-only under a
    /// name of the sandbox's own, and a private folder for its picture;
    /// the user's other files are out of its sight. Runs where bubblewrap
    /// can make its sandbox.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_sandboxed_thumbnailer_sees_only_its_item() {
        let Some(sandbox) = Sandbox::find() else {
            eprintln!("skipped: bubblewrap cannot make its sandbox here");
            return;
        };
        let folder = tempfile::tempdir().expect("a temporary folder");
        let secret = folder.path().join("secret.txt");
        fs::write(&secret, b"another file of the user's").expect("a file");
        let escaped = folder.path().join("escaped");
        // The item is a picture, so the program can hand it back as its
        // own: what the app gets proves the program read it.
        let item = folder.path().join("Sprite.ab");
        let pixbuf = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 300, 200).expect("a picture");
        pixbuf.fill(0x3366_99ff);
        pixbuf.savev(&item, "png", &[]).expect("the item");
        let script = folder.path().join("ox-sandboxed-thumbnailer");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 test \"$1\" = /tmp/openxplorer-item.ab || exit 1\n\
                 test \"$2\" = file:///tmp/openxplorer-item.ab || exit 2\n\
                 test -e '{secret}' && exit 3\n\
                 touch '{escaped}' 2>/dev/null\n\
                 (: > \"$1\") 2>/dev/null && exit 4\n\
                 cp \"$1\" \"$3\"\n",
                secret = secret.display(),
                escaped = escaped.display(),
            ),
        )
        .expect("the script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("the mode");
        let mut catalogue = Catalogue::for_test(vec![Thumbnailer::for_test(
            &[script.to_str().expect("a path"), "%i", "%u", "%o", "%s"],
            &[BUNDLE],
        )]);
        catalogue.sandbox = Some(sandbox);
        let cache = folder.path().join("cache");
        let job = Job {
            cache_dir: cache.clone(),
            uri: gio::File::for_path(&item).uri().to_string(),
            content_type: BUNDLE.to_owned(),
            modified: 1_700_000_000,
            flavor: ThumbnailFlavor::Normal,
        };

        let written = write_thumbnail(&job, &catalogue, &gio::Cancellable::new())
            .expect("a thumbnail made in the sandbox");

        assert_eq!(written, thumbnail_file(&cache, &job.uri, ThumbnailFlavor::Normal));
        let thumbnail = Pixbuf::from_file(&written).expect("a PNG");
        assert_eq!((thumbnail.width(), thumbnail.height()), (128, 85));
        assert!(!escaped.exists(), "the program wrote nothing outside its folder");
        assert!(secret.exists() && item.exists());
    }

    /// What a thumbnailer leaves is read only when it is a picture file of
    /// a sensible size: a link is not followed, and a pipe is not opened
    /// for a read that would never end.
    ///
    /// parity: VIEW-057
    #[test]
    fn only_a_picture_file_is_read_back() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let picture = folder.path().join("picture.png");
        let pixbuf = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 300, 200).expect("a picture");
        pixbuf.fill(0x3366_99ff);
        pixbuf.savev(&picture, "png", &[]).expect("the picture");
        let link = folder.path().join("link.png");
        std::os::unix::fs::symlink(&picture, &link).expect("the link");
        let pipe = folder.path().join("pipe.png");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &pipe,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .expect("the pipe");
        let large = folder.path().join("large.png");
        fs::File::create(&large)
            .and_then(|file| file.set_len(OUTPUT_SIZE_LIMIT + 1))
            .expect("the large file");

        let read = read_picture(&picture, 128).expect("a picture file is read");
        assert_eq!((read.width(), read.height()), (128, 85));
        assert!(read_picture(&link, 128).is_none(), "a link is not followed");
        assert!(read_picture(&pipe, 128).is_none(), "a pipe is not read");
        assert!(read_picture(&large, 128).is_none(), "a huge file is not read");
        // A PNG declaring a picture far larger than any thumbnail is not
        // decoded, nor is a file of another kind.
        let declared = folder.path().join("declared.png");
        let mut header = PNG_SIGNATURE.to_vec();
        header.extend_from_slice(&13_u32.to_be_bytes());
        header.extend_from_slice(b"IHDR");
        header.extend_from_slice(&100_000_u32.to_be_bytes());
        header.extend_from_slice(&100_000_u32.to_be_bytes());
        header.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        fs::write(&declared, &header).expect("the header");
        assert!(read_picture(&declared, 128).is_none(), "a bomb is not decoded");
        // A text chunk is dropped before decoding; the picture stays.
        let mut texted = fs::read(&picture).expect("the picture");
        let mut text = Vec::new();
        text.extend_from_slice(&10_u32.to_be_bytes());
        text.extend_from_slice(b"zTXtComment\0\0x");
        text.extend_from_slice(&[0, 0, 0, 0]);
        texted.splice(33..33, text);
        let drawn = drawn_png(&texted).expect("a PNG");
        assert!(!drawn.windows(4).any(|window| window == b"zTXt"));
        assert!(drawn.len() < texted.len());
        assert!(
            drawn_png(&texted[..texted.len() - 12]).is_none(),
            "a PNG without its end"
        );
        let other = folder.path().join("other.png");
        fs::write(&other, b"GIF89a, not a PNG at all").expect("the file");
        assert!(read_picture(&other, 128).is_none(), "only a PNG is read");
        // A folder the program locked is still removed with its files.
        let workspace = Workspace::new().expect("a workspace");
        let path = workspace.path().to_path_buf();
        let locked = path.join("locked");
        fs::create_dir(&locked).expect("a folder");
        fs::write(locked.join("kept"), b"left behind").expect("a file");
        // Folders nested deeper than a path may name, made as a program
        // makes them, one inside the other.
        let mut deep = rustix::fs::open(
            &path,
            rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("the workspace");
        let long = "d".repeat(250);
        for _ in 0..20 {
            rustix::fs::mkdirat(&deep, long.as_str(), rustix::fs::Mode::RWXU).expect("a folder");
            deep = rustix::fs::openat(
                &deep,
                long.as_str(),
                rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .expect("the folder");
        }
        drop(deep);
        // A link out of the workspace goes, and what it names stays.
        std::os::unix::fs::symlink(&picture, path.join("out")).expect("the link");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("the mode");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).expect("the mode");
        drop(workspace);
        assert!(!path.exists(), "the workspace is gone");
        assert!(picture.exists(), "and a link in it was not followed");
        assert_eq!(extension_of(Path::new("/a/Clip.MP4")), ".MP4");
        assert_eq!(extension_of(Path::new("/a/odd.p n g")), "");
        assert_eq!(extension_of(Path::new("/a/none")), "");
    }
}
