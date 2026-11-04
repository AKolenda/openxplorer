#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Turn the strokes of an icon drawn with rounded rectangles into fills.

GTK 4.14 recolours a symbolic icon by its fills only, so a stroke keeps the
colour it was drawn in and an outline drawn with fill="none" is filled in.
This writes the same picture with every stroked rounded rectangle replaced
by the area its stroke paints: a solid stroke becomes the ring between the
rounded rectangles half the stroke width outside and inside it, and a dashed
stroke becomes its dashes, cut along the outline as SVG cuts them (from the
end of the top-left corner, clockwise, with butt caps). Nothing else changes:
the result paints the same pixels, which the tests check. Stroked shapes
take the foreground colour; filled shapes in a colour listed in COLOURED
keep it through GTK's icon palette, as the class it names.

The sources are the original drawings in native/icon-sources/ (see the
README there); run this after changing one:

    python3 native/tools/expand_icon_strokes.py
"""
from __future__ import annotations

from dataclasses import dataclass
import math
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

NATIVE = Path(__file__).resolve().parents[1]
SOURCES = NATIVE / 'icon-sources'
OUTPUT = NATIVE / 'crates' / 'ox-app' / 'resources' / 'icons' / 'hicolor' / 'scalable' / 'actions'
SVG = 'http://www.w3.org/2000/svg'

# The fill colours that stay as drawn: the class GTK's icon palette gives
# them (-gtk-icon-palette in resources/skin/menus.css sets its colour).
COLOURED = {'#0078d4': 'success'}

# Each source drawing and the symbolic icon written from it.
ICONS = {
    'select-all.svg': 'ox-select-all-24-symbolic.svg',
    'select-none.svg': 'ox-select-none-24-symbolic.svg',
    'invert-selection.svg': 'ox-invert-selection-24-symbolic.svg',
}


def number(value: float) -> str:
    """Write a coordinate without a needless tail of digits."""
    text = f'{value:.4f}'.rstrip('0').rstrip('.')
    return '0' if text in ('-0', '') else text


@dataclass(frozen=True)
class RoundedRect:
    """An SVG <rect> with equal corner radii."""

    x: float
    y: float
    width: float
    height: float
    radius: float

    def grown(self, by: float) -> RoundedRect:
        """The rounded rectangle `by` outside this one (inside when negative)."""
        return RoundedRect(self.x - by, self.y - by, self.width + 2 * by, self.height + 2 * by,
                           max(self.radius + by, 0.0))

    def path(self) -> str:
        """Its outline as path data, clockwise from the end of the top-left corner."""
        x, y, w, h, r = self.x, self.y, self.width, self.height, self.radius
        if r == 0:
            return f'M{number(x)} {number(y)}H{number(x + w)}V{number(y + h)}H{number(x)}Z'
        arc = f'A{number(r)} {number(r)} 0 0 1 '
        return (f'M{number(x + r)} {number(y)}H{number(x + w - r)}'
                f'{arc}{number(x + w)} {number(y + r)}V{number(y + h - r)}'
                f'{arc}{number(x + w - r)} {number(y + h)}H{number(x + r)}'
                f'{arc}{number(x)} {number(y + h - r)}V{number(y + r)}'
                f'{arc}{number(x + r)} {number(y)}Z')

    def pieces(self) -> list[tuple[str, tuple, float]]:
        """The outline's sides and corners in path order: ('line', (start, end, normal), length)
        and ('arc', (centre, start angle), length)."""
        x, y, w, h, r = self.x, self.y, self.width, self.height, self.radius
        quarter = math.pi / 2 * r
        return [
            ('line', ((x + r, y), (x + w - r, y), (0, -1)), w - 2 * r),
            ('arc', ((x + w - r, y + r), -math.pi / 2), quarter),
            ('line', ((x + w, y + r), (x + w, y + h - r), (1, 0)), h - 2 * r),
            ('arc', ((x + w - r, y + h - r), 0.0), quarter),
            ('line', ((x + w - r, y + h), (x + r, y + h), (0, 1)), w - 2 * r),
            ('arc', ((x + r, y + h - r), math.pi / 2), quarter),
            ('line', ((x, y + h - r), (x, y + r), (-1, 0)), h - 2 * r),
            ('arc', ((x + r, y + r), math.pi), quarter),
        ]


def ring(rect: RoundedRect, stroke: float) -> str:
    """The area a solid stroke of width `stroke` paints, for fill-rule evenodd."""
    return rect.grown(stroke / 2).path() + rect.grown(-stroke / 2).path()


def point_at(piece: tuple[str, tuple, float], along: float, offset: float, radius: float):
    """The point `along` a piece, `offset` outside the outline."""
    kind, geometry, length = piece
    if kind == 'line':
        (x0, y0), (x1, y1), (nx, ny) = geometry
        share = along / length if length else 0.0
        return (x0 + (x1 - x0) * share + nx * offset, y0 + (y1 - y0) * share + ny * offset)
    (cx, cy), start = geometry
    angle = start + (along / length) * math.pi / 2 if length else start
    return (cx + (radius + offset) * math.cos(angle), cy + (radius + offset) * math.sin(angle))


def dash(rect: RoundedRect, stroke: float, start: float, end: float) -> str:
    """The area of one dash of a stroke along `rect`'s outline, from `start` to `end`."""
    half = stroke / 2
    spans = []  # (piece, from, to) along the dash
    position = 0.0
    for piece in rect.pieces():
        length = piece[2]
        lo, hi = max(start, position), min(end, position + length)
        if hi > lo or (length == 0 and position >= start and position <= end):
            spans.append((piece, lo - position, hi - position))
        position += length
    if not spans:
        return ''
    first = spans[0]
    outer = point_at(first[0], first[1], half, rect.radius)
    data = [f'M{number(outer[0])} {number(outer[1])}']
    for piece, lo, hi in spans:
        x, y = point_at(piece, hi, half, rect.radius)
        if piece[0] == 'arc':
            radius = rect.radius + half
            data.append(f'A{number(radius)} {number(radius)} 0 0 1 {number(x)} {number(y)}')
        else:
            data.append(f'L{number(x)} {number(y)}')
    for piece, lo, hi in reversed(spans):
        x, y = point_at(piece, hi, -half, rect.radius)
        data.append(f'L{number(x)} {number(y)}')
        x, y = point_at(piece, lo, -half, rect.radius)
        if piece[0] == 'arc':
            radius = max(rect.radius - half, 0.0)
            data.append(f'A{number(radius)} {number(radius)} 0 0 0 {number(x)} {number(y)}')
        else:
            data.append(f'L{number(x)} {number(y)}')
    data.append('Z')
    return ''.join(data)


def dashes(rect: RoundedRect, stroke: float, pattern: list[float]) -> str:
    """Every dash of a dashed stroke along `rect`, as SVG lays them out."""
    total = sum(piece[2] for piece in rect.pieces())
    lengths = pattern if len(pattern) % 2 == 0 else pattern * 2
    data, position, index = [], 0.0, 0
    while position < total:
        length = lengths[index % len(lengths)]
        if index % 2 == 0:
            data.append(dash(rect, stroke, position, min(position + length, total)))
        position += length
        index += 1
    return ''.join(data)


def expand(source: str) -> str:
    """The symbolic icon for the drawing `source`."""
    root = ET.fromstring(source)
    shapes = []
    for element in root:
        tag = element.tag.split('}')[-1]
        if tag != 'rect':
            raise ValueError(f'only <rect> is supported, not <{tag}>')
        rect = RoundedRect(*(float(element.get(name, '0')) for name in ('x', 'y', 'width', 'height', 'rx')))
        stroke = element.get('stroke')
        if stroke and stroke != 'none':
            width = float(element.get('stroke-width', '1'))
            pattern = element.get('stroke-dasharray')
            if pattern:
                values = [float(value) for value in pattern.replace(',', ' ').split()]
                shapes.append(f'<path d="{dashes(rect, width, values)}"/>')
            else:
                shapes.append(f'<path fill-rule="evenodd" d="{ring(rect, width)}"/>')
        fill = element.get('fill')
        if fill and fill != 'none':
            colour = COLOURED.get(fill.lower())
            if colour is None:
                raise ValueError(f'fill {fill} is neither the foreground nor a palette colour')
            shapes.append(f'<path class="{colour}" fill="{fill}" d="{rect.path()}"/>')
    width, height = root.get('width'), root.get('height')
    view_box = root.get('viewBox')
    return (f'<svg xmlns="{SVG}" width="{width}" height="{height}" viewBox="{view_box}">'
            + ''.join(shapes) + '</svg>\n')


def main() -> int:
    """Write every symbolic icon from its source drawing."""
    for source, target in ICONS.items():
        (OUTPUT / target).write_text(expand((SOURCES / source).read_text(encoding='utf-8')),
                                     encoding='utf-8')
        print(f'{SOURCES / source} -> {OUTPUT / target}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
