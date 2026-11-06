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
//! A thumbnailer writes a plain PNG, without the keys that keep a
//! thumbnail valid; the app writes the picture into the shared cache
//! itself, as Files does, through [`super::writer`].

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{thumbnail_file, ThumbnailFlavor};
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

/// The end of the name of a thumbnailer's own picture in the cache folder,
/// which tells a picture no one claimed from a thumbnail of the cache.
const TEMPORARY: &str = "openxplorer-thumbnailer.png";

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

/// Where the desktop's thumbnailers are read, and what the user turned
/// off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Dirs {
    /// The data folders, the user's first.
    pub folders: Vec<PathBuf>,
    /// The MIME types `disable` names no external thumbnailer is started
    /// for.
    pub disabled_types: Vec<String>,
    /// `disable-all` is set: no external thumbnailer is started at all.
    pub disable_all: bool,
}

impl Dirs {
    /// The folders of this session, and what the desktop turned off.
    fn of_session() -> Self {
        let mut folders = vec![glib::user_data_dir()];
        folders.extend(glib::system_data_dirs());
        let (disabled_types, disable_all) = disabled();
        Self {
            folders,
            disabled_types,
            disable_all,
        }
    }
}

/// The thumbnailers a desktop offers, and the ones its user turned off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Catalogue {
    /// The thumbnailers installed, in the order they are tried.
    thumbnailers: Vec<Thumbnailer>,
    /// The MIME types `disable` names no program for.
    disabled_types: Vec<String>,
    /// `disable-all` is set: no program is started for any type.
    disable_all: bool,
}

impl Catalogue {
    /// The thumbnailers `dirs` holds: a desktop with every external
    /// thumbnailer turned off has none read.
    fn of(dirs: &Dirs) -> Self {
        let thumbnailers = if dirs.disable_all { Vec::new() } else { read(dirs) };
        Self {
            thumbnailers,
            disabled_types: dirs.disabled_types.clone(),
            disable_all: dirs.disable_all,
        }
    }

    /// The thumbnailers of a test, which read no folder of the desktop's.
    #[cfg(test)]
    pub(super) fn for_test(thumbnailers: Vec<Thumbnailer>) -> Self {
        Self {
            thumbnailers,
            disabled_types: Vec::new(),
            disable_all: false,
        }
    }

    /// The thumbnailer that makes `content_type`, unless the user turned
    /// external thumbnailers off for the type or for every type.
    fn for_type(&self, content_type: &str) -> Option<&Thumbnailer> {
        if self.disable_all || self.disables(content_type) {
            return None;
        }
        self.thumbnailers
            .iter()
            .find(|thumbnailer| thumbnailer.handles(content_type))
    }

    /// Whether the user turned external thumbnailers off for
    /// `content_type`: `disable` names types, and a type it names takes the
    /// kinds of it along, as an entry naming a parent type does.
    fn disables(&self, content_type: &str) -> bool {
        self.disabled_types
            .iter()
            .any(|declared| covers(content_type, declared))
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

/// The thumbnailers this desktop installs and what its user turned off,
/// read the first time a thumbnail has to be made and once per run
/// (`thumbnailers/` holds a handful of small files).
pub(super) fn installed() -> Arc<Catalogue> {
    Arc::clone(INSTALLED.get_or_init(|| Arc::new(Catalogue::of(&Dirs::of_session()))))
}

/// The thumbnailers `dirs` holds, the user's folders before the system
/// ones and each folder's files by name, which is the order Files tries
/// them in.
fn read(dirs: &Dirs) -> Vec<Thumbnailer> {
    let mut thumbnailers = Vec::new();
    for folder in &dirs.folders {
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

/// What the user turned external thumbnailers off for in
/// `org.gnome.desktop.thumbnailers`: the MIME types `disable` names, and
/// whether `disable-all` turns every one of them off. A session without
/// the schema turns none off.
fn disabled() -> (Vec<String>, bool) {
    let Some(schema) =
        gio::SettingsSchemaSource::default().and_then(|source| source.lookup(DISABLE_SCHEMA, true))
    else {
        return (Vec::new(), false);
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
    (types, all)
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
    // A thumbnailer takes a file of this computer, its `%i` being a path: a
    // file on a share, a phone or in an archive is left to the service.
    let path = gio::File::for_uri(&job.uri).path()?;
    let target = thumbnail_file(&job.cache_dir, &job.uri, job.flavor);
    let folder = target.parent()?;
    // The program's own picture lands in the cache folder, which only its
    // owner may enter, and is replaced by the cache's own file below.
    writer::create_folder(folder)?;
    sweep(folder);
    let output = folder.join(format!(".{}.{}", glib::random_int(), TEMPORARY));
    let command = thumbnailer.command(&path, &job.uri, &output, job.flavor.pixels());
    let stopped = stopped_early(&command, cancellation);
    // A stopped thumbnailer left no picture; one that failed on its own may
    // still have written a good one, so the picture alone decides. A
    // thumbnailer writes at the size it was asked for, and one that ignores
    // `%s` is brought within the flavor.
    let picture = (!stopped)
        .then(|| Pixbuf::from_file(&output).ok())
        .flatten()
        .filter(|picture| picture.width() > 0 && picture.height() > 0)
        .map(|picture| writer::fit_thumbnail(&picture, job.flavor.pixels()));
    let _ = fs::remove_file(&output);
    let picture = picture?;
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

/// Removes the pictures of thumbnailers an earlier run of the app was
/// killed with: quitting stops a program mid-run, and its picture would
/// stay in the cache for ever. One older than twice [`TIMEOUT`] belongs to
/// no running request, so a sibling one is never touched.
fn sweep(folder: &Path) {
    let Ok(read) = fs::read_dir(folder) else {
        return;
    };
    for entry in read.filter_map(Result::ok) {
        let name = entry.file_name();
        let abandoned = name.to_str().is_some_and(|name| name.ends_with(TEMPORARY));
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > TIMEOUT * 2));
        if abandoned && old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Runs `command` with nothing of its output shown and returns whether it
/// had to be killed, because the request was cancelled or it outlasted
/// [`TIMEOUT`].
///
/// The wait polls instead of blocking on the child so that a cancelled
/// request stops the program within [`POLL`]: a request scrolled away must
/// not keep a slot for the whole timeout.
fn stopped_early(command: &[OsString], cancellation: &gio::Cancellable) -> bool {
    let Some((program, arguments)) = command.split_first() else {
        return true;
    };
    let spawned = std::process::Command::new(program)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // The thumbnailer gets a process group of its own, so stopping it
        // stops the programs it started too: a wrapper script's `cp`, or
        // ffmpeg, would otherwise keep writing the picture the app deletes.
        .process_group(0)
        .spawn();
    let Ok(mut child) = spawned else {
        return true;
    };
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            return false;
        }
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            stop(&mut child);
            return true;
        }
        thread::sleep(POLL);
    }
}

/// Kills the process group `child` leads, and reaps the child.
///
/// The group is the child's own, [`stopped_early`] having put it in one:
/// its number is the child's. A group that is gone already is no
/// failure — the program may have exited between the last look and this
/// kill — so the outcome is not reported.
fn stop(child: &mut std::process::Child) {
    let group = i32::try_from(child.id())
        .ok()
        .and_then(Pid::from_raw)
        .is_some_and(|group| kill_process_group(group, Signal::KILL).is_ok());
    if !group {
        // Without a group of its own, as when the spawn itself failed, the
        // child is all the app can end.
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    /// The type of the Unity asset bundles the app is asked to preview.
    const BUNDLE: &str = "application/x-unity-assetbundle";

    /// A data folder with `entries` written as `(file name, text)`.
    fn data_dir(entries: &[(&str, &str)]) -> (tempfile::TempDir, Dirs) {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let data = folder.path().join("share");
        for (name, text) in entries {
            let path = data.join(FOLDER).join(name);
            fs::create_dir_all(path.parent().expect("a folder")).expect("the folder");
            fs::write(&path, text).expect("the entry");
        }
        let dirs = Dirs {
            folders: vec![data],
            disabled_types: Vec::new(),
            disable_all: false,
        };
        (folder, dirs)
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
        let catalogue = Catalogue::of(&dirs);

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
        assert!(!thumbnail_file(&cache, &job.uri, ThumbnailFlavor::Normal).exists());
        let left = fs::read_dir(cache.join("thumbnails").join(ThumbnailFlavor::Normal.as_str()))
            .expect("the cache folder")
            .count();
        assert_eq!(left, 0, "and leaves nothing in the cache");
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
        catalogue.disabled_types = vec!["image/png".to_owned()];
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

    /// A desktop with `disable-all` set reads none of its thumbnailer files
    /// and starts no program.
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
        let on = Catalogue::of(&dirs);
        let off = Catalogue::of(&Dirs {
            disable_all: true,
            ..dirs
        });

        assert_eq!(on.thumbnailers.len(), 1, "the entry a desktop installs");
        assert!(
            off.thumbnailers.is_empty(),
            "with every thumbnailer off, none is read"
        );
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

    /// A picture a thumbnailer wrote for a run of the app that quit with it
    /// is swept from the cache, while a running one and a thumbnail of the
    /// cache are kept.
    ///
    /// parity: VIEW-057
    #[test]
    fn the_pictures_of_a_killed_run_are_swept() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let running = folder.path().join(format!(".1.{TEMPORARY}"));
        let killed = folder.path().join(format!(".2.{TEMPORARY}"));
        let thumbnail = folder.path().join("c4ca4238a0b923820dcc509a6f75849b.png");
        for path in [&running, &killed, &thumbnail] {
            fs::write(path, b"a picture").expect("the file");
        }
        let long_ago = SystemTime::now() - TIMEOUT * 4;
        fs::OpenOptions::new()
            .write(true)
            .open(&killed)
            .expect("the file")
            .set_times(fs::FileTimes::new().set_modified(long_ago))
            .expect("the time");

        sweep(folder.path());

        assert!(!killed.exists(), "a killed run's picture goes");
        assert!(running.exists(), "a running request's picture stays");
        assert!(thumbnail.exists(), "a thumbnail of the cache stays");
    }
}
