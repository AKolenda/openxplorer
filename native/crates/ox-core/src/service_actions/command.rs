// SPDX-License-Identifier: AGPL-3.0-only
//! Strict argv expansion. Shell snippets with embedded file codes are unsupported.
use super::ServiceAction;
use gio::prelude::*;
use std::ffi::OsString;
use std::path::PathBuf;

/// A reviewed program invocation, without a shell.
#[derive(Debug)]
pub struct Command {
    /// Program followed by arguments. File names stay separate values.
    pub argv: Vec<OsString>,
    /// The current local folder when available.
    pub directory: Option<PathBuf>,
    /// Nautilus script environment; empty for KDE actions.
    pub environment: Vec<(String, String)>,
}

pub(super) fn validate(exec: &str) -> Result<Vec<OsString>, String> {
    let words = glib::shell_parse_argv(exec).map_err(|e| e.to_string())?;
    let Some(program) = words.first() else {
        return Err("No program is specified.".into());
    };
    let program = PathBuf::from(program);
    let basename = program.file_name().unwrap_or_default().to_string_lossy();
    if [
        "sh", "bash", "dash", "zsh", "fish", "env", "perl", "ruby", "node", "python", "python3", "python2",
    ]
    .contains(&basename.as_ref())
        || basename.starts_with("python3.")
    {
        return Err("Use an executable script instead of an interpreter command.".into());
    }
    if program.to_string_lossy().contains('%') {
        return Err("The program must be fixed.".into());
    }
    for word in words.iter().skip(1) {
        let word = word.to_string_lossy();
        if word.contains('%') && !["%f", "%F", "%u", "%U", "%c", "%k", "%%"].contains(&word.as_ref()) {
            return Err("File field codes must be standalone arguments.".into());
        }
    }
    Ok(words)
}

pub(super) fn build(action: &ServiceAction, uris: &[String], folder: &str) -> Result<Command, String> {
    let paths: Option<Vec<PathBuf>> = uris.iter().map(|uri| gio::File::for_uri(uri).path()).collect();
    let directory = gio::File::for_uri(folder).path();
    let mut environment = Vec::new();
    let argv = if let Some(exec) = &action.exec {
        let words = validate(exec)?;
        let mut argv = Vec::new();
        for word in words {
            match word.to_str().unwrap_or_default() {
                "%F" | "%f" => {
                    let paths = paths.as_ref().ok_or("This service requires local files.")?;
                    let count = if word == "%f" { 1 } else { paths.len() };
                    argv.extend(paths.iter().take(count).map(|path| path.as_os_str().to_owned()));
                }
                "%U" => argv.extend(uris.iter().map(OsString::from)),
                "%u" => argv.extend(uris.first().map(OsString::from)),
                "%c" => argv.push(OsString::from(&action.name)),
                "%k" => argv.push(action.path.as_os_str().to_owned()),
                "%%" => argv.push(OsString::from("%")),
                _ => argv.push(word),
            }
        }
        argv
    } else {
        let paths = paths.as_ref().ok_or("Nautilus scripts require local files.")?;
        if paths
            .iter()
            .any(|path| path.to_string_lossy().contains(['\n', '\r']))
        {
            return Err("Nautilus script variables cannot represent newline-containing names.".into());
        }
        environment.push((
            "NAUTILUS_SCRIPT_SELECTED_FILE_PATHS".into(),
            paths
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        ));
        environment.push(("NAUTILUS_SCRIPT_SELECTED_URIS".into(), uris.join("\n") + "\n"));
        environment.push(("NAUTILUS_SCRIPT_CURRENT_URI".into(), folder.into()));
        std::iter::once(action.path.as_os_str().to_owned())
            .chain(paths.iter().map(|path| path.as_os_str().to_owned()))
            .collect()
    };
    Ok(Command {
        argv,
        directory,
        environment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filenames_cannot_become_shell_source() {
        assert!(validate("sh -c %f").is_err());
        assert!(validate("sh -c 'cat %f'").is_err());
        assert!(validate("cat --option=%f").is_err());
        assert!(validate("cat %F").is_ok());
    }
}
