// SPDX-License-Identifier: AGPL-3.0-only
//! Locations inside a ZIP, so a tab can browse one like a folder, as
//! Windows Explorer does ("Compressed (zipped) Folder").
//!
//! New in the native app (ARC-026). The URI is the app's own scheme,
//! which no GIO backend claims:
//!
//! ```text
//! ox-zip:<archive URI, escaped as one component>/<member path>
//! ox-zip:file%3A%2F%2F%2Fhome%2Fana%2FDownloads%2Ftidewater.zip/
//! ox-zip:file%3A%2F%2F%2Fhome%2Fana%2FDownloads%2Ftidewater.zip/tidewater/
//! ox-zip:file%3A%2F%2F%2Fhome%2Fana%2FDownloads%2Ftidewater.zip/tidewater/readme.txt
//! ```
//!
//! The archive's own URI is escaped whole, so it holds no `/` and the first
//! `/` ends it; each member path component is escaped like a virtual
//! folder's (Python's `quote(component, safe='')`). A folder ends with `/`;
//! the archive's root is the empty path. Member paths obey the archive
//! reader's safe-name rule (no `..`, no absolute or empty components), so
//! a location can never name anything outside its archive. The archive
//! itself must be a canonical location, as [`normalise_location`] gives.

use std::path::Path;

use super::text::{quote_component, unquote_without_controls};
use super::{normalise_location, LocationError};

/// The scheme of a location inside a ZIP.
pub const ARCHIVE_SCHEME: &str = "ox-zip:";

/// A folder or file inside a ZIP.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArchiveLocation {
    /// The ZIP's canonical location.
    pub archive_uri: String,
    /// The member's path inside it: `""` for the root, `"Docs/"` for a
    /// folder, `"Docs/a.txt"` for a file, as the archive reader names
    /// members.
    pub member: String,
}

impl ArchiveLocation {
    /// The root of the ZIP at `archive_uri`, made canonical first: GIO
    /// leaves characters such as `(` and `)` unescaped that the canonical
    /// form escapes (`Download (1).zip`). An address that is not a
    /// location is kept as it is, and [`Self::parse`] refuses it later.
    pub fn root(archive_uri: &str) -> Self {
        let canonical = super::normalise(archive_uri).unwrap_or_else(|_| archive_uri.to_owned());
        Self {
            archive_uri: canonical,
            member: String::new(),
        }
    }

    /// The member `member` of the same ZIP.
    #[must_use]
    pub fn member(&self, member: &str) -> Self {
        Self {
            archive_uri: self.archive_uri.clone(),
            member: member.to_owned(),
        }
    }

    /// Reads an `ox-zip:` location; `Ok(None)` for any other scheme.
    ///
    /// # Errors
    ///
    /// When the text is an `ox-zip:` location whose archive is not a
    /// location the app accepts, or whose member path is not a safe
    /// member name.
    pub fn parse(uri: &str) -> Result<Option<Self>, LocationError> {
        let Some(rest) = uri.strip_prefix(ARCHIVE_SCHEME) else {
            return Ok(None);
        };
        let invalid = || LocationError::new(crate::i18n::gettext("This is not a location inside a ZIP."));
        let (escaped_archive, escaped_member) = rest.split_once('/').ok_or_else(invalid)?;
        if escaped_member.contains(['?', '#']) {
            return Err(LocationError::query_or_fragment());
        }
        let archive = unquote_without_controls(escaped_archive)?;
        // Only a canonical location: a relative path would depend on the
        // folder the address was typed in. A canonical location is
        // absolute, so no home folder is needed: `~` never stays as typed.
        let archive_uri = normalise_location(&archive, None, Path::new("/"))?;
        if archive_uri != archive {
            return Err(invalid());
        }
        let is_folder = escaped_member.is_empty() || escaped_member.ends_with('/');
        let mut segments = Vec::new();
        for escaped in escaped_member.split('/').filter(|segment| !segment.is_empty()) {
            segments.push(unquote_without_controls(escaped)?);
        }
        let mut member = segments.join("/");
        if is_folder && !member.is_empty() {
            member.push('/');
        }
        if !member.is_empty() && !crate::archive::is_safe_member_name(&member) {
            return Err(invalid());
        }
        Ok(Some(Self { archive_uri, member }))
    }

    /// The location inside a ZIP that `uri` names; `None` for any other
    /// location and for a malformed `ox-zip:` one (see [`Self::parse`]).
    pub fn from_uri(uri: &str) -> Option<Self> {
        Self::parse(uri).ok().flatten()
    }

    /// The canonical `ox-zip:` URI.
    pub fn uri(&self) -> String {
        let segments: Vec<String> = self
            .member
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(quote_component)
            .collect();
        let mut path = segments.join("/");
        if self.is_folder() && !path.is_empty() {
            path.push('/');
        }
        format!("{ARCHIVE_SCHEME}{}/{path}", quote_component(&self.archive_uri))
    }

    /// True for the root and for folders inside the ZIP.
    pub fn is_folder(&self) -> bool {
        self.member.is_empty() || self.member.ends_with('/')
    }

    /// The member path's components, without the trailing `/`.
    pub fn segments(&self) -> Vec<&str> {
        self.member
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect()
    }

    /// The folder one level up inside the ZIP, or `None` at its root
    /// (where Up leaves the ZIP for the folder that holds it).
    pub fn parent(&self) -> Option<Self> {
        let segments = self.segments();
        let (_, parents) = segments.split_last()?;
        let mut member = parents.join("/");
        if !member.is_empty() {
            member.push('/');
        }
        Some(self.member(&member))
    }

    /// The prefix the archive browser lists for this location: the member
    /// itself for a folder, its folder for a file.
    pub fn listing_prefix(&self) -> String {
        if self.is_folder() {
            return self.member.clone();
        }
        self.parent().map(|parent| parent.member).unwrap_or_default()
    }
}

/// Whether `uri` is a location inside a ZIP (by its scheme only).
pub fn is_archive_location(uri: &str) -> bool {
    uri.starts_with(ARCHIVE_SCHEME)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZIP: &str = "file:///home/ana/Downloads/tide%20water.zip";

    fn parse(uri: &str) -> Result<Option<ArchiveLocation>, LocationError> {
        ArchiveLocation::parse(uri)
    }

    /// parity: ARC-026
    #[test]
    fn locations_round_trip_and_name_their_parents() {
        let root = ArchiveLocation::root(ZIP);
        assert_eq!(
            root.uri(),
            "ox-zip:file%3A%2F%2F%2Fhome%2Fana%2FDownloads%2Ftide%2520water.zip/"
        );
        let folder = root.member("tidewater/maps/");
        let file = root.member("tidewater/read me#1.txt");
        for location in [&root, &folder, &file] {
            assert_eq!(parse(&location.uri()).expect("valid").as_ref(), Some(location));
        }
        assert!(folder.uri().ends_with("/tidewater/maps/"));
        assert!(file.uri().ends_with("/tidewater/read%20me%231.txt"));
        assert_eq!(folder.parent(), Some(root.member("tidewater/")));
        assert_eq!(root.member("tidewater/").parent(), Some(root.clone()));
        assert_eq!(root.parent(), None);
        assert_eq!(file.listing_prefix(), "tidewater/");
        assert_eq!(folder.listing_prefix(), "tidewater/maps/");
        assert!(is_archive_location(&file.uri()));
        assert!(!is_archive_location(ZIP));
    }

    /// Nothing outside the archive can be named, and the archive itself
    /// must be a location the app accepts.
    ///
    /// parity: ARC-026
    #[test]
    fn unsafe_members_and_bad_archives_are_refused() {
        let root = ArchiveLocation::root(ZIP).uri();
        for member in ["../etc/passwd", "a/../../b", "a/./b", "a\\b", "a%00b"] {
            assert!(parse(&format!("{root}{member}")).is_err(), "{member}");
        }
        assert!(parse("ox-zip:not-a-location/").is_err());
        assert!(
            parse("ox-zip:file%3A%2F%2F%2Fa.zip").is_err(),
            "no path separator"
        );
        assert!(parse(&format!("{root}a?b")).is_err());
        assert_eq!(parse("file:///home/ana").expect("another scheme"), None);
    }

    /// The address bar, crumbs, titles and Up show a ZIP like a folder, as
    /// Explorer does, and nothing can be created in it.
    ///
    /// parity: ARC-026
    #[test]
    fn a_zip_is_shown_like_a_folder_and_is_read_only() {
        use crate::location::{crumb_divider, parent_location, LocationContext};

        let context = LocationContext {
            home: Some("/home/ana".into()),
            devices: Vec::new(),
            snapshot_roots: Vec::new(),
            network_mounts: Vec::new(),
        };
        let root = ArchiveLocation::root(ZIP);
        let maps = root.member("tidewater/maps/");
        assert_eq!(
            context.display_location(&maps.uri()),
            "/home/ana/Downloads/tide water.zip/tidewater/maps"
        );
        assert_eq!(context.title_for(&root.uri()), "tide water.zip");
        assert_eq!(context.base_name(&maps.uri()), "maps");
        let crumbs = context.breadcrumbs(&maps.uri());
        let labels: Vec<&str> = crumbs.iter().map(|crumb| crumb.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "/",
                "home",
                "ana",
                "Downloads",
                "tide water.zip",
                "tidewater",
                "maps"
            ]
        );
        assert_eq!(
            crumbs[4].uri,
            root.uri(),
            "the ZIP's crumb opens its root, not the file"
        );
        assert_eq!(crumbs[5].uri, root.member("tidewater/").uri());
        assert_eq!(
            crumb_divider(&maps.uri(), 1),
            None,
            "after the / root, as for folders"
        );
        assert_eq!(crumb_divider(&maps.uri(), 5), Some("/"));
        assert_eq!(
            parent_location(&maps.uri()),
            Some(root.member("tidewater/").uri())
        );
        assert_eq!(
            parent_location(&root.uri()).as_deref(),
            Some("file:///home/ana/Downloads"),
            "Up from the root leaves the ZIP"
        );
        assert!(!context.is_writable_location(&root.uri()));
        assert!(!context.is_writable_location(&maps.uri()));
        assert_eq!(
            crate::location::normalise_navigation(&maps.uri(), None, Path::new("/home/ana")),
            Ok(maps.uri())
        );
    }
}
