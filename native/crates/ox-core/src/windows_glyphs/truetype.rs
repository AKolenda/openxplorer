// SPDX-License-Identifier: AGPL-3.0-only
//! Glyph outlines of a TrueType font, as SVG paths.
//!
//! Only what drawing one glyph needs is read: the table directory, `head`
//! (units per em and the `loca` format), `hhea` (ascender, descender and
//! the count of metrics), `maxp` (the glyph count), `hmtx` (advance
//! widths), a Unicode `cmap` subtable of format 4 or 12, `loca` and
//! `glyf`, whose simple and composite glyphs are read. Every offset is
//! checked against its table and every count is capped, so a damaged or
//! hostile file gives an error or no glyph, never a panic. Fonts with
//! PostScript outlines (`OTTO`) are not read.

use std::fmt::Write as _;

/// The largest font read.
pub(super) const MAX_FONT_SIZE: usize = 32 << 20;
/// The most tables a font may list.
const MAX_TABLES: usize = 64;
/// The most points a glyph may have, its components' included.
const MAX_POINTS: usize = 16_384;
/// The most contours one simple glyph may have.
const MAX_CONTOURS: usize = 1024;
/// The most components a composite glyph may have, nested ones included.
const MAX_COMPONENTS: usize = 64;
/// How deeply composite glyphs may nest.
const MAX_DEPTH: usize = 8;

/// Why a font gives no glyphs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FontError {
    /// The file is larger than the reader takes.
    #[error("the font is too large")]
    TooLarge,
    /// The file is not a TrueType font, or has PostScript outlines.
    #[error("the file is not a TrueType font")]
    NotTrueType,
    /// A table is missing, short or points outside the file.
    #[error("the font is damaged")]
    Damaged,
    /// A count is over its cap.
    #[error("the font holds more than OpenXplorer reads")]
    OverLimit,
}

/// A big-endian `u16` at `at`.
fn u16_at(bytes: &[u8], at: usize) -> Result<u16, FontError> {
    let field = bytes
        .get(at..at.checked_add(2).ok_or(FontError::Damaged)?)
        .ok_or(FontError::Damaged)?;
    Ok(u16::from_be_bytes([field[0], field[1]]))
}

/// A big-endian `i16` at `at`.
fn i16_at(bytes: &[u8], at: usize) -> Result<i16, FontError> {
    u16_at(bytes, at).map(|value| i16::from_be_bytes(value.to_be_bytes()))
}

/// A big-endian `u32` at `at`.
fn u32_at(bytes: &[u8], at: usize) -> Result<u32, FontError> {
    let field = bytes
        .get(at..at.checked_add(4).ok_or(FontError::Damaged)?)
        .ok_or(FontError::Damaged)?;
    Ok(u32::from_be_bytes([field[0], field[1], field[2], field[3]]))
}

/// One point of an outline, in font units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Point {
    x: f32,
    y: f32,
    on_curve: bool,
}

/// A TrueType font, ready to give glyph outlines by codepoint.
pub struct Font {
    bytes: Vec<u8>,
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    long_offsets: bool,
    glyph_count: u16,
    metric_count: u16,
    cmap: (usize, usize),
    loca: (usize, usize),
    glyf: (usize, usize),
    hmtx: (usize, usize),
}

impl Font {
    /// Reads the tables of the font in `bytes`.
    ///
    /// # Errors
    ///
    /// Any [`FontError`]: too large, not TrueType, damaged or over a cap.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, FontError> {
        if bytes.len() > MAX_FONT_SIZE {
            return Err(FontError::TooLarge);
        }
        let version = u32_at(&bytes, 0).map_err(|_| FontError::NotTrueType)?;
        if version != 0x0001_0000 && version != u32::from_be_bytes(*b"true") {
            return Err(FontError::NotTrueType);
        }
        let table_count = usize::from(u16_at(&bytes, 4)?);
        if table_count > MAX_TABLES {
            return Err(FontError::OverLimit);
        }
        let table = |tag: &[u8; 4]| -> Result<(usize, usize), FontError> {
            for index in 0..table_count {
                let record = 12 + index * 16;
                if bytes.get(record..record + 4) == Some(tag.as_slice()) {
                    let offset = u32_at(&bytes, record + 8)? as usize;
                    let length = u32_at(&bytes, record + 12)? as usize;
                    let end = offset.checked_add(length).ok_or(FontError::Damaged)?;
                    if end > bytes.len() {
                        return Err(FontError::Damaged);
                    }
                    return Ok((offset, length));
                }
            }
            Err(FontError::Damaged)
        };
        let head = table(b"head")?;
        let hhea = table(b"hhea")?;
        let maxp = table(b"maxp")?;
        let cmap = table(b"cmap")?;
        let loca = table(b"loca")?;
        let glyf = table(b"glyf")?;
        let hmtx = table(b"hmtx")?;
        let head_bytes = bytes.get(head.0..head.0 + head.1).ok_or(FontError::Damaged)?;
        let units_per_em = u16_at(head_bytes, 18)?;
        if !(16..=16_384).contains(&units_per_em) {
            return Err(FontError::Damaged);
        }
        let long_offsets = match i16_at(head_bytes, 50)? {
            0 => false,
            1 => true,
            _ => return Err(FontError::Damaged),
        };
        let hhea_bytes = bytes.get(hhea.0..hhea.0 + hhea.1).ok_or(FontError::Damaged)?;
        let maxp_bytes = bytes.get(maxp.0..maxp.0 + maxp.1).ok_or(FontError::Damaged)?;
        let font = Self {
            units_per_em,
            ascender: i16_at(hhea_bytes, 4)?,
            descender: i16_at(hhea_bytes, 6)?,
            metric_count: u16_at(hhea_bytes, 34)?,
            glyph_count: u16_at(maxp_bytes, 4)?,
            long_offsets,
            cmap,
            loca,
            glyf,
            hmtx,
            bytes,
        };
        if font.metric_count == 0 || font.ascender <= font.descender {
            return Err(FontError::Damaged);
        }
        Ok(font)
    }

    /// The bytes of the table at `range`.
    fn table(&self, (offset, length): (usize, usize)) -> &[u8] {
        // Checked when the font was read.
        &self.bytes[offset..offset + length]
    }

    /// The glyph `codepoint` maps to, `None` when the font has none.
    pub fn glyph_index(&self, codepoint: u32) -> Option<u16> {
        let cmap = self.table(self.cmap);
        let count = usize::from(u16_at(cmap, 2).ok()?);
        // Unicode subtables, the full repertoire (format 12) first.
        let mut best: Option<(u8, usize)> = None;
        for index in 0..count.min(64) {
            let record = 4 + index * 8;
            let platform = u16_at(cmap, record).ok()?;
            let encoding = u16_at(cmap, record + 2).ok()?;
            let offset = u32_at(cmap, record + 4).ok()? as usize;
            let rank = match (platform, encoding) {
                (3, 10) | (0, 4 | 6) => 3,
                (3, 1) | (0, 0..=3) => 2,
                (3, 0) => 1,
                _ => continue,
            };
            if best.is_none_or(|(known, _)| rank > known) {
                best = Some((rank, offset));
            }
        }
        let (_, offset) = best?;
        let subtable = cmap.get(offset..)?;
        let glyph = match u16_at(subtable, 0).ok()? {
            4 => format_4(subtable, codepoint),
            12 => format_12(subtable, codepoint),
            _ => None,
        }?;
        (glyph != 0 && glyph < self.glyph_count).then_some(glyph)
    }

    /// The bytes of glyph `glyph` in `glyf`, empty for a glyph without
    /// outlines.
    fn glyph_bytes(&self, glyph: u16) -> Result<&[u8], FontError> {
        if glyph >= self.glyph_count {
            return Err(FontError::Damaged);
        }
        let loca = self.table(self.loca);
        let index = usize::from(glyph);
        let (start, end) = if self.long_offsets {
            (
                u32_at(loca, index * 4)? as usize,
                u32_at(loca, index * 4 + 4)? as usize,
            )
        } else {
            (
                usize::from(u16_at(loca, index * 2)?) * 2,
                usize::from(u16_at(loca, index * 2 + 2)?) * 2,
            )
        };
        if start > end {
            return Err(FontError::Damaged);
        }
        self.table(self.glyf).get(start..end).ok_or(FontError::Damaged)
    }

    /// The contours of glyph `glyph`, its components placed.
    fn contours(&self, glyph: u16, depth: usize, budget: &mut Budget) -> Result<Vec<Vec<Point>>, FontError> {
        if depth > MAX_DEPTH {
            return Err(FontError::OverLimit);
        }
        let data = self.glyph_bytes(glyph)?;
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let contour_count = i16_at(data, 0)?;
        if contour_count >= 0 {
            return simple_contours(data, usize::try_from(contour_count).unwrap_or(0), budget);
        }
        let mut contours = Vec::new();
        let mut at = 10;
        loop {
            budget.components += 1;
            if budget.components > MAX_COMPONENTS {
                return Err(FontError::OverLimit);
            }
            let flags = u16_at(data, at)?;
            let component = u16_at(data, at + 2)?;
            at += 4;
            let (dx, dy) = if flags & 0x0001 != 0 {
                let pair = (f32::from(i16_at(data, at)?), f32::from(i16_at(data, at + 2)?));
                at += 4;
                pair
            } else {
                let x = i8::from_be_bytes([*data.get(at).ok_or(FontError::Damaged)?]);
                let y = i8::from_be_bytes([*data.get(at + 1).ok_or(FontError::Damaged)?]);
                at += 2;
                (f32::from(x), f32::from(y))
            };
            let f2dot14 = |at: usize| i16_at(data, at).map(|value| f32::from(value) / 16_384.0);
            let (a, b, c, d) = if flags & 0x0008 != 0 {
                let scale = f2dot14(at)?;
                at += 2;
                (scale, 0.0, 0.0, scale)
            } else if flags & 0x0040 != 0 {
                let (x, y) = (f2dot14(at)?, f2dot14(at + 2)?);
                at += 4;
                (x, 0.0, 0.0, y)
            } else if flags & 0x0080 != 0 {
                let matrix = (f2dot14(at)?, f2dot14(at + 2)?, f2dot14(at + 4)?, f2dot14(at + 6)?);
                at += 8;
                matrix
            } else {
                (1.0, 0.0, 0.0, 1.0)
            };
            // Components placed by matching points are left out.
            if flags & 0x0002 != 0 {
                for contour in self.contours(component, depth + 1, budget)? {
                    let placed = contour
                        .into_iter()
                        .map(|point| Point {
                            x: a * point.x + c * point.y + dx,
                            y: b * point.x + d * point.y + dy,
                            on_curve: point.on_curve,
                        })
                        .collect();
                    contours.push(placed);
                }
            }
            if flags & 0x0020 == 0 {
                break;
            }
        }
        Ok(contours)
    }

    /// Glyph `codepoint` as an SVG file `size` pixels square whose em is
    /// `em` pixels: the font's units per em are mapped to `em`, as a
    /// renderer draws the glyph at that font size, so the glyph keeps the
    /// font's own side bearings and baseline and is never stretched to its
    /// box. The em is centred in the square, horizontally by the glyph's
    /// advance and vertically by the font's ascender and descender, and the
    /// outlines are filled with the nonzero rule. `None` when the font has
    /// no outlines for it.
    ///
    /// # Errors
    ///
    /// [`FontError::Damaged`] or [`FontError::OverLimit`] for a glyph
    /// that is.
    pub fn glyph_svg(&self, codepoint: u32, em: u32, size: u32) -> Result<Option<String>, FontError> {
        let Some(glyph) = self.glyph_index(codepoint) else {
            return Ok(None);
        };
        let contours = self.contours(glyph, 0, &mut Budget::default())?;
        if contours.iter().all(Vec::is_empty) {
            return Ok(None);
        }
        let hmtx = self.table(self.hmtx);
        let metric = usize::from(glyph.min(self.metric_count - 1));
        let advance = f32::from(u16_at(hmtx, metric * 4)?);
        let units_per_em = f32::from(self.units_per_em);
        let size = size.max(1);
        let em = em.clamp(1, size);
        #[expect(clippy::cast_precision_loss, reason = "pixel sizes are small")]
        let (side, margin) = (
            units_per_em * size as f32 / em as f32,
            units_per_em * ((size - em) / 2) as f32 / em as f32,
        );
        // The em sits in the middle of the square, moved up and left to a
        // whole pixel so its edges are not blurred across two (a 16-pixel
        // em in a 17-pixel square starts 0 pixels in, not half of one).
        // The advance sits in the middle of the em, and the em's top and
        // bottom are the middle of the line's ascender and descender (the
        // ascender itself when the line is one em high).
        let left = margin + (units_per_em - advance) / 2.0;
        let baseline =
            margin + units_per_em / 2.0 + f32::midpoint(f32::from(self.ascender), f32::from(self.descender));
        let mut path = String::new();
        for contour in &contours {
            let placed: Vec<Point> = contour
                .iter()
                .map(|point| Point {
                    x: point.x + left,
                    y: baseline - point.y,
                    on_curve: point.on_curve,
                })
                .collect();
            write_contour(&mut path, &placed);
        }
        Ok(Some(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\" \
             viewBox=\"0 0 {side} {side}\"><path fill=\"#000000\" d=\"{path}\"/></svg>\n"
        )))
    }
}

/// How much of the caps one glyph has used.
#[derive(Debug, Default)]
struct Budget {
    points: usize,
    components: usize,
}

/// The contours of a simple glyph with `count` contours.
fn simple_contours(data: &[u8], count: usize, budget: &mut Budget) -> Result<Vec<Vec<Point>>, FontError> {
    if count > MAX_CONTOURS {
        return Err(FontError::OverLimit);
    }
    let mut ends = Vec::with_capacity(count);
    for index in 0..count {
        ends.push(usize::from(u16_at(data, 10 + index * 2)?));
    }
    let point_count = match ends.last() {
        Some(last) => last + 1,
        None => return Ok(Vec::new()),
    };
    if ends.windows(2).any(|pair| pair[1] < pair[0]) {
        return Err(FontError::Damaged);
    }
    budget.points += point_count;
    if budget.points > MAX_POINTS {
        return Err(FontError::OverLimit);
    }
    let instructions = usize::from(u16_at(data, 10 + count * 2)?);
    let mut at = 12 + count * 2 + instructions;
    let mut flags = Vec::with_capacity(point_count);
    while flags.len() < point_count {
        let flag = *data.get(at).ok_or(FontError::Damaged)?;
        at += 1;
        flags.push(flag);
        if flag & 0x08 != 0 {
            let repeat = *data.get(at).ok_or(FontError::Damaged)?;
            at += 1;
            for _ in 0..repeat {
                flags.push(flag);
            }
        }
    }
    flags.truncate(point_count);
    let mut read_axis = |short: u8, same: u8| -> Result<Vec<f32>, FontError> {
        let mut value = 0i32;
        let mut values = Vec::with_capacity(point_count);
        for flag in &flags {
            if flag & short != 0 {
                let delta = i32::from(*data.get(at).ok_or(FontError::Damaged)?);
                at += 1;
                value += if flag & same != 0 { delta } else { -delta };
            } else if flag & same == 0 {
                value += i32::from(i16_at(data, at)?);
                at += 2;
            }
            #[expect(clippy::cast_precision_loss, reason = "font units fit in f32 exactly")]
            values.push(value as f32);
        }
        Ok(values)
    };
    let xs = read_axis(0x02, 0x10)?;
    let ys = read_axis(0x04, 0x20)?;
    let mut contours = Vec::with_capacity(count);
    let mut start = 0;
    for end in ends {
        let contour = (start..=end)
            .map(|index| Point {
                x: xs[index],
                y: ys[index],
                on_curve: flags[index] & 0x01 != 0,
            })
            .collect();
        contours.push(contour);
        start = end + 1;
    }
    Ok(contours)
}

/// Appends one closed contour of quadratic TrueType points to `path`.
fn write_contour(path: &mut String, points: &[Point]) {
    if points.is_empty() {
        return;
    }
    let midpoint = |a: Point, b: Point| Point {
        x: f32::midpoint(a.x, b.x),
        y: f32::midpoint(a.y, b.y),
        on_curve: true,
    };
    // Start on a point on the curve; between two off-curve points lies
    // an implied one.
    let first_on = points.iter().position(|point| point.on_curve);
    let (start, rotated): (Point, Vec<Point>) = if let Some(index) = first_on {
        let after = points[index + 1..].iter().chain(&points[..index]);
        (points[index], after.copied().collect())
    } else {
        (midpoint(points[points.len() - 1], points[0]), points.to_vec())
    };
    let _ = write!(path, "M{} {}", start.x, start.y);
    let mut control: Option<Point> = None;
    for point in rotated.into_iter().chain(std::iter::once(start)) {
        match (control, point.on_curve) {
            (None, true) => {
                let _ = write!(path, "L{} {}", point.x, point.y);
            }
            (None, false) => control = Some(point),
            (Some(off), true) => {
                let _ = write!(path, "Q{} {} {} {}", off.x, off.y, point.x, point.y);
                control = None;
            }
            (Some(off), false) => {
                let implied = midpoint(off, point);
                let _ = write!(path, "Q{} {} {} {}", off.x, off.y, implied.x, implied.y);
                control = Some(point);
            }
        }
    }
    path.push('Z');
}

/// The glyph format 4 maps `codepoint` to.
fn format_4(table: &[u8], codepoint: u32) -> Option<u16> {
    let code = u16::try_from(codepoint).ok()?;
    let segments = usize::from(u16_at(table, 6).ok()? / 2);
    let ends = 14;
    let starts = ends + segments * 2 + 2;
    let deltas = starts + segments * 2;
    let ranges = deltas + segments * 2;
    for segment in 0..segments.min(16_384) {
        let end = u16_at(table, ends + segment * 2).ok()?;
        if code > end {
            continue;
        }
        let start = u16_at(table, starts + segment * 2).ok()?;
        if code < start {
            return None;
        }
        let delta = u16_at(table, deltas + segment * 2).ok()?;
        let range_at = ranges + segment * 2;
        let range = usize::from(u16_at(table, range_at).ok()?);
        if range == 0 {
            return Some(code.wrapping_add(delta));
        }
        let glyph_at = range_at + range + usize::from(code - start) * 2;
        let glyph = u16_at(table, glyph_at).ok()?;
        return (glyph != 0).then(|| glyph.wrapping_add(delta));
    }
    None
}

/// The glyph format 12 maps `codepoint` to.
fn format_12(table: &[u8], codepoint: u32) -> Option<u16> {
    let groups = u32_at(table, 12).ok()? as usize;
    for group in 0..groups.min(1 << 16) {
        let at = 16 + group * 12;
        let start = u32_at(table, at).ok()?;
        let end = u32_at(table, at + 4).ok()?;
        if (start..=end).contains(&codepoint) {
            let first = u32_at(table, at + 8).ok()?;
            return u16::try_from(first.checked_add(codepoint - start)?).ok();
        }
    }
    None
}
