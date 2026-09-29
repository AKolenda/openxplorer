// SPDX-License-Identifier: AGPL-3.0-only
//! Sharing a local folder on the network with Samba user shares
//! (NET-035).
//!
//! Dolphin's Properties → Share tab (kdenetwork-filesharing) runs Samba's
//! `net usershare`; so does this module. User shares let a member of the
//! `sambashare` group share their own folders without administrator
//! rights; where Samba is missing or user shares are not permitted,
//! `net usershare info` fails and sharing is reported as unavailable.
//!
//! The command runs with an argument list, never through a shell, and only
//! ever shares or stops sharing a folder on this computer: it never
//! changes permissions on another server. Every call blocks, so callers
//! run it on a worker thread.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Characters Samba refuses in a share name (`validate_net_name`).
const FORBIDDEN_NAME_CHARACTERS: &str = "%<>*?|/\\+=;:\",";
/// The longest share name Samba accepts.
const MAX_NAME_CHARS: usize = 80;

/// One folder shared with Samba user shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usershare {
    /// The share name others see: `\\computer\name`.
    pub name: String,
    /// The shared folder.
    pub path: PathBuf,
    /// The comment others see.
    pub comment: String,
    /// Whether guests may connect without an account.
    pub allows_guests: bool,
    /// Whether others may only read.
    pub is_read_only: bool,
}

/// Why a sharing request failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsershareError {
    /// Samba is missing or user shares are not permitted for this user.
    #[error(
        "Folder sharing needs Samba with user shares enabled, and your account in the sambashare \
         group. {0}"
    )]
    Unavailable(String),
    /// The share name is empty, too long or has a character Samba refuses.
    #[error("Use a share name of up to 80 characters without % < > * ? | / \\ + = ; : \" or ,.")]
    InvalidName,
    /// Samba refused the request; the text is Samba's.
    #[error("Samba could not change the share: {0}")]
    Refused(String),
}

/// Samba's `net` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usershares {
    program: PathBuf,
}

impl Default for Usershares {
    fn default() -> Self {
        Self::with_program("net")
    }
}

impl Usershares {
    /// Runs `program` instead of `net`, for tests.
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Every user share of this computer.
    ///
    /// # Errors
    ///
    /// [`UsershareError::Unavailable`] when Samba is missing or user shares
    /// are not permitted.
    pub fn list(&self) -> Result<Vec<Usershare>, UsershareError> {
        let listed = self.run(["usershare", "info", "-l"]).map_err(UsershareError::Unavailable)?;
        Ok(parse_info(&listed))
    }

    /// The share of `folder`, if it is shared.
    ///
    /// # Errors
    ///
    /// As [`Self::list`].
    pub fn share_of(&self, folder: &Path) -> Result<Option<Usershare>, UsershareError> {
        Ok(self.list()?.into_iter().find(|share| share.path == folder))
    }

    /// Shares `share.path` as `share` says, replacing a share of the same
    /// name.
    ///
    /// # Errors
    ///
    /// [`UsershareError::InvalidName`] for a name Samba refuses, and
    /// [`UsershareError::Refused`] with Samba's reason.
    pub fn share(&self, share: &Usershare) -> Result<(), UsershareError> {
        validate_share_name(&share.name)?;
        let access = if share.is_read_only { "Everyone:R" } else { "Everyone:F" };
        let guests = if share.allows_guests { "guest_ok=y" } else { "guest_ok=n" };
        let args: [&OsStr; 7] = [
            "usershare".as_ref(),
            "add".as_ref(),
            share.name.as_ref(),
            share.path.as_os_str(),
            share.comment.as_ref(),
            access.as_ref(),
            guests.as_ref(),
        ];
        self.run(args).map(drop).map_err(UsershareError::Refused)
    }

    /// Stops sharing the share called `name`.
    ///
    /// # Errors
    ///
    /// [`UsershareError::Refused`] with Samba's reason.
    pub fn unshare(&self, name: &str) -> Result<(), UsershareError> {
        self.run(["usershare", "delete", name])
            .map(drop)
            .map_err(UsershareError::Refused)
    }

    /// Runs the command with `args`; its output, or its error text.
    fn run<I, S>(&self, args: I) -> Result<String, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = Command::new(&self.program)
            .args(args)
            .env("LC_ALL", "C")
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let said = if said.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        } else {
            said
        };
        Err(said)
    }
}

/// Checks a share name as Samba does.
///
/// # Errors
///
/// [`UsershareError::InvalidName`] for an empty or too long name, or one
/// with a character Samba refuses or a control character.
pub fn validate_share_name(name: &str) -> Result<(), UsershareError> {
    let is_valid = !name.trim().is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name
            .chars()
            .any(|character| character.is_control() || FORBIDDEN_NAME_CHARACTERS.contains(character));
    if is_valid {
        Ok(())
    } else {
        Err(UsershareError::InvalidName)
    }
}

/// Parses `net usershare info -l`: an INI section per share.
fn parse_info(text: &str) -> Vec<Usershare> {
    let mut shares: Vec<Usershare> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            shares.push(Usershare {
                name: name.to_owned(),
                path: PathBuf::new(),
                comment: String::new(),
                allows_guests: false,
                is_read_only: true,
            });
            continue;
        }
        let (Some(share), Some((key, value))) = (shares.last_mut(), line.split_once('=')) else {
            continue;
        };
        match key {
            "path" => share.path = PathBuf::from(value),
            "comment" => value.clone_into(&mut share.comment),
            "guest_ok" => share.allows_guests = value.eq_ignore_ascii_case("y"),
            "usershare_acl" => share.is_read_only = !value.to_ascii_uppercase().contains(":F"),
            _ => {}
        }
    }
    shares
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A fake `net` that records its arguments and prints `listing`, or
    /// fails with `refusal` when that is not empty.
    fn fake_net(folder: &Path, listing: &str, refusal: &str) -> Usershares {
        let program = folder.join("net");
        let log = folder.join("calls");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ -n '{refusal}' ]; then echo '{refusal}' >&2; exit \
             255; fi\ncat <<'EOF'\n{listing}\nEOF\n",
            log.display()
        );
        fs::write(&program, script).expect("the fake net");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("executable");
        Usershares::with_program(program)
    }

    fn calls(folder: &Path) -> Vec<String> {
        let log = fs::read_to_string(folder.join("calls")).unwrap_or_default();
        log.lines().map(str::to_owned).collect()
    }

    /// parity: NET-035
    #[test]
    fn a_folder_is_shared_read_only_by_default_and_its_state_is_read_back() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let listing = "[Projects]\npath=/home/anna/Projects\ncomment=Team files\nusershare_acl=Everyone:R,\n\
                       guest_ok=y\n\n[Music]\npath=/home/anna/Music\ncomment=\nusershare_acl=Everyone:F,\n\
                       guest_ok=n";
        let net = fake_net(folder.path(), listing, "");

        let shared = net.share_of(Path::new("/home/anna/Projects")).expect("listed");
        let expected = Usershare {
            name: "Projects".into(),
            path: "/home/anna/Projects".into(),
            comment: "Team files".into(),
            allows_guests: true,
            is_read_only: true,
        };
        assert_eq!(shared, Some(expected.clone()));
        let music = net.share_of(Path::new("/home/anna/Music")).expect("listed");
        assert!(music.is_some_and(|music| !music.is_read_only && !music.allows_guests));

        net.share(&expected).expect("shared");
        net.unshare("Projects").expect("unshared");
        let recorded = calls(folder.path());
        assert_eq!(
            recorded[2..],
            [
                "usershare add Projects /home/anna/Projects Team files Everyone:R guest_ok=y",
                "usershare delete Projects",
            ]
        );
        assert_eq!(
            net.share(&Usershare {
                name: "a/b".into(),
                ..expected
            }),
            Err(UsershareError::InvalidName)
        );
    }

    /// parity: NET-035
    #[test]
    fn sharing_is_unavailable_without_permitted_user_shares() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let refusal = "net usershare: cannot open usershare directory. Error Permission denied";
        let net = fake_net(folder.path(), "", refusal);

        let listed = net.list();

        assert_eq!(listed, Err(UsershareError::Unavailable(refusal.to_owned())));
        let missing = Usershares::with_program(folder.path().join("no-such-net"));
        assert!(matches!(missing.list(), Err(UsershareError::Unavailable(_))));
    }
}
