// SPDX-License-Identifier: AGPL-3.0-only
//! SRCH-004: finding a wildcard word within a name costs time in
//! proportion to the name's length times the pattern's, with a fixed
//! number of allocations, however few names match.
//!
//! The search cache leaves wildcard words out of its trigram prefilter, so
//! a wildcard search such as `zz?zz` that finds little or nothing tests
//! every cached name with the matcher that the folder's filter uses too
//! (`ox_name_matches` in `search/query.rs`). When the matcher tried the
//! pattern from every character of the name and copied the rest of the
//! name each time, a 255-character name took hundreds of allocations and
//! tens of thousands of copied characters, so such searches stalled as the
//! cache grew. This test matches 100,000 names of 255 characters, the
//! longest a Linux filesystem allows, and counts the allocations.
//!
//! The allocator counts the allocations of every thread in this test
//! executable, so the file holds this one test only.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ox_core::search::NamePattern;

/// The names matched, as a large search cache holds them.
const NAME_COUNT: usize = 100_000;
/// The longest name a Linux filesystem allows, in bytes.
const NAME_LENGTH: usize = 255;
/// The most allocations one name may take for one wildcard word: a
/// fixed number, never one per character of the name.
const ALLOCATIONS_PER_NAME: usize = 16;
/// The longest all names may take, generous for an unoptimized build on a
/// busy machine; trying the pattern from every character took minutes.
const TIME_BUDGET: Duration = Duration::from_secs(60);

/// The system allocator, counting the allocations made.
struct CountingAllocator {
    allocations: AtomicUsize,
}

// SAFETY: every call goes to `System` with its arguments unchanged; the
// counter only records how many allocations were made.
#[expect(
    unsafe_code,
    reason = "a global allocator is an unsafe trait implementation; this one only forwards to System"
)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocations.fetch_add(1, Ordering::SeqCst);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which
        // is `System.alloc`'s.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, allocation: *mut u8, layout: Layout) {
        // SAFETY: `allocation` came from `alloc` above, so from `System`,
        // with this `layout`.
        unsafe { System.dealloc(allocation, layout) };
    }
}

#[global_allocator]
static HEAP: CountingAllocator = CountingAllocator {
    allocations: AtomicUsize::new(0),
};

/// Name `number` of the cache: 255 characters ending in `.txt`, with no
/// `zz` in it.
fn cached_name(number: usize) -> String {
    let stem = format!("report {number:06} ");
    let padding = "abcdefghij".repeat(NAME_LENGTH);
    format!(
        "{stem}{}.txt",
        &padding[..NAME_LENGTH - stem.len() - ".txt".len()]
    )
}

/// Matches `pattern` against `names`, returning how many matched and how
/// many allocations the matching took.
fn count_matches(pattern: &NamePattern, names: &[String]) -> (usize, usize) {
    let before = HEAP.allocations.load(Ordering::SeqCst);
    let matched = names
        .iter()
        .filter(|name| pattern.matches_lowercase(name, ""))
        .count();
    (matched, HEAP.allocations.load(Ordering::SeqCst) - before)
}

/// parity: SRCH-004
#[test]
fn a_wildcard_word_that_finds_nothing_is_cheap_for_every_cached_name() {
    let names: Vec<String> = (0..NAME_COUNT).map(cached_name).collect();
    assert!(names.iter().all(|name| name.len() == NAME_LENGTH));
    let pattern = NamePattern::new("zz?zz");

    // A thousand names first, so a matcher that allocates per character
    // fails here rather than after minutes.
    let (matched, allocations) = count_matches(&pattern, &names[..1_000]);
    assert_eq!(matched, 0);
    assert!(
        allocations <= 1_000 * ALLOCATIONS_PER_NAME,
        "{allocations} allocations for 1,000 names"
    );

    let started = Instant::now();
    let (matched, allocations) = count_matches(&pattern, &names);
    let elapsed = started.elapsed();
    assert_eq!(matched, 0);
    assert!(
        allocations <= NAME_COUNT * ALLOCATIONS_PER_NAME,
        "{allocations} allocations for {NAME_COUNT} names"
    );
    assert!(elapsed < TIME_BUDGET, "{NAME_COUNT} names took {elapsed:?}");

    // The same names still match a pattern found within them.
    let found = NamePattern::new("report 0000?? *.txt");
    assert_eq!(count_matches(&found, &names[..1_000]).0, 100);
}
