// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of reading icons from Windows icon files, on synthetic files
//! built in the tests ([`super::synthetic`]): no Windows file is used.

use std::path::{Path, PathBuf};

use super::synthetic::{palette_bitmap, rgba_bitmap, test_pattern, SyntheticIconFile, SyntheticImage};
use super::*;
use crate::test_support::temporary_folder;

/// The roles `library` draws.
fn roles_of(library: IconLibrary) -> Vec<IconRole> {
    IconRole::ALL
        .into_iter()
        .filter(|role| role.source().0 == library)
        .collect()
}

/// The group IDs of `library`'s roles, each once: several roles may share
/// a group.
fn groups_of(library: IconLibrary) -> Vec<u16> {
    let mut groups: Vec<u16> = roles_of(library)
        .into_iter()
        .map(|role| role.source().1)
        .collect();
    groups.sort_unstable();
    groups.dedup();
    groups
}

/// Writes a synthetic file called `name` with every group of `library`
/// into `folder`.
fn write_library(folder: &Path, name: &str, library: IconLibrary) -> PathBuf {
    let path = folder.join(name);
    let bytes = SyntheticIconFile::with_groups(&groups_of(library)).build();
    std::fs::write(&path, bytes).expect("the temporary folder takes the file");
    path
}

/// A tiny pseudo-random sequence, the same in every run.
struct Shuffle(u64);

impl Shuffle {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Reads every group of `bytes` and decodes every image, as loading does;
/// what matters is that it returns.
fn read_everything(bytes: Vec<u8>) {
    let Ok(file) = IconFile::from_bytes(bytes) else {
        return;
    };
    let ids: Vec<u16> = file.group_ids().collect();
    for id in ids {
        for image in file.group(id).unwrap_or_default() {
            if let Ok(decoded) = decode(&image.data) {
                let _ = decoded.to_png();
            }
        }
    }
}

/// parity: LOOK-030
#[test]
fn a_synthetic_file_gives_its_groups_and_images() {
    for wide in [false, true] {
        let synthetic = SyntheticIconFile {
            wide,
            ..SyntheticIconFile::with_groups(&[3, 55])
        };
        let file = IconFile::from_bytes(synthetic.build()).expect("a valid file");
        assert_eq!(file.group_ids().collect::<Vec<_>>(), [3, 55], "PE32+ {wide}");
        let images = file.group(3).expect("a valid group");
        let edges: Vec<u32> = images.iter().map(|image| image.width).collect();
        assert_eq!(edges, [16, 32, 256]);
        assert!(file.group(4).expect("a missing group is no error").is_empty());
    }
}

/// parity: LOOK-030
#[test]
fn damaged_or_unsupported_images_are_refused() {
    let pattern = test_pattern(16, [1, 2, 3]);
    let bitmap = rgba_bitmap(&pattern);
    let with = |at: usize, bytes: &[u8]| {
        let mut changed = bitmap.clone();
        changed[at..at + bytes.len()].copy_from_slice(bytes);
        changed
    };
    let cases = [
        ("cut short", bitmap[..bitmap.len() / 2].to_vec()),
        ("too wide", with(4, &512u32.to_le_bytes())),
        ("no width", with(4, &0u32.to_le_bytes())),
        ("top down", with(8, &(-32i32).to_le_bytes())),
        ("odd height", with(8, &31u32.to_le_bytes())),
        ("16-bit", with(14, &16u16.to_le_bytes())),
        ("compressed", with(16, &1u32.to_le_bytes())),
        ("odd header", with(0, &12u32.to_le_bytes())),
        ("empty", Vec::new()),
    ];
    for (case, data) in cases {
        assert!(decode(&data).is_err(), "{case}");
    }
    let mut palette = palette_bitmap(&pattern);
    palette[32..36].copy_from_slice(&300u32.to_le_bytes());
    assert!(decode(&palette).is_err(), "more colors than 8 bits hold");

    let png = encode_png(&pattern);
    let mut bad_checksum = png.clone();
    bad_checksum[29] ^= 0xff;
    let mut too_large = png.clone();
    too_large[16..20].copy_from_slice(&1024u32.to_be_bytes());
    for (case, data) in [
        ("checksum", bad_checksum),
        ("1024 wide", too_large),
        ("cut", png[..20].to_vec()),
    ] {
        assert!(decode(&data).is_err(), "PNG {case}");
    }
    assert_eq!(decode(&vec![0; 2 << 20]), Err(IconFileError::OverLimit));
}

/// Files that are not PE files, or whose headers point outside the file,
/// give an error and no icons.
///
/// parity: LOOK-030
#[test]
fn files_that_are_not_icon_files_are_refused() {
    let good = SyntheticIconFile::with_groups(&[3]).build();
    let with = |at: usize, bytes: &[u8]| {
        let mut changed = good.clone();
        changed[at..at + bytes.len()].copy_from_slice(bytes);
        changed
    };
    let cases = [
        ("empty", Vec::new(), IconFileError::NotPe),
        ("text", b"Fictional notes".to_vec(), IconFileError::NotPe),
        ("no MZ", with(0, b"ZM"), IconFileError::NotPe),
        (
            "PE header past the end",
            with(0x3c, &u32::MAX.to_le_bytes()),
            IconFileError::NotPe,
        ),
        ("no PE signature", with(64, b"NE\0\0"), IconFileError::NotPe),
        (
            "unknown optional header",
            with(88, &0x107u16.to_le_bytes()),
            IconFileError::NotPe,
        ),
        (
            "too many sections",
            with(70, &1000u16.to_le_bytes()),
            IconFileError::OverLimit,
        ),
        (
            "resources outside every section",
            with(88 + 96 + 16, &0x9000u32.to_le_bytes()),
            IconFileError::Damaged,
        ),
        (
            "no resource directory",
            with(88 + 96 + 16, &0u32.to_le_bytes()),
            IconFileError::NoIcons,
        ),
        (
            "section past the end",
            with(312 + 20, &0x7fff_0000u32.to_le_bytes()),
            IconFileError::Damaged,
        ),
    ];
    for (case, bytes, expected) in cases {
        let error = IconFile::from_bytes(bytes).err();
        assert_eq!(error, Some(expected), "{case}");
    }
}

/// A file cut at any length, or with any byte changed, never panics or
/// reads out of bounds; at worst it gives no icons.
///
/// parity: LOOK-030
#[test]
fn truncated_and_mutated_files_never_panic() {
    let good = SyntheticIconFile::with_groups(&[3, 2, 102]).build();
    for length in (0..good.len()).step_by(7) {
        read_everything(good[..length].to_vec());
    }
    let mut shuffle = Shuffle(0x9e37_79b9_7f4a_7c15);
    for _ in 0..3000 {
        let mut bytes = good.clone();
        for _ in 0..=shuffle.next() % 4 {
            let at = usize::try_from(shuffle.next() % bytes.len() as u64).expect("within the file");
            bytes[at] = shuffle.next().to_le_bytes()[0];
        }
        read_everything(bytes);
    }
    // Every byte of the headers and directories set to its extremes.
    for at in 0..0x400.min(good.len()) {
        for value in [0x00, 0x7f, 0x80, 0xff] {
            let mut bytes = good.clone();
            bytes[at] = value;
            read_everything(bytes);
        }
    }
}

/// A group that lists more images than the cap, or whose images point
/// past the section, gives what it can and nothing else.
///
/// parity: LOOK-030
#[test]
fn groups_are_capped_and_missing_images_left_out() {
    let image = |id: u16| SyntheticImage {
        id,
        edge: 16,
        bit_count: 32,
        data: rgba_bitmap(&test_pattern(16, [9, 9, 9])),
    };
    let crowded = SyntheticIconFile {
        groups: vec![(3, (1..=65).map(image).collect())],
        ..SyntheticIconFile::default()
    };
    let file = IconFile::from_bytes(crowded.build()).expect("a valid file");
    assert_eq!(file.group(3).err(), Some(IconFileError::OverLimit));

    let stub = SyntheticIconFile {
        without_images: true,
        ..SyntheticIconFile::with_groups(&[3])
    };
    let file = IconFile::from_bytes(stub.build()).expect("a valid file");
    assert!(
        file.group(3).expect("a valid group").is_empty(),
        "a group without its images"
    );
}

/// A changed file is read afresh, a broken cache folder is made again,
/// and folders of files no longer chosen are removed.
///
/// parity: LOOK-030
#[test]
fn the_cache_follows_the_chosen_files() {
    let folder = temporary_folder();
    let cache = folder.path().join("cache");
    let imageres = write_library(folder.path(), "imageres.dll.mun", IconLibrary::Imageres);
    let folders = || -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(&cache)
            .expect("the cache exists")
            .map(|entry| entry.expect("readable").path())
            .collect();
        found.sort();
        found
    };
    let imageres_roles = roles_of(IconLibrary::Imageres).len();
    assert_eq!(
        load_icons(std::slice::from_ref(&imageres), &cache).found(),
        imageres_roles
    );
    let first = folders();
    assert_eq!(first.len(), 1);

    // Only a folder group now.
    std::fs::write(&imageres, SyntheticIconFile::with_groups(&[3]).build()).expect("rewritten");
    assert_eq!(load_icons(std::slice::from_ref(&imageres), &cache).found(), 1);
    let second = folders();
    assert_eq!(second.len(), 1, "the old folder is removed");
    assert_ne!(first, second);

    std::fs::remove_file(second[0].join("index")).expect("the index exists");
    assert_eq!(
        load_icons(std::slice::from_ref(&imageres), &cache).found(),
        1,
        "made again"
    );

    let unrelated = cache.join("notes");
    std::fs::create_dir(&unrelated).expect("a folder");
    assert_eq!(load_icons(&[], &cache).found(), 0);
    assert_eq!(
        folders(),
        [unrelated],
        "only the reader's own folders are removed"
    );
}
