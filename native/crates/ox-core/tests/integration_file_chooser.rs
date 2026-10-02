// SPDX-License-Identifier: AGPL-3.0-only
//! The file dialog backend: checking `FileChooser` calls, the opt-in in
//! the portal configuration, and the service on a real, private D-Bus
//! daemon.
//!
//! New in the native app (INT-032). The bus cases start their own
//! `dbus-daemon` with no service directories, as
//! `integration_file_manager.rs` does; a second connection owns
//! `org.freedesktop.portal.Desktop` and plays the desktop portal.

use std::cell::RefCell;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gio::prelude::*;
use ox_core::integration::{
    glob_matches, options_from_entries, path_variant, preferred_value, with_preference, without_preference,
    ChooserAnswer, ChooserCall, ChooserMode, ChooserNotShown, ChooserRequest, ChooserRequestError,
    DisabledFileDialogs, FileChooserBus, FileDialogError, FileDialogPaths, FileDialogRegistration,
    FilterPattern, Sandbox, FILE_CHOOSER_INTERFACE, FILE_CHOOSER_KEY, KDE_PORTAL_VARIABLE, MAX_LIST_ITEMS,
    PORTAL_BACKEND_PATH, RESPONSE_CANCELLED, RESPONSE_OTHER, RESPONSE_SUCCESS,
};
use tempfile::TempDir;

// ---------------------------------------------------------------- requests

/// `(name, [(kind, pattern)])` as the portal sends a filter.
fn filter(name: &str, patterns: &[(u32, &str)]) -> glib::Variant {
    let patterns: Vec<(u32, String)> = patterns
        .iter()
        .map(|(kind, pattern)| (*kind, (*pattern).to_owned()))
        .collect();
    (name.to_owned(), patterns).to_variant()
}

/// A request for `method` with `entries` as its options.
fn request(method: &str, entries: &[(&str, glib::Variant)]) -> Result<ChooserRequest, ChooserRequestError> {
    ChooserRequest::from_call(method, "", &options_from_entries(entries))
}

/// parity: INT-032
#[test]
fn open_file_reads_its_options() {
    let images = filter("Images", &[(0, "*.png"), (1, "image/jpeg")]);
    let request = request(
        "OpenFile",
        &[
            ("multiple", true.to_variant()),
            ("accept_label", "_Upload".to_variant()),
            (
                "filters",
                glib::Variant::array_from_iter_with_type(images.type_(), [images.clone()]),
            ),
            ("current_filter", images),
            ("current_folder", path_variant("/home/someone/Pictures")),
        ],
    )
    .expect("a valid call");
    assert_eq!(
        request.mode,
        ChooserMode::Open {
            multiple: true,
            directory: false
        }
    );
    assert_eq!(request.accept_label(), "Upload");
    assert_eq!(request.window_title(), "Open files");
    assert_eq!(request.filters.len(), 1);
    assert_eq!(request.current_filter, Some(0));
    assert_eq!(
        request.current_folder.as_deref(),
        Some(Path::new("/home/someone/Pictures"))
    );
    assert!(request.filters[0].matches("Photo.PNG", None));
    assert!(request.filters[0].matches("photo.jpg", Some("image/jpeg")));
    assert!(!request.filters[0].matches("notes.txt", Some("text/plain")));
}

/// A current filter the caller did not list joins the list, as GTK's
/// dialog shows it; unknown pattern kinds and empty filters are dropped.
///
/// parity: INT-032
#[test]
fn an_unlisted_current_filter_is_added() {
    let pdf = filter("PDF", &[(0, "*.pdf")]);
    let odd = filter("Odd", &[(7, "x")]);
    let filters = glib::Variant::array_from_iter_with_type(pdf.type_(), [pdf.clone(), odd]);
    let request = request(
        "OpenFile",
        &[
            ("filters", filters),
            ("current_filter", filter("All", &[(0, "*")])),
        ],
    )
    .expect("a valid call");
    assert_eq!(
        request
            .filters
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["PDF", "All"]
    );
    assert_eq!(request.current_filter, Some(1));
}

/// parity: INT-032
#[test]
fn save_file_takes_the_name_and_folder_of_the_current_file() {
    let request = request(
        "SaveFile",
        &[("current_file", path_variant("/home/someone/Documents/report.odt"))],
    )
    .expect("a valid call");
    assert_eq!(
        request.mode,
        ChooserMode::Save {
            name: "report.odt".to_owned()
        }
    );
    assert_eq!(
        request.current_folder.as_deref(),
        Some(Path::new("/home/someone/Documents"))
    );
    assert_eq!(request.accept_label(), "Save");
    let named =
        self::request("SaveFile", &[("current_name", "download.zip".to_variant())]).expect("a valid call");
    assert_eq!(
        named.mode,
        ChooserMode::Save {
            name: "download.zip".to_owned()
        }
    );
}

/// Safety rule "a suggested name stays in its folder".
///
/// parity: INT-032
#[test]
fn a_name_that_leaves_its_folder_is_refused() {
    for name in ["../escape", "a/b", "..", "."] {
        let refused = request("SaveFile", &[("current_name", name.to_variant())]);
        assert!(
            matches!(refused, Err(ChooserRequestError::BadName(_))),
            "{name} must be refused"
        );
    }
    let files =
        glib::Variant::array_from_iter_with_type(glib::VariantTy::BYTE_STRING, [path_variant("../x")]);
    assert!(matches!(
        request("SaveFiles", &[("files", files)]),
        Err(ChooserRequestError::BadName(_))
    ));
    assert!(matches!(
        request("SaveFiles", &[]),
        Err(ChooserRequestError::BadName(_))
    ));
}

/// parity: INT-032
#[test]
fn oversized_lists_and_unknown_methods_are_refused() {
    let one = filter("Any", &[(0, "*")]);
    let many = glib::Variant::array_from_iter_with_type(one.type_(), vec![one.clone(); MAX_LIST_ITEMS + 1]);
    assert_eq!(
        request("OpenFile", &[("filters", many)]),
        Err(ChooserRequestError::TooMany("filters"))
    );
    assert!(matches!(
        request("Delete", &[]),
        Err(ChooserRequestError::UnknownMethod(_))
    ));
}

/// Options of the wrong type are ignored, as the portal's own backends do,
/// and a relative folder is never taken.
///
/// parity: INT-032
#[test]
fn mistyped_options_and_relative_folders_are_ignored() {
    let request = request(
        "OpenFile",
        &[
            ("multiple", "yes".to_variant()),
            ("current_folder", path_variant("relative/folder")),
        ],
    )
    .expect("a valid call");
    assert_eq!(
        request.mode,
        ChooserMode::Open {
            multiple: false,
            directory: false
        }
    );
    assert_eq!(request.current_folder, None);
}

/// parity: INT-032
#[test]
fn replies_carry_uris_filter_and_choices() {
    let choices = vec![(
        "encoding".to_owned(),
        "Encoding".to_owned(),
        vec![("utf8".to_owned(), "UTF-8".to_owned())],
        "utf8".to_owned(),
    )];
    let request = request(
        "SaveFile",
        &[
            ("current_name", "a b.txt".to_variant()),
            (
                "filters",
                glib::Variant::array_from_iter_with_type(
                    filter("T", &[(0, "*.txt")]).type_(),
                    [filter("T", &[(0, "*.txt")])],
                ),
            ),
            ("choices", choices.to_variant()),
        ],
    )
    .expect("a valid call");
    let (response, results) = request.reply(&ChooserAnswer::Chosen {
        locations: vec![PathBuf::from("/home/someone/a b.txt")],
        filter: Some(0),
        choices: Vec::new(),
    });
    assert_eq!(response, RESPONSE_SUCCESS);
    let results = glib::VariantDict::new(Some(&results));
    let uris: Vec<String> = results.lookup("uris").expect("typed").expect("uris");
    assert_eq!(uris, ["file:///home/someone/a%20b.txt"]);
    let chosen: Vec<(String, String)> = results.lookup("choices").expect("typed").expect("choices");
    assert_eq!(chosen, [("encoding".to_owned(), "utf8".to_owned())]);
    assert!(results.contains("current_filter"));
    assert_eq!(request.reply(&ChooserAnswer::Cancelled).0, RESPONSE_CANCELLED);
    assert_eq!(request.reply(&ChooserAnswer::Ended).0, RESPONSE_OTHER);
}

/// An Open reply says whether the user may write what was chosen, so a
/// sandboxed caller gets write access to a writable file and only then.
///
/// parity: INT-032
#[test]
fn open_replies_say_whether_the_choice_is_writable() {
    use std::os::unix::fs::PermissionsExt;

    let folder = tempfile::tempdir().expect("temporary folder");
    let writable = folder.path().join("notes.txt");
    let read_only = folder.path().join("signed.pdf");
    fs::write(&writable, "x").expect("a file");
    fs::write(&read_only, "x").expect("a file");
    fs::set_permissions(&read_only, fs::Permissions::from_mode(0o444)).expect("read-only");
    let request = request("OpenFile", &[("multiple", true.to_variant())]).expect("a valid call");
    let writable_flag = |locations: Vec<PathBuf>| -> Option<bool> {
        let (_, results) = request.reply(&ChooserAnswer::Chosen {
            locations,
            filter: None,
            choices: Vec::new(),
        });
        glib::VariantDict::new(Some(&results))
            .lookup("writable")
            .expect("typed")
    };
    assert_eq!(writable_flag(vec![writable.clone()]), Some(true));
    assert_eq!(
        writable_flag(vec![folder.path().to_owned()]),
        Some(true),
        "a folder"
    );
    // root may write anything, so the read-only cases hold for users only.
    if !is_root() {
        assert_eq!(writable_flag(vec![writable, read_only]), Some(false));
    }
    assert_eq!(
        writable_flag(vec![folder.path().join("missing")]),
        Some(false),
        "unknown is not writable"
    );
}

/// Whether the tests run as root.
fn is_root() -> bool {
    fs::metadata("/proc/self").is_ok_and(|metadata| {
        use std::os::unix::fs::MetadataExt;
        metadata.uid() == 0
    })
}

/// `SaveFiles` answers one URI per name, in the folder the user chose.
///
/// parity: INT-032
#[test]
fn save_files_answers_each_name_in_the_chosen_folder() {
    let names = [path_variant("one.png"), path_variant("two.png")];
    let request = request(
        "SaveFiles",
        &[(
            "files",
            glib::Variant::array_from_iter_with_type(glib::VariantTy::BYTE_STRING, names),
        )],
    )
    .expect("a valid call");
    assert!(request.chooses_folder());
    let (_, results) = request.reply(&ChooserAnswer::Chosen {
        locations: vec![PathBuf::from("/srv/out")],
        filter: None,
        choices: Vec::new(),
    });
    let uris: Vec<String> = glib::VariantDict::new(Some(&results))
        .lookup("uris")
        .expect("typed")
        .expect("uris");
    assert_eq!(uris, ["file:///srv/out/one.png", "file:///srv/out/two.png"]);
}

/// parity: INT-032
#[test]
fn globs_match_like_a_shell_without_case() {
    assert!(glob_matches("*.png", "SHOT.PNG"));
    assert!(glob_matches("*", ""));
    assert!(glob_matches("photo-??.jp*g", "photo-12.jpeg"));
    assert!(glob_matches("[a-c]*.[!x]x", "brief.tx"));
    assert!(!glob_matches("[a-c]*", "draft"));
    assert!(!glob_matches("*.png", "png"));
    assert!(glob_matches("*.tar.*", "backup.tar.gz"));
    assert!(glob_matches("[", "["));
    assert!(matches!(
        FilterPattern::Glob("*.a".to_owned()),
        FilterPattern::Glob(_)
    ));
}

// ------------------------------------------------------------- the opt-in

/// The KDE configuration the system ships, which the user file must keep.
const SYSTEM_KDE: &str = "[preferred]\ndefault=kde\norg.freedesktop.impl.portal.Settings=kde;gtk\n";

/// User and system folders in a temporary folder.
struct OptInFixture {
    root: TempDir,
}

impl OptInFixture {
    fn new() -> Self {
        let fixture = Self {
            root: tempfile::tempdir().expect("temporary folder"),
        };
        let system = fixture.system().join("xdg-desktop-portal");
        fs::create_dir_all(&system).expect("system folder");
        fs::write(system.join("kde-portals.conf"), SYSTEM_KDE).expect("system file");
        fixture
    }

    fn system(&self) -> PathBuf {
        self.root.path().join("usr-share")
    }

    fn paths(&self) -> FileDialogPaths {
        FileDialogPaths {
            settings: self.root.path().join("settings"),
            config_home: self.root.path().join("config"),
            system_dirs: vec![self.root.path().join("etc"), self.system()],
            desktops: vec!["kde".to_owned()],
        }
    }

    fn registration(&self) -> FileDialogRegistration {
        FileDialogRegistration::new(self.paths(), "io.winspace.Development", Sandbox::Host)
    }

    fn user_file(&self) -> PathBuf {
        self.root
            .path()
            .join("config/xdg-desktop-portal/kde-portals.conf")
    }
}

/// Enabling copies the desktop's system preferences into the user file and
/// sets only the file chooser; disabling removes the file again.
///
/// parity: INT-032
#[test]
fn enabling_keeps_every_other_backend_and_disabling_restores() {
    let fixture = OptInFixture::new();
    let registration = fixture.registration();
    assert!(!registration.is_enabled());
    assert_eq!(registration.current_backend().as_deref(), Some("kde"));
    registration.enable().expect("enable");
    let written = fs::read_to_string(fixture.user_file()).expect("user file");
    assert_eq!(preferred_value(&written, "default").as_deref(), Some("kde"));
    assert_eq!(
        preferred_value(&written, "org.freedesktop.impl.portal.Settings").as_deref(),
        Some("kde;gtk")
    );
    assert_eq!(
        preferred_value(&written, FILE_CHOOSER_KEY).as_deref(),
        Some("io.winspace.Development")
    );
    assert!(registration.is_enabled());
    assert_eq!(
        registration.current_backend().as_deref(),
        Some("io.winspace.Development")
    );
    registration.enable().expect("enabling again does nothing");
    assert_eq!(
        fs::read_to_string(fixture.user_file()).expect("user file"),
        written
    );
    assert_eq!(
        registration.disable().expect("disable"),
        DisabledFileDialogs::Restored
    );
    assert!(!fixture.user_file().exists());
    assert!(!registration.is_enabled());
}

/// A user file the user wrote is changed in one line and put back byte
/// for byte.
///
/// parity: INT-032
#[test]
fn an_existing_user_file_is_restored_exactly() {
    let fixture = OptInFixture::new();
    let original =
        "# mine\n[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.FileChooser=gtk\n\n[other]\nx=1\n";
    fs::create_dir_all(fixture.user_file().parent().expect("folder")).expect("folder");
    fs::write(fixture.user_file(), original).expect("user file");
    let registration = fixture.registration();
    registration.enable().expect("enable");
    let written = fs::read_to_string(fixture.user_file()).expect("user file");
    assert_eq!(
        preferred_value(&written, FILE_CHOOSER_KEY).as_deref(),
        Some("io.winspace.Development")
    );
    assert_eq!(preferred_value(&written, "default").as_deref(), Some("gtk"));
    assert!(written.contains("[other]\nx=1"));
    assert_eq!(
        registration.disable().expect("disable"),
        DisabledFileDialogs::Restored
    );
    assert_eq!(
        fs::read_to_string(fixture.user_file()).expect("user file"),
        original
    );
}

/// When the user's settings are in another file the portal reads (here
/// `portals.conf`), that file is changed in one line and put back; no new
/// desktop file is created to hide it.
///
/// parity: INT-032
#[test]
fn the_user_file_in_use_is_changed_and_not_hidden() {
    let fixture = OptInFixture::new();
    let folder = fixture.root.path().join("config/xdg-desktop-portal");
    let in_use = folder.join("portals.conf");
    let original = "[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.Screenshot=gnome\n";
    fs::create_dir_all(&folder).expect("folder");
    fs::write(&in_use, original).expect("user file");
    let registration = fixture.registration();
    assert_eq!(registration.config_file(), in_use);

    registration.enable().expect("enable");
    assert!(
        !fixture.user_file().exists(),
        "no kde-portals.conf hides portals.conf"
    );
    let written = fs::read_to_string(&in_use).expect("user file");
    assert_eq!(preferred_value(&written, "default").as_deref(), Some("gtk"));
    assert_eq!(
        preferred_value(&written, "org.freedesktop.impl.portal.Screenshot").as_deref(),
        Some("gnome")
    );
    assert!(registration.is_enabled());
    assert_eq!(
        registration.disable().expect("disable"),
        DisabledFileDialogs::Restored
    );
    assert_eq!(fs::read_to_string(&in_use).expect("user file"), original);
}

/// On KDE, enabling also writes the login script that makes KDE's own
/// apps ask the portal; disabling removes it; a file of the user's with
/// that name is never replaced; other desktops get no script.
///
/// parity: INT-032
#[test]
fn kde_apps_are_covered_by_a_login_script() {
    let fixture = OptInFixture::new();
    let registration = fixture.registration();
    let script = registration.kde_env_file();
    assert!(script.ends_with("plasma-workspace/env/openxplorer-file-dialogs.sh"));
    assert!(!registration.covers_kde_apps());

    registration.enable().expect("enable");
    let written = fs::read_to_string(&script).expect("the login script");
    assert!(
        written.contains(&format!("export {KDE_PORTAL_VARIABLE}=1\n")),
        "{written}"
    );
    assert!(registration.covers_kde_apps());
    registration.disable().expect("disable");
    assert!(!script.exists());

    // Enabled before KDE apps were covered: enabling again adds the script.
    registration.enable().expect("enable");
    fs::remove_file(&script).expect("an older version wrote none");
    registration.enable().expect("enable again");
    assert!(registration.covers_kde_apps());
    registration.disable().expect("disable");

    fs::write(&script, "export SOMETHING_ELSE=1\n").expect("the user's own file");
    registration.enable().expect("enable");
    registration.disable().expect("disable");
    assert_eq!(
        fs::read_to_string(&script).expect("kept"),
        "export SOMETHING_ELSE=1\n"
    );

    let gnome = FileDialogRegistration::new(
        FileDialogPaths {
            desktops: vec!["gnome".to_owned()],
            ..fixture.paths()
        },
        "io.winspace.Development",
        Sandbox::Host,
    );
    fs::remove_file(&script).expect("cleared");
    gnome.enable().expect("enable on GNOME");
    assert!(!script.exists(), "GNOME apps already ask the portal");
}

/// A later edit by the user wins: disabling then removes only the app's
/// line.
///
/// parity: INT-032
#[test]
fn a_later_edit_is_kept_when_disabling() {
    let fixture = OptInFixture::new();
    let registration = fixture.registration();
    registration.enable().expect("enable");
    let edited = format!(
        "{}org.freedesktop.impl.portal.Screenshot=gnome\n",
        fs::read_to_string(fixture.user_file()).expect("file")
    );
    fs::write(fixture.user_file(), &edited).expect("edit");
    assert_eq!(
        registration.disable().expect("disable"),
        DisabledFileDialogs::LineRemoved
    );
    let left = fs::read_to_string(fixture.user_file()).expect("user file");
    assert_eq!(preferred_value(&left, FILE_CHOOSER_KEY), None);
    assert_eq!(
        preferred_value(&left, "org.freedesktop.impl.portal.Screenshot").as_deref(),
        Some("gnome")
    );
    assert!(!left.contains("OpenXplorer"));
}

/// parity: INT-032
#[test]
fn a_symlink_is_never_replaced_and_flatpak_is_refused() {
    let fixture = OptInFixture::new();
    fs::create_dir_all(fixture.user_file().parent().expect("folder")).expect("folder");
    symlink("/etc/hostname", fixture.user_file()).expect("symlink");
    assert!(matches!(
        fixture.registration().enable(),
        Err(FileDialogError::Symlink(_))
    ));
    let sandboxed = FileDialogRegistration::new(fixture.paths(), "io.winspace.Development", Sandbox::Flatpak);
    assert!(!sandboxed.is_available());
    assert!(matches!(sandboxed.enable(), Err(FileDialogError::Unsupported)));
}

/// parity: INT-032
#[test]
fn preferences_are_set_and_removed_in_their_section() {
    let set = with_preference("[other]\na=1\n", FILE_CHOOSER_KEY, "x");
    assert_eq!(preferred_value(&set, FILE_CHOOSER_KEY).as_deref(), Some("x"));
    assert!(set.starts_with("[other]\na=1\n\n[preferred]\n"));
    let replaced = with_preference(&set, FILE_CHOOSER_KEY, "y");
    assert_eq!(replaced.matches(FILE_CHOOSER_KEY).count(), 1);
    assert_eq!(preferred_value(&replaced, FILE_CHOOSER_KEY).as_deref(), Some("y"));
    let removed = without_preference(&replaced, FILE_CHOOSER_KEY);
    assert_eq!(preferred_value(&removed, FILE_CHOOSER_KEY), None);
    assert!(!removed.contains("OpenXplorer"));
    assert_eq!(
        ox_core::integration::desktops_from(Some("ubuntu:GNOME:bad/name")),
        ["ubuntu".to_owned(), "gnome".to_owned()]
    );
}

// -------------------------------------------------------------- the service

/// The desktop portal's bus name, which the frontend connection owns.
const FRONTEND_NAME: &str = "org.freedesktop.portal.Desktop";

/// A `dbus-daemon` of its own, stopped when dropped.
struct PrivateBus {
    daemon: Child,
    address: String,
    _directory: TempDir,
}

impl PrivateBus {
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("temporary folder");
        let config = directory.path().join("bus.conf");
        let listen = format!("unix:dir={}", directory.path().display());
        fs::write(
            &config,
            format!(
                "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n \
                 \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n<busconfig><type>session</type>\
                 <listen>{listen}</listen><auth>EXTERNAL</auth><policy context=\"default\">\
                 <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/><allow own=\"*\"/>\
                 </policy></busconfig>\n"
            ),
        )
        .expect("bus configuration");
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon is installed with dbus-run-session");
        let stdout = daemon.stdout.take().expect("standard output is piped");
        let mut address = String::new();
        BufReader::new(stdout)
            .read_line(&mut address)
            .expect("the daemon prints its address");
        Self {
            daemon,
            address: address.trim().to_owned(),
            _directory: directory,
        }
    }

    fn connect(&self) -> gio::DBusConnection {
        let flags = gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
        gio::DBusConnection::for_address_sync(&self.address, flags, None, gio::Cancellable::NONE)
            .expect("connect to the private bus")
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

/// The backend on a private bus, a frontend that owns the portal's name,
/// and the calls the handler received.
struct ServiceFixture {
    context: glib::MainContext,
    _service: FileChooserBus,
    service_name: String,
    frontend: gio::DBusConnection,
    received: Rc<RefCell<Vec<ChooserCall>>>,
    bus: PrivateBus,
}

impl ServiceFixture {
    fn new() -> Self {
        let context = glib::MainContext::new();
        let bus = PrivateBus::start();
        let received: Rc<RefCell<Vec<ChooserCall>>> = Rc::default();
        let connection = bus.connect();
        let service_name = connection
            .unique_name()
            .expect("a bus connection has a name")
            .to_string();
        let service = context
            .with_thread_default(|| {
                let recorded = Rc::clone(&received);
                let mut service = FileChooserBus::new(connection.clone(), move |call| {
                    recorded.borrow_mut().push(call);
                    Ok::<(), ChooserNotShown>(())
                });
                service.export().expect("export");
                service
            })
            .expect("the context is free");
        let frontend = bus.connect();
        let fixture = Self {
            context,
            _service: service,
            service_name,
            frontend,
            received,
            bus,
        };
        fixture.own(&fixture.frontend.clone(), FRONTEND_NAME);
        fixture
    }

    /// Makes `connection` own `name` before going on.
    fn own(&self, connection: &gio::DBusConnection, name: &str) {
        let reply = self.context.block_on(connection.call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(name, 4_u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
        ));
        assert_eq!(
            reply.expect("RequestName").get::<(u32,)>(),
            Some((1,)),
            "{name} is owned"
        );
    }

    /// Starts `method` from `caller` and returns its pending reply.
    fn start_call(
        &self,
        caller: &gio::DBusConnection,
        method: &str,
        handle: &str,
        options: &glib::VariantDict,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<glib::Variant, glib::Error>>>> {
        let handle = glib::variant::ObjectPath::try_from(handle).expect("path");
        let parameters = glib::Variant::tuple_from_iter([
            handle.to_variant(),
            "org.example.App".to_variant(),
            "".to_variant(),
            "Pick".to_variant(),
            options.end(),
        ]);
        Box::pin(caller.call_future(
            Some(&self.service_name),
            PORTAL_BACKEND_PATH,
            FILE_CHOOSER_INTERFACE,
            method,
            Some(&parameters),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            10_000,
        ))
    }

    /// Runs the loop until `condition` holds, for at most five seconds.
    fn run_until(&self, what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        self.context
            .with_thread_default(|| {
                while !condition() {
                    assert!(Instant::now() < deadline, "timed out waiting for {what}");
                    self.context.iteration(false);
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
            .expect("the context is free");
    }

    /// Runs the loop until `future` is ready.
    fn finish<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        self.context.block_on(future)
    }
}

/// A call from the portal reaches the window, and the user's choice comes
/// back as its reply.
///
/// parity: INT-032
#[test]
fn the_portal_gets_the_users_choice() {
    let fixture = ServiceFixture::new();
    let pending = fixture.start_call(
        &fixture.frontend,
        "OpenFile",
        "/org/freedesktop/portal/desktop/request/1_1/t1",
        &options_from_entries(&[("multiple", true.to_variant())]),
    );
    let reply = fixture.context.spawn_local(pending);
    fixture.run_until("the call", || !fixture.received.borrow().is_empty());
    let call = fixture.received.borrow_mut().remove(0);
    assert_eq!(call.app_id, "org.example.App");
    assert_eq!(call.request.title, "Pick");
    call.reply.send(&ChooserAnswer::Chosen {
        locations: vec![PathBuf::from("/tmp/one"), PathBuf::from("/tmp/two")],
        filter: None,
        choices: Vec::new(),
    });
    call.reply.send(&ChooserAnswer::Cancelled);
    let reply = fixture.finish(reply).expect("joined").expect("answered");
    let (response, results) = reply.get::<(u32, glib::VariantDict)>().expect("(ua{sv})");
    assert_eq!(response, RESPONSE_SUCCESS);
    let uris: Vec<String> = results.lookup("uris").expect("typed").expect("uris");
    assert_eq!(uris, ["file:///tmp/one", "file:///tmp/two"]);
}

/// Safety rule "only the portal may ask".
///
/// parity: INT-032
#[test]
fn another_program_is_refused() {
    let fixture = ServiceFixture::new();
    let stranger = fixture.bus.connect();
    let pending = fixture.start_call(
        &stranger,
        "SaveFile",
        "/org/example/request/1",
        &options_from_entries(&[]),
    );
    let error = fixture.finish(pending).expect_err("refused");
    assert!(error.to_string().contains("Only the desktop portal"), "{error}");
    assert!(fixture.received.borrow().is_empty());
}

/// The portal's `Close` ends the call with "other" and lets the window go;
/// a reply dropped unanswered ends it the same way.
///
/// parity: INT-032
#[test]
fn closing_or_dropping_ends_the_call() {
    let fixture = ServiceFixture::new();
    let handle = "/org/freedesktop/portal/desktop/request/1_1/t2";
    let pending = fixture.start_call(&fixture.frontend, "SaveFile", handle, &options_from_entries(&[]));
    let reply = fixture.context.spawn_local(pending);
    fixture.run_until("the call", || !fixture.received.borrow().is_empty());
    let call = fixture.received.borrow_mut().remove(0);
    let closed = Rc::new(std::cell::Cell::new(false));
    let noticed = Rc::clone(&closed);
    call.reply.connect_closed(move || noticed.set(true));
    let close = fixture.frontend.call_future(
        Some(&fixture.service_name),
        handle,
        "org.freedesktop.impl.portal.Request",
        "Close",
        None,
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        5000,
    );
    fixture.finish(close).expect("Close is answered");
    assert!(closed.get());
    assert!(call.reply.is_answered());
    let reply = fixture.finish(reply).expect("joined").expect("answered");
    assert_eq!(
        reply.get::<(u32, glib::VariantDict)>().expect("(ua{sv})").0,
        RESPONSE_OTHER
    );

    let pending = fixture.start_call(
        &fixture.frontend,
        "OpenFile",
        "/org/freedesktop/portal/desktop/request/1_1/t3",
        &options_from_entries(&[]),
    );
    let reply = fixture.context.spawn_local(pending);
    fixture.run_until("the second call", || !fixture.received.borrow().is_empty());
    drop(fixture.received.borrow_mut().remove(0));
    let reply = fixture.finish(reply).expect("joined").expect("answered");
    assert_eq!(
        reply.get::<(u32, glib::VariantDict)>().expect("(ua{sv})").0,
        RESPONSE_OTHER
    );
}

/// A call the app refuses ends at once with a D-Bus error.
///
/// parity: INT-032
#[test]
fn a_refused_call_is_an_error() {
    let fixture = ServiceFixture::new();
    let bad = options_from_entries(&[("current_name", "../x".to_variant())]);
    let pending = fixture.start_call(
        &fixture.frontend,
        "SaveFile",
        "/org/freedesktop/portal/desktop/request/1_1/t4",
        &bad,
    );
    let error = fixture.finish(pending).expect_err("refused");
    assert!(error.to_string().contains("Not a file name"), "{error}");
}

/// The portal loads a backend's properties while it starts, when a GTK
/// application's startup may be blocked waiting for the portal. The
/// backend answers from its own thread, so the request succeeds while the
/// main context is not running at all.
///
/// parity: INT-032
#[test]
fn properties_are_answered_while_the_main_thread_is_blocked() {
    let fixture = ServiceFixture::new();
    // A synchronous call, with the fixture's main context never iterated
    // meanwhile, stands in for the app's blocked startup.
    let reply = fixture.frontend.call_sync(
        Some(&fixture.service_name),
        PORTAL_BACKEND_PATH,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(FILE_CHOOSER_INTERFACE,).to_variant()),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        2000,
        gio::Cancellable::NONE,
    );
    let properties = reply.expect("GetAll is answered without the main context");
    assert_eq!(properties.type_().as_str(), "(a{sv})");
    let introspection = fixture.frontend.call_sync(
        Some(&fixture.service_name),
        PORTAL_BACKEND_PATH,
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
        None,
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        2000,
        gio::Cancellable::NONE,
    );
    assert!(introspection.is_ok(), "introspection is answered too");
}
