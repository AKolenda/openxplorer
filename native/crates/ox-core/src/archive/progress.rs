// SPDX-License-Identifier: AGPL-3.0-only
//! Progress of compressing and extracting, in the terms a copy reports it
//! (OPS-021): the bytes processed against the whole job's size, the files
//! done out of all of them and the file being handled. The progress window
//! then shows the bar, its "N% complete" title, the speed graph, the time
//! left, the items remaining and the name as it does for a copy, moving
//! block by block through a single large file.

use crate::transfer::{progress_fraction, ByteProgress, ItemCount, Progress, ProgressScope};

/// How far a job is: the bytes and files done and in all.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct JobProgress {
    /// Bytes processed, including earlier files.
    pub(super) bytes_done: u64,
    /// Bytes the whole job processes.
    pub(super) bytes_total: u64,
    /// Files finished.
    pub(super) files_done: u64,
    /// Files the whole job handles.
    pub(super) files_total: u64,
}

impl JobProgress {
    /// The files done of all of them, when the job handles any.
    fn items(&self) -> Option<ItemCount> {
        (self.files_total > 0).then_some(ItemCount {
            done: self.files_done.min(self.files_total),
            total: self.files_total,
        })
    }

    /// A report on the file `path` (an archive path, `Photos/a.jpg`),
    /// `written` bytes into its `size`, labelled `label`. `bytes_done`
    /// already counts `written`.
    pub(super) fn file_report(&self, label: String, path: &str, (written, size): (u64, u64)) -> Progress {
        Progress {
            label,
            fraction: progress_fraction(written, size),
            scope: ProgressScope::File,
            bytes: Some(ByteProgress {
                file_written: written,
                file_size: size,
                batch_written: self.bytes_done,
                batch_size: Some(self.bytes_total),
                file_started: written == 0,
            }),
            items: self.items(),
            name: Some(last_segment(path).to_owned()),
        }
    }

    /// The report that ends the job, labelled `label`: every item done.
    pub(super) fn finished_report(&self, label: String) -> Progress {
        Progress {
            label,
            fraction: 1.0,
            scope: ProgressScope::Batch,
            bytes: None,
            items: self.items().map(|items| ItemCount {
                done: items.total,
                ..items
            }),
            name: None,
        }
    }
}

/// The last segment of an archive path: `a.jpg` of `Photos/a.jpg`.
fn last_segment(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_names_the_file_and_counts_the_job() {
        let job = JobProgress {
            bytes_done: 150,
            bytes_total: 400,
            files_done: 1,
            files_total: 3,
        };

        let report = job.file_report(
            "Compressing Photos/a.jpg (2/3)".to_owned(),
            "Photos/a.jpg",
            (50, 200),
        );

        assert_eq!(report.name.as_deref(), Some("a.jpg"));
        assert_eq!(report.scope, ProgressScope::File);
        assert!((report.fraction - 0.25).abs() < f64::EPSILON);
        let bytes = report.bytes.unwrap();
        assert_eq!((bytes.batch_written, bytes.batch_size), (150, Some(400)));
        assert_eq!(report.items, Some(ItemCount { done: 1, total: 3 }));
        let finished = job.finished_report("Compressed Photos.zip".to_owned());
        assert_eq!(finished.items, Some(ItemCount { done: 3, total: 3 }));
        assert!((finished.fraction - 1.0).abs() < f64::EPSILON);
        assert_eq!(JobProgress::default().finished_report(String::new()).items, None);
    }
}
