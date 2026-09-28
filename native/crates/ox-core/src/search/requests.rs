// SPDX-License-Identifier: AGPL-3.0-only
//! Requests other windows and processes leave for the index owner.
//!
//! Ports `enqueue` and `drain_requests` in `desktop/search_index.py` and
//! the request kinds `refresh_due` in `desktop/index_service.py` handles
//! (SRCH-027). The `index_requests` table is shared with the Python app,
//! so the kind words are Python's, and a change is stored as the JSON
//! array `[root, folder]` that both apps read.

use super::error::SearchError;
use super::index::{begin_immediate, SearchIndex};
use super::text::unix_now;

/// Work only the index owner may do, asked for by another process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IndexRequest {
    /// Rescan a root.
    Refresh {
        /// The root's URI.
        root: String,
    },
    /// Stop a root's running scan and updates.
    Cancel {
        /// The root's URI.
        root: String,
    },
    /// Re-read a folder that changed below a root.
    Changed {
        /// The root's URI.
        root: String,
        /// The folder that changed.
        folder: String,
    },
    /// Stop indexing an SMB server while the user signs out.
    PauseServer {
        /// The server's host name.
        host: String,
    },
    /// Index an SMB server again after the user signed in.
    ResumeServer {
        /// The server's host name.
        host: String,
    },
}

impl IndexRequest {
    /// The stored kind word.
    fn kind(&self) -> &'static str {
        match self {
            Self::Refresh { .. } => "refresh",
            Self::Cancel { .. } => "cancel",
            Self::Changed { .. } => "changed",
            Self::PauseServer { .. } => "pause-server",
            Self::ResumeServer { .. } => "resume-server",
        }
    }

    /// The stored subject: a URI, a host, or for a change the JSON array
    /// `[root, folder]`.
    fn subject(&self) -> String {
        match self {
            Self::Refresh { root } | Self::Cancel { root } => root.clone(),
            Self::PauseServer { host } | Self::ResumeServer { host } => host.clone(),
            Self::Changed { root, folder } => serde_json::json!([root, folder]).to_string(),
        }
    }

    /// The request a stored row describes; `None` for a kind or subject
    /// this version does not know, which is dropped as Python drops it.
    fn from_stored(kind: &str, subject: String) -> Option<Self> {
        let request = match kind {
            "refresh" => Self::Refresh { root: subject },
            "cancel" => Self::Cancel { root: subject },
            "pause-server" => Self::PauseServer { host: subject },
            "resume-server" => Self::ResumeServer { host: subject },
            "changed" => {
                let (root, folder) = serde_json::from_str(&subject).ok()?;
                Self::Changed { root, folder }
            }
            _ => return None,
        };
        Some(request)
    }
}

impl SearchIndex {
    /// Leaves `request` for the index owner. A request equal to one still
    /// waiting replaces it, so repeated requests run once.
    pub(crate) fn enqueue(&self, request: &IndexRequest) -> Result<(), SearchError> {
        let kind = request.kind();
        let subject = request.subject();
        let key = format!("{kind}:{subject}");
        let connection = self.connect()?;
        connection.execute(
            "INSERT OR REPLACE INTO index_requests(key, kind, uri, created) VALUES(?1, ?2, ?3, ?4)",
            (key, kind, subject, unix_now()),
        )?;
        Ok(())
    }

    /// Takes every waiting request, oldest first; requests left in the same
    /// instant keep the order they were left in.
    ///
    /// Reading and deleting run in one immediate transaction, so a request
    /// another process leaves in between is kept for the next call instead
    /// of being deleted unread.
    pub(crate) fn drain_requests(&self) -> Result<Vec<IndexRequest>, SearchError> {
        let mut connection = self.connect()?;
        let transaction = begin_immediate(&mut connection)?;
        let rows: Vec<(String, String)> = {
            let mut statement =
                transaction.prepare("SELECT kind, uri FROM index_requests ORDER BY created, rowid")?;
            let rows = statement.query_map((), |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        transaction.execute("DELETE FROM index_requests", ())?;
        transaction.commit()?;
        let requests = rows
            .into_iter()
            .filter_map(|(kind, subject)| IndexRequest::from_stored(&kind, subject))
            .collect();
        Ok(requests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `desktop/tests/test_v05.py::IndexTests::test_command_queue_deduplicates`
    /// parity: SRCH-027
    #[test]
    fn repeated_requests_are_queued_once() {
        let directory = tempfile::tempdir().unwrap();
        let index = SearchIndex::open(directory.path()).unwrap();
        let refresh = IndexRequest::Refresh {
            root: "smb://nas/share".to_owned(),
        };

        index.enqueue(&refresh).unwrap();
        index.enqueue(&refresh).unwrap();

        assert_eq!(index.drain_requests().unwrap(), [refresh]);
        assert_eq!(index.drain_requests().unwrap(), []);
    }

    #[test]
    fn every_request_kind_survives_the_table() {
        let directory = tempfile::tempdir().unwrap();
        let index = SearchIndex::open(directory.path()).unwrap();
        let requests = [
            IndexRequest::Cancel {
                root: "file:///data".to_owned(),
            },
            IndexRequest::Changed {
                root: "file:///data".to_owned(),
                folder: "file:///data/a%20b".to_owned(),
            },
            IndexRequest::PauseServer {
                host: "nas".to_owned(),
            },
            IndexRequest::ResumeServer {
                host: "nas".to_owned(),
            },
        ];
        for request in &requests {
            index.enqueue(request).unwrap();
        }

        assert_eq!(index.drain_requests().unwrap(), requests);
    }

    /// The Python app writes `json.dumps([root, folder])`, with a space.
    #[test]
    fn a_change_written_by_the_python_app_is_read() {
        let stored =
            IndexRequest::from_stored("changed", r#"["file:///data", "file:///data/new"]"#.to_owned());

        let expected = IndexRequest::Changed {
            root: "file:///data".to_owned(),
            folder: "file:///data/new".to_owned(),
        };
        assert_eq!(stored, Some(expected));
        assert_eq!(
            IndexRequest::from_stored("reindex-everything", String::new()),
            None
        );
    }
}
