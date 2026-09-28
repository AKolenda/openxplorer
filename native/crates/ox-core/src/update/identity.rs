// SPDX-License-Identifier: AGPL-3.0-only
//! The identity of an installed or running build, used to notice that a
//! package upgrade replaced the files under a running process. Ports
//! `identity`, `PROTOCOL` and `same_build` in `desktop/runtime_guard.py`,
//! and the `runtime-info` action state of `desktop/winspace.py`.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::release::sha256_checksum;

/// The version of the identity format and of the single-instance D-Bus
/// protocol.
pub const RUNTIME_PROTOCOL: u32 = 1;

/// File types of the Python app's interface folder that belong to a build.
const PYTHON_UI_SUFFIXES: [&str; 5] = ["py", "html", "js", "css", "svg"];

/// Which build is installed or running: its version, protocol and a
/// SHA-256 digest of its files.
///
/// Serialised, this is the JSON state of the `runtime-info` action, in the
/// Python app's format. A running instance's report may lack `protocol`
/// or `build` (early 1.0 releases); they then read as 0 and "", which
/// match no installed build, as Python's `same_build` compares them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    /// The application version, for example "1.1.4".
    pub version: String,
    /// [`RUNTIME_PROTOCOL`] for current builds.
    #[serde(default)]
    pub protocol: u32,
    /// The hexadecimal SHA-256 digest of the build's files.
    #[serde(default)]
    pub build: String,
}

impl RuntimeIdentity {
    /// The identity of the Python app installed in `root`, exactly as
    /// `identity` in `desktop/runtime_guard.py` computes it: every `*.py`
    /// in `root`, then every `.py`, `.html`, `.js`, `.css` and `.svg`
    /// file in `root/ui`, each in name order, each hashed as its relative
    /// path, a NUL byte and its contents.
    ///
    /// # Errors
    ///
    /// Any error listing a folder or reading a file.
    pub fn of_python_install(root: &Path, version: &str) -> io::Result<Self> {
        let ui = Path::new("ui");
        let mut inputs = file_names_with_suffixes(root, &["py"])?;
        let ui_names = file_names_with_suffixes(&root.join(ui), &PYTHON_UI_SUFFIXES)?;
        inputs.extend(ui_names.into_iter().map(|name| ui.join(name)));
        let mut checksum = sha256_checksum();
        for relative in inputs {
            checksum.update(relative.as_os_str().as_encoded_bytes());
            checksum.update(b"\0");
            add_file_contents(&mut checksum, &root.join(&relative))?;
        }
        Ok(Self::with_digest(version, checksum))
    }

    /// The identity of the native app's executable at `path`: its
    /// version and the SHA-256 digest of the file. A package upgrade
    /// replaces the file, so a running process whose executable changed
    /// no longer matches the installed one.
    ///
    /// # Errors
    ///
    /// Any error reading the file.
    pub fn of_executable(path: &Path, version: &str) -> io::Result<Self> {
        let mut checksum = sha256_checksum();
        add_file_contents(&mut checksum, path)?;
        Ok(Self::with_digest(version, checksum))
    }

    /// Reads the state of a running instance's `runtime-info` action;
    /// `None` unless it is a JSON object with a text `version`, as
    /// `Session.running` in `runtime_guard.py` requires.
    pub fn from_action_state(state: &str) -> Option<Self> {
        let report: serde_json::Value = serde_json::from_str(state).ok()?;
        // Serde would also read a struct from a JSON array.
        if !report.is_object() {
            return None;
        }
        serde_json::from_value(report).ok()
    }

    /// The state of this process's `runtime-info` action.
    ///
    /// # Panics
    ///
    /// Never: an identity is two strings and a number.
    pub fn to_action_state(&self) -> String {
        serde_json::to_string(self).expect("an identity is two strings and a number")
    }

    fn with_digest(version: &str, checksum: glib::Checksum) -> Self {
        Self {
            version: version.to_owned(),
            protocol: RUNTIME_PROTOCOL,
            build: checksum
                .string()
                .expect("a SHA-256 checksum has a hexadecimal form"),
        }
    }
}

/// Where the installed build is, so its identity can be read again after
/// an installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledBuild {
    /// The Python app's files in a folder.
    PythonInstall {
        /// The installation folder, `/opt/openxplorer` for the package.
        root: PathBuf,
        /// The version the files carry.
        version: String,
    },
    /// The native app's executable.
    Executable {
        /// The installed executable.
        path: PathBuf,
        /// The version it carries.
        version: String,
    },
}

impl InstalledBuild {
    /// The identity of what is installed now.
    ///
    /// # Errors
    ///
    /// Any error reading the installed files.
    pub fn read_identity(&self) -> io::Result<RuntimeIdentity> {
        match self {
            Self::PythonInstall { root, version } => RuntimeIdentity::of_python_install(root, version),
            Self::Executable { path, version } => RuntimeIdentity::of_executable(path, version),
        }
    }
}

/// Adds the contents of the file at `path` to `checksum`, streamed: a
/// release executable is tens of megabytes, and is never held in memory
/// whole.
fn add_file_contents(checksum: &mut glib::Checksum, path: &Path) -> io::Result<()> {
    let mut file = File::open(path)?;
    io::copy(&mut file, &mut ChecksumWriter(checksum))?;
    Ok(())
}

/// Adds everything written to it to a checksum.
struct ChecksumWriter<'a>(&'a mut glib::Checksum);

impl Write for ChecksumWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The names of the regular files in `folder` whose suffix is one of
/// `suffixes`, in name order. A missing folder has none, as Python's
/// `glob` finds none.
///
/// Python's `pathlib` glob also matches hidden files, and so does this.
fn file_names_with_suffixes(folder: &Path, suffixes: &[&str]) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let has_suffix = path
            .extension()
            .and_then(|suffix| suffix.to_str())
            .is_some_and(|suffix| suffixes.contains(&suffix));
        if has_suffix && path.is_file() {
            files.extend(path.file_name().map(PathBuf::from));
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_round_trips_through_its_action_state() {
        let identity = RuntimeIdentity {
            version: "1.1.4".to_owned(),
            protocol: RUNTIME_PROTOCOL,
            build: "abc".to_owned(),
        };

        let state = identity.to_action_state();

        assert_eq!(state, r#"{"version":"1.1.4","protocol":1,"build":"abc"}"#);
        assert_eq!(RuntimeIdentity::from_action_state(&state), Some(identity));
    }

    #[test]
    fn a_report_needs_a_text_version_but_not_the_other_fields() {
        let early = RuntimeIdentity::from_action_state(r#"{"version": "1.0.0", "extra": true}"#).unwrap();

        assert_eq!((early.protocol, early.build.as_str()), (0, ""));
        assert!(RuntimeIdentity::from_action_state(r#"{"version": 1}"#).is_none());
        assert!(RuntimeIdentity::from_action_state(r#"["1.0.0"]"#).is_none());
        assert!(RuntimeIdentity::from_action_state("not JSON").is_none());
    }

    #[test]
    fn an_executable_identity_follows_its_contents() {
        let folder = tempfile::tempdir().unwrap();
        let executable = folder.path().join("openxplorer");
        fs::write(&executable, b"first build").unwrap();
        let first = RuntimeIdentity::of_executable(&executable, "2.0.0").unwrap();

        fs::write(&executable, b"second build").unwrap();
        let second = RuntimeIdentity::of_executable(&executable, "2.0.0").unwrap();

        assert_eq!(first.build.len(), 64);
        assert_ne!(first, second);
    }

    #[test]
    fn a_streamed_executable_digest_is_the_digest_of_the_whole_file() {
        let folder = tempfile::tempdir().unwrap();
        let executable = folder.path().join("openxplorer");
        let contents: Vec<u8> = (0..300_000_u32).map(|index| index.to_le_bytes()[0]).collect();
        fs::write(&executable, &contents).unwrap();

        let identity = RuntimeIdentity::of_executable(&executable, "2.0.0").unwrap();

        let whole = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &contents).unwrap();
        assert_eq!(identity.build, whole.as_str());
    }
}
