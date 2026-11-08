// SPDX-License-Identifier: AGPL-3.0-only
//! The update helper, `openxplorer-install-update`: installs the latest
//! official release as root, so that an administrator at the machine
//! updates without typing a password. Ports nothing: the Python app ran
//! `pkexec apt-get` on the installer it had downloaded.
//!
//! # Why a helper
//!
//! `pkexec apt-get install <file>` runs under polkit's generic
//! `org.freedesktop.policykit.exec` action, which asks for the
//! administrator's password every time, and must: whoever may run it
//! without a password may install any package file at all. This helper has
//! a polkit action of its own (`<id>.install-update`, with
//! `org.freedesktop.policykit.exec.path` naming exactly this program), and
//! the Debian package's polkit rule lets an administrator (a member of
//! `sudo`, `admin` or `wheel`) in an active local session run it without
//! authenticating, as Ubuntu's Software lets them install system updates.
//! Anyone else is asked for an administrator's password, as before
//! (`allow_active` is `auth_admin_keep`).
//!
//! # Threat model
//!
//! Granting that without a password is safe only because of what the
//! helper accepts. Whoever starts it, a program running as the user
//! included, controls its command line, its standard input, its
//! environment as pkexec passes it, its resource limits and its signal
//! state; it controls nothing else. So the helper:
//!
//! - takes one argument, a `MAJOR.MINOR.PATCH` version
//!   ([`ReleaseVersion`]), and no path, address, digest or option;
//! - runs only as root, real and effective, so a helper that was somehow
//!   made set-user-ID is refused rather than run without polkit;
//! - throws its environment away before anything reads it and sets a
//!   fixed one ([`ENVIRONMENT`]), resets the resource limits a caller could
//!   lower to make a download or dpkg fail part-way, works from `/` with
//!   umask 022, and reads no settings and no file a user can write;
//! - asks GitHub's fixed "latest release" endpoint itself, over HTTPS with
//!   redirects only to the trusted hosts ([`super::TrustedUrl`]), and
//!   accepts the release only by the rules the app checks
//!   ([`super::parse_release`]);
//! - installs only if that release is exactly the version asked for and
//!   newer than the `openxplorer` package `dpkg-query` reports installed, so
//!   it can never downgrade or reinstall;
//! - runs one installation at a time: a second helper started while one
//!   runs is refused at once, before it asks anything or downloads, so
//!   repeated starts cannot pile up root processes or downloads;
//! - never waits on its standard output or error, which belong to the
//!   caller: a thread of their own writes them from a short queue, a
//!   report that does not fit is dropped, and at the end the helper waits
//!   at most [`REPORT_GRACE`] for the last one, so a caller that stops
//!   reading cannot hold a root process, its lock or its download;
//! - downloads into a fresh 0700 folder in its own root-owned
//!   [`STAGING_FOLDER`], which only root can write, and accepts the file
//!   only with the release's exact size and SHA-256 digest
//!   ([`super::download`]), then only if `dpkg-deb` says it is package
//!   `openxplorer`, that version, architecture `all`;
//! - runs `apt-get -y --no-remove install` on it with input from
//!   `/dev/null`, and reports success only when `dpkg-query` confirms the
//!   version; the download is deleted in every case.
//!
//! What the caller can still do is make the installation fail (cut the
//! network, cancel before APT starts, send odd signal state), or start the
//! installation of the newest official release earlier than the user
//! would have. It cannot make the helper install anything else, keep it
//! from verifying, or point it at a file. The trust in the release is the
//! app's own: GitHub's TLS and its asset digest, which is not an
//! independent publisher signature, exactly as when the user typed a
//! password for the same file.
//!
//! It writes one [`HelperReport`] per line on its standard output and
//! stops before APT starts when it reads a line `cancel` on its standard
//! input. Its exit status:
//!
//! | Status | Meaning |
//! |---|---|
//! | 0 | The release is installed and confirmed |
//! | 1 | The installation was refused or failed; the last line says why |
//! | 2 | The request was refused before anything was fetched |

use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use super::download::download_installer;
use super::helper_report::{HelperFailure, HelperReport};
use super::package::{
    check_installed_version, check_installer_fields, installation_failure, installed_version,
};
use super::process::CANCEL_LINE;
use super::server::read_release_answer;
use super::{
    parse_release, GitHubReleases, Installation, PackageCommand, PackageManager, ReleaseServer,
    ReleaseVersion, SystemPackageManager, UpdateError,
};
use crate::private_storage::private_directory;
use crate::transfer::Cancellation;

/// The helper program's file name.
pub const INSTALL_HELPER_COMMAND: &str = "openxplorer-install-update";

/// Where the helper keeps its downloads while it verifies and installs
/// them: a folder of its own in `/var/cache`, which only root can write.
/// Each download gets a fresh 0700 folder inside, deleted afterwards.
///
/// Never a shared folder such as `/tmp`: the download makes this folder
/// owned and 0700 ([`crate::private_storage`]).
pub const STAGING_FOLDER: &str = "/var/cache/openxplorer-update";

/// The whole environment the helper runs with, and its children inherit.
/// Nothing of the caller's survives: no loader, GIO, proxy, locale or APT
/// variable reaches the network code, `dpkg` or APT.
pub const ENVIRONMENT: [(&str, &str); 10] = [
    ("PATH", "/usr/sbin:/usr/bin:/sbin:/bin"),
    ("HOME", "/root"),
    ("LANG", "C.UTF-8"),
    ("LC_ALL", "C.UTF-8"),
    // APT and debconf never ask: there is nobody to answer.
    ("DEBIAN_FRONTEND", "noninteractive"),
    ("APT_LISTCHANGES_FRONTEND", "none"),
    ("APT_LISTBUGS_FRONTEND", "none"),
    // GIO stays local: no GVfs daemon or session bus is started as root.
    ("GIO_USE_VFS", "local"),
    ("GSETTINGS_BACKEND", "memory"),
    ("TERM", "dumb"),
];

/// The release was installed and confirmed.
const INSTALLED: u8 = 0;
/// The installation was refused or failed, and the helper said why.
const FAILED: u8 = 1;
/// The request was refused before anything was fetched.
const REFUSED: u8 = 2;

/// The longest line of standard input the helper looks at. Longer lines
/// are skipped without being kept, so the caller cannot make a root
/// process hold an endless line.
const MAX_INPUT_LINE: usize = 64;

/// How many reports may wait for a caller that is slow to read them. An
/// installation makes at most four.
const REPORT_QUEUE: usize = 16;

/// How long the helper waits, after it has finished, for its last report
/// to be read before it exits anyway.
pub const REPORT_GRACE: Duration = Duration::from_secs(5);

/// The usage text, which `verify_layout.py` runs the installed helper to
/// check that it starts at all.
const USAGE: &str = "\
Usage: openxplorer-install-update <version>

Installs OpenXplorer <version> (MAJOR.MINOR.PATCH) as root, if it is the
latest stable release on GitHub and newer than the installed openxplorer
package. It downloads the release's Debian installer itself, verifies its
size and SHA-256 digest against the release and its identity with
dpkg-deb, and installs it with apt-get. OpenXplorer starts it through
pkexec when the user installs an update; it takes no path and no option.

It writes one JSON report per line on standard output, and stops before
apt-get starts when it reads a line 'cancel' on standard input.
";

/// What the program was asked to do.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    /// Install this version.
    Install(ReleaseVersion),
    /// Print the usage text and stop.
    Usage,
    /// Anything else.
    Refuse(&'static str),
}

/// The parts of an installation as root.
pub struct RootInstallParts<'a> {
    /// Where the release and the installer are fetched from.
    pub server: &'a dyn ReleaseServer,
    /// What runs `dpkg-query`, `dpkg-deb` and APT.
    pub package_manager: &'a dyn PackageManager,
    /// The helper's own download folder, [`STAGING_FOLDER`].
    pub staging_folder: &'a Path,
}

/// Runs the helper program and returns its exit status.
pub fn install_update_command(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let version = match read_request(arguments) {
        Request::Usage => {
            say(std::io::stdout(), USAGE.to_owned());
            return INSTALLED;
        }
        Request::Refuse(reason) => {
            say(std::io::stderr(), format!("{INSTALL_HELPER_COMMAND}: {reason}\n"));
            return REFUSED;
        }
        Request::Install(version) => version,
    };
    if let Err(reason) = prepare_process() {
        say(std::io::stderr(), format!("{INSTALL_HELPER_COMMAND}: {reason}\n"));
        return REFUSED;
    }
    let cancel = Cancellation::new();
    let watched = cancel.clone();
    // Ends with the process; nothing waits for it.
    std::thread::spawn(move || watch_input(std::io::stdin().lock(), &watched));
    let server = GitHubReleases::new(helper_version());
    let parts = RootInstallParts {
        server: &server,
        package_manager: &SystemPackageManager,
        staging_folder: Path::new(STAGING_FOLDER),
    };
    let reporter = Reporter::start(std::io::stdout());
    let mut report = |line: HelperReport| reporter.send(line.encode() + "\n");
    let status = match install_as_root(&parts, version, &mut report, &cancel) {
        Ok(()) => INSTALLED,
        Err(error) => {
            report(HelperReport::Failed(HelperFailure::of(&error)));
            FAILED
        }
    };
    reporter.finish(REPORT_GRACE);
    status
}

/// Installs `requested` as root: the steps of the threat model in the
/// module documentation, each reported to `report` as it starts.
///
/// # Errors
///
/// [`UpdateError::InstallUnavailable`] when `openxplorer` is not installed,
/// [`UpdateError::NotChecked`] when `requested` is not the latest release
/// or not newer than the installed version, every refusal of the release,
/// the download and `dpkg-deb`, [`UpdateError::Cancelled`] before APT, and
/// APT's and `dpkg-query`'s failures.
pub fn install_as_root(
    parts: &RootInstallParts<'_>,
    requested: ReleaseVersion,
    report: &mut dyn FnMut(HelperReport),
    cancel: &Cancellation,
) -> Result<(), UpdateError> {
    let _only_installation = lock_staging_folder(parts.staging_folder)?;
    let packages = parts.package_manager;
    let query = packages.run(&PackageCommand::QueryInstalled)?;
    let installed = installed_version(&query).ok_or(UpdateError::InstallUnavailable {
        installation: Installation::DebianPackageWithoutTools,
    })?;
    // Never a downgrade or a reinstallation, whatever GitHub says.
    if requested <= installed {
        return Err(UpdateError::NotChecked);
    }
    let answer = read_release_answer(parts.server, cancel)?;
    let release = parse_release(&answer, installed)?;
    if release.version != requested || !release.is_newer {
        return Err(UpdateError::NotChecked);
    }
    report(HelperReport::Downloading);
    let staged = download_installer(parts.server, &release.installer, parts.staging_folder, cancel)?;
    let installer = staged.path();
    let inspection = packages.run(&PackageCommand::InspectInstaller(installer.clone()))?;
    check_installer_fields(&inspection, requested)?;
    // The last moment cancelling can stop the installation.
    if cancel.is_cancelled() {
        return Err(UpdateError::Cancelled);
    }
    report(HelperReport::Installing);
    let installation = packages.run(&PackageCommand::InstallAsRoot(installer))?;
    if let Some(failure) = installation_failure(&installation) {
        return Err(failure);
    }
    let query = packages.run(&PackageCommand::QueryInstalled)?;
    check_installed_version(&query, requested)?;
    drop(staged);
    report(HelperReport::Installed(requested));
    Ok(())
}

/// Makes `staging_folder` the helper's private folder and takes its
/// lock, which the returned file holds until it is dropped.
///
/// # Errors
///
/// [`UpdateError::TaskRunning`] while another helper holds it, and the
/// private folder's refusals.
fn lock_staging_folder(staging_folder: &Path) -> Result<File, UpdateError> {
    use rustix::fs::{flock, FlockOperation, OFlags};
    private_directory(staging_folder)?;
    let folder = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::DIRECTORY | OFlags::NOFOLLOW).bits().cast_signed())
        .open(staging_folder)
        .map_err(|error| UpdateError::io(staging_folder, error))?;
    match flock(&folder, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(folder),
        Err(rustix::io::Errno::WOULDBLOCK) => Err(UpdateError::TaskRunning),
        Err(error) => Err(UpdateError::io(staging_folder, error.into())),
    }
}

/// Writes to an output the caller owns from a thread of its own, so the
/// helper never waits for the caller to read.
///
/// Setting `O_NONBLOCK` would not do: the caller shares the open file
/// description and can clear the flag again.
struct Reporter {
    /// Where texts wait for the writer; `None` once finished.
    queue: Option<SyncSender<String>>,
    /// Told when the writer has written everything, or given up.
    written: Receiver<()>,
}

impl Reporter {
    /// Starts the writer for `output`.
    fn start(mut output: impl Write + Send + 'static) -> Self {
        let (queue, texts) = mpsc::sync_channel::<String>(REPORT_QUEUE);
        let (done, written) = mpsc::channel();
        // Not joined: the process may end while it still waits.
        std::thread::spawn(move || {
            for text in texts {
                if output
                    .write_all(text.as_bytes())
                    .and_then(|()| output.flush())
                    .is_err()
                {
                    break;
                }
            }
            let _ = done.send(());
        });
        Self {
            queue: Some(queue),
            written,
        }
    }

    /// Queues `text`, or drops it if the queue is full or the writer gave
    /// up. Never waits.
    fn send(&self, text: String) {
        if let Some(queue) = &self.queue {
            let _ = queue.try_send(text);
        }
    }

    /// Waits at most `grace` for everything queued to be written.
    fn finish(mut self, grace: Duration) {
        drop(self.queue.take());
        let _ = self.written.recv_timeout(grace);
    }
}

/// Writes `text` to `output` without waiting longer than [`REPORT_GRACE`].
fn say(output: impl Write + Send + 'static, text: String) {
    let reporter = Reporter::start(output);
    reporter.send(text);
    reporter.finish(REPORT_GRACE);
}

/// Reads the command line: one version, or `--help`.
fn read_request(arguments: impl IntoIterator<Item = OsString>) -> Request {
    let arguments: Vec<OsString> = arguments.into_iter().take(2).collect();
    let [argument] = arguments.as_slice() else {
        return Request::Refuse("This helper takes exactly one version.");
    };
    if argument == "--help" || argument == "-h" {
        return Request::Usage;
    }
    match argument.to_str().map(str::parse::<ReleaseVersion>) {
        Some(Ok(version)) => Request::Install(version),
        _ => Request::Refuse("The version must be MAJOR.MINOR.PATCH."),
    }
}

/// Makes this process safe to run as root for whoever started it: root
/// only, a fixed environment, resource limits a caller cannot have
/// lowered, `/` as the working folder and umask 022.
///
/// # Errors
///
/// Why it is not safe to go on; nothing has been fetched or run yet.
fn prepare_process() -> Result<(), &'static str> {
    use rustix::process::{geteuid, getuid};
    if !getuid().is_root() || !geteuid().is_root() {
        return Err("This helper runs only as root, through pkexec.");
    }
    reset_environment();
    reset_resource_limits().map_err(|_| "The resource limits could not be reset.")?;
    std::env::set_current_dir("/").map_err(|_| "The working folder could not be changed to /.")?;
    rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o022));
    Ok(())
}

/// Replaces the whole environment with [`ENVIRONMENT`]. The helper is
/// still single-threaded here, and nothing has read the environment yet.
fn reset_environment() {
    for (name, _) in std::env::vars_os() {
        std::env::remove_var(name);
    }
    for (name, value) in ENVIRONMENT {
        std::env::set_var(name, value);
    }
}

/// Lifts the limits a caller could lower to make the download or `dpkg`
/// fail part-way, and turns core dumps off.
fn reset_resource_limits() -> rustix::io::Result<()> {
    use rustix::process::{setrlimit, Resource, Rlimit};
    let unlimited = Rlimit {
        current: None,
        maximum: None,
    };
    for resource in [Resource::Fsize, Resource::Cpu, Resource::As, Resource::Data] {
        setrlimit(resource, unlimited)?;
    }
    let none = Rlimit {
        current: Some(0),
        maximum: Some(0),
    };
    setrlimit(Resource::Core, none)
}

/// The version the helper introduces itself with to GitHub: its own
/// package's.
fn helper_version() -> ReleaseVersion {
    env!("CARGO_PKG_VERSION")
        .parse()
        .unwrap_or(ReleaseVersion::new(0, 0, 0))
}

/// Cancels `cancel` when `input` holds the line `cancel`, until `input`
/// ends. Only lines of at most [`MAX_INPUT_LINE`] bytes are looked at, and
/// nothing longer is kept.
fn watch_input(mut input: impl Read, cancel: &Cancellation) {
    let mut line = Vec::with_capacity(MAX_INPUT_LINE);
    let mut too_long = false;
    let mut block = [0_u8; 256];
    loop {
        let count = match input.read(&mut block) {
            Ok(0) => return,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        for &byte in &block[..count] {
            if byte == b'\n' {
                line.push(byte);
                if !too_long && line == CANCEL_LINE {
                    cancel.cancel();
                }
                line.clear();
                too_long = false;
            } else if line.len() < MAX_INPUT_LINE {
                line.push(byte);
            } else {
                too_long = true;
            }
        }
    }
}

/// Where an installed app finds the helper: `<prefix>/libexec/` beside
/// `<prefix>/bin/<program>`, as the Debian package installs both.
pub(super) fn helper_beside(app_root: &Path) -> PathBuf {
    app_root.join("libexec").join(INSTALL_HELPER_COMMAND)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(arguments: &[&str]) -> Request {
        read_request(arguments.iter().map(OsString::from))
    }

    /// The helper takes one plain version and nothing else: no path, no
    /// option, no second argument.
    #[test]
    fn only_one_plain_version_is_accepted() {
        assert_eq!(
            request(&["2.0.16"]),
            Request::Install(ReleaseVersion::new(2, 0, 16))
        );
        assert_eq!(request(&["--help"]), Request::Usage);
        let refused: [&[&str]; 13] = [
            &[],
            &["2.0.16", "2.0.17"],
            &["2.0.16", "--allow-downgrades"],
            &["--", "2.0.16"],
            &["v2.0.16"],
            &["2.0.16-1"],
            &["02.0.16"],
            &["2.0"],
            &["../../tmp/evil.deb"],
            &["/tmp/openxplorer_2.0.16_all.deb"],
            &["https://example.invalid/openxplorer.deb"],
            &["--version=2.0.16"],
            &[" 2.0.16"],
        ];
        for arguments in refused {
            assert!(
                matches!(request(arguments), Request::Refuse(_)),
                "{arguments:?} is refused"
            );
        }
    }

    #[test]
    fn a_version_that_is_not_text_is_refused() {
        use std::os::unix::ffi::OsStringExt;
        let arguments = [OsString::from_vec(b"2.0.\xff".to_vec())];

        assert!(matches!(read_request(arguments), Request::Refuse(_)));
    }

    /// Nothing that changes how a program loads, where GIO finds modules,
    /// which proxy it uses, or how APT and dpkg behave survives.
    #[test]
    fn the_environment_is_fixed_and_carries_nothing_of_the_callers() {
        let names: Vec<&str> = ENVIRONMENT.iter().map(|(name, _)| *name).collect();
        let dangerous = [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "GIO_MODULE_DIR",
            "GIO_EXTRA_MODULES",
            "GSETTINGS_SCHEMA_DIR",
            "XDG_DATA_DIRS",
            "XDG_CONFIG_HOME",
            "http_proxy",
            "https_proxy",
            "APT_CONFIG",
            "DPKG_ADMINDIR",
            "TMPDIR",
            "DBUS_SESSION_BUS_ADDRESS",
            "SSL_CERT_FILE",
            "GNUTLS_SYSTEM_PRIORITY_FILE",
        ];

        for name in dangerous {
            assert!(!names.contains(&name), "{name}");
        }
        assert!(ENVIRONMENT.contains(&("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")));
        assert!(ENVIRONMENT.contains(&("DEBIAN_FRONTEND", "noninteractive")));
    }

    /// The staging folder is the helper's own, never a shared one that
    /// making it private would lock everyone else out of.
    #[test]
    fn the_staging_folder_is_the_helpers_own() {
        let folder = Path::new(STAGING_FOLDER);

        assert!(folder.is_absolute());
        assert_eq!(folder.parent(), Some(Path::new("/var/cache")));
        assert!(!["/tmp", "/var/tmp", "/var/cache", "/dev/shm", "/run"].contains(&STAGING_FOLDER));
    }

    fn watched(input: &[u8]) -> bool {
        let cancel = Cancellation::new();
        watch_input(input, &cancel);
        cancel.is_cancelled()
    }

    /// Only the exact line `cancel` stops the helper, and an endless line
    /// is neither kept nor mistaken for one.
    #[test]
    fn only_the_line_cancel_cancels() {
        assert!(watched(b"cancel\n"));
        assert!(watched(b"noise\ncancel\n"));
        assert!(!watched(b""));
        assert!(!watched(b"cancel"));
        assert!(!watched(b"cancelled\n"));
        assert!(!watched(b"xcancel\n"));
        assert!(!watched(b" cancel\n"));
        let mut long = vec![b'x'; 10_000];
        long.extend_from_slice(b"cancel\n");
        assert!(!watched(&long));
        long.extend_from_slice(b"cancel\n");
        assert!(watched(&long));
    }

    /// An output whose writes never return, as a pipe nobody reads.
    struct NeverRead(Receiver<()>);

    impl Write for NeverRead {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Err(std::io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A caller that never reads the reports cannot hold the helper:
    /// sending never waits, and finishing waits only the grace period.
    #[test]
    fn reports_never_wait_for_a_reader() {
        let (_never, blocked) = mpsc::channel();
        let started = std::time::Instant::now();
        let reporter = Reporter::start(NeverRead(blocked));

        for _ in 0..100_000 {
            reporter.send(HelperReport::Downloading.encode());
        }
        reporter.finish(Duration::from_millis(100));

        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A caller that reads gets every report, in order.
    #[test]
    fn a_reading_caller_gets_every_report() {
        let (reader, writer) = std::io::pipe().unwrap();
        let reporter = Reporter::start(writer);

        reporter.send(HelperReport::Downloading.encode() + "\n");
        reporter.send(HelperReport::Installing.encode() + "\n");
        reporter.finish(REPORT_GRACE);

        let mut text = String::new();
        std::io::BufReader::new(reader).read_to_string(&mut text).unwrap();
        let reports: Vec<_> = text.lines().filter_map(HelperReport::decode).collect();
        assert_eq!(reports, [HelperReport::Downloading, HelperReport::Installing]);
    }

    /// Only one installation runs at a time.
    #[test]
    fn a_second_installation_is_refused_while_one_holds_the_lock() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("staging");

        let held = lock_staging_folder(&folder).unwrap();
        let second = lock_staging_folder(&folder);

        assert!(matches!(second, Err(UpdateError::TaskRunning)), "{second:?}");
        drop(held);
        assert!(lock_staging_folder(&folder).is_ok());
    }

    #[test]
    fn the_helper_lives_in_libexec_beside_the_app() {
        assert_eq!(
            helper_beside(Path::new("/opt/openxplorer")),
            Path::new("/opt/openxplorer/libexec/openxplorer-install-update")
        );
    }
}
