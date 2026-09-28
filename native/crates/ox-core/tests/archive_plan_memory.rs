// SPDX-License-Identifier: AGPL-3.0-only
//! ARC-017: checking an archive before extraction takes memory in
//! proportion to its member names, not to their depth times their length.
//!
//! A crafted 12.8 MB ZIP of 1,560 members, each 128 levels deep with names
//! of about 4,000 characters, passes every extraction limit. When the check
//! stored every folder's whole path, the Extract dialog's automatic check
//! of that file peaked at 1.5 GB, where `plan` in
//! `desktop/zip_extraction.py` stays under 300 MB. This test checks a
//! smaller archive of the same shape and measures the heap with a counting
//! allocator.
//!
//! The allocator counts the allocations of every thread in this test
//! executable, so the file holds this one test only.

mod archive_support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ox_core::archive::ZipExtractor;
use ox_core::transfer::Cancellation;

use archive_support::{gio_factory, memory_opener, zip_bytes, LocalFileOutput, TestMember};

/// The members of the test archive.
const MEMBER_COUNT: usize = 64;
/// The levels of every member path: the deepest the extractor accepts.
const MEMBER_DEPTH: usize = 128;
/// The length of every path segment, which makes each name 4,095
/// characters long, just within the 4,096-character limit.
const SEGMENT_LENGTH: usize = 31;
/// The most heap the check may use, in multiples of the archive's size.
/// Storing each segment once takes about 7 times the size of this archive;
/// storing each folder's whole path took about 69 times.
const HEAP_BUDGET_PER_ARCHIVE_BYTE: usize = 16;

/// The system allocator, counting the bytes in use and the most that were
/// in use at once.
struct CountingAllocator {
    in_use: AtomicUsize,
    peak: AtomicUsize,
}

impl CountingAllocator {
    /// Runs `measured` and returns its result with the most bytes that were
    /// in use at once while it ran, beyond those in use when it started.
    fn peak_growth_during<T>(&self, measured: impl FnOnce() -> T) -> (T, usize) {
        let in_use_before = self.in_use.load(Ordering::SeqCst);
        self.peak.store(in_use_before, Ordering::SeqCst);
        let result = measured();
        let growth = self.peak.load(Ordering::SeqCst) - in_use_before;
        (result, growth)
    }

    /// Counts `size` newly allocated bytes.
    fn count_allocation(&self, size: usize) {
        let in_use = self.in_use.fetch_add(size, Ordering::SeqCst) + size;
        self.peak.fetch_max(in_use, Ordering::SeqCst);
    }
}

// SAFETY: every call goes to `System` with its arguments unchanged; the
// counters only record the sizes.
#[expect(
    unsafe_code,
    reason = "a global allocator is an unsafe trait implementation; this one only forwards to System"
)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which
        // is `System.alloc`'s.
        let allocation = unsafe { System.alloc(layout) };
        if !allocation.is_null() {
            self.count_allocation(layout.size());
        }
        allocation
    }

    unsafe fn dealloc(&self, allocation: *mut u8, layout: Layout) {
        // SAFETY: `allocation` came from `alloc` above, so from `System`,
        // with this `layout`.
        unsafe { System.dealloc(allocation, layout) };
        self.in_use.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static HEAP: CountingAllocator = CountingAllocator {
    in_use: AtomicUsize::new(0),
    peak: AtomicUsize::new(0),
};

/// Member `member` of the test archive: an empty file `MEMBER_DEPTH`
/// levels deep, whose folders no other member shares.
fn deep_member(member: usize) -> TestMember {
    let padding = "x".repeat(SEGMENT_LENGTH - "0000-000-".len());
    let segments: Vec<String> = (0..MEMBER_DEPTH)
        .map(|level| format!("{member:04}-{level:03}-{padding}"))
        .collect();
    TestMember::file(&segments.join("/"), b"")
}

/// parity: ARC-017
#[test]
fn checking_deep_long_names_takes_memory_in_proportion_to_the_names() {
    let members: Vec<TestMember> = (0..MEMBER_COUNT).map(deep_member).collect();
    let archive = zip_bytes(&members);
    let archive_size = archive.len();
    let extractor = ZipExtractor::new(
        memory_opener(archive),
        gio_factory(),
        Arc::new(LocalFileOutput::owner_only()),
    );

    let (summary, peak_growth) =
        HEAP.peak_growth_during(|| extractor.inspect("file:///deep.zip", &Cancellation::new()));

    let summary = summary.expect("the archive is within every limit");
    assert_eq!(summary.file_count, MEMBER_COUNT);
    assert_eq!(summary.folder_count, MEMBER_COUNT * (MEMBER_DEPTH - 1));
    assert!(
        peak_growth < HEAP_BUDGET_PER_ARCHIVE_BYTE * archive_size,
        "checking a {archive_size}-byte archive used {peak_growth} bytes of heap"
    );
}
