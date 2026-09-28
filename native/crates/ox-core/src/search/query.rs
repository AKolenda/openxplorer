// SPDX-License-Identifier: AGPL-3.0-only
//! Searching the cache.
//!
//! Ports `search` in `desktop/search_index.py` (SRCH-007 to SRCH-011 and
//! SRCH-017).

use std::time::{Duration, Instant};

use gio::prelude::*;
use rusqlite::types::Value;
use rusqlite::{Connection, Row};

use super::error::{check_cancelled, SearchError};
use super::index::SearchIndex;
use super::root::{HiddenItems, RootStatus, SearchEngine};
use super::text::{display_path, fold, folder_prefix, whole_seconds};
use crate::entry::EntryKind;
use crate::location::normalise;

/// Longest search text, in characters.
pub const MAX_QUERY_CHARS: usize = 512;

/// Results returned when the caller does not choose a limit.
pub const DEFAULT_RESULT_LIMIT: usize = 500;

/// Most results one search can return.
pub const MAX_RESULT_LIMIT: usize = 2000;

/// Most distinct words of a query that are matched.
const MAX_QUERY_WORDS: usize = 20;

/// Words this long use the trigram index; shorter ones are too short for
/// trigrams and only get the substring check.
const MIN_TRIGRAM_WORD_CHARS: usize = 3;

/// SQLite virtual machine steps between two cancellation checks (SRCH-017).
const STEPS_PER_CANCEL_CHECK: i32 = 4000;

/// Every [`EntryKind`], to read the stored kind words back.
const ENTRY_KINDS: [EntryKind; 7] = [
    EntryKind::Directory,
    EntryKind::File,
    EntryKind::Symlink,
    EntryKind::Special,
    EntryKind::Mountable,
    EntryKind::Shortcut,
    EntryKind::Unknown,
];

/// What to search for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// Words that must all occur in an item's name or parent path.
    pub text: String,
    /// Only this folder and its subfolders; `None` for every cached folder
    /// ("All cached folders").
    pub scope: Option<String>,
    /// Most results to return, 1 to [`MAX_RESULT_LIMIT`].
    pub limit: usize,
    /// Whether hidden items are returned (the "Hidden items" setting).
    pub hidden_items: HiddenItems,
}

impl SearchQuery {
    /// A search of every cached folder for `text`, with the default limit,
    /// leaving out hidden items.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            scope: None,
            limit: DEFAULT_RESULT_LIMIT,
            hidden_items: HiddenItems::Skip,
        }
    }
}

/// One cached item that matched.
///
/// Results are never virtual and can always be operated on: the index
/// holds no shares, shortcuts or links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// The item's URI.
    pub uri: String,
    /// The folder that holds it (the "Folder" column).
    pub parent_uri: String,
    /// Display name.
    pub name: String,
    /// Opens as a folder. A cached regular file never does.
    pub is_dir: bool,
    /// Size in bytes; `None` for folders.
    pub size: Option<u64>,
    /// Modification time in seconds since the epoch.
    pub modified: Option<u64>,
    /// The Type column text.
    pub type_label: String,
    /// What GIO said the item was.
    pub kind: EntryKind,
    /// Hidden by name or by the backend.
    pub is_hidden: bool,
    /// The item's display path, a UNC path for SMB.
    pub path: String,
    /// When the item was cached, in seconds since the epoch.
    pub cached_at: Option<u64>,
    /// When its root's last complete scan finished.
    pub root_updated: Option<u64>,
    /// Its root's status.
    pub root_status: RootStatus,
}

/// The matches of one search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResults {
    /// Folders first, then by name ignoring case.
    pub hits: Vec<SearchHit>,
    /// More items matched than [`SearchResults::limit`].
    pub is_truncated: bool,
    /// The limit that applied.
    pub limit: usize,
    /// How long the search took.
    pub elapsed: Duration,
}

impl SearchIndex {
    /// Searches the enabled roots (`search` in Python). Every word of the
    /// query must occur, ignoring case, in the item's name followed by its
    /// parent path; an empty query matches nothing.
    ///
    /// A running query checks `cancellable` every 4,000 SQLite steps, so a
    /// newer keystroke stops it instead of waiting behind it.
    ///
    /// # Errors
    ///
    /// [`SearchError::QueryTooLong`] for more than [`MAX_QUERY_CHARS`]
    /// characters, [`SearchError::Cancelled`], [`SearchError::Location`]
    /// for an invalid scope, and database errors.
    pub fn search(
        &self,
        query: &SearchQuery,
        cancellable: Option<&gio::Cancellable>,
    ) -> Result<SearchResults, SearchError> {
        if let Some(cancellable) = cancellable {
            check_cancelled(cancellable)?;
        }
        if query.text.chars().count() > MAX_QUERY_CHARS {
            return Err(SearchError::QueryTooLong);
        }
        let scope = query.scope.as_deref().map(normalise).transpose()?;
        let limit = query.limit.clamp(1, MAX_RESULT_LIMIT);
        let started = Instant::now();
        let words = query_words(&query.text);
        if words.is_empty() {
            return Ok(SearchResults {
                hits: Vec::new(),
                is_truncated: false,
                limit,
                elapsed: Duration::ZERO,
            });
        }
        let plan = QueryPlan::new(&words, scope.as_deref(), query.hidden_items, self.engine(), limit);
        let connection = self.connect()?;
        let mut hits = run_cancellable(&connection, &plan, cancellable)?;
        let is_truncated = hits.len() > limit;
        hits.truncate(limit);
        Ok(SearchResults {
            hits,
            is_truncated,
            limit,
            elapsed: started.elapsed(),
        })
    }
}

/// The folded, distinct words of `text`, at most [`MAX_QUERY_WORDS`]
/// (`list(dict.fromkeys(fold(text).split()))[:20]` in Python).
fn query_words(text: &str) -> Vec<String> {
    let folded = fold(text);
    let mut words: Vec<String> = Vec::new();
    for word in folded.split(is_python_space).filter(|word| !word.is_empty()) {
        if !words.iter().any(|known| known == word) {
            words.push(word.to_owned());
        }
    }
    words.truncate(MAX_QUERY_WORDS);
    words
}

/// Whether Python's `str.split()` splits at `character`: Unicode
/// `White_Space` plus the information separators U+001C to U+001F, which
/// Rust's `split_whitespace` keeps.
fn is_python_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

/// The SQL of one search and the values bound to it.
struct QueryPlan {
    sql: String,
    values: Vec<Value>,
}

impl QueryPlan {
    /// Builds the statement. Every word and path is bound as a value, never
    /// written into the SQL.
    fn new(
        words: &[String],
        scope: Option<&str>,
        hidden_items: HiddenItems,
        engine: SearchEngine,
        limit: usize,
    ) -> Self {
        let mut plan = Self {
            sql: String::new(),
            values: Vec::new(),
        };
        let mut conditions = vec!["r.enabled=1".to_owned()];
        if engine == SearchEngine::Trigram {
            conditions.extend(plan.trigram_condition(words));
        }
        for word in words {
            conditions.push("instr(e.search_text, ?)>0".to_owned());
            plan.values.push(Value::Text(word.clone()));
        }
        if let Some(scope) = scope {
            conditions.push(plan.scope_condition(scope));
        }
        if hidden_items == HiddenItems::Skip {
            conditions.push("e.hidden=0".to_owned());
        }
        // One row more than the limit tells whether results were cut off.
        plan.values.push(Value::Integer(sql_integer(limit + 1)));
        plan.sql = format!(
            "SELECT e.uri, e.parent, e.name, e.is_dir, e.size, e.modified, e.type, e.kind, e.hidden, e.seen,
                    r.updated, r.status
             FROM entries e JOIN roots r ON r.uri=e.root
             WHERE {}
             GROUP BY e.uri
             ORDER BY e.is_dir DESC, e.name COLLATE NOCASE, e.uri
             LIMIT ?",
            conditions.join(" AND "),
        );
        plan
    }

    /// The trigram prefilter for the words long enough to have trigrams;
    /// `None` when there are none.
    ///
    /// Safety rule "a filename is never query syntax" (SRCH-010): each word
    /// is quoted as an FTS5 string, doubling its quotes, and the whole
    /// expression is bound as a value.
    fn trigram_condition(&mut self, words: &[String]) -> Option<String> {
        let quoted: Vec<String> = words
            .iter()
            .filter(|word| word.chars().count() >= MIN_TRIGRAM_WORD_CHARS)
            .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
            .collect();
        if quoted.is_empty() {
            return None;
        }
        self.values.push(Value::Text(quoted.join(" AND ")));
        Some("e.id IN (SELECT rowid FROM names_fts WHERE names_fts MATCH ?)".to_owned())
    }

    /// Limits results to `scope` and everything below it.
    fn scope_condition(&mut self, scope: &str) -> String {
        let prefix = folder_prefix(scope);
        // SQLite's substr counts characters, not bytes.
        let prefix_chars = sql_integer(prefix.chars().count());
        self.values.push(Value::Text(scope.to_owned()));
        self.values.push(Value::Integer(prefix_chars));
        self.values.push(Value::Text(prefix));
        "(e.uri=? OR substr(e.uri, 1, ?)=?)".to_owned()
    }
}

/// Runs `plan`, stopping when `cancellable` is cancelled.
fn run_cancellable(
    connection: &Connection,
    plan: &QueryPlan,
    cancellable: Option<&gio::Cancellable>,
) -> Result<Vec<SearchHit>, SearchError> {
    if let Some(cancellable) = cancellable {
        let cancellable = cancellable.clone();
        connection.progress_handler(STEPS_PER_CANCEL_CHECK, Some(move || cancellable.is_cancelled()))?;
    }
    let hits = read_hits(connection, plan);
    match (hits, cancellable) {
        // SQLite reports the interruption as its own error; the caller
        // needs to know it was a cancellation.
        (Err(_), Some(cancellable)) if cancellable.is_cancelled() => Err(SearchError::Cancelled),
        (hits, _) => Ok(hits?),
    }
}

/// Runs `plan` and reads every row.
fn read_hits(connection: &Connection, plan: &QueryPlan) -> rusqlite::Result<Vec<SearchHit>> {
    let mut statement = connection.prepare(&plan.sql)?;
    let parameters = rusqlite::params_from_iter(plan.values.iter());
    let hits = statement.query_map(parameters, hit_from_row)?;
    hits.collect()
}

/// Reads one result row.
fn hit_from_row(row: &Row<'_>) -> rusqlite::Result<SearchHit> {
    let uri: String = row.get("uri")?;
    let kind = entry_kind_from_stored(&row.get::<_, String>("kind")?);
    let size: Option<i64> = row.get("size")?;
    Ok(SearchHit {
        path: display_path(&uri),
        is_dir: opens_as_folder(kind, row.get("is_dir")?),
        uri,
        parent_uri: row.get("parent")?,
        name: row.get("name")?,
        size: size.and_then(|size| u64::try_from(size).ok()),
        modified: whole_seconds(row.get("modified")?),
        type_label: row.get("type")?,
        kind,
        is_hidden: row.get("hidden")?,
        cached_at: whole_seconds(row.get("seen")?),
        root_updated: whole_seconds(row.get("updated")?),
        root_status: RootStatus::from_stored(&row.get::<_, String>("status")?),
    })
}

/// Whether a cached item opens as a folder. The stored kind decides: a
/// regular file, device node or link never does, even if an older row
/// says it is a folder (SRCH-009).
fn opens_as_folder(kind: EntryKind, stored_is_dir: bool) -> bool {
    match kind {
        EntryKind::Directory => true,
        EntryKind::File | EntryKind::Special | EntryKind::Symlink => false,
        EntryKind::Mountable | EntryKind::Shortcut | EntryKind::Unknown => stored_is_dir,
    }
}

/// The kind a stored word names; unknown words are [`EntryKind::Unknown`].
fn entry_kind_from_stored(word: &str) -> EntryKind {
    ENTRY_KINDS
        .into_iter()
        .find(|kind| kind.as_str() == word)
        .unwrap_or(EntryKind::Unknown)
}

/// A count as an SQLite integer.
fn sql_integer(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::fixtures::{found_names, listed_file, listed_folder, search, ScannedShare, SHARE};
    use crate::search::scan::ListedItem;

    /// A search of every cached folder for `text`, returning at most
    /// `limit` hits.
    fn limited(text: &str, limit: usize) -> SearchQuery {
        SearchQuery {
            limit,
            ..SearchQuery::new(text)
        }
    }

    /// A search for `text` below `scope`.
    fn scoped(text: &str, scope: &str) -> SearchQuery {
        SearchQuery {
            scope: Some(scope.to_owned()),
            ..SearchQuery::new(text)
        }
    }

    /// parity: SRCH-008
    #[test]
    fn query_words_are_folded_distinct_and_limited() {
        assert_eq!(query_words("Bank  bank BANK Report"), ["bank", "report"]);
        assert_eq!(query_words("a\u{1f}b"), ["a", "b"]);
        let many: Vec<String> = (0..30).map(|number| format!("w{number}")).collect();
        assert_eq!(query_words(&many.join(" ")).len(), MAX_QUERY_WORDS);
    }

    /// parity: SRCH-008
    #[test]
    fn every_word_must_occur_in_the_name_or_parent_path() {
        let share = ScannedShare::new();
        let reports = format!("{SHARE}/Reports");
        share.store(&[
            listed_file(&reports, "q3-bank.pdf"),
            listed_file(SHARE, "bank.pdf"),
        ]);

        let in_reports = share.found_names("REPORTS bank");
        let everywhere = share.found_names("Bank");

        assert_eq!(in_reports, ["q3-bank.pdf"]);
        assert_eq!(everywhere, ["bank.pdf", "q3-bank.pdf"]);
    }

    /// parity: SRCH-008
    #[test]
    fn a_query_longer_than_512_characters_is_refused() {
        let share = ScannedShare::new();
        let longest = "é".repeat(MAX_QUERY_CHARS);
        let too_long = "é".repeat(MAX_QUERY_CHARS + 1);

        let accepted = share.index.search(&SearchQuery::new(longest), None);
        let refused = share.index.search(&SearchQuery::new(too_long), None);

        assert!(accepted.is_ok());
        let error = refused.expect_err("513 characters are too many");
        assert_eq!(error.to_string(), "Search must be at most 512 characters.");
    }

    /// parity: SRCH-008
    #[test]
    fn an_empty_query_finds_nothing() {
        let share = ScannedShare::new();
        share.store(&[listed_file(SHARE, "bank.pdf")]);

        let results = share.index.search(&SearchQuery::new(" \t "), None).unwrap();

        assert!(results.hits.is_empty());
        assert!(!results.is_truncated);
        assert_eq!(results.elapsed, Duration::ZERO);
    }

    /// parity: SRCH-009
    #[test]
    fn folders_come_first_then_names_ignoring_case() {
        let share = ScannedShare::new();
        share.store(&[
            listed_file(SHARE, "item b.txt"),
            listed_file(SHARE, "Item a.txt"),
            listed_folder(SHARE, "item z"),
        ]);

        assert_eq!(share.found_names("item"), ["item z", "Item a.txt", "item b.txt"]);
    }

    /// parity: SRCH-009
    #[test]
    fn results_beyond_the_limit_are_cut_off_and_flagged() {
        let share = ScannedShare::new();
        let items: Vec<ListedItem> = ["a", "b", "c"]
            .into_iter()
            .map(|name| listed_file(SHARE, &format!("{name}-report.txt")))
            .collect();
        share.store(&items);

        let cut_off = share.index.search(&limited("report", 2), None).unwrap();
        let complete = share.index.search(&limited("report", 3), None).unwrap();

        assert_eq!(cut_off.hits.len(), 2);
        assert!(cut_off.is_truncated);
        assert_eq!(complete.hits.len(), 3);
        assert!(!complete.is_truncated);
    }

    /// parity: SRCH-009
    #[test]
    fn the_limit_stays_between_1_and_2000() {
        let share = ScannedShare::new();

        let smallest = share.index.search(&limited("report", 0), None).unwrap();
        let largest = share.index.search(&limited("report", 5000), None).unwrap();

        assert_eq!(smallest.limit, 1);
        assert_eq!(largest.limit, MAX_RESULT_LIMIT);
        assert_eq!(SearchQuery::new("report").limit, DEFAULT_RESULT_LIMIT);
    }

    /// A hit carries what the search results show: size, time, type and
    /// kind, when it was cached, and its root's state.
    ///
    /// parity: SRCH-009
    #[test]
    fn a_hit_carries_its_metadata_and_root_state() {
        let share = ScannedShare::new();
        share.store(&[listed_file(SHARE, "bank.pdf")]);

        let hit = &search(&share.index, "bank")[0];

        assert_eq!(hit.uri, "smb://nas/share/bank.pdf");
        assert_eq!(hit.size, Some(12));
        assert_eq!(hit.modified, Some(1));
        assert_eq!(hit.type_label, "File");
        assert_eq!(hit.kind, EntryKind::File);
        assert!(!hit.is_hidden);
        assert!(hit.cached_at.is_some());
        assert!(hit.root_updated.is_some());
        assert_eq!(hit.root_status, RootStatus::Ready);
    }

    /// The scope is the folder and its subfolders; a sibling whose name
    /// starts the same is outside it. Without a scope every cached folder
    /// is searched ("All cached folders").
    ///
    /// parity: SRCH-007, SRCH-011
    #[test]
    fn a_scope_limits_results_to_its_folder_and_subfolders() {
        let share = ScannedShare::new();
        let folder = format!("{SHARE}/a");
        let sibling = format!("{SHARE}/ab");
        share.store(&[
            listed_file(&folder, "one.txt"),
            listed_file(&format!("{folder}/deep"), "two.txt"),
            listed_file(&sibling, "three.txt"),
        ]);

        let in_scope = share.index.search(&scoped("txt", &folder), None).unwrap();
        let names: Vec<&str> = in_scope.hits.iter().map(|hit| hit.name.as_str()).collect();

        assert_eq!(names, ["one.txt", "two.txt"]);
        assert_eq!(share.found_names("txt").len(), 3);
    }

    /// parity: SRCH-007
    #[test]
    fn hidden_items_are_found_only_when_shown() {
        let share = ScannedShare::new();
        let mut hidden = listed_file(SHARE, ".bank.pdf");
        hidden.is_hidden = true;
        share.store(&[hidden]);
        let shown = SearchQuery {
            hidden_items: HiddenItems::Include,
            ..SearchQuery::new("bank")
        };

        let with_hidden = share.index.search(&shown, None).unwrap();

        assert!(share.found_names("bank").is_empty());
        assert_eq!(with_hidden.hits[0].name, ".bank.pdf");
        assert!(with_hidden.hits[0].is_hidden);
    }

    /// parity: SRCH-010
    #[test]
    fn the_engine_is_the_trigram_index() {
        let share = ScannedShare::new();

        assert_eq!(share.index.engine(), SearchEngine::Trigram);
        assert_eq!(SearchEngine::Trigram.label(), "SQLite FTS5 trigram");
        assert_eq!(
            SearchEngine::SubstringFallback.label(),
            "SQLite substring fallback"
        );
    }

    /// Words of one or two characters have no trigrams and are matched as
    /// substrings; longer words go through the trigram index first.
    ///
    /// parity: SRCH-010
    #[test]
    fn short_and_long_words_both_match_inside_names() {
        let share = ScannedShare::new();
        share.store(&[listed_file(SHARE, "xy-report.pdf")]);

        assert_eq!(share.found_names("y"), ["xy-report.pdf"]);
        assert_eq!(share.found_names("y-r"), ["xy-report.pdf"]);
        assert_eq!(share.found_names("port.p"), ["xy-report.pdf"]);
    }

    /// parity: SRCH-017
    #[test]
    fn a_cancelled_search_does_not_run() {
        let share = ScannedShare::new();
        let cancellable = gio::Cancellable::new();
        cancellable.cancel();

        let result = share.index.search(&SearchQuery::new("bank"), Some(&cancellable));

        assert!(matches!(result, Err(SearchError::Cancelled)));
    }

    /// A query that is already running stops at its next check, every
    /// 4,000 SQLite steps, instead of reading every row first.
    ///
    /// parity: SRCH-017
    #[test]
    fn a_running_search_stops_when_cancelled() {
        let share = ScannedShare::new();
        let items: Vec<ListedItem> = (0..3000)
            .map(|number| listed_file(SHARE, &format!("{number}.pdf")))
            .collect();
        share.store(&items);
        let words = vec!["pd".to_owned()];
        let plan = QueryPlan::new(&words, None, HiddenItems::Skip, SearchEngine::Trigram, 2000);
        let connection = share.index.connect().unwrap();
        let cancellable = gio::Cancellable::new();

        cancellable.cancel();
        let result = run_cancellable(&connection, &plan, Some(&cancellable));

        assert!(matches!(result, Err(SearchError::Cancelled)));
        assert_eq!(found_names(&share.index, "2999.pdf"), ["2999.pdf"]);
    }

    /// parity: SRCH-010
    #[test]
    fn short_words_skip_the_trigram_prefilter() {
        let words = vec!["ab".to_owned(), "a\"b\"c".to_owned()];

        let plan = QueryPlan::new(&words, None, HiddenItems::Skip, SearchEngine::Trigram, 10);

        assert_eq!(plan.values[0], Value::Text("\"a\"\"b\"\"c\"".to_owned()));
        assert_eq!(plan.sql.matches("MATCH ?").count(), 1);
    }

    /// parity: SRCH-010
    #[test]
    fn the_substring_fallback_never_uses_the_full_text_table() {
        let words = vec!["report".to_owned()];

        let plan = QueryPlan::new(
            &words,
            None,
            HiddenItems::Include,
            SearchEngine::SubstringFallback,
            10,
        );

        assert!(!plan.sql.contains("names_fts"));
        assert!(!plan.sql.contains("e.hidden=0"));
    }

    /// parity: SRCH-009
    #[test]
    fn only_real_folders_open_as_folders() {
        assert!(opens_as_folder(EntryKind::Directory, false));
        assert!(!opens_as_folder(EntryKind::File, true));
        assert!(!opens_as_folder(EntryKind::Special, true));
        assert!(opens_as_folder(EntryKind::Mountable, true));
    }
}
