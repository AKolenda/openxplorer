// SPDX-License-Identifier: AGPL-3.0-only
//! The search options that narrow what a search shows by kind of item, by
//! when it was modified and by size.
//!
//! New in the native app, from Dolphin's search facets
//! (`DolphinFacetsWidget`: type and date, SRCH-037) and Windows File
//! Explorer's "Kind", "Date modified" and "Size" search options
//! (SRCH-043; Microsoft: "Select Search options on the top toolbar [...]
//! You can also select date modified, kind (like document, picture, or
//! video), size", <https://support.microsoft.com/en-us/office/foundations-experiences/find-what-you-need-with-microsoft-search>).
//! A kind is decided from the item's MIME type, as Dolphin's are from
//! Baloo's types; a date range is counted in local time from the start of
//! today, the week (Monday), the month or the year; the size groups are
//! those Microsoft documents for `System.Size` on Windows 10, version 1809
//! and later
//! (<https://learn.microsoft.com/en-us/windows/win32/properties/props-system-size>).
//! Options typed into the search box ([`super::TypedQuery`], SRCH-044)
//! and options chosen from the lists narrow together.

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
    pub fn label(self) -> &'static str {
        match self {
            KindFacet::Any => crate::i18n::gettext_static("Any kind"),
            KindFacet::Folders => crate::i18n::gettext_static("Folders"),
            KindFacet::Documents => crate::i18n::gettext_static("Documents"),
            KindFacet::Images => crate::i18n::gettext_static("Images"),
            KindFacet::Audio => crate::i18n::gettext_static("Audio files"),
            KindFacet::Videos => crate::i18n::gettext_static("Videos"),
        }
    }

    /// Whether `entry` is of this kind. An entry whose type is unknown,
    /// as a cached result's is, is judged by the type its name suggests.
    pub fn matches(self, entry: &Entry) -> bool {
        if matches!(self, KindFacet::Any | KindFacet::Folders) || entry.is_dir {
            return self.matches_type(entry, "");
        }
        if let Some(content_type) = entry.content_type.as_deref() {
            return self.matches_type(entry, content_type);
        }
        let (guessed, _) = gio::content_type_guess(Some(entry.name.as_str()), None);
        self.matches_type(entry, &guessed)
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
    /// From the Monday before this week's until this week's.
    LastWeek,
    /// Since the first of the month.
    ThisMonth,
    /// Since the first of January.
    ThisYear,
}

impl DateFacet {
    /// Every range, in the order the search options list them.
    pub const ALL: [DateFacet; 7] = [
        DateFacet::Any,
        DateFacet::Today,
        DateFacet::Yesterday,
        DateFacet::ThisWeek,
        DateFacet::LastWeek,
        DateFacet::ThisMonth,
        DateFacet::ThisYear,
    ];

    /// What the search options show.
    pub fn label(self) -> &'static str {
        match self {
            DateFacet::Any => crate::i18n::gettext_static("Any date"),
            DateFacet::Today => crate::i18n::gettext_static("Today"),
            DateFacet::Yesterday => crate::i18n::gettext_static("Yesterday"),
            DateFacet::ThisWeek => crate::i18n::gettext_static("This week"),
            DateFacet::LastWeek => crate::i18n::gettext_static("Last week"),
            DateFacet::ThisMonth => crate::i18n::gettext_static("This month"),
            DateFacet::ThisYear => crate::i18n::gettext_static("This year"),
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
            DateFacet::LastWeek => {
                let monday = today.add_days(1 - now.day_of_week()).ok()?;
                let before = monday.add_days(-7).ok()?;
                return Some((before.to_unix(), monday.to_unix()));
            }
            DateFacet::ThisMonth => today.add_days(1 - now.day_of_month()).ok()?,
            DateFacet::ThisYear => today.add_days(1 - now.day_of_year()).ok()?,
        };
        Some((from.to_unix(), i64::MAX))
    }
}

/// How big the files a search shows are. The groups and their first
/// sizes are `System.Size`'s on Windows 10, version 1809 and later; each
/// group ends where the next starts. Folders have no size, so a size
/// option shows files only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SizeFacet {
    /// Any size, folders too.
    #[default]
    Any,
    /// 0 bytes.
    Empty,
    /// From 1 byte: "Tiny (0 - 16 KB)".
    Tiny,
    /// From 16,383 bytes: "Small (16 - 1 MB)".
    Small,
    /// From 1,048,577 bytes: "Medium (1 - 128 MB)".
    Medium,
    /// From 134,217,729 bytes: "Large (128 MB - 1 GB)".
    Large,
    /// From 1,073,741,825 bytes: "Huge (1 - 4 GB)".
    Huge,
    /// From 4,294,967,297 bytes: "Gigantic (>4 GB)".
    Gigantic,
    /// More than this many bytes, as `size:>500` asks.
    Larger(u64),
    /// Fewer than this many bytes, as `size:<500` asks.
    Smaller(u64),
}

/// The first size of each group after Empty, in bytes (`minValue`).
const SIZE_GROUP_STARTS: [u64; 6] = [1, 16_383, 1_048_577, 134_217_729, 1_073_741_825, 4_294_967_297];

impl SizeFacet {
    /// Every group the search options list, in order.
    pub const ALL: [SizeFacet; 8] = [
        SizeFacet::Any,
        SizeFacet::Empty,
        SizeFacet::Tiny,
        SizeFacet::Small,
        SizeFacet::Medium,
        SizeFacet::Large,
        SizeFacet::Huge,
        SizeFacet::Gigantic,
    ];

    /// What the search options show.
    pub fn label(self) -> &'static str {
        match self {
            SizeFacet::Any => crate::i18n::gettext_static("Any size"),
            SizeFacet::Empty => crate::i18n::gettext_static("Empty (0 KB)"),
            SizeFacet::Tiny => crate::i18n::gettext_static("Tiny (0 - 16 KB)"),
            SizeFacet::Small => crate::i18n::gettext_static("Small (16 KB - 1 MB)"),
            SizeFacet::Medium => crate::i18n::gettext_static("Medium (1 - 128 MB)"),
            SizeFacet::Large => crate::i18n::gettext_static("Large (128 MB - 1 GB)"),
            SizeFacet::Huge => crate::i18n::gettext_static("Huge (1 - 4 GB)"),
            SizeFacet::Gigantic => crate::i18n::gettext_static("Gigantic (> 4 GB)"),
            SizeFacet::Larger(_) | SizeFacet::Smaller(_) => crate::i18n::gettext_static("Size"),
        }
    }

    /// The sizes in bytes the option admits, both ends included; `None`
    /// for any size, and an empty range (`from > until`) for none.
    fn bounds(self) -> Option<(u64, u64)> {
        let group = |index: usize| {
            let from = SIZE_GROUP_STARTS[index];
            let until = SIZE_GROUP_STARTS.get(index + 1).map_or(u64::MAX, |next| next - 1);
            Some((from, until))
        };
        match self {
            SizeFacet::Any => None,
            SizeFacet::Empty => Some((0, 0)),
            SizeFacet::Tiny => group(0),
            SizeFacet::Small => group(1),
            SizeFacet::Medium => group(2),
            SizeFacet::Large => group(3),
            SizeFacet::Huge => group(4),
            SizeFacet::Gigantic => group(5),
            SizeFacet::Larger(bytes) => Some(bytes.checked_add(1).map_or((1, 0), |from| (from, u64::MAX))),
            SizeFacet::Smaller(bytes) => Some(bytes.checked_sub(1).map_or((1, 0), |until| (0, until))),
        }
    }
}

/// The search options: kind, modification date and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchFacets {
    /// The kind of item shown.
    pub kind: KindFacet,
    /// When the items shown were modified.
    pub date: DateFacet,
    /// How big the files shown are.
    pub size: SizeFacet,
}

impl SearchFacets {
    /// Whether no option narrows the search.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// A check of entries against these options, with date ranges seen at
    /// the local time `now`.
    pub fn matcher(&self, now: &DateTime) -> FacetMatcher {
        self.matcher_with(&SearchFacets::default(), now)
    }

    /// A check of entries against these options and `other` together, as
    /// the lists' options and those typed into the box narrow together.
    pub fn matcher_with(&self, other: &SearchFacets, now: &DateTime) -> FacetMatcher {
        let (kind, kinds_clash) = match (self.kind, other.kind) {
            (KindFacet::Any, kind) | (kind, KindFacet::Any) => (kind, false),
            (one, two) => (one, one != two),
        };
        let range = both(self.date.range(now), other.date.range(now), |one, two| {
            (one.0.max(two.0), one.1.min(two.1))
        });
        let size = both(self.size.bounds(), other.size.bounds(), |one, two| {
            (one.0.max(two.0), one.1.min(two.1))
        });
        FacetMatcher {
            kind,
            kinds_clash,
            range,
            size,
        }
    }
}

/// `one` and `two` together: either alone, or `join` of both.
fn both<T>(one: Option<T>, two: Option<T>, join: impl FnOnce(T, T) -> T) -> Option<T> {
    match (one, two) {
        (Some(one), Some(two)) => Some(join(one, two)),
        (one, two) => one.or(two),
    }
}

/// [`SearchFacets`] with their date range and sizes worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FacetMatcher {
    kind: KindFacet,
    /// Two different kinds were asked for, which no item is.
    kinds_clash: bool,
    /// Modification times, `until` excluded.
    range: Option<(i64, i64)>,
    /// Sizes in bytes, both ends included.
    size: Option<(u64, u64)>,
}

impl FacetMatcher {
    /// Whether `entry` passes every option. An item with no known
    /// modification time passes no date range, and a folder, or a file of
    /// unknown size, passes no size.
    pub fn matches(&self, entry: &Entry) -> bool {
        let in_range = self.range.is_none_or(|(from, until)| {
            let modified = entry.modified.and_then(|seconds| i64::try_from(seconds).ok());
            modified.is_some_and(|seconds| (from..until).contains(&seconds))
        });
        let in_size = self.size.is_none_or(|(from, until)| {
            !entry.is_dir && entry.size.is_some_and(|size| (from..=until).contains(&size))
        });
        !self.kinds_clash && in_range && in_size && self.kind.matches(entry)
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

    fn sized(name: &str, size: Option<u64>) -> Entry {
        let mut item = entry(name, "text/plain", None);
        item.size = size;
        item
    }

    /// The size groups are Windows' own (`System.Size`, Windows 10
    /// version 1809 and later), each starting at its documented
    /// `minValue`; a folder, which has no size, is in none.
    ///
    /// parity: SRCH-043
    #[test]
    fn sizes_follow_the_documented_groups() {
        let sizes: [u64; 13] = [
            0,
            1,
            16_382,
            16_383,
            1_048_576,
            1_048_577,
            134_217_728,
            134_217_729,
            1_073_741_824,
            1_073_741_825,
            4_294_967_296,
            4_294_967_297,
            u64::MAX,
        ];
        let mut items: Vec<Entry> = sizes
            .iter()
            .map(|size| sized(&size.to_string(), Some(*size)))
            .collect();
        let mut folder = sized("Work", None);
        folder.is_dir = true;
        items.push(folder);
        let now = local(2026, 9, 16, 12);
        let shown = |size: SizeFacet| -> Vec<String> {
            let matcher = SearchFacets {
                size,
                ..SearchFacets::default()
            }
            .matcher(&now);
            items
                .iter()
                .filter(|item| matcher.matches(item))
                .map(|item| item.name.clone())
                .collect()
        };
        assert_eq!(shown(SizeFacet::Any).len(), 14);
        assert_eq!(shown(SizeFacet::Empty), ["0"]);
        assert_eq!(shown(SizeFacet::Tiny), ["1", "16382"]);
        assert_eq!(shown(SizeFacet::Small), ["16383", "1048576"]);
        assert_eq!(shown(SizeFacet::Medium), ["1048577", "134217728"]);
        assert_eq!(shown(SizeFacet::Large), ["134217729", "1073741824"]);
        assert_eq!(shown(SizeFacet::Huge), ["1073741825", "4294967296"]);
        assert_eq!(
            shown(SizeFacet::Gigantic),
            ["4294967297", u64::MAX.to_string().as_str()]
        );
        assert_eq!(shown(SizeFacet::Larger(16_382)).len(), 10);
        assert_eq!(shown(SizeFacet::Smaller(16_383)), ["0", "1", "16382"]);
        assert_eq!(shown(SizeFacet::Larger(4_294_967_296)).len(), 2);
        assert!(shown(SizeFacet::Smaller(0)).is_empty());
        assert_eq!(
            SizeFacet::ALL.map(SizeFacet::label),
            [
                "Any size",
                "Empty (0 KB)",
                "Tiny (0 - 16 KB)",
                "Small (16 KB - 1 MB)",
                "Medium (1 - 128 MB)",
                "Large (128 MB - 1 GB)",
                "Huge (1 - 4 GB)",
                "Gigantic (> 4 GB)",
            ]
        );
    }

    /// Last week runs from the Monday before this week's to this week's.
    ///
    /// parity: SRCH-043
    #[test]
    fn last_week_is_the_week_before_this_one() {
        // Wednesday 16 September 2026, noon.
        let now = local(2026, 9, 16, 12);
        let (from, until) = DateFacet::LastWeek.range(&now).expect("a range");
        assert_eq!(from, local(2026, 9, 7, 0).to_unix());
        assert_eq!(until, local(2026, 9, 14, 0).to_unix());
        assert!(DateFacet::ALL.contains(&DateFacet::LastWeek));
        assert_eq!(DateFacet::LastWeek.label(), "Last week");
    }

    /// Options typed into the box and options chosen from the lists
    /// narrow together: an item must pass both.
    ///
    /// parity: SRCH-044
    #[test]
    fn typed_and_chosen_options_narrow_together() {
        let now = local(2026, 9, 16, 12);
        let mut small_doc = entry("a.txt", "text/plain", at(&now));
        small_doc.size = Some(20_000);
        let mut tiny_doc = entry("b.txt", "text/plain", at(&now));
        tiny_doc.size = Some(10);
        let mut tiny_old = entry("c.txt", "text/plain", at(&local(2025, 1, 1, 9)));
        tiny_old.size = Some(10);
        let mut tiny_photo = entry("d.png", "image/png", at(&now));
        tiny_photo.size = Some(10);
        let all = [&small_doc, &tiny_doc, &tiny_old, &tiny_photo];
        let chosen = SearchFacets {
            kind: KindFacet::Documents,
            ..SearchFacets::default()
        };
        let typed = SearchFacets {
            size: SizeFacet::Tiny,
            date: DateFacet::ThisYear,
            ..SearchFacets::default()
        };
        let matcher = chosen.matcher_with(&typed, &now);
        let shown: Vec<&str> = all
            .iter()
            .filter(|item| matcher.matches(item))
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(shown, ["b.txt"]);
        let clash = SearchFacets {
            kind: KindFacet::Images,
            ..SearchFacets::default()
        };
        let matcher = chosen.matcher_with(&clash, &now);
        assert!(
            all.iter().all(|item| !matcher.matches(item)),
            "a document is not a picture"
        );
        let matcher = SearchFacets {
            date: DateFacet::Today,
            ..SearchFacets::default()
        }
        .matcher_with(
            &SearchFacets {
                date: DateFacet::Yesterday,
                ..SearchFacets::default()
            },
            &now,
        );
        assert!(
            all.iter().all(|item| !matcher.matches(item)),
            "today is not yesterday"
        );
    }
}
