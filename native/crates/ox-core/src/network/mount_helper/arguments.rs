// SPDX-License-Identifier: AGPL-3.0-only
//! The helper's command line, as `argparse` read it in `mount_share.py`:
//! `--share //server/share`, required, and the flags `--remove` and
//! `--plan`.

/// The one-line usage `argparse` printed.
pub(super) const USAGE: &str = "usage: openxplorer-mount-share [-h] --share SHARE [--remove] [--plan]";

/// The `--help` text: the usage, what the helper does and its options.
pub(super) const HELP: &str = "usage: openxplorer-mount-share [-h] --share SHARE [--remove] [--plan]

Explicit administrator-only setup for a persistent, on-demand SMB3 mount.

NOT called automatically by the GUI. Review the printed plan; this tool prompts
in the terminal. It never edits fstab or moves Downloads/Documents. Passwords
are stored in a root-only credentials file for mount.cifs, NOT the user keyring.

options:
  -h, --help     show this help message and exit
  --share SHARE  //server/share (no password)
  --remove       Remove this managed mount, not its files
  --plan         Print the plan without making changes";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Request {
    /// `-h` or `--help`.
    Help,
    /// Set up, remove or print the plan of a managed mount.
    Run(Arguments),
}

/// The options of a run.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Arguments {
    /// The share, `//server/share` or any address the assistant accepts.
    pub(super) share: String,
    /// `--remove`: remove the managed mount, not its files.
    pub(super) remove: bool,
    /// `--plan`: only print the plan. It wins over `--remove`.
    pub(super) plan_only: bool,
}

/// A command line the helper does not understand, in `argparse`'s words.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum UsageError {
    /// No `--share`.
    #[error("the following arguments are required: --share")]
    MissingShare,
    /// `--share` as the last argument.
    #[error("argument --share: expected one argument")]
    MissingValue,
    /// Anything else.
    #[error("unrecognized arguments: {0}")]
    Unrecognized(String),
}

/// Reads the command-line `arguments`, without the program name. As with
/// `argparse`, the last `--share` wins and `-h` answers before anything is
/// checked.
///
/// # Errors
///
/// A [`UsageError`] naming what is missing or not understood.
pub(super) fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Request, UsageError> {
    let mut share = None;
    let mut remove = false;
    let mut plan_only = false;
    let mut unrecognized = Vec::new();
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(Request::Help),
            "--remove" => remove = true,
            "--plan" => plan_only = true,
            "--share" => share = Some(arguments.next().ok_or(UsageError::MissingValue)?),
            _ => match argument.strip_prefix("--share=") {
                Some(value) => share = Some(value.to_owned()),
                None => unrecognized.push(argument),
            },
        }
    }
    if !unrecognized.is_empty() {
        return Err(UsageError::Unrecognized(unrecognized.join(" ")));
    }
    let share = share.ok_or(UsageError::MissingShare)?;
    Ok(Request::Run(Arguments {
        share,
        remove,
        plan_only,
    }))
}
