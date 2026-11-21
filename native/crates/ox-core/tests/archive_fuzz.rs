// SPDX-License-Identifier: AGPL-3.0-only
//! Seeded mutation fuzzing of the archive readers: ZIP, 7z and tar (plain,
//! gzip, bzip2 and XZ) archives made in the test are cut short, flipped,
//! grown and shrunk, then listed and previewed. Every one must be listed,
//! refused with an error, or stopped by its limits: none may panic, hang
//! or allocate without end. The seeds are fixed, so a failure repeats.
//!
//! `OX_FUZZ_ROUNDS` raises the rounds for a longer local run.

mod archive_support;

use std::process::Command;
use std::time::{Duration, Instant};

use ox_core::archive::ArchiveBrowser;
use ox_core::transfer::Cancellation;

use archive_support::{memory_opener, seven_zip_bytes, zip_bytes, Compression, SevenEntry, TestMember};

/// How long one mutated archive may take to list and preview.
const PER_ARCHIVE: Duration = Duration::from_secs(10);

fn rounds() -> usize {
    std::env::var("OX_FUZZ_ROUNDS")
        .ok()
        .and_then(|rounds| rounds.parse().ok())
        .unwrap_or(400)
}

/// A small deterministic generator (xorshift).
struct Seeds(u64);

impl Seeds {
    fn next(&mut self) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % (1 << 31)).unwrap_or(0)
    }
}

/// `bytes`, mutated a few times: a bit flipped, a byte set to a boundary
/// value, a byte inserted or removed, a run repeated, or the end cut.
fn mutate(bytes: &[u8], seeds: &mut Seeds) -> Vec<u8> {
    let mut mutated = bytes.to_vec();
    for _ in 0..=seeds.next() % 6 {
        if mutated.is_empty() {
            break;
        }
        let at = seeds.next() % mutated.len();
        match seeds.next() % 6 {
            0 => mutated[at] ^= 1 << (seeds.next() % 8),
            1 => mutated[at] = [0, 0xFF, 0x7F, 0x80, 1][seeds.next() % 5],
            2 => mutated.insert(at, u8::try_from(seeds.next() % 256).unwrap_or(0)),
            3 => {
                mutated.remove(at);
            }
            4 => {
                let end = (at + 1 + seeds.next() % 16).min(mutated.len());
                let run = mutated[at..end].to_vec();
                mutated.splice(at..at, run);
            }
            _ => mutated.truncate(at),
        }
    }
    mutated
}

/// Lists the top and the folder `Docs` of `archive`, and previews the
/// members it lists, within [`PER_ARCHIVE`].
fn exercise(archive: Vec<u8>, previews: &std::path::Path) {
    let browser = ArchiveBrowser::new(memory_opener(archive), previews.to_path_buf());
    let cancel = Cancellation::new();
    let started = Instant::now();
    let uri = "file:///home/alex/Downloads/fuzz.archive";
    for prefix in ["", "Docs"] {
        if let Ok(listing) = browser.list(uri, prefix, &cancel) {
            for entry in listing.entries.iter().take(4) {
                let _ = browser.preview_member(uri, &entry.member, &cancel);
            }
        }
    }
    assert!(
        started.elapsed() < PER_ARCHIVE,
        "an archive took {:?}",
        started.elapsed()
    );
}

fn fuzz(name: &str, seed: u64, sample: &[u8]) {
    let previews = tempfile::tempdir().expect("a temporary folder");
    let mut seeds = Seeds(seed);
    let browser = ArchiveBrowser::new(memory_opener(sample.to_vec()), previews.path().to_path_buf());
    let listing = browser
        .list(
            "file:///home/alex/Downloads/fuzz.archive",
            "",
            &Cancellation::new(),
        )
        .expect("the sample is listed");
    assert!(
        !listing.entries.is_empty(),
        "{name}: the sample lists its members"
    );
    exercise(sample.to_vec(), previews.path());
    for cut in (0..sample.len()).step_by((sample.len() / 64).max(1)) {
        exercise(sample[..cut].to_vec(), previews.path());
    }
    for _ in 0..rounds() {
        exercise(mutate(sample, &mut seeds), previews.path());
    }
    eprintln!("{name}: {} mutations survived", rounds());
}

fn sample_zip() -> Vec<u8> {
    zip_bytes(&[
        TestMember::folder("Docs/"),
        TestMember::file("Docs/Guide.txt", b"hello world, hello world").compressed_with(Compression::Stored),
        TestMember::file("Docs/Notes.txt", &b"notes ".repeat(200)),
        TestMember::file("readme.txt", b"hello"),
    ])
}

fn sample_tar(mode: &str) -> Vec<u8> {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let path = folder.path().join("sample.tar");
    let script = "import io, sys, tarfile\n\
                  with tarfile.open(sys.argv[1], sys.argv[2]) as archive:\n\
                  \x20   folder = tarfile.TarInfo('Docs'); folder.type = tarfile.DIRTYPE; archive.addfile(folder)\n\
                  \x20   data = b'hello world' * 50\n\
                  \x20   member = tarfile.TarInfo('Docs/Guide.txt'); member.size = len(data)\n\
                  \x20   archive.addfile(member, io.BytesIO(data))\n\
                  \x20   long = tarfile.TarInfo('Docs/' + 'n' * 150 + '.txt'); long.size = 3\n\
                  \x20   archive.addfile(long, io.BytesIO(b'abc'))\n";
    let status = Command::new("python3")
        .args(["-c", script])
        .arg(&path)
        .arg(mode)
        .status()
        .expect("Python 3 writes the reference tars");
    assert!(status.success());
    std::fs::read(&path).expect("the tar")
}

/// parity: ARC-022, ARC-024
#[test]
fn mutated_zips_are_listed_or_refused() {
    fuzz("zip", 0x5EED_0001, &sample_zip());
}

/// parity: ARC-022
#[test]
fn mutated_seven_zips_are_listed_or_refused() {
    let entries = [
        SevenEntry::folder("Docs"),
        SevenEntry::file("Docs/Guide.txt", &b"hello world ".repeat(40)),
        SevenEntry::file("readme.txt", b"hello"),
    ];
    fuzz("7z solid", 0x5EED_0002, &seven_zip_bytes(&entries, true));
    fuzz("7z", 0x5EED_0003, &seven_zip_bytes(&entries, false));
}

/// parity: ARC-024
#[test]
fn mutated_tars_are_listed_or_refused() {
    for (mode, seed) in [
        ("w", 0x5EED_0004),
        ("w:gz", 0x5EED_0005),
        ("w:bz2", 0x5EED_0006),
        ("w:xz", 0x5EED_0007),
    ] {
        fuzz(mode, seed, &sample_tar(mode));
    }
}
