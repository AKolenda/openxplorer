// SPDX-License-Identifier: AGPL-3.0-only
//! Desktop integration: default applications, "Show in folder", Brave's
//! download folder, opening files and Open in Terminal.
//!
//! Ports `desktop/desktop_integration.py`, `desktop/filemanager_bus.py`,
//! `desktop/reveal_integration.py`, `desktop/brave_integration.py`,
//! `desktop/activation.py`, `desktop/native_opening.py`
//! (`prepare_default`), `desktop/terminal_integration.py`,
//! `desktop/app_catalog.py` and `filemanager_request` of
//! `desktop/window_state.py`. Nothing here depends on GTK.
//!
//! The rules every part keeps:
//!
//! - **Opt-in.** Nothing changes a default, writes a session file or
//!   edits Brave's preferences until the user asks; reading a status only
//!   reads (INT-010).
//! - **Compatibility contracts** (AGENTS.md). The desktop ID
//!   [`APP_ID`], the MIME types of [`MimeType`], the D-Bus names
//!   [`BUS_NAME`] and [`OBJECT_PATH`], the managed marker and the
//!   `/usr/bin/winspace` command of the session files stay byte for byte
//!   what the Python app uses, so either app recognises the other's work.
//! - **Reversible and private.** Previous handlers and preferences are
//!   recorded before they are replaced, records and backups are 0600 files
//!   written atomically, and restoring never overwrites a later choice.
//! - **Data, never commands.** Locations from other applications or the
//!   window are validated data; programs are started with argument lists
//!   and never through a shell.
//! - **Flatpak.** Inside a Flatpak sandbox ([`Sandbox`]) host files are
//!   never written directly: `xdg-mime` and the terminal run on the host
//!   through `flatpak-spawn --host`, files open through the desktop
//!   portal, "Show in folder" keeps its opt-in inside the sandbox and
//!   asks the Background portal to start the app at login, and the Brave
//!   sync, which must write host files, refuses.
//!
//! Blocking work runs on GIO worker threads: each service has a
//! `run_in_background`, `prepare_in_background` or
//! `open_terminal_in_background` that the main loop can await, and the
//! operations that query a location take a
//! [`Cancellation`](crate::transfer::Cancellation). The
//! `org.freedesktop.FileManager1` service lives on the main thread.
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | `default_apps` | Default file manager and ZIP handler | `desktop_integration.py` |
//! | `reveal` | The "Show in folder" session files | `reveal_integration.py` |
//! | `background_portal` | Starting at login from inside Flatpak | (new) |
//! | `file_manager_bus` | The `org.freedesktop.FileManager1` service | `filemanager_bus.py` |
//! | `file_manager_request` | Checking FileManager1 requests | `window_state.py` |
//! | `brave` | Brave's download folder | `brave_integration.py` |
//! | `activation`, `opening` | What activating does; opening a file | `activation.py`, `native_opening.py` |
//! | `applications`, `app_catalog` | Installed applications, Open with, editors | `app_catalog.py` |
//! | `terminal` | Open in Terminal | `terminal_integration.py` |
//! | `mime_type` | The handled MIME types | `desktop_integration.py`, `activation.py` |
//! | `sandbox`, `host_command` | Flatpak detection; running host programs | (new) |
//! | `private_file`, `worker` | Atomic private writes; worker threads | the Python modules' helpers |

mod activation;
mod app_catalog;
mod applications;
mod background_portal;
mod brave;
mod default_apps;
mod file_manager_bus;
mod file_manager_request;
mod host_command;
mod mime_type;
mod opening;
mod private_file;
mod reveal;
mod sandbox;
mod terminal;
mod worker;

pub use activation::{choose_application, Activation, OpenError};
pub use app_catalog::{editor_shortcuts, unique_applications, EditorShortcut};
pub use applications::{ApplicationDatabase, ApplicationInfo, InstalledApplications};
pub use background_portal::{request_autostart, AutostartRequest, BackgroundError, PORTAL_NAME};
pub use brave::{
    BraveActivity, BraveChannel, BraveError, BraveIntegration, BravePaths, BraveProfile, BraveReach,
    BraveStatus, Confirmation, DownloadPreference, ProcessTable, ProfileFailure, SandboxedBrave, SyncOutcome,
    MANUAL_SETTINGS_URL,
};
pub use default_apps::{
    DefaultApps, DefaultAppsError, DefaultsStatus, DesktopId, MimeDefaults, RestoreScope, XdgMime,
    ZipAssociation, APP_ID, RESTORE_NOTE,
};
pub use file_manager_bus::{
    BusStatus, FileManagerBus, RegistrationFailed, RequestNotOpened, BUS_NAME, OBJECT_PATH,
};
pub use file_manager_request::{
    FileManagerMethod, FileManagerRequest, FileManagerRequestError, MAX_REQUEST_LOCATIONS,
};
pub use mime_type::MimeType;
pub use opening::{DefaultOpener, Launcher, OpenTarget, PreparedOpen};
pub use reveal::{
    DisabledReveal, RevealError, RevealPaths, RevealRegistration, AUTOSTART_FILE, FLATPAK_OPT_IN_FILE,
    MANAGED_MARKER, SERVICE_FILE,
};
pub use sandbox::Sandbox;
pub use terminal::{
    checked_directory, find_terminal, launch_terminal, open_terminal_in_background, prepare_directory,
    terminal_arguments, DirectoryChecks, ExecutableSearch, LaunchedTerminal, PreparedDirectory, Terminal,
    TerminalError, TerminalKind, SYSTEM_PATH,
};
