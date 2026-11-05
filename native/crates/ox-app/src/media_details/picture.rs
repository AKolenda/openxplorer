// SPDX-License-Identifier: AGPL-3.0-only
//! A picture's Details: its header (size, resolution, bit depth), its EXIF
//! and Windows' XP tags, its XMP rating and a PNG's text chunks.
//!
//! Everything is read from the first [`HEAD_LIMIT`] bytes, already in
//! memory: a picture is untrusted input, so every length and offset is
//! checked against the bytes there, and nothing here allocates more than
//! they hold. EXIF is parsed by kamadak-exif, in pure Rust.

use exif::{In, Tag, Value};

use super::{MediaDetails, NaiveDateTime};

/// How much of a picture is read: EXIF, a PNG's header chunks and a JPEG's
/// frame header sit at the start, so a huge picture costs no more.
pub(super) const HEAD_LIMIT: usize = 4 * 1024 * 1024;
/// The longest text a row shows; the rest of a longer tag is cut off.
const TEXT_LIMIT: usize = 1024;

/// Windows' own tags, which its Details tab writes (UTF-16LE).
const XP_TITLE: u16 = 0x9c9b;
const XP_COMMENT: u16 = 0x9c9c;
const XP_AUTHOR: u16 = 0x9c9d;
const XP_KEYWORDS: u16 = 0x9c9e;
const XP_SUBJECT: u16 = 0x9c9f;
/// Windows' star rating (0 to 5), in the first image's IFD.
const RATING: u16 = 0x4746;

/// Reads what `head`, the start of a picture, says about it.
pub(super) fn read_picture(head: &[u8], details: &mut MediaDetails) {
    if let Some(header) = header(head) {
        details.width = details.width.or(Some(header.width));
        details.height = details.height.or(Some(header.height));
        details.bit_depth = header.bit_depth;
        details.resolution = header.resolution;
        details.rating_readable = header.rating_readable;
        if let Some(text) = header.png_text {
            apply_png_text(&text, details);
        }
    }
    if let Some(exif) = read_exif(head) {
        apply_exif(&exif, details);
    }
    if details.rating.is_none() {
        details.rating = xmp_rating(head);
    }
}

/// What a picture's header gives.
#[derive(Debug, Default)]
struct Header {
    width: u32,
    height: u32,
    bit_depth: Option<u32>,
    /// Horizontal and vertical dots per inch.
    resolution: Option<(f64, f64)>,
    /// The format keeps a rating Windows and Linux tools write (EXIF or
    /// XMP), so an unrated picture shows five empty stars.
    rating_readable: bool,
    /// A PNG's `tEXt` and `iTXt` chunks before its pixels.
    png_text: Option<Vec<(String, String)>>,
}

fn header(head: &[u8]) -> Option<Header> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_header(head)
    } else if head.starts_with(&[0xff, 0xd8]) {
        jpeg_header(head)
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        let width = u32::from(u16::from_le_bytes([*head.get(6)?, *head.get(7)?]));
        let height = u32::from(u16::from_le_bytes([*head.get(8)?, *head.get(9)?]));
        Some(Header {
            width,
            height,
            bit_depth: Some(8),
            ..Header::default()
        })
    } else if head.starts_with(b"BM") {
        bmp_header(head)
    } else {
        None
    }
}

/// `IHDR` gives the size and bit depth; `pHYs` the resolution; the text
/// chunks before `IDAT` the title, author and the rest.
fn png_header(head: &[u8]) -> Option<Header> {
    let mut header = Header {
        rating_readable: true,
        ..Header::default()
    };
    let mut texts = Vec::new();
    let mut at = 8;
    while let Some(length) = read_u32_be(head, at) {
        let length = usize::try_from(length).ok()?;
        let kind = head.get(at + 4..at + 8)?;
        let Some(data) = at
            .checked_add(8)
            .and_then(|start| head.get(start..start.checked_add(length)?))
        else {
            break;
        };
        match kind {
            b"IHDR" if data.len() >= 10 => {
                header.width = read_u32_be(data, 0)?;
                header.height = read_u32_be(data, 4)?;
                let depth = u32::from(data[8]);
                let channels = match data[9] {
                    2 => 3,
                    4 => 2,
                    6 => 4,
                    _ => 1,
                };
                header.bit_depth = Some(depth * channels);
            }
            b"pHYs" if data.len() >= 9 && data[8] == 1 => {
                // Pixels per metre.
                let per_inch = |value: u32| f64::from(value) * 0.0254;
                header.resolution = Some((per_inch(read_u32_be(data, 0)?), per_inch(read_u32_be(data, 4)?)));
            }
            b"tEXt" => {
                if let Some(split) = data.iter().position(|byte| *byte == 0) {
                    texts.push((latin1(&data[..split]), latin1(&data[split + 1..])));
                }
            }
            b"iTXt" => {
                if let Some(text) = international_text(data) {
                    texts.push(text);
                }
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        // Length, type, data and CRC.
        at = at.checked_add(12)?.checked_add(length)?;
    }
    header.png_text = Some(texts);
    (header.width > 0).then_some(header)
}

/// An uncompressed `iTXt` chunk: keyword, flags, language, translated
/// keyword and UTF-8 text.
fn international_text(data: &[u8]) -> Option<(String, String)> {
    let keyword_end = data.iter().position(|byte| *byte == 0)?;
    let keyword = latin1(&data[..keyword_end]);
    let compressed = *data.get(keyword_end + 1)?;
    if compressed != 0 {
        return None;
    }
    let rest = data.get(keyword_end + 3..)?;
    let language_end = rest.iter().position(|byte| *byte == 0)?;
    let rest = rest.get(language_end + 1..)?;
    let translated_end = rest.iter().position(|byte| *byte == 0)?;
    let text = rest.get(translated_end + 1..)?;
    Some((keyword, clip(&String::from_utf8_lossy(text))))
}

/// A JPEG's frame header (`SOFn`) gives the size and bit depth; a JFIF
/// header the resolution, unless EXIF gives it.
fn jpeg_header(head: &[u8]) -> Option<Header> {
    let mut header = Header {
        rating_readable: true,
        ..Header::default()
    };
    let mut at = 2;
    loop {
        // Fill bytes before a marker.
        while head.get(at) == Some(&0xff) && head.get(at + 1) == Some(&0xff) {
            at += 1;
        }
        if *head.get(at)? != 0xff {
            return None;
        }
        let marker = *head.get(at + 1)?;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        let length = usize::from(u16::from_be_bytes([*head.get(at + 2)?, *head.get(at + 3)?]));
        if length < 2 {
            return None;
        }
        let data = head.get(at + 4..at + 2 + length)?;
        let is_frame = matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
        if is_frame && data.len() >= 6 {
            let precision = u32::from(data[0]);
            header.height = u32::from(u16::from_be_bytes([data[1], data[2]]));
            header.width = u32::from(u16::from_be_bytes([data[3], data[4]]));
            header.bit_depth = Some(precision * u32::from(data[5]));
            break;
        }
        if marker == 0xe0 && data.starts_with(b"JFIF\0") && data.len() >= 12 {
            let horizontal = f64::from(u16::from_be_bytes([data[8], data[9]]));
            let vertical = f64::from(u16::from_be_bytes([data[10], data[11]]));
            header.resolution = match data[7] {
                1 => Some((horizontal, vertical)),
                2 => Some((horizontal * 2.54, vertical * 2.54)),
                _ => None,
            };
        }
        at += 2 + length;
    }
    (header.width > 0).then_some(header)
}

/// A Windows bitmap's info header.
fn bmp_header(head: &[u8]) -> Option<Header> {
    let width = i32::from_le_bytes(head.get(18..22)?.try_into().ok()?);
    let height = i32::from_le_bytes(head.get(22..26)?.try_into().ok()?);
    let depth = u16::from_le_bytes(head.get(28..30)?.try_into().ok()?);
    let per_metre = |range: std::ops::Range<usize>| {
        let value = i32::from_le_bytes(head.get(range)?.try_into().ok()?);
        (value > 0).then(|| f64::from(value) * 0.0254)
    };
    let resolution = per_metre(38..42).zip(per_metre(42..46));
    Some(Header {
        width: width.unsigned_abs(),
        height: height.unsigned_abs(),
        bit_depth: Some(u32::from(depth)),
        resolution,
        ..Header::default()
    })
    .filter(|header| header.width > 0)
}

/// The picture's EXIF, from the bytes read.
fn read_exif(head: &[u8]) -> Option<exif::Exif> {
    let mut cursor = std::io::Cursor::new(head);
    exif::Reader::new().read_from_container(&mut cursor).ok()
}

fn apply_exif(exif: &exif::Exif, details: &mut MediaDetails) {
    let text = |tag: Tag| ascii(exif, tag);
    let xp = |number: u16| xp_text(exif, number);
    details.rating_readable = true;
    details.title = xp(XP_TITLE)
        .or_else(|| text(Tag::ImageDescription))
        .or(details.title.take());
    details.subject = xp(XP_SUBJECT).or(details.subject.take());
    details.tags = xp(XP_KEYWORDS).or(details.tags.take());
    details.comments = xp(XP_COMMENT)
        .or_else(|| user_comment(exif))
        .or(details.comments.take());
    details.authors = xp(XP_AUTHOR)
        .or_else(|| text(Tag::Artist))
        .or(details.authors.take());
    details.copyright = text(Tag::Copyright).or(details.copyright.take());
    details.program_name = text(Tag::Software).or(details.program_name.take());
    details.date_taken = text(Tag::DateTimeOriginal)
        .or_else(|| text(Tag::DateTimeDigitized))
        .as_deref()
        .and_then(NaiveDateTime::parse_exif);
    if let Some(rating) = uint(exif, Tag(exif::Context::Tiff, RATING)) {
        details.rating = Some(u8::try_from(rating.min(5)).unwrap_or(5));
    }
    if details.width.is_none() {
        details.width = uint(exif, Tag::PixelXDimension).or_else(|| uint(exif, Tag::ImageWidth));
        details.height = uint(exif, Tag::PixelYDimension).or_else(|| uint(exif, Tag::ImageLength));
    }
    if let Some(resolution) = exif_resolution(exif) {
        details.resolution = Some(resolution);
    }
    details.color_space = uint(exif, Tag::ColorSpace).and_then(|value| u16::try_from(value).ok());
    details.camera_maker = text(Tag::Make);
    details.camera_model = text(Tag::Model);
    details.f_number = rational(exif, Tag::FNumber);
    details.exposure_time = rational_parts(exif, Tag::ExposureTime);
    details.iso = uint(exif, Tag::PhotographicSensitivity);
    details.exposure_bias = signed_rational(exif, Tag::ExposureBiasValue);
    details.focal_length = rational(exif, Tag::FocalLength);
    details.metering_mode = uint(exif, Tag::MeteringMode).and_then(|value| u16::try_from(value).ok());
    details.flash = uint(exif, Tag::Flash).and_then(|value| u16::try_from(value).ok());
    details.focal_length_35mm = uint(exif, Tag::FocalLengthIn35mmFilm).filter(|value| *value > 0);
    details.lens_maker = text(Tag::LensMake);
    details.lens_model = text(Tag::LensModel);
    details.exposure_program = uint(exif, Tag::ExposureProgram).and_then(|value| u16::try_from(value).ok());
    details.white_balance = uint(exif, Tag::WhiteBalance).and_then(|value| u16::try_from(value).ok());
    details.exif_version =
        exif.get_field(Tag::ExifVersion, In::PRIMARY)
            .and_then(|field| match &field.value {
                Value::Undefined(bytes, _) if bytes.len() == 4 && bytes.iter().all(u8::is_ascii_digit) => {
                    Some(String::from_utf8_lossy(bytes).into_owned())
                }
                _ => None,
            });
}

/// The resolution EXIF gives, in dots per inch.
fn exif_resolution(exif: &exif::Exif) -> Option<(f64, f64)> {
    let horizontal = rational(exif, Tag::XResolution)?;
    let vertical = rational(exif, Tag::YResolution)?;
    let per_inch = match uint(exif, Tag::ResolutionUnit) {
        Some(3) => 2.54,
        _ => 1.0,
    };
    (horizontal > 0.0 && vertical > 0.0).then_some((horizontal * per_inch, vertical * per_inch))
}

/// A PNG's text chunks, under the keywords the PNG specification names.
fn apply_png_text(texts: &[(String, String)], details: &mut MediaDetails) {
    for (keyword, value) in texts {
        let value = clip(value.trim());
        if value.is_empty() {
            continue;
        }
        let slot = match keyword.as_str() {
            "Title" => &mut details.title,
            "Author" => &mut details.authors,
            "Description" | "Comment" => &mut details.comments,
            "Copyright" => &mut details.copyright,
            "Software" => &mut details.program_name,
            _ => continue,
        };
        if slot.is_none() {
            *slot = Some(value);
        }
    }
}

/// `xmp:Rating` in an XMP packet among the bytes read, written as an
/// attribute (`xmp:Rating="3"`) or an element (`<xmp:Rating>3<`).
fn xmp_rating(head: &[u8]) -> Option<u8> {
    const NAME: &[u8] = b"xmp:Rating";
    let mut from = 0;
    while let Some(found) = find(&head[from..], NAME) {
        let after = from + found + NAME.len();
        let rest = head.get(after..)?;
        let digits = match rest.first() {
            Some(b'=') => rest.get(2..),
            Some(b'>') => rest.get(1..),
            _ => None,
        };
        if let Some(digit) = digits
            .and_then(|digits| digits.first())
            .filter(|byte| byte.is_ascii_digit())
        {
            return Some((digit - b'0').min(5));
        }
        from = after;
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn ascii(exif: &exif::Exif, tag: Tag) -> Option<String> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    let Value::Ascii(parts) = &field.value else {
        return None;
    };
    let text = parts
        .iter()
        .map(|part| String::from_utf8_lossy(part).trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    (!text.is_empty()).then(|| clip(&text))
}

/// A Windows XP tag: UTF-16LE bytes ending in a null character.
fn xp_text(exif: &exif::Exif, number: u16) -> Option<String> {
    let field = exif.get_field(Tag(exif::Context::Tiff, number), In::PRIMARY)?;
    let Value::Byte(bytes) = &field.value else {
        return None;
    };
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    let text = String::from_utf16_lossy(&units).trim().to_owned();
    (!text.is_empty()).then(|| clip(&text))
}

/// EXIF's user comment: eight bytes naming the character set, then the text.
fn user_comment(exif: &exif::Exif) -> Option<String> {
    let field = exif.get_field(Tag::UserComment, In::PRIMARY)?;
    let Value::Undefined(bytes, _) = &field.value else {
        return None;
    };
    let (charset, text) = (bytes.get(..8)?, bytes.get(8..)?);
    let text = match charset {
        b"ASCII\0\0\0" => String::from_utf8_lossy(text).into_owned(),
        b"UNICODE\0" => {
            let units: Vec<u16> = text
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => return None,
    };
    let text = text.trim_matches(|character: char| character == '\0' || character.is_whitespace());
    (!text.is_empty()).then(|| clip(text))
}

fn uint(exif: &exif::Exif, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY)?.value.get_uint(0)
}

/// A fraction field (an exposure time): as stored when it is one, else
/// made from its number. Some writers store a camera's values as signed
/// rationals or doubles.
fn rational_parts(exif: &exif::Exif, tag: Tag) -> Option<(u32, u32)> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Rational(values) => values
            .first()
            .map(|value| (value.num, value.denom))
            .filter(|(_, denom)| *denom > 0),
        Value::SRational(values) => values.first().and_then(|value| {
            Some((u32::try_from(value.num).ok()?, u32::try_from(value.denom).ok()?))
                .filter(|(_, denom)| *denom > 0)
        }),
        _ => {
            let value = number(exif, tag)?;
            if !(value > 0.0 && value.is_finite()) {
                None
            } else if value < 1.0 {
                Some((1, rounded(1.0 / value)?))
            } else {
                Some((rounded(value * 10.0)?, 10))
            }
        }
    }
}

/// A number field, whichever numeric type it was written with.
fn number(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    let value = match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Rational(values) => values
            .first()
            .filter(|value| value.denom != 0)
            .map(|value| f64::from(value.num) / f64::from(value.denom)),
        Value::SRational(values) => values
            .first()
            .filter(|value| value.denom != 0)
            .map(|value| f64::from(value.num) / f64::from(value.denom)),
        Value::Double(values) => values.first().copied(),
        Value::Float(values) => values.first().map(|value| f64::from(*value)),
        Value::Short(values) => values.first().map(|value| f64::from(*value)),
        Value::Long(values) => values.first().map(|value| f64::from(*value)),
        _ => None,
    }?;
    value.is_finite().then_some(value)
}

/// `value` rounded to a whole number that fits a `u32`.
fn rounded(value: f64) -> Option<u32> {
    let value = value.round();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to be within u32 first"
    )]
    (value >= 1.0 && value <= f64::from(u32::MAX)).then_some(value as u32)
}

fn rational(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    number(exif, tag)
}

fn signed_rational(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    number(exif, tag)
}

fn read_u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn latin1(bytes: &[u8]) -> String {
    clip(&bytes.iter().map(|byte| char::from(*byte)).collect::<String>())
}

/// `text`, cut to [`TEXT_LIMIT`] characters.
pub(super) fn clip(text: &str) -> String {
    text.chars().take(TEXT_LIMIT).collect()
}
