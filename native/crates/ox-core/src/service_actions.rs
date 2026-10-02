// SPDX-License-Identifier: AGPL-3.0-only
//! Opt-in KDE `ServiceMenus` and Nautilus scripts. Definitions are data until
//! explicitly enabled. File names become argv values, never shell source.
//! Reference: <https://develop.kde.org/docs/apps/dolphin/service-menus/>
mod command;
pub use command::Command;
use gio::prelude::*;
use std::fs;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const MAX_FILES: usize = 256;
const MAX_BYTES: u64 = 64 * 1024;
const GROUP: &str = "Desktop Entry";

/// One installed action. Its identity changes whenever its definition changes.
#[derive(Debug, Clone)]
pub struct ServiceAction {
    /// Persistent opt-in key; the file and its content digest, not its label.
    pub id: String,
    /// Name in the item menu, including the installer's submenu prefix.
    pub name: String,
    /// The installed definition the user can inspect before enabling.
    pub path: PathBuf,
    exec: Option<String>,
    mime_types: Vec<String>,
    protocols: Vec<String>,
    minimum: usize,
    maximum: usize,
}

impl ServiceAction {
    /// Whether every selected item's MIME type and URI matches this action.
    pub fn accepts(&self, selection: &[(String, String)]) -> bool {
        let count = selection.len();
        count >= self.minimum
            && count <= self.maximum
            && selection.iter().all(|(uri, mime)| {
                let scheme = gio::File::for_uri(uri)
                    .uri_scheme()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                (self.protocols.is_empty() || self.protocols.contains(&scheme))
                    && (self.mime_types.is_empty()
                        || self.mime_types.iter().any(|wanted| {
                            wanted == "all/all"
                                || wanted == mime
                                || (wanted == "all/allfiles" && mime != "inode/directory")
                                || wanted
                                    .strip_suffix('*')
                                    .is_some_and(|prefix| mime.starts_with(prefix))
                                || gio::content_type_is_a(mime, wanted)
                        }))
            })
    }

    /// Expands safe standalone field codes, returning argv and script environment.
    ///
    /// # Errors
    /// Unsupported field codes, embedded substitutions or non-local script inputs.
    pub fn command(&self, uris: &[String], folder: &str) -> Result<Command, String> {
        command::build(self, uris, folder)
    }
}

/// Reads installed definitions on a worker. No directory is created or executed.
pub async fn discover() -> Vec<ServiceAction> {
    let home = glib::user_data_dir();
    let mut data = vec![home.clone()];
    data.extend(glib::system_data_dirs());
    gio::spawn_blocking(move || discover_in(&data, &home.join("nautilus/scripts")))
        .await
        .unwrap_or_default()
}

fn discover_in(data: &[PathBuf], scripts: &Path) -> Vec<ServiceAction> {
    let mut actions = Vec::new();
    let mut remaining = MAX_FILES;
    for root in data {
        for relative in ["kio/servicemenus", "kservices5/ServiceMenus"] {
            for path in files(&root.join(relative), 0, &mut remaining) {
                if path.extension().is_some_and(|extension| extension == "desktop") {
                    actions.extend(desktop_actions(&path));
                }
            }
        }
    }
    for path in files(scripts, 3, &mut remaining) {
        if fs::symlink_metadata(&path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0) {
            if let Some(id) = definition(&path).and_then(|bytes| identity(&path, &bytes)) {
                let name = path.strip_prefix(scripts).unwrap_or(&path).display().to_string();
                actions.push(ServiceAction {
                    id,
                    name,
                    path,
                    exec: None,
                    mime_types: Vec::new(),
                    protocols: vec!["file".into()],
                    minimum: 1,
                    maximum: MAX_FILES,
                });
            }
        }
    }
    actions.truncate(MAX_FILES);
    actions.sort_by(|a, b| a.name.cmp(&b.name));
    actions
}

fn files(folder: &Path, depth: usize, remaining: &mut usize) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if !fs::symlink_metadata(folder).is_ok_and(|m| m.is_dir()) {
        return result;
    }
    let Ok(entries) = fs::read_dir(folder) else {
        return result;
    };
    for entry in entries.flatten() {
        if *remaining == 0 {
            break;
        }
        *remaining -= 1;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() && depth > 0 {
            result.extend(files(&path, depth - 1, remaining));
        }
        if metadata.is_file() && metadata.len() <= MAX_BYTES && metadata.permissions().mode() & 0o022 == 0 {
            result.push(path);
        }
    }
    result
}

fn definition(path: &Path) -> Option<Vec<u8>> {
    let flags = i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits()).ok()?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES || metadata.permissions().mode() & 0o022 != 0 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= usize::try_from(MAX_BYTES).ok()?).then_some(bytes)
}

fn identity(path: &Path, bytes: &[u8]) -> Option<String> {
    let digest = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, bytes)?;
    Some(format!("{}#{digest}", path.display()))
}

fn list(key: &glib::KeyFile, field: &str) -> Vec<String> {
    key.string(GROUP, field)
        .map(|value| {
            value
                .split(';')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn desktop_actions(path: &Path) -> Vec<ServiceAction> {
    let Some(bytes) = definition(path) else {
        return Vec::new();
    };
    let Ok(data) = std::str::from_utf8(&bytes) else {
        return Vec::new();
    };
    let Some(id) = identity(path, &bytes) else {
        return Vec::new();
    };
    let key = glib::KeyFile::new();
    if key.load_from_data(data, glib::KeyFileFlags::NONE).is_err()
        || key.boolean(GROUP, "Hidden").unwrap_or(false)
    {
        return Vec::new();
    }
    if !key.string(GROUP, "Type").is_ok_and(|kind| kind == "Service") {
        return Vec::new();
    }
    if let Ok(program) = key.string(GROUP, "TryExec") {
        if glib::find_program_in_path(program.as_str()).is_none() {
            return Vec::new();
        }
    }
    // Terminal activation and KDE's dynamic visibility code need KDE itself.
    if key.boolean(GROUP, "Terminal").unwrap_or(false)
        || key.has_key(GROUP, "X-KDE-ShowIfRunning").unwrap_or(false)
    {
        return Vec::new();
    }
    let mime_types = list(&key, "MimeType");
    if mime_types.is_empty() {
        return Vec::new();
    }
    let protocols = list(&key, "X-KDE-Protocols");
    let minimum = key
        .uint64(GROUP, "X-KDE-MinNumberOfUrls")
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(1);
    let maximum = key
        .uint64(GROUP, "X-KDE-MaxNumberOfUrls")
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0)
        .unwrap_or(MAX_FILES);
    let prefix = key
        .locale_string(GROUP, "X-KDE-Submenu", None)
        .unwrap_or_default();
    list(&key, "Actions")
        .into_iter()
        .filter_map(|action| {
            let group = format!("Desktop Action {action}");
            let name = key.locale_string(&group, "Name", None).ok()?;
            let exec = key.string(&group, "Exec").ok()?.to_string();
            let words = command::validate(&exec).ok()?;
            let maximum = if words.iter().any(|word| word == "%f" || word == "%u") {
                maximum.min(1)
            } else {
                maximum
            };
            Some(ServiceAction {
                id: format!("{id}:{action}"),
                name: if prefix.is_empty() {
                    name.into()
                } else {
                    format!("{prefix} / {name}")
                },
                path: path.into(),
                exec: Some(exec),
                mime_types: mime_types.clone(),
                protocols: protocols.clone(),
                minimum,
                maximum,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// parity: CMD-024
    #[test]
    fn installed_actions_are_filtered_and_changed_definitions_lose_their_opt_in_identity() {
        let temp = tempfile::tempdir().unwrap();
        let services = temp.path().join("kio/servicemenus");
        fs::create_dir_all(&services).unwrap();
        let path = services.join("test.desktop");
        let definition = "[Desktop Entry]\nType=Service\nMimeType=text/plain;\nActions=view;\n[Desktop Action view]\nName=View safely\nExec=/usr/bin/cat %F\n";
        fs::write(&path, definition).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let actions = discover_in(&[temp.path().into()], &temp.path().join("scripts"));
        assert_eq!(actions.len(), 1);
        assert!(actions[0].accepts(&[("file:///tmp/notes".into(), "text/plain".into())]));
        assert!(!actions[0].accepts(&[("file:///tmp/folder".into(), "inode/directory".into())]));
        let argv = actions[0]
            .command(&["file:///tmp/%24%28touch%20oops%29".into()], "file:///tmp")
            .unwrap()
            .argv;
        assert_eq!(argv[1], "/tmp/$(touch oops)");
        fs::write(path, definition.replace("View safely", "Changed")).unwrap();
        assert_ne!(
            actions[0].id,
            discover_in(&[temp.path().into()], &temp.path().join("scripts"))[0].id
        );
    }
    #[test]
    fn nautilus_scripts_receive_separate_paths_and_expected_environment() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("Inspect files");
        fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let actions = discover_in(&[], temp.path());
        assert_eq!(actions.len(), 1);
        let command = actions[0]
            .command(
                &[
                    "file:///tmp/one%20file".into(),
                    "file:///tmp/%24%28example%29".into(),
                ],
                "file:///tmp",
            )
            .unwrap();
        assert_eq!(command.argv[1..], ["/tmp/one file", "/tmp/$(example)"]);
        assert!(command.environment.contains(&(
            "NAUTILUS_SCRIPT_SELECTED_FILE_PATHS".into(),
            "/tmp/one file\n/tmp/$(example)\n".into()
        )));
        assert!(actions[0]
            .command(&["smb://server/share/file".into()], "file:///tmp")
            .is_err());
    }
}
