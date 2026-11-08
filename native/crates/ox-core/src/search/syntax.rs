// SPDX-License-Identifier: AGPL-3.0-only
//! Search options typed into the search box (SRCH-044).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{DateFacet, KindFacet, SearchFacets, SizeFacet};

    fn parsed(text: &str) -> (String, SearchFacets) {
        let query = TypedQuery::parse(text);
        (query.words, query.facets)
    }

    /// `kind:`, `datemodified:` (or `modified:`) and `size:` choose the
    /// same options as the strip's lists; the other words are searched
    /// for as before.
    ///
    /// parity: SRCH-044
    #[test]
    fn typed_options_are_taken_out_of_the_words() {
        let kind = |kind| SearchFacets {
            kind,
            ..SearchFacets::default()
        };
        assert_eq!(parsed("report kind:folder"), ("report".to_owned(), kind(KindFacet::Folders)));
        assert_eq!(parsed("kind:folders").1, kind(KindFacet::Folders));
        assert_eq!(parsed("kind:document").1, kind(KindFacet::Documents));
        assert_eq!(parsed("kind:docs").1, kind(KindFacet::Documents));
        assert_eq!(parsed("KIND:Picture").1, kind(KindFacet::Images));
        assert_eq!(parsed("kind:pics").1, kind(KindFacet::Images));
        assert_eq!(parsed("kind:music").1, kind(KindFacet::Audio));
        assert_eq!(parsed("kind:video").1, kind(KindFacet::Videos));
        assert_eq!(parsed("kind:videos").1, kind(KindFacet::Videos));

        let date = |date| SearchFacets {
            date,
            ..SearchFacets::default()
        };
        assert_eq!(parsed("datemodified:today").1, date(DateFacet::Today));
        assert_eq!(parsed("modified:yesterday").1, date(DateFacet::Yesterday));
        assert_eq!(
            parsed("notes datemodified:this week draft"),
            ("notes draft".to_owned(), date(DateFacet::ThisWeek))
        );
        assert_eq!(parsed("modified:last week").1, date(DateFacet::LastWeek));
        assert_eq!(parsed("datemodified:\"last week\"").1, date(DateFacet::LastWeek));

        let size = |size| SearchFacets {
            size,
            ..SearchFacets::default()
        };
        assert_eq!(parsed("size:empty").1, size(SizeFacet::Empty));
        assert_eq!(parsed("size:tiny").1, size(SizeFacet::Tiny));
        assert_eq!(parsed("size:small").1, size(SizeFacet::Small));
        assert_eq!(parsed("size:medium").1, size(SizeFacet::Medium));
        assert_eq!(parsed("size:large").1, size(SizeFacet::Large));
        assert_eq!(parsed("size:huge").1, size(SizeFacet::Huge));
        assert_eq!(parsed("Size:Gigantic").1, size(SizeFacet::Gigantic));
        assert_eq!(parsed("size:>500").1, size(SizeFacet::Larger(500)));
        assert_eq!(parsed("size:<500").1, size(SizeFacet::Smaller(500)));

        assert_eq!(
            parsed("kind:picture size:tiny datemodified:today holiday"),
            (
                "holiday".to_owned(),
                SearchFacets {
                    kind: KindFacet::Images,
                    date: DateFacet::Today,
                    size: SizeFacet::Tiny,
                }
            )
        );
    }

    /// What is not a known option stays a word to search for, so a name
    /// such as `kind:of magic.txt` is still found.
    ///
    /// parity: SRCH-044
    #[test]
    fn unknown_options_stay_words() {
        assert_eq!(
            parsed("kind:of magic"),
            ("kind:of magic".to_owned(), SearchFacets::default())
        );
        assert_eq!(
            parsed("size:>lots datemodified:this"),
            ("size:>lots datemodified:this".to_owned(), SearchFacets::default())
        );
        assert_eq!(
            parsed("datemodified:this month"),
            ("datemodified:this month".to_owned(), SearchFacets::default()),
            "only the values Microsoft documents are read"
        );
        assert_eq!(parsed("notes"), ("notes".to_owned(), SearchFacets::default()));
        assert!(TypedQuery::parse("notes").facets.is_empty());
        assert!(!TypedQuery::parse("kind:folder").facets.is_empty());
    }
}
