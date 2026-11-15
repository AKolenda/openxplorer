// SPDX-License-Identifier: AGPL-3.0-only
//! Gallery's justified rows (HOME-026): Windows 11 File Explorer's Gallery
//! lays its photos out as a photo grid does, each thumbnail keeping its
//! photo's aspect ratio, in rows of one height that fill the width, with
//! the same gap between tiles and between rows.
//!
//! [`justify`] places photos of known aspect ratios in rows near a target
//! height: it adds photos to a row until it is as wide as the view, then
//! ends the row with or without the last photo, whichever leaves its
//! height nearer the target, and scales the row to fill the width
//! exactly. The last row keeps the target height and starts at the left.
//! Positions are whole pixels; a row's rounding goes into its tiles'
//! widths so its right edge meets the view's.

/// The narrowest and widest aspect ratios a tile takes: a photo narrower
/// or wider than these is cropped to them, so neither a sliver nor a strip
/// wider than the view shows.
pub(crate) const ASPECT_RANGE: (f64, f64) = (0.25, 5.0);

/// A tile's place, in whole pixels from the top left of the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Place {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl Place {
    /// Whether (`x`, `y`) is on the tile.
    pub(crate) fn contains(&self, x: f64, y: f64) -> bool {
        x >= f64::from(self.x)
            && x < f64::from(self.x + self.width)
            && y >= f64::from(self.y)
            && y < f64::from(self.y + self.height)
    }

    /// The middle of the tile across.
    pub(crate) fn middle_x(&self) -> f64 {
        f64::from(self.x) + f64::from(self.width) / 2.0
    }
}

/// One row: the items it holds and where it is down the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    /// The first item of the row.
    pub(crate) start: usize,
    /// One past the last item.
    pub(crate) end: usize,
    pub(crate) y: i32,
    pub(crate) height: i32,
}

/// Every tile's place, row by row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) places: Vec<Place>,
    pub(crate) rows: Vec<Row>,
    /// The height of the whole layout.
    pub(crate) height: i32,
}

impl Layout {
    /// The row that holds `item`.
    pub(crate) fn row_of(&self, item: usize) -> Option<usize> {
        let index = self.rows.partition_point(|row| row.end <= item);
        (index < self.rows.len()).then_some(index)
    }

    /// The rows that show between `top` and `bottom`, as a range of rows.
    pub(crate) fn rows_between(&self, top: i32, bottom: i32) -> std::ops::Range<usize> {
        let first = self.rows.partition_point(|row| row.y + row.height <= top);
        let last = self.rows.partition_point(|row| row.y < bottom);
        first..last.max(first)
    }

    /// The item at (`x`, `y`), if a tile is there.
    pub(crate) fn item_at(&self, x: f64, y: f64) -> Option<usize> {
        #[expect(clippy::cast_possible_truncation, reason = "layout coordinates are small")]
        let rows = self.rows_between(y.floor() as i32, y.floor() as i32 + 1);
        rows.flat_map(|row| self.rows[row].start..self.rows[row].end)
            .find(|item| self.places[*item].contains(x, y))
    }

    /// The item in the row above (`up`) or below `item` nearest its middle.
    pub(crate) fn item_beside_vertically(&self, item: usize, up: bool) -> Option<usize> {
        let row = self.row_of(item)?;
        let other = if up { row.checked_sub(1)? } else { row + 1 };
        let other = self.rows.get(other)?;
        let middle = self.places[item].middle_x();
        (other.start..other.end).min_by(|a, b| {
            let distance = |item: &usize| (self.places[*item].middle_x() - middle).abs();
            distance(a).total_cmp(&distance(b))
        })
    }
}

/// Lays out tiles of `aspects` (width over height; `None` for unknown,
/// shown square) in rows `width` pixels wide near `target` pixels tall,
/// `gap` pixels apart.
pub(crate) fn justify(aspects: &[Option<f64>], width: i32, target: i32, gap: i32) -> Layout {
    let width = width.max(1);
    let target = target.max(1);
    let aspects: Vec<f64> = aspects
        .iter()
        .map(|aspect| {
            aspect
                .filter(|aspect| aspect.is_finite() && *aspect > 0.0)
                .unwrap_or(1.0)
                .clamp(ASPECT_RANGE.0, ASPECT_RANGE.1)
        })
        .collect();
    let mut layout = Layout {
        places: vec![Place::default(); aspects.len()],
        rows: Vec::new(),
        height: 0,
    };
    let mut y = 0;
    let mut start = 0;
    while start < aspects.len() {
        let (end, height) = row_end(&aspects[start..], width, target, gap);
        let end = start + end;
        let full = height.is_some();
        let height = height.unwrap_or(f64::from(target));
        #[expect(clippy::cast_possible_truncation, reason = "row heights are small")]
        let row_height = (height.round() as i32).max(1);
        place_row(
            &mut layout.places[start..end],
            &aspects[start..end],
            y,
            row_height,
            width,
            gap,
            full,
        );
        layout.rows.push(Row {
            start,
            end,
            y,
            height: row_height,
        });
        y += row_height + gap;
        start = end;
    }
    layout.height = (y - gap).max(0);
    layout
}

/// How many of `aspects` the next row takes, and its height when it is
/// full; `None` when they all fit short of the width, as the last row.
fn row_end(aspects: &[f64], width: i32, target: i32, gap: i32) -> (usize, Option<f64>) {
    let mut sum = 0.0;
    let target = f64::from(target);
    for (index, aspect) in aspects.iter().enumerate() {
        let count = index + 1;
        sum += aspect;
        let gaps = f64::from(u32::try_from(index).unwrap_or(u32::MAX));
        let room = f64::from(width) - f64::from(gap) * gaps;
        if sum * target >= room {
            let with = room / sum;
            if index > 0 {
                let before = sum - aspect;
                let room_before = room + f64::from(gap);
                let without = room_before / before;
                // Nearer the target by ratio: a row twice as tall is as
                // far as one half as tall.
                if (without / target).ln().abs() < (with / target).ln().abs() {
                    return (index, Some(without));
                }
            }
            return (count, Some(with));
        }
    }
    (aspects.len(), None)
}

/// Places one row's tiles at `y`, `height` tall; a `full` row's widths
/// are stretched to end exactly at `width`.
fn place_row(places: &mut [Place], aspects: &[f64], y: i32, height: i32, width: i32, gap: i32, full: bool) {
    let room = f64::from(width - gap * (i32::try_from(places.len()).unwrap_or(i32::MAX) - 1));
    let sum: f64 = aspects.iter().sum();
    let mut x = 0;
    let mut covered = 0.0;
    for (index, (place, aspect)) in places.iter_mut().zip(aspects).enumerate() {
        let tile_width = if full {
            covered += aspect / sum * room;
            let gaps = gap * i32::try_from(index).unwrap_or(0);
            #[expect(clippy::cast_possible_truncation, reason = "row widths are small")]
            let end = covered.round() as i32 + gaps;
            end - x
        } else {
            #[expect(clippy::cast_possible_truncation, reason = "tile widths are small")]
            let natural = (aspect * f64::from(height)).round() as i32;
            natural.min(width)
        };
        *place = Place {
            x,
            y,
            width: tile_width.max(1),
            height,
        };
        x += tile_width + gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full rows fill the width exactly with even gaps, keep one height
    /// near the target, and keep each tile's aspect ratio; the last row
    /// keeps the target height from the left.
    ///
    /// parity: HOME-026
    #[test]
    fn rows_fill_the_width_and_keep_aspect_ratios() {
        let aspects: Vec<Option<f64>> = [1.78, 1.0, 0.4, 3.0, 0.75, 1.5, 1.5, 0.42, 1.33, 1.33]
            .into_iter()
            .map(Some)
            .collect();
        let layout = justify(&aspects, 1000, 160, 4);
        assert!(layout.rows.len() >= 2);
        let last = layout.rows.len() - 1;
        for (index, row) in layout.rows.iter().enumerate() {
            let places = &layout.places[row.start..row.end];
            assert_eq!(places[0].x, 0);
            for pair in places.windows(2) {
                assert_eq!(pair[1].x - (pair[0].x + pair[0].width), 4);
            }
            let right = places.last().map(|place| place.x + place.width);
            if index < last || row.height != 160 {
                // A full row, the last one too when it reaches the width.
                assert_eq!(right, Some(1000));
                assert!((110..=230).contains(&row.height), "{row:?}");
            } else {
                assert!(right.is_some_and(|right| right < 1000));
            }
            for (place, aspect) in places.iter().zip(&aspects[row.start..row.end]) {
                let drawn = f64::from(place.width) / f64::from(place.height);
                let aspect = aspect.expect("known");
                assert!((drawn - aspect).abs() / aspect < 0.05, "{place:?} {aspect}");
            }
        }
        for pair in layout.rows.windows(2) {
            assert_eq!(pair[1].y, pair[0].y + pair[0].height + 4);
        }
        let end = layout.rows[last];
        assert_eq!(layout.height, end.y + end.height);
    }

    /// Unknown, extreme and empty inputs still lay out.
    ///
    /// parity: HOME-026
    #[test]
    fn odd_aspects_are_clamped_and_unknown_ones_are_square() {
        assert_eq!(justify(&[], 500, 160, 4), Layout::default());
        let layout = justify(&[None, Some(f64::NAN), Some(100.0), Some(0.01)], 2000, 100, 4);
        assert_eq!(layout.places[0].width, layout.places[0].height);
        assert_eq!(layout.places[1].width, layout.places[1].height);
        let wide = layout.places[2];
        assert!(wide.width <= wide.height * 5 + 1, "{wide:?}");
        // A panorama wider than the view gets a row to itself.
        let layout = justify(&[Some(5.0), Some(1.0)], 300, 160, 4);
        assert_eq!(layout.rows[0].end, 1);
        assert_eq!(layout.places[0].width, 300);
    }

    /// The rows on screen, hit testing and the tile above or below.
    ///
    /// parity: HOME-026
    #[test]
    fn the_layout_finds_rows_items_and_neighbours() {
        let layout = justify(&[Some(1.0); 12], 340, 100, 4);
        // Three tiles of 100 and their gaps fit 340 across as 108 or so.
        let first = layout.rows[0];
        assert_eq!(first.end - first.start, 3);
        assert_eq!(layout.rows_between(0, 1), 0..1);
        assert_eq!(layout.rows_between(0, layout.height), 0..layout.rows.len());
        let place = layout.places[4];
        assert_eq!(layout.item_at(place.middle_x(), f64::from(place.y + 2)), Some(4));
        assert_eq!(
            layout.item_at(f64::from(place.x - 2), f64::from(place.y + 2)),
            None
        );
        assert_eq!(layout.item_beside_vertically(4, true), Some(1));
        assert_eq!(layout.item_beside_vertically(4, false), Some(7));
        assert_eq!(layout.item_beside_vertically(1, true), None);
        assert_eq!(layout.row_of(11), Some(3));
        assert_eq!(layout.row_of(12), None);
    }
}
