// SPDX-License-Identifier: AGPL-3.0-only
//! Compiled gettext catalogues (`.mo`), as GNU gettext's "The Format of
//! GNU MO Files" describes them: a header of 32-bit numbers in the
//! file's byte order, then a table of original strings and a table of
//! translations, each entry a length and an offset. An original is the
//! context and `\x04` before the id, and `\0` and the plural id after
//! it; a translation is its plural forms separated by `\0`.

use std::collections::HashMap;

/// The magic number, as read in the file's own byte order.
const MAGIC: u32 = 0x9504_12de;

/// Reads the messages of a catalogue: each original id (context and id,
/// without the plural id) and its translations. `None` when `bytes` is
/// not a catalogue or a string is not UTF-8.
pub(super) fn read(bytes: &[u8]) -> Option<HashMap<String, Vec<String>>> {
    let word = |offset: usize, big_endian: bool| -> Option<u32> {
        let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        })
    };
    let big_endian = match word(0, false)? {
        MAGIC => false,
        magic if magic.swap_bytes() == MAGIC => true,
        _ => return None,
    };
    let number = |offset: usize| word(offset, big_endian).and_then(|value| usize::try_from(value).ok());
    let count = number(8)?;
    let (originals, translations) = (number(12)?, number(16)?);
    let string = |table: usize, index: usize| -> Option<&str> {
        let entry = table.checked_add(index.checked_mul(8)?)?;
        let (length, offset) = (number(entry)?, number(entry + 4)?);
        std::str::from_utf8(bytes.get(offset..offset.checked_add(length)?)?).ok()
    };
    let mut messages = HashMap::with_capacity(count);
    for index in 0..count {
        let original = string(originals, index)?;
        let id = original.split('\0').next().unwrap_or_default();
        let forms = string(translations, index)?
            .split('\0')
            .map(str::to_owned)
            .collect();
        messages.insert(id.to_owned(), forms);
    }
    Some(messages)
}

/// A little-endian catalogue of `entries` (original, translation), which
/// must be sorted by original, without a hash table: what msgfmt writes,
/// for tests.
#[cfg(test)]
pub(super) fn write(entries: &[(String, String)]) -> Vec<u8> {
    const HEADER: usize = 28;
    let count = entries.len();
    let tables = HEADER + 16 * count;
    let mut strings = Vec::new();
    let table = |text: &str, strings: &mut Vec<u8>| {
        let offset = tables + strings.len();
        strings.extend_from_slice(text.as_bytes());
        strings.push(0);
        (text.len(), offset)
    };
    let originals: Vec<(usize, usize)> = entries.iter().map(|(id, _)| table(id, &mut strings)).collect();
    let translated: Vec<(usize, usize)> = entries
        .iter()
        .map(|(_, text)| table(text, &mut strings))
        .collect();
    let word = |value: usize| {
        u32::try_from(value)
            .expect("a small test catalogue")
            .to_le_bytes()
    };
    let mut bytes = Vec::new();
    for value in [MAGIC, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [count, HEADER, HEADER + 8 * count, 0, tables] {
        bytes.extend_from_slice(&word(value));
    }
    for (length, offset) in originals.into_iter().chain(translated) {
        bytes.extend_from_slice(&word(length));
        bytes.extend_from_slice(&word(offset));
    }
    bytes.extend_from_slice(&strings);
    bytes
}
