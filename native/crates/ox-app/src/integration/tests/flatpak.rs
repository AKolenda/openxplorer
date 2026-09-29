// SPDX-License-Identifier: AGPL-3.0-only
//! Show in folder inside Flatpak: the running app owns `FileManager1`
//! and asks the Background portal to start it at login. A fake portal on
//! a second connection to the test's private session bus answers under
//! its unique name, so no real portal is reached and no autostart entry
//! is written.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::integration::{FileManagerMethod, DESKTOP_PORTAL_PATH};

use super::{wait_for, AttachedIntegration};
use crate::integration::DesktopIntegration;
use crate::test_support::harness::wait_until;
use crate::test_support::portal::ExportedPortal;

/// The part of `org.freedesktop.portal.Background` the app uses.
const BACKGROUND_XML: &str = r#"<node>
  <interface name="org.freedesktop.portal.Background">
    <method name="RequestBackground">
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="o" name="handle" direction="out"/>
    </method>
  </interface>
</node>"#;

/// How the fake portal answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// Grants what was asked.
    Grant,
    /// Grants it on another request object than the token names, as a
    /// portal may.
    GrantOnAnotherRequest,
    /// The user refuses.
    Refuse,
}

/// What the app asked the portal.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Asked {
    autostart: Option<bool>,
    commandline: Vec<String>,
    reason: String,
}

/// A Background portal exported on its own connection to the session
/// bus, which answers every request with `answer`.
struct FakeBackgroundPortal {
    portal: ExportedPortal,
    asked: Rc<RefCell<Vec<Asked>>>,
}

impl FakeBackgroundPortal {
    fn start(answer: Answer) -> Self {
        let asked: Rc<RefCell<Vec<Asked>>> = Rc::default();
        let recorder = Rc::clone(&asked);
        let portal = ExportedPortal::export(
            BACKGROUND_XML,
            "org.freedesktop.portal.Background",
            move |connection, sender, _, _, _, parameters, invocation| {
                let Some((_, options)) = parameters.get::<(String, glib::VariantDict)>() else {
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", "(sa{sv})");
                    return;
                };
                let autostart = options.lookup::<bool>("autostart").ok().flatten();
                recorder.borrow_mut().push(Asked {
                    autostart,
                    commandline: options.lookup("commandline").ok().flatten().unwrap_or_default(),
                    reason: options.lookup("reason").ok().flatten().unwrap_or_default(),
                });
                let token: String = options.lookup("handle_token").ok().flatten().unwrap_or_default();
                let caller = sender
                    .unwrap_or_default()
                    .trim_start_matches(':')
                    .replace('.', "_");
                let token = if answer == Answer::GrantOnAnotherRequest {
                    "chosen_by_the_portal".to_owned()
                } else {
                    token
                };
                let handle = format!("{DESKTOP_PORTAL_PATH}/request/{caller}/{token}");
                let path = glib::variant::ObjectPath::try_from(handle.clone()).expect("a valid path");
                invocation.return_value(Some(&(path,).to_variant()));
                let (code, granted) = match answer {
                    Answer::Refuse => (1u32, false),
                    Answer::Grant | Answer::GrantOnAnotherRequest => (0, autostart.unwrap_or(false)),
                };
                let results = glib::VariantDict::new(None);
                results.insert("background", code == 0);
                results.insert("autostart", granted);
                connection
                    .emit_signal(
                        sender,
                        &handle,
                        "org.freedesktop.portal.Request",
                        "Response",
                        Some(&(code, results).to_variant()),
                    )
                    .expect("the answer is sent");
            },
        );
        Self { portal, asked }
    }

    /// The bus name the portal answers under.
    fn name(&self) -> String {
        self.portal.name()
    }

    fn asked(&self) -> Vec<Asked> {
        self.asked.borrow().clone()
    }
}

/// An integration inside Flatpak that asks the portal named `portal`.
fn attached_in_flatpak(portal: &str) -> AttachedIntegration {
    AttachedIntegration::attach(|folders, backend| DesktopIntegration::in_flatpak(folders, backend, portal))
}

/// Enables Show in folder and returns the toast it asks for, if any.
fn enable(integration: &DesktopIntegration) -> Option<String> {
    let enabling = integration.clone();
    wait_for("enabling", async move { enabling.enable_show_in_folder().await })
        .expect("Show in folder can be enabled inside Flatpak")
        .message()
}

/// The command the autostart entry runs: this program without a window.
fn service_command() -> Vec<String> {
    let program = std::env::current_exe().expect("the test program");
    vec![
        program.to_string_lossy().into_owned(),
        "--filemanager-service".to_owned(),
    ]
}

/// Inside Flatpak, enabling writes only the opt-in record, owns
/// `FileManager1` so requests reach the app, and asks the portal to start
/// the service at login; disabling gives the name up and asks the portal
/// to stop.
///
/// parity: INT-013, INT-015, INT-017
#[gtk::test]
fn inside_flatpak_show_in_folder_answers_and_starts_at_login() {
    let portal = FakeBackgroundPortal::start(Answer::Grant);
    let attached = attached_in_flatpak(&portal.name());
    let integration = attached.integration.clone();

    assert_eq!(enable(&integration), None);

    wait_until("the bus name", || integration.owns_file_manager());
    let folders = &attached.folders;
    assert!(folders.settings.join("reveal-integration.flatpak").is_file());
    assert!(!folders.data_home.join("dbus-1").exists(), "no session files");
    assert!(
        !folders.config_home.join("autostart").exists(),
        "no session files"
    );
    let asked = portal.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].autostart, Some(true));
    assert_eq!(asked[0].commandline, service_command());
    assert!(!asked[0].reason.is_empty());
    let testing = integration.clone();
    let outcome = wait_for("the test", async move { testing.test_show_in_folder().await });
    assert!(outcome.is_ok(), "{outcome:?}");
    let received = attached.received.borrow().clone();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].method(), FileManagerMethod::ShowFolders);

    let disabling = integration.clone();
    wait_for(
        "disabling",
        async move { disabling.disable_show_in_folder().await },
    )
    .expect("Show in folder can be disabled");
    assert!(!integration.owns_file_manager());
    assert!(!folders.settings.join("reveal-integration.flatpak").exists());
    let asked = portal.asked();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1].autostart, Some(false));
}

/// The answer is matched to its request also when the portal names
/// another request object than the token asks for.
///
/// parity: INT-015
#[gtk::test]
fn the_portals_own_request_object_is_followed() {
    let portal = FakeBackgroundPortal::start(Answer::GrantOnAnotherRequest);
    let attached = attached_in_flatpak(&portal.name());

    assert_eq!(enable(&attached.integration), None);
}

/// When the user refuses, Show in folder still answers while the app
/// runs, and the toast says that it will not start at login.
///
/// parity: INT-015, INT-017
#[gtk::test]
fn a_refused_start_at_login_still_answers_while_running() {
    let portal = FakeBackgroundPortal::start(Answer::Refuse);
    let attached = attached_in_flatpak(&portal.name());
    let integration = attached.integration.clone();

    let toast = enable(&integration);

    assert_eq!(
        toast.as_deref(),
        Some(
            "Show in folder answers while OpenXplorer runs. The desktop did not allow OpenXplorer to \
             start at login."
        )
    );
    wait_until("the bus name", || integration.owns_file_manager());
}

/// Without a Background portal Show in folder answers while the app runs.
///
/// parity: INT-015
#[gtk::test]
fn without_a_background_portal_show_in_folder_answers_while_running() {
    let attached = attached_in_flatpak("org.openxplorer.Test.NoPortal");
    let integration = attached.integration.clone();

    let toast = enable(&integration).expect("the toast explains the limit");

    assert_eq!(
        toast,
        "Show in folder answers while OpenXplorer runs. The desktop cannot start OpenXplorer at login."
    );
    wait_until("the bus name", || integration.owns_file_manager());
}
