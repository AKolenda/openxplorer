// SPDX-License-Identifier: AGPL-3.0-only
//! The fixed-size records of the ZIP format and the little-endian fields
//! they are made of.
//!
//! Field names follow the ZIP specification (PKWARE's APPNOTE.TXT); the
//! layouts are the `struct` formats of Python's `zipfile`
//! (`structEndArchive`, `structCentralDir`, `structFileHeader` and the ZIP64
//! records).

/// Reads little-endian values from the front of a byte slice, in order.
#[derive(Debug)]
pub(super) struct Fields<'a> {
    rest: &'a [u8],
}

impl<'a> Fields<'a> {
    /// A reader positioned at the first byte of `bytes`.
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    /// The next `N` bytes, or `None` when fewer are left.
    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (value, rest) = self.rest.split_first_chunk::<N>()?;
        self.rest = rest;
        Some(*value)
    }

    /// The next byte.
    pub(super) fn u8(&mut self) -> Option<u8> {
        let [value] = self.array::<1>()?;
        Some(value)
    }

    /// The next little-endian 16-bit value.
    pub(super) fn u16(&mut self) -> Option<u16> {
        self.array().map(u16::from_le_bytes)
    }

    /// The next little-endian 32-bit value.
    pub(super) fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    /// The next little-endian 64-bit value.
    pub(super) fn u64(&mut self) -> Option<u64> {
        self.array().map(u64::from_le_bytes)
    }

    /// The bytes not read yet.
    pub(super) fn rest(&self) -> &'a [u8] {
        self.rest
    }
}

/// The end of central directory record: the entry point of every archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EndOfDirectory {
    pub(super) signature: u32,
    pub(super) size: u32,
    pub(super) offset: u32,
}

impl EndOfDirectory {
    /// `PK\x05\x06`.
    pub(super) const SIGNATURE: u32 = 0x0605_4b50;
    /// The record's length without the archive comment that follows it.
    pub(super) const SIZE: usize = 22;

    /// Reads the record from its [`Self::SIZE`] bytes.
    pub(super) fn parse(record: &[u8; Self::SIZE]) -> Self {
        Self::read(&mut Fields::new(record))
            .expect("a 22-byte record holds every end of central directory field")
    }

    /// Reads the fields in order; `None` when too few bytes are left.
    fn read(fields: &mut Fields<'_>) -> Option<Self> {
        let signature = fields.u32()?;
        // Disk numbers and entry counts: Python's `zipfile` never
        // reads them from this record.
        let _counts = fields.array::<8>()?;
        Some(Self {
            signature,
            size: fields.u32()?,
            offset: fields.u32()?,
        })
    }
}

/// The ZIP64 end of central directory locator, just before the end record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Zip64Locator {
    pub(super) signature: u32,
    pub(super) disk_with_record: u32,
    pub(super) record_offset: u64,
    pub(super) total_disks: u32,
}

impl Zip64Locator {
    /// `PK\x06\x07`.
    pub(super) const SIGNATURE: u32 = 0x0706_4b50;
    /// The record's length.
    pub(super) const SIZE: usize = 20;

    /// Reads the record from its [`Self::SIZE`] bytes.
    pub(super) fn parse(record: &[u8; Self::SIZE]) -> Self {
        Self::read(&mut Fields::new(record)).expect("a 20-byte record holds every ZIP64 locator field")
    }

    /// Reads the fields in order; `None` when too few bytes are left.
    fn read(fields: &mut Fields<'_>) -> Option<Self> {
        Some(Self {
            signature: fields.u32()?,
            disk_with_record: fields.u32()?,
            record_offset: fields.u64()?,
            total_disks: fields.u32()?,
        })
    }
}

/// The ZIP64 end of central directory record, for archives too large for
/// the 32-bit fields of [`EndOfDirectory`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Zip64EndOfDirectory {
    pub(super) signature: u32,
    pub(super) record_size: u64,
    pub(super) directory_size: u64,
    pub(super) directory_offset: u64,
}

impl Zip64EndOfDirectory {
    /// `PK\x06\x06`.
    pub(super) const SIGNATURE: u32 = 0x0606_4b50;
    /// The record's length without its extensible data sector.
    pub(super) const SIZE: usize = 56;
    /// The fields the record's own size field does not count: the
    /// signature and the size field itself.
    pub(super) const UNCOUNTED_BYTES: u64 = 12;

    /// Reads the record from its [`Self::SIZE`] bytes.
    pub(super) fn parse(record: &[u8; Self::SIZE]) -> Self {
        Self::read(&mut Fields::new(record)).expect("a 56-byte record holds every ZIP64 end record field")
    }

    /// Reads the fields in order; `None` when too few bytes are left.
    fn read(fields: &mut Fields<'_>) -> Option<Self> {
        let signature = fields.u32()?;
        let record_size = fields.u64()?;
        // Versions, disk numbers and entry counts, which `zipfile`
        // does not use either.
        let _unused = fields.array::<28>()?;
        Some(Self {
            signature,
            record_size,
            directory_size: fields.u64()?,
            directory_offset: fields.u64()?,
        })
    }
}

/// One member's record in the central directory, before its variable-length
/// name, extra field and comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CentralHeader {
    pub(super) signature: u32,
    /// The ZIP version needed to extract, times ten (6.3 is 63).
    pub(super) version_needed: u8,
    pub(super) flags: u16,
    pub(super) method: u16,
    pub(super) time: u16,
    pub(super) date: u16,
    pub(super) crc32: u32,
    pub(super) compressed_size: u32,
    pub(super) size: u32,
    pub(super) name_length: u16,
    pub(super) extra_length: u16,
    pub(super) comment_length: u16,
    pub(super) external_attributes: u32,
    pub(super) header_offset: u32,
}

impl CentralHeader {
    /// `PK\x01\x02`.
    pub(super) const SIGNATURE: u32 = 0x0201_4b50;
    /// The record's length without its name, extra field and comment.
    pub(super) const SIZE: usize = 46;

    /// Reads the record from its [`Self::SIZE`] bytes.
    pub(super) fn parse(record: &[u8; Self::SIZE]) -> Self {
        Self::read(&mut Fields::new(record)).expect("a 46-byte record holds every central directory field")
    }

    /// Reads the fields in order; `None` when too few bytes are left.
    fn read(fields: &mut Fields<'_>) -> Option<Self> {
        let signature = fields.u32()?;
        let _version_made_by = fields.u16()?;
        let version_needed = fields.u8()?;
        let _reserved = fields.u8()?;
        let flags = fields.u16()?;
        let method = fields.u16()?;
        let time = fields.u16()?;
        let date = fields.u16()?;
        let crc32 = fields.u32()?;
        let compressed_size = fields.u32()?;
        let size = fields.u32()?;
        let name_length = fields.u16()?;
        let extra_length = fields.u16()?;
        let comment_length = fields.u16()?;
        let _disk_and_internal_attributes = fields.u32()?;
        Some(Self {
            signature,
            version_needed,
            flags,
            method,
            time,
            date,
            crc32,
            compressed_size,
            size,
            name_length,
            extra_length,
            comment_length,
            external_attributes: fields.u32()?,
            header_offset: fields.u32()?,
        })
    }

    /// The bytes of the whole record, including name, extra field and
    /// comment.
    pub(super) fn total_length(&self) -> u64 {
        let variable =
            u64::from(self.name_length) + u64::from(self.extra_length) + u64::from(self.comment_length);
        Self::SIZE as u64 + variable
    }
}

/// The local header in front of each member's data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LocalHeader {
    pub(super) signature: u32,
    pub(super) flags: u16,
    pub(super) name_length: u16,
    pub(super) extra_length: u16,
}

impl LocalHeader {
    /// `PK\x03\x04`.
    pub(super) const SIGNATURE: u32 = 0x0403_4b50;
    /// The record's length without its name and extra field.
    pub(super) const SIZE: usize = 30;

    /// Reads the record from its [`Self::SIZE`] bytes.
    pub(super) fn parse(record: &[u8; Self::SIZE]) -> Self {
        Self::read(&mut Fields::new(record)).expect("a 30-byte record holds every local header field")
    }

    /// Reads the fields in order; `None` when too few bytes are left.
    fn read(fields: &mut Fields<'_>) -> Option<Self> {
        let signature = fields.u32()?;
        let _version_needed = fields.u16()?;
        let flags = fields.u16()?;
        // Method, time, date, CRC and sizes: the central directory's
        // values are the ones that count, as in `zipfile`.
        let _copies_of_central_fields = fields.array::<18>()?;
        Some(Self {
            signature,
            flags,
            name_length: fields.u16()?,
            extra_length: fields.u16()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_read_little_endian_values_in_order() {
        let mut fields = Fields::new(&[1, 2, 0, 3, 0, 0, 0, 9]);

        assert_eq!(fields.u8(), Some(1));
        assert_eq!(fields.u16(), Some(2));
        assert_eq!(fields.u32(), Some(3));
        assert_eq!(fields.rest(), &[9]);
        assert_eq!(fields.u16(), None);
    }

    #[test]
    fn a_central_header_counts_its_variable_fields() {
        let mut record = [0u8; CentralHeader::SIZE];
        record[..4].copy_from_slice(&CentralHeader::SIGNATURE.to_le_bytes());
        record[28..30].copy_from_slice(&5u16.to_le_bytes());
        record[30..32].copy_from_slice(&4u16.to_le_bytes());
        record[32..34].copy_from_slice(&3u16.to_le_bytes());

        let header = CentralHeader::parse(&record);

        assert_eq!(header.signature, CentralHeader::SIGNATURE);
        assert_eq!(header.total_length(), 46 + 5 + 4 + 3);
    }
}
