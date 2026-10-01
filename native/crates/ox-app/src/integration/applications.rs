// SPDX-License-Identifier: AGPL-3.0-only
//! The installed applications Open with lists for an item, and launching
//! the chosen one.
//!
//! Ports `list_applications` and `prepare_launch` of
//! `desktop/file_services.py` and `launch_selected` of
//! `desktop/winspace.py` (OPEN-011, OPEN-012, OPEN-015). The list is one
//! entry per visible name ([`unique_applications`]), never the app itself,
//! a hidden launcher or one that takes neither files nor URIs; on a share
//! without a local mount only applications that read URIs are available.
//! The chosen application must still be in that list when it launches,
//! and nothing changes a default unless the user ticked "Always use this
//! app", which never applies to folders.

use std::path::PathBuf;

use gtk::gio;
use gtk::prelude::*;
use ox_core::entry::{inspect, Entry, EntryError, EntryKind};
use ox_core::integration::{unique_applications, APP_ID, FOLDER_CONTENT_TYPE, UNKNOWN_CONTENT_TYPE};
use ox_core::location::{normalise, LocationError};
use ox_core::network::local_path;
use ox_core::transfer::Cancellation;

use super::tools::installed_application;

/// Why Open with could not list or launch. `Display` is the message the
/// window shows.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OpenWithError {
    /// The item could not be queried.
    #[error(transparent)]
    Entry(#[from] EntryError),
    /// The location is not one the app opens.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// A symbolic link has no content type of its own.
    #[error("Open the link target first to choose an application.")]
    Symlink,
    /// The application is not offered for this item any more.
    #[error("That installed application is unavailable for this file/location.")]
    Unavailable,
    /// The application was uninstalled since the list was made.
    #[error("That application is no longer installed.")]
    NotInstalled,
    /// The application did not start.
    #[error("The selected application could not be started.")]
    NotStarted,
    /// The worker thread stopped without an answer.
    #[error("The list of installed applications could not be read.")]
    Interrupted,
}

/// An application's `icon`, read with the application list on a worker
/// thread ([`ox_core::integration::ApplicationInfo::icon`]), drawn at
/// `size` pixels as Dolphin's Open With and GNOME's app chooser show it;
/// `None` without an icon or when it cannot be read back, so the caller
/// keeps its glyph.
pub(crate) fn application_image(icon: Option<&str>, size: i32) -> Option<gtk::Image> {
    let icon = gio::Icon::for_string(icon?).ok()?;
    let image = gtk::Image::from_gicon(&icon);
    image.set_pixel_size(size);
    image.add_css_class("app-icon");
    Some(image)
}

/// Which applications the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplicationScope {
    /// The applications registered for the item's content type.
    Recommended,
    /// Every installed application ("Show all installed applications").
    AllInstalled,
}

/// One application Open with offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplicationChoice {
    /// Its desktop ID.
    pub(crate) id: String,
    /// Its name.
    pub(crate) name: String,
    /// It is the default for the item's content type.
    pub(crate) is_default: bool,
    /// It is registered for the item's content type.
    pub(crate) is_recommended: bool,
    /// It can open the item: the item has a local path, or the
    /// application reads URIs.
    pub(crate) is_available: bool,
    /// Its icon ([`ox_core::integration::ApplicationInfo::icon`]).
    pub(crate) icon: Option<String>,
}

impl ApplicationChoice {
    /// The line under its name.
    pub(crate) fn note(&self) -> &'static str {
        if self.is_default {
            "Current default"
        } else if !self.is_available {
            "Requires a local mount"
        } else if self.is_recommended {
            "Recommended"
        } else {
            "Installed application"
        }
    }
}

/// The applications for one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplicationList {
    /// Default first, then recommended, then by name.
    pub(crate) choices: Vec<ApplicationChoice>,
    /// The item's content type; `inode/directory` for a folder.
    pub(crate) content_type: String,
    /// The item is a folder.
    pub(crate) is_folder: bool,
}

impl ApplicationList {
    /// The application chosen when the list appears: the default when it
    /// is available, else the first available one.
    pub(crate) fn preselected(&self) -> Option<&ApplicationChoice> {
        let default = self
            .choices
            .iter()
            .find(|choice| choice.is_default && choice.is_available);
        default.or_else(|| self.choices.iter().find(|choice| choice.is_available))
    }
}

/// Lists the applications for `uri` on a worker thread.
///
/// # Errors
///
/// As [`list_applications`].
pub(crate) async fn list_applications_in_background(
    uri: String,
    scope: ApplicationScope,
) -> Result<ApplicationList, OpenWithError> {
    let cancel = Cancellation::new();
    let listed = gio::spawn_blocking(move || list_applications(&uri, scope, &cancel)).await;
    listed.map_err(|_| OpenWithError::Interrupted)?
}

/// The applications for `uri` (`list_applications`). Runs GIO
/// synchronously.
///
/// # Errors
///
/// [`OpenWithError::Symlink`] for a symbolic link, and the errors of
/// querying the item.
pub(crate) fn list_applications(
    uri: &str,
    scope: ApplicationScope,
    cancel: &Cancellation,
) -> Result<ApplicationList, OpenWithError> {
    let entry = inspect(uri, Some(cancel.cancellable()))?;
    if entry.kind == EntryKind::Symlink {
        return Err(OpenWithError::Symlink);
    }
    let content_type = content_type_of(&entry);
    let default_id: Option<String> = gio::AppInfo::default_for_type(&content_type, false)
        .and_then(|app| app.id())
        .map(Into::into);
    let recommended = gio::AppInfo::all_for_type(&content_type);
    let recommended_ids: Vec<String> = recommended
        .iter()
        .filter_map(|app| app.id().map(Into::into))
        .collect();
    let shows_everything = scope == ApplicationScope::AllInstalled || entry.is_dir;
    let candidates = if shows_everything {
        gio::AppInfo::all()
    } else {
        recommended
    };
    let facts = ChoiceFacts {
        default_id: default_id.as_deref(),
        recommended_ids: &recommended_ids,
        has_local_path: local_path(&entry.uri).is_some(),
    };
    Ok(ApplicationList {
        choices: facts.choices(candidates),
        content_type,
        is_folder: entry.is_dir,
    })
}

/// The applications a menu offers for the item at `uri` beside Open
/// with… (OPEN-013): up to `limit` of those registered for its type that
/// can open it, as Open with orders them, leaving out a file's default,
/// which Open starts. Reads the desktop's application database, not the
/// item.
pub(crate) fn menu_applications(
    uri: &str,
    content_type: Option<&str>,
    is_folder: bool,
    limit: usize,
) -> Vec<ApplicationChoice> {
    let content_type = match (is_folder, content_type) {
        (true, _) => FOLDER_CONTENT_TYPE,
        (false, known) => known.unwrap_or(UNKNOWN_CONTENT_TYPE),
    };
    let default_id: Option<String> = gio::AppInfo::default_for_type(content_type, false)
        .and_then(|app| app.id())
        .map(Into::into);
    let recommended = gio::AppInfo::recommended_for_type(content_type);
    let recommended_ids: Vec<String> = recommended
        .iter()
        .filter_map(|app| app.id().map(Into::into))
        .collect();
    let facts = ChoiceFacts {
        default_id: default_id.as_deref(),
        recommended_ids: &recommended_ids,
        has_local_path: local_path(uri).is_some(),
    };
    let mut choices = facts.choices(recommended);
    choices.retain(|choice| choice.is_available && (is_folder || !choice.is_default));
    choices.truncate(limit);
    choices
}

/// The content type Open with lists applications for: `inode/directory`
/// for a folder, `application/octet-stream` when GIO could not tell.
fn content_type_of(entry: &Entry) -> String {
    if entry.is_dir {
        return FOLDER_CONTENT_TYPE.to_owned();
    }
    let content_type = entry.content_type.as_deref();
    content_type.unwrap_or(UNKNOWN_CONTENT_TYPE).to_owned()
}

/// What decides how an application is offered for one item.
struct ChoiceFacts<'a> {
    default_id: Option<&'a str>,
    recommended_ids: &'a [String],
    has_local_path: bool,
}

impl ChoiceFacts<'_> {
    /// The choices among `candidates`, one per visible name: the default
    /// first, then the recommended ones, then by name.
    fn choices(&self, candidates: Vec<gio::AppInfo>) -> Vec<ApplicationChoice> {
        let unique = unique_applications(candidates, self.default_id);
        let mut choices: Vec<ApplicationChoice> = unique.iter().filter_map(|app| self.choice(app)).collect();
        choices.sort_by_key(|choice| {
            let name = choice.name.to_lowercase();
            (!choice.is_default, !choice.is_recommended, name)
        });
        choices
    }

    /// The choice for `app`, or `None` for one Open with never offers:
    /// the app itself, a hidden launcher, or one that takes neither files
    /// nor URIs.
    fn choice(&self, app: &gio::AppInfo) -> Option<ApplicationChoice> {
        let id = app.id()?.to_string();
        let offered = id != APP_ID && app.should_show() && (app.supports_files() || app.supports_uris());
        if !offered {
            return None;
        }
        Some(ApplicationChoice {
            is_default: self.default_id == Some(id.as_str()),
            is_recommended: self.recommended_ids.contains(&id),
            is_available: self.has_local_path || app.supports_uris(),
            name: app.display_name().to_string(),
            icon: ox_core::integration::ApplicationInfo::icon(app),
            id,
        })
    }
}

/// What to hand the chosen application (`prepare_launch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedLaunch {
    /// The item's local path, also for a mounted share, else its URI.
    pub(crate) target: LaunchTarget,
    /// The item's content type, for "Always use this app".
    pub(crate) content_type: String,
    /// The item is a folder, whose default is never changed.
    pub(crate) is_folder: bool,
}

/// The file the application receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LaunchTarget {
    /// A local path.
    Path(PathBuf),
    /// A URI, for an application that reads URIs.
    Uri(String),
}

impl LaunchTarget {
    /// The target as a GIO file.
    pub(crate) fn to_file(&self) -> gio::File {
        match self {
            Self::Path(path) => gio::File::for_path(path),
            Self::Uri(uri) => gio::File::for_uri(uri),
        }
    }
}

/// Checks on a worker thread that `app_id` may still open `uri`, and what
/// to hand it (`prepare_launch`).
///
/// # Errors
///
/// [`OpenWithError::Unavailable`] when the application is not offered
/// for the item or cannot open it, and the errors of listing.
pub(crate) async fn prepare_launch(uri: String, app_id: String) -> Result<PreparedLaunch, OpenWithError> {
    let prepared = gio::spawn_blocking(move || {
        let cancel = Cancellation::new();
        let list = list_applications(&uri, ApplicationScope::AllInstalled, &cancel)?;
        let offered = list
            .choices
            .iter()
            .any(|choice| choice.id == app_id && choice.is_available);
        if !offered {
            return Err(OpenWithError::Unavailable);
        }
        let uri = normalise(&uri)?;
        let target = local_path(&uri).map_or(LaunchTarget::Uri(uri), LaunchTarget::Path);
        Ok(PreparedLaunch {
            target,
            content_type: list.content_type,
            is_folder: list.is_folder,
        })
    });
    prepared.await.map_err(|_| OpenWithError::Interrupted)?
}

/// The item at `uri` as a custom command gets it (OPEN-014): read again,
/// never a symbolic link, by its local path when it has one.
///
/// # Errors
///
/// As [`prepare_launch`], without the application checks.
pub(crate) async fn prepare_target(uri: String) -> Result<PreparedLaunch, OpenWithError> {
    let prepared = gio::spawn_blocking(move || {
        let cancel = Cancellation::new();
        let entry = inspect(&uri, Some(cancel.cancellable()))?;
        if entry.kind == EntryKind::Symlink {
            return Err(OpenWithError::Symlink);
        }
        let uri = normalise(&uri)?;
        let target = local_path(&uri).map_or(LaunchTarget::Uri(uri), LaunchTarget::Path);
        Ok(PreparedLaunch {
            target,
            content_type: content_type_of(&entry),
            is_folder: entry.is_dir,
        })
    });
    prepared.await.map_err(|_| OpenWithError::Interrupted)?
}

/// Whether "Always use this app for this file type" was ticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefaultChoice {
    /// Leave the defaults alone: the default choice.
    Keep,
    /// Make the application the default for the item's content type.
    MakeDefault,
}

/// Launches `app_id` on `prepared` with `launch_context`, which gives it
/// startup notification and focus (INT-023), and returns the toast
/// (`launch_selected`).
///
/// # Errors
///
/// [`OpenWithError::NotInstalled`] or [`OpenWithError::NotStarted`].
pub(crate) fn launch(
    app_id: &str,
    prepared: &PreparedLaunch,
    default: DefaultChoice,
    launch_context: &gio::AppLaunchContext,
) -> Result<&'static str, OpenWithError> {
    let app = installed_application(app_id).ok_or(OpenWithError::NotInstalled)?;
    app.launch(&[prepared.target.to_file()], Some(launch_context))
        .map_err(|_| OpenWithError::NotStarted)?;
    Ok(default_after_launch(&app, prepared, default))
}

/// Makes `app` the default when asked, and returns the toast.
fn default_after_launch(
    app: &gio::AppInfo,
    prepared: &PreparedLaunch,
    default: DefaultChoice,
) -> &'static str {
    const OPENED: &str = "Opened with the selected application.";
    if default == DefaultChoice::Keep {
        return OPENED;
    }
    // Safety rule "Open with never changes the file-manager default": a
    // folder's handler is changed only in Settings > Default apps.
    if prepared.is_folder {
        return "Opened the folder. Its default file-manager association was not changed.";
    }
    match app.set_as_default_for_type(&prepared.content_type) {
        Ok(()) => OPENED,
        Err(_) => "Opened the file, but the default application could not be changed.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(name: &str, is_default: bool, is_recommended: bool, is_available: bool) -> ApplicationChoice {
        ApplicationChoice {
            id: format!("{name}.desktop"),
            name: name.to_owned(),
            is_default,
            is_recommended,
            is_available,
            icon: None,
        }
    }

    /// Ported from the row labels of `openWithDialog` in
    /// `desktop/ui/app.js`.
    ///
    /// parity: OPEN-011
    #[test]
    fn each_application_says_why_it_is_offered() {
        assert_eq!(choice("Viewer", true, true, true).note(), "Current default");
        assert_eq!(
            choice("Player", false, true, false).note(),
            "Requires a local mount"
        );
        assert_eq!(choice("Editor", false, true, true).note(), "Recommended");
        assert_eq!(
            choice("Other", false, false, true).note(),
            "Installed application"
        );
    }

    /// parity: OPEN-011
    #[test]
    fn the_default_is_chosen_first_when_it_can_open_the_item() {
        let list = |choices| ApplicationList {
            choices,
            content_type: "text/plain".to_owned(),
            is_folder: false,
        };
        let with_default = list(vec![
            choice("Viewer", true, true, true),
            choice("Editor", false, true, true),
        ]);
        assert_eq!(
            with_default.preselected().map(|choice| choice.name.as_str()),
            Some("Viewer")
        );
        let default_needs_mount = list(vec![
            choice("Viewer", true, true, false),
            choice("Editor", false, true, true),
        ]);
        assert_eq!(
            default_needs_mount
                .preselected()
                .map(|choice| choice.name.as_str()),
            Some("Editor")
        );
        let nothing_opens = list(vec![choice("Viewer", true, true, false)]);
        assert_eq!(nothing_opens.preselected(), None);
    }

    /// A folder lists every installed application, and never offers the
    /// app itself.
    ///
    /// parity: OPEN-011
    #[test]
    fn a_folder_lists_installed_applications_but_never_openxplorer() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let uri = ox_core::location::file_uri(folder.path());
        let list = list_applications(&uri, ApplicationScope::Recommended, &Cancellation::new())
            .expect("a folder can be listed");
        assert!(list.is_folder);
        assert_eq!(list.content_type, FOLDER_CONTENT_TYPE);
        assert!(list.choices.iter().all(|choice| choice.id != APP_ID));
        assert!(
            list.choices.iter().all(|choice| choice.is_available),
            "a local folder has a path"
        );
    }

    /// "Always use this app" changes nothing unless ticked, never a
    /// folder's handler, and makes the application the default for a
    /// file's content type. The last part writes `mimeapps.list`, so it
    /// runs only with a private configuration folder (native/tools/check.py).
    ///
    /// parity: OPEN-012
    #[test]
    fn always_use_this_app_sets_the_default_of_files_only() {
        let app =
            gio::AppInfo::create_from_commandline("true", Some("Test viewer"), gio::AppInfoCreateFlags::NONE)
                .expect("an application made from a command line");
        let prepared = |is_folder: bool| PreparedLaunch {
            target: LaunchTarget::Uri("file:///tmp/example".to_owned()),
            content_type: "application/x-openxplorer-test".to_owned(),
            is_folder,
        };
        assert_eq!(
            default_after_launch(&app, &prepared(false), DefaultChoice::Keep),
            "Opened with the selected application."
        );
        assert_eq!(
            default_after_launch(&app, &prepared(true), DefaultChoice::MakeDefault),
            "Opened the folder. Its default file-manager association was not changed."
        );
        if !gtk::glib::user_config_dir().starts_with(std::env::temp_dir()) {
            return;
        }
        let message = default_after_launch(&app, &prepared(false), DefaultChoice::MakeDefault);
        assert_eq!(message, "Opened with the selected application.");
        let default = gio::AppInfo::default_for_type("application/x-openxplorer-test", false);
        assert_eq!(default.and_then(|default| default.id()), app.id());
    }

    /// The menu offers the installed applications of a file's type up to
    /// its limit, without the file's default, which Open starts.
    ///
    /// parity: OPEN-013
    #[gtk::test]
    fn the_menu_offers_other_applications_up_to_its_limit() {
        const MENU_TYPE: &str = "application/x-openxplorer-menu-test";
        let data = gtk::glib::user_data_dir();
        let config = gtk::glib::user_config_dir();
        let temp = std::env::temp_dir();
        assert!(
            data.starts_with(&temp) && config.starts_with(&temp),
            "private folders"
        );
        let folder = data.join("applications");
        std::fs::create_dir_all(&folder).expect("the data folder is writable");
        let names = [
            ("org.openxplorer.MenuA.desktop", "Menu viewer A"),
            ("org.openxplorer.MenuB.desktop", "Menu viewer B"),
            ("org.openxplorer.MenuC.desktop", "Menu viewer C"),
        ];
        for (id, name) in names {
            let entry = format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec=true %F\nMimeType={MENU_TYPE};\n"
            );
            std::fs::write(folder.join(id), entry).expect("the data folder is writable");
        }
        crate::test_support::harness::wait_until("GIO to list the viewers", || {
            names.iter().all(|(id, _)| installed_application(id).is_some())
        });
        // GIO lists an application for a type that shared-mime-info does
        // not know only once the user associates them.
        for (id, _) in names {
            let viewer = installed_application(id).expect("installed");
            viewer
                .add_supports_type(MENU_TYPE)
                .expect("the private config folder is writable");
        }
        let viewer_a = installed_application(names[0].0).expect("installed");
        viewer_a
            .set_as_default_for_type(MENU_TYPE)
            .expect("the private config folder is writable");
        let offered = |limit| -> Vec<String> {
            menu_applications("file:///tmp/example", Some(MENU_TYPE), false, limit)
                .into_iter()
                .map(|choice| choice.id)
                .collect()
        };

        assert_eq!(offered(3), [names[1].0, names[2].0]);
        assert_eq!(offered(1), [names[1].0]);

        gio::AppInfo::reset_type_associations(MENU_TYPE);
        for (id, _) in names {
            std::fs::remove_file(folder.join(id)).expect("the test entry is removed");
        }
    }
}
