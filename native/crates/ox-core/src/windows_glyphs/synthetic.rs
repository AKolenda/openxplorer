// SPDX-License-Identifier: AGPL-3.0-only
//! Tiny synthetic TrueType fonts, for tests and pictures of the app.
//!
//! A font made here has the tables the glyph reader needs (`head`,
//! `hhea`, `maxp`, `hmtx`, `cmap`, `loca` and `glyf`) and one glyph for
//! each codepoint the caller names: a test shape drawn from straight
//! lines and quadratic curves, or a composite glyph that places another
//! one. Nothing is copied from any real font.

/// The em square of a synthetic font.
pub const UNITS_PER_EM: u16 = 1024;

/// One glyph of a synthetic font.
#[derive(Debug, Clone)]
pub enum SyntheticGlyph {
    /// Outlines: contours of points `(x, y, on_curve)`.
    Simple(Vec<Vec<(i16, i16, bool)>>),
    /// Another glyph of the font, by its index, moved by `(dx, dy)`.
    Composite {
        /// The glyph placed.
        glyph: u16,
        /// How far it is moved.
        offset: (i16, i16),
    },
}

impl SyntheticGlyph {
    /// A test shape for `codepoint`: a square frame with a diagonal bar
    /// and a curved corner, its size chosen from the codepoint so glyphs
    /// differ.
    ///
    /// # Panics
    ///
    /// Never: the inset is at most 176 units.
    pub fn test_shape(codepoint: u32) -> Self {
        let inset = i16::try_from(64 + (codepoint % 8) * 16).expect("a small inset");
        let far = 1024 - inset;
        let outer = vec![
            (inset, 0, true),
            (far, 0, true),
            (far, 640, true),
            (far, 1024 - inset, false),
            (640, 1024 - inset, true),
            (inset, 1024 - inset, true),
        ];
        let bar = vec![
            (inset + 64, 64, true),
            (inset + 160, 64, true),
            (far - 64, 900, true),
        ];
        SyntheticGlyph::Simple(vec![outer, bar])
    }
}

/// A synthetic font: glyphs and the codepoints that name them.
#[derive(Debug, Clone, Default)]
pub struct SyntheticFont {
    /// Glyphs after the empty glyph 0, in order (glyph 1 first).
    pub glyphs: Vec<SyntheticGlyph>,
    /// Codepoints and the glyph index each names.
    pub mapping: Vec<(u32, u16)>,
    /// Write the character map as format 12 rather than format 4.
    pub format_12: bool,
}

impl SyntheticFont {
    /// A font with a test shape at each of `codepoints`.
    ///
    /// # Panics
    ///
    /// With more glyphs than a font holds, which a test never asks for.
    pub fn with_codepoints(codepoints: &[u32]) -> Self {
        let mut font = Self::default();
        for &codepoint in codepoints {
            font.glyphs.push(SyntheticGlyph::test_shape(codepoint));
            let index = u16::try_from(font.glyphs.len()).expect("a few glyphs");
            font.mapping.push((codepoint, index));
        }
        font
    }

    /// The font file's bytes.
    ///
    /// # Panics
    ///
    /// If the font is too large for its 16-bit fields, which a test's few
    /// glyphs never are.
    pub fn build(&self) -> Vec<u8> {
        let glyph_count = u16::try_from(self.glyphs.len() + 1).expect("a few glyphs");
        let mut glyf = Vec::new();
        let mut loca: Vec<u32> = vec![0];
        // Glyph 0, .notdef, is empty.
        loca.push(0);
        for glyph in &self.glyphs {
            glyf.extend(glyph_bytes(glyph));
            glyf.resize(glyf.len().next_multiple_of(4), 0);
            loca.push(small(glyf.len()));
        }
        let loca_bytes: Vec<u8> = loca.iter().flat_map(|offset| offset.to_be_bytes()).collect();
        let tables: Vec<([u8; 4], Vec<u8>)> = vec![
            (*b"cmap", self.cmap()),
            (*b"glyf", glyf),
            (*b"head", head()),
            (*b"hhea", hhea(glyph_count)),
            (*b"hmtx", hmtx(glyph_count)),
            (*b"loca", loca_bytes),
            (*b"maxp", maxp(glyph_count)),
        ];
        let count = u16::try_from(tables.len()).expect("seven tables");
        let mut font = Vec::new();
        font.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        font.extend_from_slice(&count.to_be_bytes());
        font.extend_from_slice(&[0; 6]);
        let mut offset = 12 + tables.len() * 16;
        let mut bodies = Vec::new();
        for (tag, body) in &tables {
            font.extend_from_slice(tag);
            font.extend_from_slice(&0u32.to_be_bytes());
            font.extend_from_slice(&small(offset).to_be_bytes());
            font.extend_from_slice(&small(body.len()).to_be_bytes());
            let padded = body.len().next_multiple_of(4);
            bodies.extend_from_slice(body);
            bodies.resize(bodies.len() + padded - body.len(), 0);
            offset += padded;
        }
        font.extend(bodies);
        font
    }

    /// A `cmap` with one Windows Unicode subtable of format 4 or 12.
    fn cmap(&self) -> Vec<u8> {
        let mut mapping = self.mapping.clone();
        mapping.sort_unstable();
        let (encoding, subtable) = if self.format_12 {
            (10u16, format_12(&mapping))
        } else {
            (1u16, format_4(&mapping))
        };
        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&1u16.to_be_bytes());
        cmap.extend_from_slice(&3u16.to_be_bytes());
        cmap.extend_from_slice(&encoding.to_be_bytes());
        cmap.extend_from_slice(&12u32.to_be_bytes());
        cmap.extend(subtable);
        cmap
    }
}

/// `value` as a 32-bit field of a synthetic font, which is small.
fn small(value: usize) -> u32 {
    u32::try_from(value).expect("a synthetic font is small")
}

/// A format 4 subtable: one segment per codepoint, then the final one.
fn format_4(mapping: &[(u32, u16)]) -> Vec<u8> {
    let mut segments: Vec<(u16, u16, i16)> = mapping
        .iter()
        .map(|&(codepoint, glyph)| {
            let code = u16::try_from(codepoint).expect("format 4 holds the first plane");
            #[expect(clippy::cast_possible_wrap, reason = "idDelta is taken modulo 65536")]
            let delta = glyph.wrapping_sub(code) as i16;
            (code, code, delta)
        })
        .collect();
    segments.push((0xffff, 0xffff, 1));
    let count = u16::try_from(segments.len()).expect("a few segments");
    let mut table = Vec::new();
    table.extend_from_slice(&4u16.to_be_bytes());
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&(count * 2).to_be_bytes());
    table.extend_from_slice(&[0; 6]);
    for (_, end, _) in &segments {
        table.extend_from_slice(&end.to_be_bytes());
    }
    table.extend_from_slice(&0u16.to_be_bytes());
    for (start, _, _) in &segments {
        table.extend_from_slice(&start.to_be_bytes());
    }
    for (_, _, delta) in &segments {
        table.extend_from_slice(&delta.to_be_bytes());
    }
    for _ in &segments {
        table.extend_from_slice(&0u16.to_be_bytes());
    }
    let length = u16::try_from(table.len()).expect("a small table");
    table[2..4].copy_from_slice(&length.to_be_bytes());
    table
}

/// A format 12 subtable: one group per codepoint.
fn format_12(mapping: &[(u32, u16)]) -> Vec<u8> {
    let mut table = Vec::new();
    table.extend_from_slice(&12u16.to_be_bytes());
    table.extend_from_slice(&0u16.to_be_bytes());
    let length = small(16 + mapping.len() * 12);
    table.extend_from_slice(&length.to_be_bytes());
    table.extend_from_slice(&0u32.to_be_bytes());
    table.extend_from_slice(&small(mapping.len()).to_be_bytes());
    for &(codepoint, glyph) in mapping {
        table.extend_from_slice(&codepoint.to_be_bytes());
        table.extend_from_slice(&codepoint.to_be_bytes());
        table.extend_from_slice(&u32::from(glyph).to_be_bytes());
    }
    table
}

/// The bytes of one glyph in `glyf`.
fn glyph_bytes(glyph: &SyntheticGlyph) -> Vec<u8> {
    let mut bytes = Vec::new();
    match glyph {
        SyntheticGlyph::Simple(contours) => {
            let count = i16::try_from(contours.len()).expect("a few contours");
            bytes.extend_from_slice(&count.to_be_bytes());
            bytes.extend_from_slice(&[0, 0, 0, 0, 4, 0, 4, 0]);
            let mut end = 0u16;
            for contour in contours {
                end += u16::try_from(contour.len()).expect("a few points");
                bytes.extend_from_slice(&(end - 1).to_be_bytes());
            }
            bytes.extend_from_slice(&0u16.to_be_bytes());
            let points: Vec<&(i16, i16, bool)> = contours.iter().flatten().collect();
            for (_, _, on) in &points {
                bytes.push(u8::from(*on));
            }
            let (mut x, mut y) = (0i16, 0i16);
            for (px, _, _) in &points {
                bytes.extend_from_slice(&(px - x).to_be_bytes());
                x = *px;
            }
            for (_, py, _) in &points {
                bytes.extend_from_slice(&(py - y).to_be_bytes());
                y = *py;
            }
        }
        SyntheticGlyph::Composite { glyph, offset } => {
            bytes.extend_from_slice(&(-1i16).to_be_bytes());
            bytes.extend_from_slice(&[0; 8]);
            // ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES.
            bytes.extend_from_slice(&0x0003u16.to_be_bytes());
            bytes.extend_from_slice(&glyph.to_be_bytes());
            bytes.extend_from_slice(&offset.0.to_be_bytes());
            bytes.extend_from_slice(&offset.1.to_be_bytes());
        }
    }
    bytes
}

/// `head`: 1024 units per em and long `loca` offsets.
fn head() -> Vec<u8> {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&UNITS_PER_EM.to_be_bytes());
    head[50..52].copy_from_slice(&1i16.to_be_bytes());
    head
}

/// `hhea`: an ascender of the em and no descender, one metric a glyph.
fn hhea(glyph_count: u16) -> Vec<u8> {
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&UNITS_PER_EM.to_be_bytes());
    hhea[34..36].copy_from_slice(&glyph_count.to_be_bytes());
    hhea
}

/// `hmtx`: every glyph one em wide.
fn hmtx(glyph_count: u16) -> Vec<u8> {
    (0..glyph_count)
        .flat_map(|_| {
            let mut metric = UNITS_PER_EM.to_be_bytes().to_vec();
            metric.extend_from_slice(&[0, 0]);
            metric
        })
        .collect()
}

/// `maxp`, version 0.5: the glyph count.
fn maxp(glyph_count: u16) -> Vec<u8> {
    let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
    maxp.extend_from_slice(&glyph_count.to_be_bytes());
    maxp
}
