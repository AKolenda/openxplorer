// SPDX-License-Identifier: AGPL-3.0-only
//! Where a search of a folder looks: the folder's own tree, the cache, or
//! both.
//!
//! Ports `cacheRootsFor`, `cacheCovers` and the choice `runSearch` and
//! `renderSearchInfo` make in `desktop/ui/app.js` (SRCH-003, SRCH-007,
//! SRCH-011). A folder no indexed folder covers, contains or sits under
//! is filtered at once and then searched live with its subfolders, as
//! Dolphin does (SRCH-035; Python filtered it only); one an indexed folder
//! covers is searched in the cache; one with only indexed subfolders gets
//! its own matches and the cached ones, because a cached child does not
//! cover its parent.

use ox_core::location::same_location;
use ox_core::search::IndexRoot;

/// The search scope the strip offers (`Search scope`, SRCH-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SearchScope {
    /// The folder and its subfolders (`folder` in app.js).
    #[default]
    ThisFolder,
    /// Every indexed folder, wherever the tab is (`all`).
    AllCachedFolders,
}

impl SearchScope {
    /// Both scopes, in the order the strip lists them.
    pub(crate) const ALL: [SearchScope; 2] = [SearchScope::ThisFolder, SearchScope::AllCachedFolders];

    /// What the strip's scope list shows.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            SearchScope::ThisFolder => "This folder + subfolders",
            SearchScope::AllCachedFolders => "All cached folders",
        }
    }
}

/// Where a search of a folder looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchSource {
    /// No indexed folder covers, contains or sits under the folder: its
    /// listing is filtered (SRCH-003), then its tree is walked live
    /// (SRCH-035).
    CurrentFolder,
    /// Only folders inside it are indexed: the listing's matches, then
    /// the cached ones below it (SRCH-007).
    CurrentFolderAndCachedSubfolders,
    /// An indexed folder covers it, or the scope is every cached folder:
    /// the cache.
    Cache,
}

impl SearchSource {
    /// Where a search of `folder` in `scope` looks, given the indexed
    /// folders `roots`.
    pub(crate) fn choose(roots: &[IndexRoot], folder: &str, scope: SearchScope) -> Self {
        if scope == SearchScope::AllCachedFolders {
            return SearchSource::Cache;
        }
        let mut related = related_roots(roots, folder).peekable();
        if related.peek().is_none() {
            return SearchSource::CurrentFolder;
        }
        if related.any(|root| covers(root, folder)) {
            SearchSource::Cache
        } else {
            SearchSource::CurrentFolderAndCachedSubfolders
        }
    }

    /// Whether the search asks the cache.
    pub(crate) const fn uses_cache(self) -> bool {
        !matches!(self, SearchSource::CurrentFolder)
    }

    /// What the search strip says it searched.
    pub(crate) const fn caption(self) -> &'static str {
        match self {
            SearchSource::CurrentFolder => "Current folder + subfolders",
            SearchSource::CurrentFolderAndCachedSubfolders => "Current folder + cached subfolders",
            SearchSource::Cache => "Cached names & paths",
        }
    }
}

/// The enabled roots that cover, contain or sit under `folder`
/// (`cacheRootsFor`), in the status's order.
pub(crate) fn related_roots<'a>(
    roots: &'a [IndexRoot],
    folder: &'a str,
) -> impl Iterator<Item = &'a IndexRoot> {
    roots.iter().filter(move |root| {
        let is_related = covers(root, folder) || is_below(&root.uri, folder);
        root.is_enabled() && is_related
    })
}

/// Whether `root` is `folder` or holds it (`cacheCovers`).
fn covers(root: &IndexRoot, folder: &str) -> bool {
    same_location(folder, &root.uri) || is_below(folder, &root.uri)
}

/// Whether `uri` lies inside `folder`: it starts with the folder and a
/// slash, so `/data2` is not inside `/data`.
fn is_below(uri: &str, folder: &str) -> bool {
    let prefix = format!("{}/", folder.trim_end_matches('/'));
    uri.starts_with(&prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::test_roots::enabled_root;

    /// One case of [`the_source_follows_the_indexed_folders_around_the_folder`].
    struct SourceCase {
        roots: &'static [&'static str],
        folder: &'static str,
        scope: SearchScope,
        source: SearchSource,
    }

    const SOURCE_CASES: [SourceCase; 6] = [
        SourceCase {
            roots: &[],
            folder: "file:///home/demo/Work",
            scope: SearchScope::ThisFolder,
            source: SearchSource::CurrentFolder,
        },
        SourceCase {
            roots: &["file:///home/demo/Work"],
            folder: "file:///home/demo/Work",
            scope: SearchScope::ThisFolder,
            source: SearchSource::Cache,
        },
        SourceCase {
            roots: &["file:///home/demo"],
            folder: "file:///home/demo/Work/2026",
            scope: SearchScope::ThisFolder,
            source: SearchSource::Cache,
        },
        SourceCase {
            roots: &["file:///home/demo/Work/2026"],
            folder: "file:///home/demo/Work",
            scope: SearchScope::ThisFolder,
            source: SearchSource::CurrentFolderAndCachedSubfolders,
        },
        SourceCase {
            roots: &["file:///home/demo/Work2"],
            folder: "file:///home/demo/Work",
            scope: SearchScope::ThisFolder,
            source: SearchSource::CurrentFolder,
        },
        SourceCase {
            roots: &[],
            folder: "file:///home/demo/Work",
            scope: SearchScope::AllCachedFolders,
            source: SearchSource::Cache,
        },
    ];

    /// Ported from `cacheRootsFor`, `cacheCovers` and `runSearch` in
    /// `desktop/ui/app.js`, including the 1.1.0 fix for a folder with only
    /// a cached child.
    ///
    /// parity: SRCH-003, SRCH-007, SRCH-011
    #[test]
    fn the_source_follows_the_indexed_folders_around_the_folder() {
        for case in SOURCE_CASES {
            let roots: Vec<IndexRoot> = case.roots.iter().map(|uri| enabled_root(uri)).collect();

            let source = SearchSource::choose(&roots, case.folder, case.scope);

            assert_eq!(source, case.source, "{} with {:?}", case.folder, case.roots);
        }
    }

    #[test]
    fn a_disabled_root_does_not_count() {
        let mut root = enabled_root("file:///home/demo/Work");
        root.caching = ox_core::search::Caching::Disabled;

        let source = SearchSource::choose(&[root], "file:///home/demo/Work", SearchScope::ThisFolder);

        assert_eq!(source, SearchSource::CurrentFolder);
    }
}
