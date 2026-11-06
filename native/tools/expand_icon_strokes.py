#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Write the app's own symbolic icons from their original drawings.

Two kinds of drawing are read. A drawing made of rounded rectangles has its
strokes turned into fills, as below. A drawing traced as filled paths on a
larger canvas keeps every path as drawn: its grey becomes the foreground,
its blue keeps its colour through the palette, and the icon's view box
frames the drawing, its longer side 20 of the 24 units and centred, the
share of the box Fluent's glyphs fill (about 16 of 20). Drawings made together on one canvas are
framed together (FRAMED), so they keep their sizes relative to each other.

Strokes:
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

# The fill colour of a traced drawing that follows the foreground.
FOREGROUND = {'#424242'}

# Each source drawing and the symbolic icon written from it.
ICONS = {
    'select-all.svg': 'ox-select-all-24-symbolic.svg',
    'select-none.svg': 'ox-select-none-24-symbolic.svg',
    'invert-selection.svg': 'ox-invert-selection-24-symbolic.svg',
    'options.svg': 'ox-options-24-symbolic.svg',
    'add-network-location.svg': 'ox-add-network-location-24-symbolic.svg',
    'map-network-drive.svg': 'ox-map-network-drive-24-symbolic.svg',
    'disconnect-network-drive.svg': 'ox-disconnect-network-drive-24-symbolic.svg',
}

# The traced drawings framed together: the three network commands were
# drawn on one 340 by 320 canvas, at one scale.
NETWORK = ('add-network-location.svg', 'map-network-drive.svg', 'disconnect-network-drive.svg')
FRAMED = {name: NETWORK for name in NETWORK}

# The icon's size, and the share of it a traced drawing's longer side fills:
# about what Fluent's 20-unit glyphs fill (16 of 20), so the traced icons
# match the menu's other glyphs in size.
SIZE = 24
DRAWN = 20


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


def translation(element: ET.Element) -> tuple[float, float]:
    """The offset of a traced path's `translate(x,y)`, the only transform traced drawings use."""
    transform = element.get('transform', '').strip()
    if not transform:
        return (0.0, 0.0)
    if not (transform.startswith('translate(') and transform.endswith(')')):
        raise ValueError(f'only translate() is supported, not {transform}')
    values = [float(value) for value in transform[len('translate('):-1].replace(',', ' ').split()]
    return (values[0], values[1] if len(values) > 1 else 0.0)


def path_points(element: ET.Element) -> list[tuple[float, float]]:
    """Every point and control point of a traced path, translated. A cubic curve lies
    inside its control points, so these bound the shape."""
    data = element.get('d', '')
    for command in set(ch for ch in data if ch.isalpha()):
        if command not in 'MLCZ':
            raise ValueError(f'only absolute M, L, C and Z are supported, not {command}')
    numbers = [float(value) for value in data.replace('M', ' ').replace('L', ' ').replace('C', ' ')
               .replace('Z', ' ').replace(',', ' ').split()]
    dx, dy = translation(element)
    return [(numbers[i] + dx, numbers[i + 1] + dy) for i in range(0, len(numbers) - 1, 2)]


def bounds(source: str) -> tuple[float, float, float, float]:
    """The left, top, right and bottom of a traced drawing's paths."""
    points = [point for element in ET.fromstring(source) if element.tag.split('}')[-1] == 'path'
              for point in path_points(element)]
    xs, ys = [x for x, _ in points], [y for _, y in points]
    return (min(xs), min(ys), max(xs), max(ys))


def frame(drawings: list[str]) -> str:
    """The view box that shows `drawings` at one scale, their longer side DRAWN of SIZE, centred."""
    edges = [bounds(drawing) for drawing in drawings]
    left, top = min(edge[0] for edge in edges), min(edge[1] for edge in edges)
    right, bottom = max(edge[2] for edge in edges), max(edge[3] for edge in edges)
    side = max(right - left, bottom - top) * SIZE / DRAWN
    x, y = (left + right - side) / 2, (top + bottom - side) / 2
    return f'{number(x)} {number(y)} {number(side)} {number(side)}'


def traced(source: str, drawings: list[str]) -> str:
    """The symbolic icon for the traced drawing `source`, framed with `drawings`."""
    shapes = []
    for element in ET.fromstring(source):
        tag = element.tag.split('}')[-1]
        if tag != 'path':
            raise ValueError(f'a traced drawing holds only <path>, not <{tag}>')
        fill = element.get('fill', '')
        place = f' transform="{element.get("transform")}"' if element.get('transform') else ''
        d = element.get('d')
        if fill.lower() in FOREGROUND:
            shapes.append(f'<path{place} d="{d}"/>')
        elif fill.lower() in COLOURED:
            shapes.append(f'<path class="{COLOURED[fill.lower()]}" fill="{fill}"{place} d="{d}"/>')
        else:
            raise ValueError(f'fill {fill} is neither the foreground nor a palette colour')
    return (f'<svg xmlns="{SVG}" width="{SIZE}" height="{SIZE}" viewBox="{frame(drawings)}">'
            + ''.join(shapes) + '</svg>\n')


def expand(source: str, drawings: list[str] | None = None) -> str:
    """The symbolic icon for the drawing `source`; a traced one is framed with
    `drawings` (itself alone by default)."""
    root = ET.fromstring(source)
    if any(element.tag.split('}')[-1] == 'path' for element in root):
        return traced(source, drawings or [source])
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


def icon(name: str) -> str:
    """The symbolic icon for the drawing `name` in SOURCES, framed with its group."""
    read = lambda drawing: (SOURCES / drawing).read_text(encoding='utf-8')
    return expand(read(name), [read(drawing) for drawing in FRAMED.get(name, (name,))])


def main() -> int:
    """Write every symbolic icon from its source drawing."""
    for source, target in ICONS.items():
        (OUTPUT / target).write_text(icon(source), encoding='utf-8')
        print(f'{SOURCES / source} -> {OUTPUT / target}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
