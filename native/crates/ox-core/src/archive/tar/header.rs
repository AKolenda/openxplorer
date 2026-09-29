// SPDX-License-Identifier: AGPL-3.0-only
//! The 512-byte headers of a TAR archive: POSIX ustar, with the GNU long
//! names (`L`, `K`) and POSIX pax records (`x`) that `tar` writes for long
//! paths and large files.

/// The size of a header and of the blocks data is padded to.
pub(super) const BLOCK: usize = 512;

/// What a header describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeaderKind {
    /// A regular file.
    File,
    /// A folder.
    Folder,
    /// A link, device, FIFO or anything else: never extracted.
    Special,
    /// The next header's long name (GNU `L`).
    LongName,
    /// The next header's long link target (GNU `K`), which is ignored.
    LongLink,
    /// The next header's pax records (`x`).
    Pax,
    /// Pax records for the whole archive (`g`), which are ignored.
    GlobalPax,
}

/// One parsed header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Header {
    pub(super) kind: HeaderKind,
    /// The path the header names, prefix included.
    pub(super) name: String,
    /// The permission bits.
    pub(super) mode: u32,
    /// The size of the data after the header.
    pub(super) size: u64,
    /// Seconds since the Unix epoch.
    pub(super) modified: u64,
}

/// Why a header could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BadHeader;

/// True for the all-zero block that ends an archive.
pub(super) fn is_end(block: &[u8; BLOCK]) -> bool {
    block.iter().all(|&byte| byte == 0)
}

/// Parses `block`, checking its checksum.
pub(super) fn parse(block: &[u8; BLOCK]) -> Result<Header, BadHeader> {
    let recorded = octal(&block[148..156]).ok_or(BadHeader)?;
    let computed: u64 = block
        .iter()
        .enumerate()
        .map(|(index, &byte)| {
            if (148..156).contains(&index) {
                32
            } else {
                u64::from(byte)
            }
        })
        .sum();
    if recorded != computed {
        return Err(BadHeader);
    }
    let kind = match block[156] {
        b'0' | 0 | b'7' => HeaderKind::File,
        b'5' => HeaderKind::Folder,
        b'L' => HeaderKind::LongName,
        b'K' => HeaderKind::LongLink,
        b'x' => HeaderKind::Pax,
        b'g' => HeaderKind::GlobalPax,
        _ => HeaderKind::Special,
    };
    let mut name = text(&block[0..100]);
    let is_ustar = &block[257..262] == b"ustar";
    let prefix = text(&block[345..500]);
    if is_ustar && !prefix.is_empty() {
        name = format!("{prefix}/{name}");
    }
    Ok(Header {
        kind,
        name,
        mode: u32::try_from(octal(&block[100..108]).unwrap_or(0) & 0o7777).unwrap_or(0),
        size: number(&block[124..136]).ok_or(BadHeader)?,
        modified: number(&block[136..148]).unwrap_or(0),
    })
}

/// The `path` and `size` of a pax record set (`<length> <key>=<value>\n`
/// lines).
pub(super) fn pax_path_and_size(records: &[u8]) -> (Option<String>, Option<u64>) {
    let (mut path, mut size) = (None, None);
    let mut rest = records;
    while let Some(space) = rest.iter().position(|&byte| byte == b' ') {
        let Some(length) = std::str::from_utf8(&rest[..space])
            .ok()
            .and_then(|length| length.parse::<usize>().ok())
        else {
            break;
        };
        if length <= space + 1 || length > rest.len() {
            break;
        }
        let record = &rest[space + 1..length - 1];
        if let Some(value) = record.strip_prefix(b"path=") {
            path = Some(String::from_utf8_lossy(value).into_owned());
        } else if let Some(value) = record.strip_prefix(b"size=") {
            size = std::str::from_utf8(value)
                .ok()
                .and_then(|value| value.parse().ok());
        }
        rest = &rest[length..];
    }
    (path, size)
}

/// A NUL-terminated field as text.
pub(super) fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&byte| byte == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// An octal field, or GNU's base-256 form for numbers too large for it.
fn number(field: &[u8]) -> Option<u64> {
    if field.first().is_some_and(|&byte| byte & 0x80 != 0) {
        let mut value: u64 = u64::from(field[0] & 0x7f);
        for &byte in &field[1..] {
            value = value.checked_mul(256)?.checked_add(u64::from(byte))?;
        }
        return Some(value);
    }
    octal(field)
}

/// An octal field padded with spaces or NULs.
fn octal(field: &[u8]) -> Option<u64> {
    let digits = text(field);
    let digits = digits.trim_matches(|character: char| character == ' ' || character == '\0');
    if digits.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(digits, 8).ok()
}

/// A ustar header for `name`, for tests and for writing archives.
#[cfg(test)]
pub(super) fn ustar(name: &str, kind: u8, mode: u32, size: u64) -> [u8; BLOCK] {
    let mut block = [0u8; BLOCK];
    block[..name.len()].copy_from_slice(name.as_bytes());
    block[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
    block[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
    block[136..147].copy_from_slice(b"15000000000");
    block[156] = kind;
    block[257..263].copy_from_slice(b"ustar\0");
    block[263..265].copy_from_slice(b"00");
    block[148..156].copy_from_slice(b"        ");
    let sum: u32 = block.iter().map(|&byte| u32::from(byte)).sum();
    block[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ustar_header_reads_back_and_a_damaged_one_is_refused() {
        let block = ustar("docs/plan.txt", b'0', 0o644, 1234);
        let header = parse(&block).expect("a valid header");
        assert_eq!(
            (header.kind, header.name.as_str(), header.mode, header.size),
            (HeaderKind::File, "docs/plan.txt", 0o644, 1234)
        );
        let mut damaged = block;
        damaged[0] = b'x';
        assert_eq!(parse(&damaged), Err(BadHeader));
    }

    #[test]
    fn pax_records_give_long_paths_and_sizes() {
        let records = b"29 path=a/very/long/name.txt\n17 size=99999999\n";
        assert_eq!(
            pax_path_and_size(records),
            (Some("a/very/long/name.txt".to_owned()), Some(99_999_999))
        );
    }
}
