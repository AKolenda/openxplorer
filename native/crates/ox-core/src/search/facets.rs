// SPDX-License-Identifier: AGPL-3.0-only
//! The search options that narrow what a search shows by kind of item and
//! by when it was modified.
//!
//! New in the native app, from Dolphin's search facets
//! (`DolphinFacetsWidget`: type and date, SRCH-037) and Windows File
//! Explorer's "Kind" and "Date modified" search options. A kind is decided
//! from the item's MIME type, as Dolphin's are from Baloo's types; a date
//! range is counted in local time from the start of today, the week
//! (Monday), the month or the year.

use glib::DateTime;

use crate::entry::Entry;

/// The kinds of item a search can be narrowed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KindFacet {
    /// Every item.
    #[default]
    Any,
    /// Folders only.
    Folders,
    /// Text, PDF, office and e-book files.
    Documents,
    /// Pictures.
    Images,
    /// Music and other sound.
    Audio,
    /// Films and clips.
    Videos,
}

/// MIME types, other than `text/*`, that count as documents.
const DOCUMENT_TYPES: [&str; 9] = [
    "application/pdf",
    "application/rtf",
    "application/msword",
    "application/vnd.ms-excel",
    "application/vnd.ms-powerpoint",
    "application/epub+zip",
    "application/x-tex",
    "application/vnd.openxmlformats-officedocument.",
    "application/vnd.oasis.opendocument.",
];

impl KindFacet {
    /// Every kind, in the order the search options list them.
    pub const ALL: [KindFacet; 6] = [
        KindFacet::Any,
        KindFacet::Folders,
        KindFacet::Documents,
        KindFacet::Images,
        KindFacet::Audio,
        KindFacet::Videos,
    ];

    /// What the search options show.
    pub const fn label(self) -> &'static str {
        match self {
            KindFacet::Any => "Any kind",
            KindFacet::Folders => "Folders",
            KindFacet::Documents => "Documents",
            KindFacet::Images => "Images",
            KindFacet::Audio => "Audio files",
            KindFacet::Videos => "Videos",
        }
    }

    /// Whether `entry` is of this kind. An entry whose type is unknown,
    /// as a cached result's is, is judged by the type its name suggests.
    pub fn matches(self, entry: &Entry) -> bool {
        if matches!(self, KindFacet::Any | KindFacet::Folders) || entry.is_dir {
            return self.matches_type(entry, "");
        }
        match entry.content_type.as_deref() {
            Some(content_type) => self.matches_type(entry, content_type),
            None => {
                let (guessed, _) = gio::content_type_guess(Some(entry.name.as_str()), None);
                self.matches_type(entry, &guessed)
            }
        }
    }

    /// Whether `entry`, of `content_type`, is of this kind.
    fn matches_type(self, entry: &Entry, content_type: &str) -> bool {
        match self {
            KindFacet::Any => true,
            KindFacet::Folders => entry.is_dir,
            _ if entry.is_dir => false,
            KindFacet::Documents => {
                content_type.starts_with("text/")
                    || DOCUMENT_TYPES.iter().any(|known| {
                        content_type == *known || (known.ends_with('.') && content_type.starts_with(known))
                    })
            }
            KindFacet::Images => content_type.starts_with("image/"),
            KindFacet::Audio => content_type.starts_with("audio/"),
            KindFacet::Videos => content_type.starts_with("video/"),
        }
    }
}

/// When the items a search shows were modified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateFacet {
    /// Any time.
    #[default]
    Any,
    /// Since the start of today.
    Today,
    /// During yesterday.
    Yesterday,
    /// Since Monday.
    ThisWeek,
    /// Since the first of the month.
    ThisMonth,
    /// Since the first of January.
    ThisYear,
}

impl DateFacet {
    /// Every range, in the order the search options list them.
    pub const ALL: [DateFacet; 6] = [
        DateFacet::Any,
        DateFacet::Today,
        DateFacet::Yesterday,
        DateFacet::ThisWeek,
        DateFacet::ThisMonth,
        DateFacet::ThisYear,
    ];

    /// What the search options show.
    pub const fn label(self) -> &'static str {
        match self {
            DateFacet::Any => "Any date",
            DateFacet::Today => "Today",
            DateFacet::Yesterday => "Yesterday",
            DateFacet::ThisWeek => "This week",
            DateFacet::ThisMonth => "This month",
            DateFacet::ThisYear => "This year",
        }
    }

    /// The range of modification times, in seconds since the Unix epoch,
    /// as `(from, until)` with `until` excluded, seen at the local time
    /// `now`; `None` for any time.
    pub fn range(self, now: &DateTime) -> Option<(i64, i64)> {
        let today = DateTime::new(
            &now.timezone(),
            now.year(),
            now.month(),
            now.day_of_month(),
            0,
            0,
            0.0,
        )
        .ok()?;
        let from = match self {
            DateFacet::Any => return None,
            DateFacet::Today => today.clone(),
            DateFacet::Yesterday => {
                let yesterday = today.add_days(-1).ok()?;
                return Some((yesterday.to_unix(), today.to_unix()));
            }
            DateFacet::ThisWeek => today.add_days(1 - now.day_of_week()).ok()?,
            DateFacet::ThisMonth => today.add_days(1 - now.day_of_month()).ok()?,
            DateFacet::ThisYear => today.add_days(1 - now.day_of_year()).ok()?,
        };
        Some((from.to_unix(), i64::MAX))
    }
}

/// The search options: kind and modification date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchFacets {
    /// The kind of item shown.
    pub kind: KindFacet,
    /// When the items shown were modified.
    pub date: DateFacet,
}

impl SearchFacets {
    /// Whether no option narrows the search.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// A check of entries against these options, with date ranges seen at
    /// the local time `now`.
    pub fn matcher(&self, now: &DateTime) -> FacetMatcher {
        FacetMatcher {
            kind: self.kind,
            range: self.date.range(now),
        }
    }
}

/// [`SearchFacets`] with their date range worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FacetMatcher {
    kind: KindFacet,
    range: Option<(i64, i64)>,
}

impl FacetMatcher {
    /// Whether `entry` passes every option. An item with no known
    /// modification time passes no date range.
    pub fn matches(&self, entry: &Entry) -> bool {
        let in_range = self.range.is_none_or(|(from, until)| {
            let modified = entry.modified.and_then(|seconds| i64::try_from(seconds).ok());
            modified.is_some_and(|seconds| (from..until).contains(&seconds))
        });
        in_range && self.kind.matches(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::entry_from_info;

    fn entry(name: &str, content_type: &str, modified: Option<u64>) -> Entry {
        let info = gio::FileInfo::new();
        info.set_display_name(name);
        let file = gio::File::for_uri(&format!("file:///tmp/{name}"));
        let mut entry = entry_from_info(&file, &info);
        entry.content_type = Some(content_type.to_owned());
        entry.modified = modified;
        entry
    }

    fn local(year: i32, month: i32, day: i32, hour: i32) -> DateTime {
        DateTime::new(&glib::TimeZone::local(), year, month, day, hour, 0, 0.0).unwrap()
    }

    fn at(time: &DateTime) -> Option<u64> {
        u64::try_from(time.to_unix()).ok()
    }

    /// parity: SRCH-037
    #[test]
    fn kinds_and_dates_narrow_what_is_shown() {
        // Wednesday 16 September 2026, noon.
        let now = local(2026, 9, 16, 12);
        let report = entry(
            "report.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            at(&now),
        );
        let photo = entry("photo.jpg", "image/jpeg", at(&local(2026, 9, 15, 9)));
        let song = entry("song.ogg", "audio/ogg", at(&local(2026, 9, 14, 9)));
        let clip = entry("clip.mp4", "video/mp4", at(&local(2026, 1, 3, 9)));
        let mut folder = entry("Work", "inode/directory", None);
        folder.is_dir = true;
        let all = [&report, &photo, &song, &clip, &folder];
        let shown = |facets: SearchFacets| -> Vec<&str> {
            let matcher = facets.matcher(&now);
            all.iter()
                .filter(|item| matcher.matches(item))
                .map(|item| item.name.as_str())
                .collect()
        };
        let kind = |kind| SearchFacets {
            kind,
            ..SearchFacets::default()
        };
        let date = |date| SearchFacets {
            date,
            ..SearchFacets::default()
        };

        assert_eq!(shown(SearchFacets::default()).len(), 5);
        assert_eq!(shown(kind(KindFacet::Folders)), ["Work"]);
        assert_eq!(shown(kind(KindFacet::Documents)), ["report.docx"]);
        assert_eq!(shown(kind(KindFacet::Images)), ["photo.jpg"]);
        assert_eq!(shown(kind(KindFacet::Audio)), ["song.ogg"]);
        assert_eq!(shown(kind(KindFacet::Videos)), ["clip.mp4"]);
        let mut cached = entry("scan.png", "", None);
        cached.content_type = None;
        assert!(
            KindFacet::Images.matches(&cached),
            "a cached result's type is guessed from its name"
        );
        assert_eq!(shown(date(DateFacet::Today)), ["report.docx"]);
        assert_eq!(shown(date(DateFacet::Yesterday)), ["photo.jpg"]);
        assert_eq!(
            shown(date(DateFacet::ThisWeek)),
            ["report.docx", "photo.jpg", "song.ogg"]
        );
        assert_eq!(shown(date(DateFacet::ThisMonth)).len(), 3);
        assert_eq!(
            shown(date(DateFacet::ThisYear)).len(),
            4,
            "the folder has no time"
        );
    }
}
