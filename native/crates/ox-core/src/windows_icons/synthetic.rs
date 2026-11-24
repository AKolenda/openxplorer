// SPDX-License-Identifier: AGPL-3.0-only
//! Tiny synthetic Windows icon files, for tests and pictures of the app.
//!
//! A file made here is a valid PE32 or PE32+ image with one `.rsrc`
//! section that holds the icon groups and images the caller gives, and
//! nothing else. Its pictures are drawn here from plain colors and
//! shapes: no Windows file or artwork is copied, so tests and pictures
//! show these test icons, never Windows ones.

use super::picture::{encode_png, Rgba};

/// Where the resource section starts in memory.
const RESOURCES_RVA: u32 = 0x1000;
/// Where it starts in the file.
const RESOURCES_OFFSET: u32 = 0x200;

/// One image of a synthetic icon group.
#[derive(Debug, Clone)]
pub struct SyntheticImage {
    /// Its `RT_ICON` ID.
    pub id: u16,
    /// The edge the group gives it, 1 to 256.
    pub edge: u32,
    /// The color depth the group gives it.
    pub bit_count: u16,
    /// The image's bytes: a PNG or a bitmap.
    pub data: Vec<u8>,
}

/// A synthetic icon file: groups by ID, each with its images.
#[derive(Debug, Clone, Default)]
pub struct SyntheticIconFile {
    /// The icon groups, by `RT_GROUP_ICON` ID, and their images.
    pub groups: Vec<(u16, Vec<SyntheticImage>)>,
    /// Build a PE32+ (64-bit) file instead of PE32.
    pub wide: bool,
    /// Leave the `RT_ICON` images out, as the stub libraries in
    /// `System32` do, keeping the groups.
    pub without_images: bool,
}

impl SyntheticIconFile {
    /// A file whose groups `ids` each hold a 16, 32 and 256 pixel image:
    /// the 16-pixel one an 8-bit palette bitmap, the 32-pixel one a 32-bit
    /// bitmap and the 256-pixel one a PNG, every one a test pattern in
    /// colors chosen from its group's ID.
    pub fn with_groups(ids: &[u16]) -> Self {
        let mut next_image = 1;
        let groups = ids
            .iter()
            .map(|&group| {
                let color = test_color(group);
                let images = [(16, 8), (32, 32), (256, 32)]
                    .into_iter()
                    .map(|(edge, bit_count)| {
                        let pattern = test_pattern(edge, color);
                        let data = match (edge, bit_count) {
                            (256, _) => encode_png(&pattern),
                            (_, 8) => palette_bitmap(&pattern),
                            _ => rgba_bitmap(&pattern),
                        };
                        let image = SyntheticImage {
                            id: next_image,
                            edge,
                            bit_count,
                            data,
                        };
                        next_image += 1;
                        image
                    })
                    .collect();
                (group, images)
            })
            .collect();
        Self {
            groups,
            ..Self::default()
        }
    }

    /// The file's bytes.
    ///
    /// # Panics
    ///
    /// If the groups and images are too large for a PE file, which a
    /// test's few small icons never are.
    pub fn build(&self) -> Vec<u8> {
        let resources = self.resources();
        let raw_size = u32::try_from(resources.len().next_multiple_of(0x200)).expect("a small file");
        let optional_size: u16 = if self.wide { 240 } else { 224 };
        let mut file = vec![0u8; RESOURCES_OFFSET as usize];
        file[..2].copy_from_slice(b"MZ");
        file[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        let pe = 64;
        file[pe..pe + 4].copy_from_slice(b"PE\0\0");
        let machine: u16 = if self.wide { 0x8664 } else { 0x14c };
        file[pe + 4..pe + 6].copy_from_slice(&machine.to_le_bytes());
        file[pe + 6..pe + 8].copy_from_slice(&1u16.to_le_bytes());
        file[pe + 20..pe + 22].copy_from_slice(&optional_size.to_le_bytes());
        // A DLL with no code.
        file[pe + 22..pe + 24].copy_from_slice(&0x2102u16.to_le_bytes());
        let optional = pe + 24;
        let (magic, count_at, first): (u16, usize, usize) = if self.wide {
            (0x20b, 108, 112)
        } else {
            (0x10b, 92, 96)
        };
        file[optional..optional + 2].copy_from_slice(&magic.to_le_bytes());
        file[optional + count_at..optional + count_at + 4].copy_from_slice(&16u32.to_le_bytes());
        let directory = optional + first + 16;
        file[directory..directory + 4].copy_from_slice(&RESOURCES_RVA.to_le_bytes());
        let size = u32::try_from(resources.len()).expect("a small file");
        file[directory + 4..directory + 8].copy_from_slice(&size.to_le_bytes());
        let section = optional + usize::from(optional_size);
        file[section..section + 5].copy_from_slice(b".rsrc");
        file[section + 8..section + 12].copy_from_slice(&size.to_le_bytes());
        file[section + 12..section + 16].copy_from_slice(&RESOURCES_RVA.to_le_bytes());
        file[section + 16..section + 20].copy_from_slice(&raw_size.to_le_bytes());
        file[section + 20..section + 24].copy_from_slice(&RESOURCES_OFFSET.to_le_bytes());
        file.extend_from_slice(&resources);
        file.resize(RESOURCES_OFFSET as usize + raw_size as usize, 0);
        file
    }

    /// The resource section: the type directory, a directory of groups
    /// and one of images, a language directory and a data entry for each
    /// resource, then the resources' bytes.
    fn resources(&self) -> Vec<u8> {
        let images: Vec<&SyntheticImage> = if self.without_images {
            Vec::new()
        } else {
            self.groups.iter().flat_map(|(_, images)| images).collect()
        };
        let groups: Vec<(u16, Vec<u8>)> = self
            .groups
            .iter()
            .map(|(id, images)| (*id, group_bytes(images)))
            .collect();
        // Every resource in the order its directories list them: images
        // (type 3) first, then groups (type 14), as Windows sorts types.
        let mut resources: Vec<(u16, &[u8])> = images
            .iter()
            .map(|image| (image.id, image.data.as_slice()))
            .collect();
        let image_count = resources.len();
        resources.extend(groups.iter().map(|(id, bytes)| (*id, bytes.as_slice())));

        let root = 16 + 2 * 8;
        let image_directory = root;
        let group_directory = image_directory + 16 + image_count * 8;
        let languages = group_directory + 16 + groups.len() * 8;
        let data_entries = languages + resources.len() * 24;
        let mut blobs = data_entries + resources.len() * 16;

        let mut section = Vec::new();
        directory(
            &mut section,
            &[
                (3, small(image_directory) | 0x8000_0000),
                (14, small(group_directory) | 0x8000_0000),
            ],
        );
        let language_of = |index: usize| small(languages + index * 24) | 0x8000_0000;
        let image_entries: Vec<(u16, u32)> = (0..image_count)
            .map(|index| (resources[index].0, language_of(index)))
            .collect();
        directory(&mut section, &image_entries);
        let group_entries: Vec<(u16, u32)> = (image_count..resources.len())
            .map(|index| (resources[index].0, language_of(index)))
            .collect();
        directory(&mut section, &group_entries);
        for index in 0..resources.len() {
            directory(&mut section, &[(1033, small(data_entries + index * 16))]);
        }
        for (_, bytes) in &resources {
            let rva = RESOURCES_RVA + small(blobs);
            section.extend_from_slice(&rva.to_le_bytes());
            section.extend_from_slice(&small(bytes.len()).to_le_bytes());
            section.extend_from_slice(&[0; 8]);
            blobs += bytes.len().next_multiple_of(8);
        }
        for (_, bytes) in &resources {
            section.extend_from_slice(bytes);
            section.resize(section.len().next_multiple_of(8), 0);
        }
        section
    }
}

/// `value` as a 32-bit field of a synthetic file, which is small.
fn small(value: usize) -> u32 {
    u32::try_from(value).expect("a synthetic file is small")
}

/// `value` as a 16-bit count of a synthetic file, which lists few items.
fn count(value: usize) -> u16 {
    u16::try_from(value).expect("a synthetic file lists few items")
}

/// Appends a resource directory of numbered `entries` to `section`.
fn directory(section: &mut Vec<u8>, entries: &[(u16, u32)]) {
    section.extend_from_slice(&[0; 12]);
    section.extend_from_slice(&0u16.to_le_bytes());
    section.extend_from_slice(&count(entries.len()).to_le_bytes());
    for (id, target) in entries {
        section.extend_from_slice(&u32::from(*id).to_le_bytes());
        section.extend_from_slice(&target.to_le_bytes());
    }
}

/// A `GRPICONDIR` listing `images`.
fn group_bytes(images: &[SyntheticImage]) -> Vec<u8> {
    let mut group = vec![0, 0, 1, 0];
    group.extend_from_slice(&count(images.len()).to_le_bytes());
    for image in images {
        let edge = u8::try_from(image.edge).unwrap_or(0);
        group.extend_from_slice(&[edge, edge, 0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&image.bit_count.to_le_bytes());
        group.extend_from_slice(&small(image.data.len()).to_le_bytes());
        group.extend_from_slice(&image.id.to_le_bytes());
    }
    group
}

/// A color of its own for each group: red, green and blue from the ID.
fn test_color(id: u16) -> [u8; 3] {
    let spread = u32::from(id).wrapping_mul(2_654_435_761);
    let [red, green, blue, _] = spread.to_le_bytes();
    [red | 0x40, green | 0x40, blue | 0x40]
}

/// A test pattern `edge` pixels square: a rounded tile in `color` with a
/// white diagonal, on transparency.
pub fn test_pattern(edge: u32, color: [u8; 3]) -> Rgba {
    let mut pixels = Vec::with_capacity((edge * edge * 4) as usize);
    let inset = edge / 8;
    for y in 0..edge {
        for x in 0..edge {
            let inside = (inset..edge - inset).contains(&x) && (inset..edge - inset).contains(&y);
            let pixel = if !inside {
                [0, 0, 0, 0]
            } else if x.abs_diff(y) <= edge / 16 {
                [255, 255, 255, 255]
            } else {
                [color[0], color[1], color[2], 255]
            };
            pixels.extend_from_slice(&pixel);
        }
    }
    Rgba {
        width: edge,
        height: edge,
        pixels,
    }
}

/// The stored rows of a bitmap of `height` rows `stride` bytes long,
/// bottom row first, from `row(y)` for each row top first.
fn bottom_up(height: u32, stride: usize, row: impl Fn(u32) -> Vec<u8>) -> Vec<u8> {
    let mut rows = Vec::new();
    for y in (0..height).rev() {
        let mut bytes = row(y);
        bytes.resize(stride, 0);
        rows.extend_from_slice(&bytes);
    }
    rows
}

/// A `BITMAPINFOHEADER` for an icon image of `image`'s size.
fn bitmap_header(image: &Rgba, bit_count: u16, colors: u32) -> Vec<u8> {
    let mut header = Vec::with_capacity(40);
    header.extend_from_slice(&40u32.to_le_bytes());
    header.extend_from_slice(&image.width.to_le_bytes());
    header.extend_from_slice(&(image.height * 2).to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&bit_count.to_le_bytes());
    header.extend_from_slice(&[0; 16]);
    header.extend_from_slice(&colors.to_le_bytes());
    header.extend_from_slice(&[0; 4]);
    header
}

/// The AND mask of `image`: set where it is transparent.
fn mask(image: &Rgba) -> Vec<u8> {
    let stride = (image.width as usize).div_ceil(32) * 4;
    bottom_up(image.height, stride, |y| {
        let mut row = vec![0u8; stride];
        for x in 0..image.width {
            let alpha = image.pixels[((y * image.width + x) * 4 + 3) as usize];
            if alpha == 0 {
                row[(x / 8) as usize] |= 0x80 >> (x % 8);
            }
        }
        row
    })
}

/// `image` as a 32-bit bitmap with alpha, and its mask.
pub fn rgba_bitmap(image: &Rgba) -> Vec<u8> {
    let mut data = bitmap_header(image, 32, 0);
    let stride = image.width as usize * 4;
    data.extend(bottom_up(image.height, stride, |y| {
        let start = (y * image.width * 4) as usize;
        image.pixels[start..start + stride]
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
            .collect()
    }));
    data.extend(mask(image));
    data
}

/// `image` as an 8-bit palette bitmap whose transparency is its mask. The
/// palette holds the image's colors in the order they first appear; an
/// image of more than 256 colors is not one this makes.
///
/// # Panics
///
/// If `image` has more than 256 opaque colors.
pub fn palette_bitmap(image: &Rgba) -> Vec<u8> {
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut indexes = Vec::new();
    for pixel in image.pixels.chunks_exact(4) {
        let color = [pixel[0], pixel[1], pixel[2]];
        let index = palette
            .iter()
            .position(|known| *known == color)
            .unwrap_or_else(|| {
                palette.push(color);
                palette.len() - 1
            });
        indexes.push(u8::try_from(index).expect("at most 256 colors"));
    }
    let mut data = bitmap_header(image, 8, small(palette.len()));
    for [red, green, blue] in &palette {
        data.extend_from_slice(&[*blue, *green, *red, 0]);
    }
    let stride = (image.width as usize).div_ceil(4) * 4;
    data.extend(bottom_up(image.height, stride, |y| {
        let start = (y * image.width) as usize;
        indexes[start..start + image.width as usize].to_vec()
    }));
    data.extend(mask(image));
    data
}
