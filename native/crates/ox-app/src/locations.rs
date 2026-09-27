// SPDX-License-Identifier: AGPL-3.0-only
//! Locations as the window presents them.
//!
//! Real folders are handled by `ox_core::location`. This module adds what
//! the window needs on top: the virtual pages (`home:`, `pc:`, `network:`)
//! from `desktop/ui/app.js`, tab and window titles (`titleFor`), the
//! breadcrumb divider and small comparisons. ox-core does not model the
//! virtual pages, so they live here.

use ox_core::location::{self, Crumb};

/// A virtual page shown instead of a folder listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Quick access, network locations and recently opened files.
    Home,
    /// Quick access plus devices and drives.
    ThisPc,
    /// Saved network locations.
    Network,
}

impl Page {
    /// Every page, in sidebar order.
    pub const ALL: [Page; 3] = [Page::Home, Page::ThisPc, Page::Network];

    /// The pseudo-URI that identifies the page in tab history.
    pub const fn uri(self) -> &'static str {
        match self {
            Page::Home => location::HOME_URI,
            Page::ThisPc => location::PC_URI,
            Page::Network => location::NETWORK_URI,
        }
    }

    /// The page for a pseudo-URI, if it is one.
    pub fn from_uri(uri: &str) -> Option<Page> {
        match location::virtual_place(uri) {
            Some(location::VirtualPlace::Home) => Some(Self::Home),
            Some(location::VirtualPlace::ThisPc) => Some(Self::ThisPc),
            Some(location::VirtualPlace::Network) => Some(Self::Network),
            _ => None,
        }
    }

    /// The page typed into the address bar, by pseudo-URI or by title
    /// ("This PC"), ignoring case and surrounding spaces.
    pub fn from_address(text: &str) -> Option<Page> {
        let text = text.trim();
        Self::from_uri(text).or_else(|| {
            Self::ALL
                .into_iter()
                .find(|page| page.title().eq_ignore_ascii_case(text))
        })
    }

    /// Heading, tab title and breadcrumb label.
    pub const fn title(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::ThisPc => "This PC",
            Page::Network => "Network",
        }
    }

    /// The line under the heading.
    pub const fn subtitle(self) -> &'static str {
        match self {
            Page::Home => "Your folders and network locations, in one place.",
            Page::ThisPc => "Folders, devices, and connected storage.",
            Page::Network => "Connect to your NAS, Windows PC, or shared folders.",
        }
    }

    /// Glyph for the address bar and tab.
    pub const fn glyph(self) -> &'static str {
        match self {
            Page::Home => "home",
            Page::ThisPc => "desktop",
            Page::Network => "network",
        }
    }
}

/// True for SMB locations, which are drawn with the green network pipe.
pub fn is_network(uri: &str) -> bool {
    location::scheme(uri) == "smb"
}

/// Compares two locations, ignoring a trailing slash.
pub fn same_location(a: &str, b: &str) -> bool {
    fn trimmed(uri: &str) -> &str {
        let without = uri.trim_end_matches('/');
        if without.ends_with(':') || without.ends_with("://") {
            uri
        } else {
            without
        }
    }
    trimmed(a) == trimmed(b)
}

/// Tab and window title: the page title, "Home" for the home folder,
/// otherwise the folder name.
pub fn title_for(uri: &str, home_uri: &str) -> String {
    if let Some(page) = Page::from_uri(uri) {
        return page.title().to_string();
    }
    if same_location(uri, home_uri) {
        return Page::Home.title().to_string();
    }
    if !location::scheme(uri).eq_ignore_ascii_case("file") && is_root(uri) {
        return host(uri).unwrap_or_else(|| location::base_name(uri));
    }
    location::base_name(uri)
}

/// Text for the editable address bar.
pub fn address_text(uri: &str) -> String {
    match Page::from_uri(uri) {
        Some(page) => page.title().to_string(),
        None => location::display_location(uri),
    }
}

/// Breadcrumbs from the root to `uri`; a page has a single crumb.
pub fn crumbs(uri: &str) -> Vec<Crumb> {
    if let Some(page) = Page::from_uri(uri) {
        return vec![Crumb {
            label: page.title().to_string(),
            uri: uri.to_string(),
        }];
    }
    let mut crumbs = location::breadcrumbs(uri);
    // A remote root has no file name; label it with the host, as app.js does.
    if let (Some(first), false) = (crumbs.first_mut(), location::scheme(uri) == "file") {
        if let Some(name) = host(&first.uri) {
            first.label = name;
        }
    }
    crumbs
}

/// The separator drawn between breadcrumbs: `\` for SMB like Windows.
pub fn crumb_divider(uri: &str) -> &'static str {
    if is_network(uri) {
        "\\"
    } else {
        "/"
    }
}

/// The parent location for Up, or `None` for pages and roots.
pub fn parent(uri: &str) -> Option<String> {
    if Page::from_uri(uri).is_some() {
        return None;
    }
    location::parent_location(uri)
}

/// The host of a remote URI (`nas` for `smb://nas/share`).
fn host(uri: &str) -> Option<String> {
    let rest = uri.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    let without_user = authority.rsplit('@').next()?;
    (!without_user.is_empty()).then(|| without_user.to_string())
}

fn is_root(uri: &str) -> bool {
    uri.split_once("://")
        .map(|(_, rest)| rest.trim_end_matches('/').split('/').count() <= 1)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "file:///home/demo";

    #[test]
    fn pages_round_trip_through_their_uris() {
        for page in Page::ALL {
            assert_eq!(Page::from_uri(page.uri()), Some(page));
        }
        assert_eq!(Page::from_uri("file:///"), None);
    }

    #[test]
    fn pages_can_be_typed_by_title() {
        assert_eq!(Page::from_address(" this pc "), Some(Page::ThisPc));
        assert_eq!(Page::from_address("network:"), Some(Page::Network));
        assert_eq!(Page::from_address("/tmp"), None);
        assert_eq!(address_text("pc:"), "This PC");
    }

    #[test]
    fn titles_follow_app_js() {
        assert_eq!(title_for("pc:", HOME), "This PC");
        assert_eq!(title_for(HOME, HOME), "Home");
        assert_eq!(title_for("file:///home/demo/", HOME), "Home");
        assert_eq!(title_for("file:///srv/Brand%20assets", HOME), "Brand assets");
        assert_eq!(title_for("smb://nas/", HOME), "nas");
    }

    #[test]
    fn trailing_slashes_do_not_matter() {
        assert!(same_location("smb://nas/share/", "smb://nas/share"));
        assert!(!same_location("file:///a", "file:///b"));
        assert!(same_location("file:///", "file:///"));
    }

    #[test]
    fn remote_roots_are_labelled_with_the_host() {
        let crumbs = crumbs("smb://studio-nas/projects/Design");
        let labels: Vec<&str> = crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels.first(), Some(&"studio-nas"));
        assert_eq!(labels.last(), Some(&"Design"));
    }

    #[test]
    fn pages_have_one_crumb_and_no_parent() {
        assert_eq!(crumbs("network:").len(), 1);
        assert_eq!(parent("pc:"), None);
        assert_eq!(parent("file:///srv/data").as_deref(), Some("file:///srv"));
    }

    #[test]
    fn smb_breadcrumbs_use_a_backslash() {
        assert_eq!(crumb_divider("smb://nas/share"), "\\");
        assert_eq!(crumb_divider("file:///srv"), "/");
    }

    #[test]
    fn host_ignores_user_names() {
        assert_eq!(host("smb://ana@nas/share").as_deref(), Some("nas"));
        assert_eq!(host("file:///x"), None);
    }
}
