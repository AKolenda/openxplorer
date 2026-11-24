// SPDX-License-Identifier: AGPL-3.0-only
//! The icon resources of a Windows PE file (`.dll`, `.exe`, `.mun`):
//! its icon groups (`RT_GROUP_ICON`) and their images (`RT_ICON`).
//!
//! Only what the icons need is read: the DOS and PE headers, the section
//! table, the resource directory's first two levels for the two resource
//! types, and then the one group and the images it asks for. Everything
//! is read at checked offsets with `pread`, so a file of any size costs a
//! few small reads, and a value that points outside the file, past its
//! section or beyond a cap ends the read with an [`IconFileError`].

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

/// The largest file read; only its headers and icons are read, never all
/// of it.
const MAX_FILE_SIZE: u64 = 1 << 30;
/// The most sections a file may declare.
const MAX_SECTIONS: usize = 96;
/// The most entries one resource directory may list.
const MAX_DIRECTORY_ENTRIES: usize = 1 << 16;
/// The most images one icon group may name.
const MAX_GROUP_ENTRIES: usize = 64;
/// The largest image read: a 256-pixel 32-bit bitmap with its mask is
/// about 270 KiB.
pub(super) const MAX_IMAGE_BYTES: usize = 1 << 20;

/// `RT_ICON`: one image of an icon.
const RT_ICON: u16 = 3;
/// `RT_GROUP_ICON`: an icon, as the list of its images.
const RT_GROUP_ICON: u16 = 14;

/// Why a file gives no icons.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IconFileError {
    /// The file could not be opened or read.
    #[error("the file could not be read: {0}")]
    Unreadable(String),
    /// The file is larger than [`MAX_FILE_SIZE`].
    #[error("the file is too large")]
    TooLarge,
    /// The file is not a Windows PE file.
    #[error("the file is not a Windows program or library")]
    NotPe,
    /// A header, table or resource lies outside the file or its section.
    #[error("the file is damaged")]
    Damaged,
    /// A count or size is over its cap.
    #[error("the file holds more than OpenXplorer reads")]
    OverLimit,
    /// The file has no icon resources.
    #[error("the file holds no icons")]
    NoIcons,
}

/// Bytes a file is read from: an open file, or bytes in memory for tests.
trait Source {
    /// How many bytes there are.
    fn size(&self) -> u64;
    /// The `len` bytes at `offset`, or [`IconFileError::Damaged`] when they
    /// run past the end.
    fn read(&self, offset: u64, len: usize) -> Result<Vec<u8>, IconFileError>;
}

/// A file on disk.
struct DiskSource {
    file: File,
    size: u64,
}

impl Source for DiskSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read(&self, offset: u64, len: usize) -> Result<Vec<u8>, IconFileError> {
        let end = offset.checked_add(len as u64).ok_or(IconFileError::Damaged)?;
        if end > self.size {
            return Err(IconFileError::Damaged);
        }
        let mut bytes = vec![0; len];
        self.file
            .read_exact_at(&mut bytes, offset)
            .map_err(|error| IconFileError::Unreadable(error.to_string()))?;
        Ok(bytes)
    }
}

impl Source for Vec<u8> {
    fn size(&self) -> u64 {
        self.len() as u64
    }

    fn read(&self, offset: u64, len: usize) -> Result<Vec<u8>, IconFileError> {
        let start = usize::try_from(offset).map_err(|_| IconFileError::Damaged)?;
        let end = start.checked_add(len).ok_or(IconFileError::Damaged)?;
        self.get(start..end)
            .map(<[u8]>::to_vec)
            .ok_or(IconFileError::Damaged)
    }
}

/// A little-endian `u16` at `at` in `bytes`.
fn u16_at(bytes: &[u8], at: usize) -> Result<u16, IconFileError> {
    let end = at.checked_add(2).ok_or(IconFileError::Damaged)?;
    let field = bytes.get(at..end).ok_or(IconFileError::Damaged)?;
    Ok(u16::from_le_bytes([field[0], field[1]]))
}

/// A little-endian `u32` at `at` in `bytes`.
fn u32_at(bytes: &[u8], at: usize) -> Result<u32, IconFileError> {
    let end = at.checked_add(4).ok_or(IconFileError::Damaged)?;
    let field = bytes.get(at..end).ok_or(IconFileError::Damaged)?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

/// A section's place in memory and in the file.
#[derive(Debug, Clone, Copy)]
struct Section {
    /// Where it starts in memory.
    virtual_address: u32,
    /// How many of its bytes the file holds.
    raw_size: u32,
    /// Where those bytes start in the file.
    raw_offset: u32,
}

/// One entry of a resource directory: its numeric ID and the offset it
/// points to, from the start of the resources. Named entries are skipped:
/// icons are found by number.
#[derive(Debug, Clone, Copy)]
struct DirectoryEntry {
    id: u16,
    target: u32,
}

/// The high bit of an entry's target: it points to another directory.
const SUBDIRECTORY: u32 = 0x8000_0000;

/// One image of an icon group, as the file stores it: a PNG, or a
/// bitmap (a `BITMAPINFOHEADER` and its pixels and mask).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconImage {
    /// The width the group gives it, in pixels (256 for the group's 0).
    pub width: u32,
    /// The height the group gives it, in pixels (256 for the group's 0).
    pub height: u32,
    /// Its color depth in bits per pixel, as the group gives it.
    pub bit_count: u16,
    /// The image's bytes.
    pub data: Vec<u8>,
}

/// A PE file's icon resources, ready to give icon groups by number.
pub struct IconFile {
    source: Box<dyn Source>,
    sections: Vec<Section>,
    /// Where the resource directory starts in memory.
    resources: u32,
    /// The icon groups, by ID.
    groups: Vec<DirectoryEntry>,
    /// The icon images, by ID.
    images: Vec<DirectoryEntry>,
}

impl IconFile {
    /// Reads the headers and the icon directories of the file at `path`.
    ///
    /// # Errors
    ///
    /// Any [`IconFileError`]: the file is unreadable, too large, not a PE
    /// file, damaged, over a cap or without icons.
    pub fn open(path: &Path) -> Result<Self, IconFileError> {
        let file = File::open(path).map_err(|error| IconFileError::Unreadable(error.to_string()))?;
        let metadata = file
            .metadata()
            .map_err(|error| IconFileError::Unreadable(error.to_string()))?;
        if !metadata.is_file() {
            return Err(IconFileError::NotPe);
        }
        let size = metadata.len();
        Self::read(Box::new(DiskSource { file, size }))
    }

    /// [`Self::open`] for a file already in memory.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, IconFileError> {
        Self::read(Box::new(bytes))
    }

    fn read(source: Box<dyn Source>) -> Result<Self, IconFileError> {
        if source.size() > MAX_FILE_SIZE {
            return Err(IconFileError::TooLarge);
        }
        let dos = source.read(0, 64).map_err(|_| IconFileError::NotPe)?;
        if dos.get(..2) != Some(b"MZ".as_slice()) {
            return Err(IconFileError::NotPe);
        }
        let pe_offset = u64::from(u32_at(&dos, 0x3c)?);
        let signature_and_header = source.read(pe_offset, 24).map_err(|_| IconFileError::NotPe)?;
        if signature_and_header.get(..4) != Some(b"PE\0\0".as_slice()) {
            return Err(IconFileError::NotPe);
        }
        let section_count = usize::from(u16_at(&signature_and_header, 6)?);
        let optional_size = usize::from(u16_at(&signature_and_header, 20)?);
        if section_count > MAX_SECTIONS {
            return Err(IconFileError::OverLimit);
        }
        let optional_offset = pe_offset + 24;
        let optional = source.read(optional_offset, optional_size)?;
        let resources = resource_directory(&optional)?;
        let section_table = source.read(optional_offset + optional_size as u64, section_count * 40)?;
        let sections = (0..section_count)
            .map(|index| {
                let at = index * 40;
                Ok(Section {
                    virtual_address: u32_at(&section_table, at + 12)?,
                    raw_size: u32_at(&section_table, at + 16)?,
                    raw_offset: u32_at(&section_table, at + 20)?,
                })
            })
            .collect::<Result<Vec<_>, IconFileError>>()?;
        let mut file = Self {
            source,
            sections,
            resources,
            groups: Vec::new(),
            images: Vec::new(),
        };
        let types = file.directory(0)?;
        let subdirectory = |kind: u16| {
            types
                .iter()
                .find(|entry| entry.id == kind && entry.target & SUBDIRECTORY != 0)
                .map(|entry| entry.target & !SUBDIRECTORY)
        };
        let (Some(groups), Some(images)) = (subdirectory(RT_GROUP_ICON), subdirectory(RT_ICON)) else {
            return Err(IconFileError::NoIcons);
        };
        file.groups = file.directory(groups)?;
        file.images = file.directory(images)?;
        Ok(file)
    }

    /// The `len` bytes at the address `rva` in memory, read from the
    /// section that holds all of them in the file.
    fn read_rva(&self, rva: u32, len: usize) -> Result<Vec<u8>, IconFileError> {
        let section = self
            .sections
            .iter()
            .find(|section| {
                rva >= section.virtual_address && rva - section.virtual_address < section.raw_size
            })
            .ok_or(IconFileError::Damaged)?;
        let inside = u64::from(rva - section.virtual_address);
        if inside + len as u64 > u64::from(section.raw_size) {
            return Err(IconFileError::Damaged);
        }
        self.source.read(u64::from(section.raw_offset) + inside, len)
    }

    /// The bytes `len` long at `offset` from the start of the resources.
    fn read_resource(&self, offset: u32, len: usize) -> Result<Vec<u8>, IconFileError> {
        let rva = self.resources.checked_add(offset).ok_or(IconFileError::Damaged)?;
        self.read_rva(rva, len)
    }

    /// The numbered entries of the resource directory at `offset`.
    fn directory(&self, offset: u32) -> Result<Vec<DirectoryEntry>, IconFileError> {
        let header = self.read_resource(offset, 16)?;
        let named = usize::from(u16_at(&header, 12)?);
        let numbered = usize::from(u16_at(&header, 14)?);
        if named + numbered > MAX_DIRECTORY_ENTRIES {
            return Err(IconFileError::OverLimit);
        }
        let first = offset.checked_add(16).ok_or(IconFileError::Damaged)?;
        let table = self.read_resource(first, (named + numbered) * 8)?;
        (named..named + numbered)
            .map(|index| {
                let name = u32_at(&table, index * 8)?;
                let target = u32_at(&table, index * 8 + 4)?;
                // A numbered entry's name is its ID; a set high bit would
                // make it a name after all.
                if name > u32::from(u16::MAX) {
                    return Err(IconFileError::Damaged);
                }
                Ok(DirectoryEntry {
                    id: u16::try_from(name).map_err(|_| IconFileError::Damaged)?,
                    target,
                })
            })
            .collect()
    }

    /// The data of resource `id` among `entries`, in its first language,
    /// at most `cap` bytes; `None` when there is no such resource.
    fn resource(
        &self,
        entries: &[DirectoryEntry],
        id: u16,
        cap: usize,
    ) -> Result<Option<Vec<u8>>, IconFileError> {
        let Some(entry) = entries.iter().find(|entry| entry.id == id) else {
            return Ok(None);
        };
        if entry.target & SUBDIRECTORY == 0 {
            return Err(IconFileError::Damaged);
        }
        let languages = self.directory(entry.target & !SUBDIRECTORY)?;
        let Some(language) = languages.first() else {
            return Ok(None);
        };
        if language.target & SUBDIRECTORY != 0 {
            return Err(IconFileError::Damaged);
        }
        let data_entry = self.read_resource(language.target, 16)?;
        let rva = u32_at(&data_entry, 0)?;
        let size = usize::try_from(u32_at(&data_entry, 4)?).map_err(|_| IconFileError::OverLimit)?;
        if size > cap {
            return Err(IconFileError::OverLimit);
        }
        self.read_rva(rva, size).map(Some)
    }

    /// The IDs of the file's icon groups.
    pub fn group_ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.groups.iter().map(|entry| entry.id)
    }

    /// The images of icon group `id`, in the group's order; empty when the
    /// file has no such group or none of its images. An image that is
    /// missing, damaged or over the cap is left out.
    ///
    /// # Errors
    ///
    /// [`IconFileError::Damaged`] or [`IconFileError::OverLimit`] when the
    /// group itself is.
    pub fn group(&self, id: u16) -> Result<Vec<IconImage>, IconFileError> {
        let Some(group) = self.resource(&self.groups, id, 6 + MAX_GROUP_ENTRIES * 14)? else {
            return Ok(Vec::new());
        };
        if u16_at(&group, 0)? != 0 || u16_at(&group, 2)? != 1 {
            return Err(IconFileError::Damaged);
        }
        let count = usize::from(u16_at(&group, 4)?);
        if count > MAX_GROUP_ENTRIES {
            return Err(IconFileError::OverLimit);
        }
        let mut images = Vec::new();
        for index in 0..count {
            let at = 6 + index * 14;
            let width = *group.get(at).ok_or(IconFileError::Damaged)?;
            let height = *group.get(at + 1).ok_or(IconFileError::Damaged)?;
            let bit_count = u16_at(&group, at + 6)?;
            let image_id = u16_at(&group, at + 12)?;
            let Ok(Some(data)) = self.resource(&self.images, image_id, MAX_IMAGE_BYTES) else {
                continue;
            };
            images.push(IconImage {
                width: edge(width),
                height: edge(height),
                bit_count,
                data,
            });
        }
        Ok(images)
    }
}

/// A group entry's edge: 0 stands for 256.
fn edge(byte: u8) -> u32 {
    if byte == 0 {
        256
    } else {
        u32::from(byte)
    }
}

/// The address of the resource directory, from the optional header's data
/// directory; [`IconFileError::NoIcons`] when it has none.
fn resource_directory(optional: &[u8]) -> Result<u32, IconFileError> {
    // PE32 and PE32+ keep the count of data directories, and then the
    // directories, at different offsets.
    let (count_at, first) = match u16_at(optional, 0).map_err(|_| IconFileError::NotPe)? {
        0x10b => (92, 96),
        0x20b => (108, 112),
        _ => return Err(IconFileError::NotPe),
    };
    let count = u32_at(optional, count_at)?;
    // The resource table is the third directory.
    if count < 3 {
        return Err(IconFileError::NoIcons);
    }
    let address = u32_at(optional, first + 16)?;
    let size = u32_at(optional, first + 20)?;
    if address == 0 || size == 0 {
        return Err(IconFileError::NoIcons);
    }
    Ok(address)
}
