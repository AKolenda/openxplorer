// SPDX-License-Identifier: AGPL-3.0-only
//! Installed applications, as the opening policy and the Open with list
//! see them.
//!
//! The Python modules call `Gio.AppInfo` directly and their tests pass
//! mocks with the same methods. [`ApplicationInfo`] names exactly those
//! methods, so the policies can be tested with plain values, and
//! [`InstalledApplications`] is the desktop's application database.

/// What the opening policies ask of an installed application.
pub trait ApplicationInfo {
    /// The desktop ID, for example `org.gnome.Evince.desktop`; `None` for
    /// an application without a desktop file.
    fn id(&self) -> Option<String>;
    /// The name menus show.
    fn display_name(&self) -> String;
    /// False for launchers hidden from menus (`NoDisplay`, `OnlyShowIn`).
    fn should_show(&self) -> bool;
    /// True if it accepts local files as arguments.
    fn supports_files(&self) -> bool;
    /// True if it accepts URIs such as `smb://` as arguments.
    fn supports_uris(&self) -> bool;
}

impl ApplicationInfo for gio::AppInfo {
    fn id(&self) -> Option<String> {
        gio::prelude::AppInfoExt::id(self).map(Into::into)
    }

    fn display_name(&self) -> String {
        gio::prelude::AppInfoExt::display_name(self).into()
    }

    fn should_show(&self) -> bool {
        gio::prelude::AppInfoExt::should_show(self)
    }

    fn supports_files(&self) -> bool {
        gio::prelude::AppInfoExt::supports_files(self)
    }

    fn supports_uris(&self) -> bool {
        gio::prelude::AppInfoExt::supports_uris(self)
    }
}

/// Where applications for a content type are looked up.
pub trait ApplicationDatabase {
    /// The kind of application the database returns.
    type Application: ApplicationInfo;

    /// The user's default application for `content_type`, if any.
    fn default_for_type(&self, content_type: &str) -> Option<Self::Application>;

    /// Every application registered for `content_type`, recommended ones
    /// first.
    fn all_for_type(&self, content_type: &str) -> Vec<Self::Application>;
}

/// The applications installed on the desktop, through GIO.
///
/// `GAppInfo` values cannot leave the thread that created them, so the
/// policies return plain IDs and names; the caller launches the chosen
/// application again from its ID on the main thread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstalledApplications;

impl ApplicationDatabase for InstalledApplications {
    type Application = gio::AppInfo;

    fn default_for_type(&self, content_type: &str) -> Option<gio::AppInfo> {
        // `must_support_uris` false: an application that needs a local
        // path is still the default; the caller supplies one or refuses.
        gio::AppInfo::default_for_type(content_type, false)
    }

    fn all_for_type(&self, content_type: &str) -> Vec<gio::AppInfo> {
        gio::AppInfo::all_for_type(content_type)
    }
}
