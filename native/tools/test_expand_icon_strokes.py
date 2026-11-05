# SPDX-License-Identifier: AGPL-3.0-only
"""Tests for expand_icon_strokes.py: the committed icons are its output."""
from __future__ import annotations

import unittest

import expand_icon_strokes as expand


class ExpandIconStrokesTest(unittest.TestCase):
    def test_the_committed_icons_are_the_sources_expanded(self) -> None:
        """Each symbolic icon is exactly what the tool writes from its drawing."""
        for source, target in expand.ICONS.items():
            committed = (expand.OUTPUT / target).read_text(encoding='utf-8')
            self.assertEqual(committed, expand.icon(source), target)

    def test_a_solid_stroke_becomes_the_ring_it_paints(self) -> None:
        """A 1-wide stroke of a rounded square is the ring between radius 1 and 3."""
        drawing = ('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">'
                   '<rect x="4" y="4" width="8" height="8" rx="2" stroke="#3b3b3b" stroke-width="2" fill="none"/>'
                   '</svg>')
        icon = expand.expand(drawing)
        self.assertIn('fill-rule="evenodd"', icon)
        self.assertIn('M6 3H10A3 3 0 0 1 13 6', icon)
        self.assertIn('M6 5H10A1 1 0 0 1 11 6', icon)
        self.assertNotIn('stroke', icon)

    def test_a_dashed_stroke_becomes_its_dashes(self) -> None:
        """A square with no corners and dashes 2 on 2 gives a dash per 4 units of outline."""
        drawing = ('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">'
                   '<rect x="0" y="0" width="8" height="8" stroke="#4a4a4a" stroke-width="1" '
                   'stroke-dasharray="2 2" fill="none"/></svg>')
        icon = expand.expand(drawing)
        self.assertEqual(icon.count('Z'), 8, icon)
        self.assertIn('M0 -0.5L2 -0.5L2 0.5L0 0.5Z', icon)

    def test_the_windows_blue_keeps_its_colour_through_the_palette(self) -> None:
        """A fill in the Windows blue is the palette's success colour; another fill is refused."""
        blue = ('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">'
                '<rect x="1" y="1" width="4" height="4" rx="1" fill="#0078d4"/></svg>')
        self.assertIn('class="success" fill="#0078d4"', expand.expand(blue))
        with self.assertRaises(ValueError):
            expand.expand(blue.replace('#0078d4', '#ff0000'))


    def test_a_traced_drawing_keeps_its_paths_with_grey_as_the_foreground(self) -> None:
        """Grey fills follow the text colour, the blue keeps its colour, and the paths are unchanged."""
        drawing = ('<svg version="1.1" xmlns="http://www.w3.org/2000/svg" width="340" height="320">'
                   '<path d="M0 0 C10 0 20 10 20 20 Z" fill="#424242" transform="translate(10,30)"/>'
                   '<path d="M0 0 L40 0 L40 40 Z" fill="#0078D4" transform="translate(30,30)"/></svg>')
        icon = expand.expand(drawing)
        self.assertIn('<path transform="translate(10,30)" d="M0 0 C10 0 20 10 20 20 Z"/>', icon)
        self.assertIn('<path class="success" fill="#0078D4" transform="translate(30,30)" '
                      'd="M0 0 L40 0 L40 40 Z"/>', icon)
        # The drawing spans 10..70 by 30..70: its longer side fills 20 of the
        # 24 units, centred, as Fluent's glyphs fill their box.
        self.assertIn('width="24" height="24" viewBox="4 14 72 72"', icon)
        with self.assertRaises(ValueError):
            expand.expand(drawing.replace('#424242', '#ff0000'))

    def test_drawings_framed_together_share_one_scale(self) -> None:
        """A small drawing framed with a larger one is drawn at the larger one's scale."""
        small = ('<svg xmlns="http://www.w3.org/2000/svg" width="340" height="320">'
                 '<path d="M0 0 L60 0 L60 60 Z" fill="#424242"/></svg>')
        large = small.replace('L60 0 L60 60', 'L120 0 L120 120')
        self.assertIn('viewBox="-6 -6 72 72"', expand.expand(small))
        self.assertIn('viewBox="-12 -12 144 144"', expand.expand(small, [small, large]))

    def test_the_traced_icons_show_their_whole_drawing(self) -> None:
        """No part of a traced drawing falls outside its icon: nothing is cropped."""
        for source, target in expand.ICONS.items():
            drawing = (expand.SOURCES / source).read_text(encoding='utf-8')
            if '<path' not in drawing:
                continue
            left, top, right, bottom = expand.bounds(drawing)
            x, y, side, _ = (float(value) for value in
                             expand.ET.fromstring(expand.icon(source)).get('viewBox').split())
            self.assertTrue(x <= left and y <= top and right <= x + side and bottom <= y + side, target)

    def test_the_four_command_icons_are_traced_drawings(self) -> None:
        """Options and the three network commands are written from the drawings in icon-sources."""
        for source, target in [
            ('options.svg', 'ox-options-24-symbolic.svg'),
            ('add-network-location.svg', 'ox-add-network-location-24-symbolic.svg'),
            ('map-network-drive.svg', 'ox-map-network-drive-24-symbolic.svg'),
            ('disconnect-network-drive.svg', 'ox-disconnect-network-drive-24-symbolic.svg'),
        ]:
            self.assertEqual(expand.ICONS.get(source), target)


if __name__ == '__main__':
    unittest.main()
