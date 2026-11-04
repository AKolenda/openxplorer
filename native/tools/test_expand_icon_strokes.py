# SPDX-License-Identifier: AGPL-3.0-only
"""Tests for expand_icon_strokes.py: the committed icons are its output."""
from __future__ import annotations

import unittest

import expand_icon_strokes as expand


class ExpandIconStrokesTest(unittest.TestCase):
    def test_the_committed_icons_are_the_sources_expanded(self) -> None:
        """Each symbolic icon is exactly what the tool writes from its drawing."""
        for source, target in expand.ICONS.items():
            drawing = (expand.SOURCES / source).read_text(encoding='utf-8')
            committed = (expand.OUTPUT / target).read_text(encoding='utf-8')
            self.assertEqual(committed, expand.expand(drawing), target)

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


if __name__ == '__main__':
    unittest.main()
