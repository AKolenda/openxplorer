// SPDX-License-Identifier: AGPL-3.0-only
//! The command line the running instance receives: its options and
//! locations, read into one [`CommandRequest`].
//!
//! Ports `CLI_OPTIONS`, `argument_parser` and the parsing half of
//! `command_line` in `desktop/winspace.py` (INT-004). A later launch hands
//! its whole command line and working directory to the running instance
//! and exits (INT-001), so relative paths are resolved against the
//! directory the command was typed in, not the running instance's.
//! `--version`, `--restart` and `--quit` are handled before the
//! application starts ([`crate::update::LaunchCheck`]).

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use ox_core::location::{normalise, LocationError};

/// A command-line option the application registers, as `CLI_OPTIONS`
/// lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommandOption {
    /// `--new-window`: the locations open in a window of their own.
    NewWindow,
    /// `--windows`: shows the list of open windows.
    Windows,
    /// `--settings`: opens Settings.
    Settings,
    /// `--select`: reveals the files in their folders.
    Select,
    /// `--filemanager-service`: starts the opted-in Show in folder
    /// service without a window.
    FileManagerService,
    /// `--software-rendering`: draws without the GPU. Only a first
    /// launch can choose GTK's renderer, before any window exists.
    SoftwareRendering,
    /// `--quit`: closes every window once file operations finish.
    Quit,
}

impl CommandOption {
    /// Every option, in `CLI_OPTIONS` order.
    pub(super) const ALL: [Self; 7] = [
        Self::NewWindow,
        Self::Windows,
        Self::Settings,
        Self::Select,
        Self::FileManagerService,
        Self::SoftwareRendering,
        Self::Quit,
    ];

    /// The option's long name, without the dashes.
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::NewWindow => "new-window",
            Self::Windows => "windows",
            Self::Settings => "settings",
            Self::Select => "select",
            Self::FileManagerService => "filemanager-service",
            Self::SoftwareRendering => "software-rendering",
            Self::Quit => "quit",
        }
    }

    /// What `--help` says about the option, word for word from
    /// `CLI_OPTIONS`.
    pub(super) const fn description(self) -> &'static str {
        match self {
            Self::NewWindow => "Create a separate OpenXplorer window",
            Self::Windows => "Show existing OpenXplorer windows",
            Self::Settings => "Open Settings",
            Self::Select => "Reveal files in their parent folders",
            Self::FileManagerService => "Start the opted-in FileManager1 service",
            Self::SoftwareRendering => "Use software rendering for a new window",
            Self::Quit => "Close all OpenXplorer windows after file operations finish",
        }
    }
}

/// What the command line asks the running instance to do, in the order
/// `command_line` checks the options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CommandRequest {
    /// `--quit`.
    Quit,
    /// `--filemanager-service`.
    FileManagerService,
    /// `--select <files>`.
    Select(Vec<String>),
    /// `--windows`.
    Windows,
    /// `--settings`.
    Settings,
    /// `--new-window [locations]`.
    NewWindow(Vec<String>),
    /// Locations to open in the active window.
    Open(Vec<String>),
    /// A launch without options or locations.
    Activate,
}

/// Why a command line was refused. `Display` is printed to the terminal,
/// and the launch exits with status 2, as `argparse` errors did.
#[derive(Debug, thiserror::Error)]
pub(super) enum CommandLineError {
    /// `--select` without a file.
    #[error("--select needs a file path.")]
    SelectWithoutFile,
    /// A location the app cannot open.
    #[error(transparent)]
    Location(#[from] LocationError),
}

impl CommandRequest {
    /// Reads `command_line`: its options, and its locations resolved
    /// against its working directory.
    ///
    /// # Errors
    ///
    /// A [`CommandLineError`] for `--select` without a file or a location
    /// the app cannot open.
    pub(super) fn from_command_line(
        command_line: &gio::ApplicationCommandLine,
    ) -> Result<Self, CommandLineError> {
        let options = command_line.options_dict();
        let arguments = command_line.arguments();
        let locations = arguments
            .iter()
            .skip(1)
            .map(|argument| command_line.create_file_for_arg(argument))
            .map(|file| normalise(&file.uri()))
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_options(|option| options.contains(option.name()), locations)
    }

    /// The request of a command line with the options `has_option` says
    /// are given, and `locations`.
    ///
    /// # Errors
    ///
    /// [`CommandLineError::SelectWithoutFile`].
    pub(super) fn from_options(
        has_option: impl Fn(CommandOption) -> bool,
        locations: Vec<String>,
    ) -> Result<Self, CommandLineError> {
        let request = if has_option(CommandOption::Quit) {
            Self::Quit
        } else if has_option(CommandOption::FileManagerService) {
            Self::FileManagerService
        } else if has_option(CommandOption::Select) {
            if locations.is_empty() {
                return Err(CommandLineError::SelectWithoutFile);
            }
            Self::Select(locations)
        } else if has_option(CommandOption::Windows) {
            Self::Windows
        } else if has_option(CommandOption::Settings) {
            Self::Settings
        } else if has_option(CommandOption::NewWindow) {
            Self::NewWindow(locations)
        } else if locations.is_empty() {
            Self::Activate
        } else {
            Self::Open(locations)
        };
        Ok(request)
    }
}

/// Registers every option of [`CommandOption::ALL`] on `app`, and the
/// launcher's own options so `--help` lists them
/// ([`crate::update::LAUNCHER_OPTIONS`]).
pub(super) fn add_options(app: &impl IsA<gio::Application>) {
    let options = CommandOption::ALL.map(|option| (option.name(), option.description()));
    for (name, description) in options.into_iter().chain(crate::update::LAUNCHER_OPTIONS) {
        app.add_main_option(
            name,
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            description,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(options: &[CommandOption], locations: &[&str]) -> Result<CommandRequest, CommandLineError> {
        let locations = locations.iter().map(|location| (*location).to_owned()).collect();
        CommandRequest::from_options(|option| options.contains(&option), locations)
    }

    /// A command line and what it asks for.
    struct RequestCase {
        options: &'static [CommandOption],
        locations: &'static [&'static str],
        request: CommandRequest,
    }

    /// Ported from the branches of `command_line` in
    /// `desktop/winspace.py`, in their order.
    ///
    /// parity: INT-004, INT-006
    #[test]
    fn each_option_asks_for_its_own_action() {
        const DOCUMENTS: &str = "file:///home/demo/Documents";
        let documents = DOCUMENTS;
        let cases = [
            RequestCase {
                options: &[CommandOption::Quit, CommandOption::NewWindow],
                locations: &[],
                request: CommandRequest::Quit,
            },
            RequestCase {
                options: &[CommandOption::FileManagerService],
                locations: &[],
                request: CommandRequest::FileManagerService,
            },
            RequestCase {
                options: &[CommandOption::Select],
                locations: &[DOCUMENTS],
                request: CommandRequest::Select(vec![documents.to_owned()]),
            },
            RequestCase {
                options: &[CommandOption::Windows],
                locations: &[],
                request: CommandRequest::Windows,
            },
            RequestCase {
                options: &[CommandOption::Settings],
                locations: &[],
                request: CommandRequest::Settings,
            },
            RequestCase {
                options: &[CommandOption::NewWindow],
                locations: &[DOCUMENTS],
                request: CommandRequest::NewWindow(vec![documents.to_owned()]),
            },
            RequestCase {
                options: &[],
                locations: &[DOCUMENTS],
                request: CommandRequest::Open(vec![documents.to_owned()]),
            },
            RequestCase {
                options: &[CommandOption::SoftwareRendering],
                locations: &[],
                request: CommandRequest::Activate,
            },
        ];
        for case in cases {
            let parsed = request(case.options, case.locations).expect("a valid command line");
            assert_eq!(parsed, case.request, "{:?}", case.options);
        }
    }

    /// parity: INT-007
    #[test]
    fn select_needs_a_file() {
        let error = request(&[CommandOption::Select], &[]).expect_err("nothing to select");
        assert_eq!(error.to_string(), "--select needs a file path.");
    }

    #[test]
    fn the_options_keep_the_python_names() {
        let names = CommandOption::ALL.map(CommandOption::name);
        assert_eq!(
            names,
            [
                "new-window",
                "windows",
                "settings",
                "select",
                "filemanager-service",
                "software-rendering",
                "quit",
            ]
        );
    }
}
