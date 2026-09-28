// SPDX-License-Identifier: AGPL-3.0-only
//! The extra fields of a central directory record that change how a member
//! is read. Ports `ZipInfo._decodeExtra` of Python's `zipfile`.
//!
//! - ZIP64 (0x0001) carries the 64-bit sizes and offset of a member whose
//!   32-bit fields hold the `0xFFFFFFFF` marker.
//! - The Info-ZIP Unicode path (0x7075) replaces the member's name when its
//!   checksum matches the recorded name. A member renamed this way no
//!   longer has its recorded name, which the archive services refuse.
//!
//! Other fields are skipped.

use super::error::{Zip64Field, ZipFormatError};
use super::member::ZipMember;
use super::names::cut_at_nul;
use super::records::Fields;

/// The header id of the ZIP64 extended information field.
const ZIP64_FIELD: u16 = 0x0001;
/// The header id of the Info-ZIP Unicode path field.
const UNICODE_PATH_FIELD: u16 = 0x7075;
/// The only Unicode path field version there is.
const UNICODE_PATH_VERSION: u8 = 1;
/// A 32-bit field holding this value is found in the ZIP64 field instead.
const ZIP64_MARKER_32: u64 = 0xffff_ffff;
/// `zipfile` accepts a 64-bit size marker too.
const ZIP64_MARKER_64: u64 = u64::MAX;

/// Applies the extra fields of `member`'s central directory record.
/// `name_checksum` is the CRC-32 of the raw recorded name.
///
/// # Errors
///
/// A field longer than the bytes left, or a ZIP64 or Unicode path field
/// that is too short or not UTF-8.
pub(super) fn apply_extra_fields(
    member: &mut ZipMember,
    extra: &[u8],
    name_checksum: u32,
) -> Result<(), ZipFormatError> {
    let mut fields = Fields::new(extra);
    while let (Some(field_id), Some(size)) = (fields.u16(), fields.u16()) {
        let rest = fields.rest();
        let Some(data) = rest.get(..usize::from(size)) else {
            return Err(ZipFormatError::CorruptExtraField { field_id, size });
        };
        match field_id {
            ZIP64_FIELD => apply_zip64_field(member, data)?,
            UNICODE_PATH_FIELD => apply_unicode_path(member, data, name_checksum)?,
            _ => {}
        }
        fields = Fields::new(&rest[data.len()..]);
    }
    Ok(())
}

/// Takes the 64-bit values of the fields that hold the ZIP64 marker, in
/// the order the specification lists them.
fn apply_zip64_field(member: &mut ZipMember, data: &[u8]) -> Result<(), ZipFormatError> {
    let mut values = Fields::new(data);
    let missing = ZipFormatError::CorruptZip64Field;
    if member.size == ZIP64_MARKER_32 || member.size == ZIP64_MARKER_64 {
        member.size = values.u64().ok_or(missing(Zip64Field::FileSize))?;
    }
    if member.compressed_size == ZIP64_MARKER_32 {
        member.compressed_size = values.u64().ok_or(missing(Zip64Field::CompressedSize))?;
    }
    if member.header_offset == ZIP64_MARKER_32 {
        member.header_offset = values.u64().ok_or(missing(Zip64Field::HeaderOffset))?;
    }
    Ok(())
}

/// Renames the member to the field's UTF-8 name when the field belongs to
/// the recorded name; an empty name is ignored, as in `zipfile`.
fn apply_unicode_path(member: &mut ZipMember, data: &[u8], name_checksum: u32) -> Result<(), ZipFormatError> {
    let mut fields = Fields::new(data);
    let (Some(version), Some(checksum)) = (fields.u8(), fields.u32()) else {
        return Err(ZipFormatError::CorruptUnicodePathField);
    };
    if version != UNICODE_PATH_VERSION || checksum != name_checksum {
        return Ok(());
    }
    let unicode_name = std::str::from_utf8(fields.rest()).map_err(|_| ZipFormatError::InvalidUnicodePath)?;
    if !unicode_name.is_empty() {
        member.name = cut_at_nul(unicode_name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::member::{CompressionMethod, DosDateTime};
    use super::*;

    /// A member recorded as `name` with 32-bit sizes and offset.
    fn member(name: &str, size: u64, compressed_size: u64, header_offset: u64) -> ZipMember {
        ZipMember {
            name: name.to_owned(),
            original_name: name.to_owned(),
            flags: 0,
            method: CompressionMethod::Stored,
            crc32: 0,
            compressed_size,
            size,
            header_offset,
            data_limit: 0,
            external_attributes: 0,
            modified: DosDateTime { date: 0, time: 0 },
        }
    }

    /// An extra field with the given id and data.
    fn field(field_id: u16, data: &[u8]) -> Vec<u8> {
        let size = u16::try_from(data.len()).expect("test fields are small");
        let mut bytes = field_id.to_le_bytes().to_vec();
        bytes.extend(size.to_le_bytes());
        bytes.extend(data);
        bytes
    }

    #[test]
    fn zip64_values_replace_only_the_marked_fields() {
        let mut marked = member("big.bin", ZIP64_MARKER_32, 10, ZIP64_MARKER_32);
        let mut data = (5u64 << 32).to_le_bytes().to_vec();
        data.extend(7u64.to_le_bytes());

        apply_extra_fields(&mut marked, &field(ZIP64_FIELD, &data), 0).expect("a valid ZIP64 field");

        assert_eq!(
            (marked.size, marked.compressed_size, marked.header_offset),
            (5 << 32, 10, 7)
        );
    }

    #[test]
    fn a_short_zip64_field_names_the_missing_value() {
        let mut marked = member("big.bin", 1, ZIP64_MARKER_32, 0);

        let result = apply_extra_fields(&mut marked, &field(ZIP64_FIELD, &[0; 4]), 0);

        assert_eq!(
            result,
            Err(ZipFormatError::CorruptZip64Field(Zip64Field::CompressedSize))
        );
    }

    #[test]
    fn a_field_longer_than_the_extra_data_is_corrupt() {
        let mut plain = member("a.txt", 1, 1, 0);
        let mut extra = field(0x5455, &[0; 9]);
        extra.truncate(8);

        let result = apply_extra_fields(&mut plain, &extra, 0);

        assert_eq!(
            result,
            Err(ZipFormatError::CorruptExtraField {
                field_id: 0x5455,
                size: 9
            })
        );
    }

    #[test]
    fn a_matching_unicode_path_renames_the_member() {
        let mut renamed = member("caf\u{e9}.txt", 1, 1, 0);
        let mut data = vec![UNICODE_PATH_VERSION];
        data.extend(1234u32.to_le_bytes());
        data.extend("Café\0hidden.txt".as_bytes());

        apply_extra_fields(&mut renamed, &field(UNICODE_PATH_FIELD, &data), 1234).expect("valid field");

        assert_eq!(renamed.name, "Café");
        assert!(!renamed.has_unaltered_name());
    }

    #[test]
    fn a_unicode_path_for_another_name_is_ignored() {
        let mut unchanged = member("a.txt", 1, 1, 0);
        let mut data = vec![UNICODE_PATH_VERSION];
        data.extend(1u32.to_le_bytes());
        data.extend("b.txt".as_bytes());

        apply_extra_fields(&mut unchanged, &field(UNICODE_PATH_FIELD, &data), 2).expect("valid field");

        assert_eq!(unchanged.name, "a.txt");
    }

    #[test]
    fn a_broken_unicode_path_is_corrupt() {
        let mut plain = member("a.txt", 1, 1, 0);
        let short = field(UNICODE_PATH_FIELD, &[1, 0]);
        let mut not_utf8 = vec![UNICODE_PATH_VERSION];
        not_utf8.extend(9u32.to_le_bytes());
        not_utf8.push(0xff);

        assert_eq!(
            apply_extra_fields(&mut plain, &short, 9),
            Err(ZipFormatError::CorruptUnicodePathField)
        );
        assert_eq!(
            apply_extra_fields(&mut plain, &field(UNICODE_PATH_FIELD, &not_utf8), 9),
            Err(ZipFormatError::InvalidUnicodePath)
        );
    }
}
