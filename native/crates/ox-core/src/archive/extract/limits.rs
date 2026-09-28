// SPDX-License-Identifier: AGPL-3.0-only
//! The ZIP-bomb limits of an extraction (ARC-017). Ports `Limits` of
//! `desktop/zip_extraction.py` and the byte checks of
//! `ZipExtractor.extract` that apply them while data is written.

use crate::archive::ArchiveError;

/// ARC-017: how much an extraction accepts before it refuses the archive.
/// [`Default`] gives the limits of the Python app; tests lower them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionLimits {
    /// The most members an archive may have.
    pub max_entries: usize,
    /// The most paths, counting the folders that member paths imply.
    pub max_paths: usize,
    /// The deepest folder nesting of a member path.
    pub max_depth: usize,
    /// The most uncompressed bytes of all members together.
    pub max_total_bytes: u64,
    /// The most uncompressed bytes of one member.
    pub max_member_bytes: u64,
    /// The highest ratio of a member's uncompressed to compressed size.
    pub max_ratio: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_paths: 200_000,
            max_depth: 128,
            max_total_bytes: 20 * 1024 * 1024 * 1024,
            max_member_bytes: 8 * 1024 * 1024 * 1024,
            max_ratio: 1000,
        }
    }
}

/// The bytes an extraction has written so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct WrittenBytes {
    /// Of the member being written.
    pub(super) member: u64,
    /// Of all members.
    pub(super) total: u64,
}

impl WrittenBytes {
    /// Counts `count` more bytes of the member being written.
    pub(super) fn add(&mut self, count: usize) {
        let count = count as u64;
        self.member += count;
        self.total += count;
    }

    /// Starts counting the next member.
    pub(super) fn start_member(&mut self) {
        self.member = 0;
    }
}

impl ExtractionLimits {
    /// ARC-017: stops an extraction whose member yielded more than the
    /// `declared_size` its archive records, or more than the limits allow.
    /// The ZIP reader never yields more than a member declares, so this is
    /// a second line of defence, as in the Python extractor.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::SizeLimitExceeded`] when `written` is over a limit.
    pub(super) fn check_written(
        &self,
        written: WrittenBytes,
        declared_size: u64,
    ) -> Result<(), ArchiveError> {
        let is_member_over = written.member > declared_size || written.member > self.max_member_bytes;
        if is_member_over || written.total > self.max_total_bytes {
            return Err(ArchiveError::SizeLimitExceeded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes written, and whether the check lets them through.
    struct WrittenCase {
        member: u64,
        total: u64,
        is_accepted: bool,
    }

    /// parity: ARC-017
    #[test]
    fn written_bytes_stop_at_the_declared_size_and_the_limits() {
        let limits = ExtractionLimits {
            max_member_bytes: 8,
            max_total_bytes: 20,
            ..ExtractionLimits::default()
        };
        let declared_size = 6;
        let cases = [
            WrittenCase {
                member: 6,
                total: 20,
                is_accepted: true,
            },
            WrittenCase {
                member: 7,
                total: 7,
                is_accepted: false,
            },
            WrittenCase {
                member: 5,
                total: 21,
                is_accepted: false,
            },
        ];
        for case in cases {
            let written = WrittenBytes {
                member: case.member,
                total: case.total,
            };

            let result = limits.check_written(written, declared_size);

            assert_eq!(result.is_ok(), case.is_accepted, "{written:?}");
        }
        let over_member_limit = WrittenBytes { member: 9, total: 9 };
        assert_eq!(
            limits.check_written(over_member_limit, 100),
            Err(ArchiveError::SizeLimitExceeded)
        );
    }

    #[test]
    fn counting_a_new_member_keeps_the_total() {
        let mut written = WrittenBytes::default();
        written.add(4);
        written.start_member();
        written.add(3);

        assert_eq!(written, WrittenBytes { member: 3, total: 7 });
    }
}
