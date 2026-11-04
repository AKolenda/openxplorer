<!-- SPDX-License-Identifier: AGPL-3.0-only -->
# Icon sources

The original drawings of the app's own icons: the few that no published
icon set has. Every other icon is a Fluent file, listed in
`crates/ox-app/resources/icons/SOURCES.md`.

| Drawing | Icon | What it is |
|---|---|---|
| `select-all.svg` | `ox-select-all-24-symbolic.svg` | Select all in the command bar's "…" menu |
| `select-none.svg` | `ox-select-none-24-symbolic.svg` | Select none in the same menu |
| `invert-selection.svg` | `ox-invert-selection-24-symbolic.svg` | Invert selection in the same menu |

The three are original icons contributed to the project by its contributor
on 2026-11-04 (the user's request in CMD-007), drawn to match Windows 11
File Explorer's command-bar glyphs: Microsoft publishes no Fluent design of
them, and Windows' own bitmaps cannot be shipped. They have no third-party
source and are under the repository's licence, AGPL-3.0-only.

These drawings are the form to edit. GTK 4.14 recolours a symbolic icon by
its fills only, so `native/tools/expand_icon_strokes.py` writes each icon
from its drawing with the strokes turned into the areas they paint, which
keeps every shape and size; the grey outlines then follow the menu's text
colour in both themes, and their squares take Windows' blue through GTK's
icon palette (`resources/skin/menus.css`): #0078d4 as drawn in the light
theme, and Windows' lighter dark-theme accent, #60cdff, in the dark one.
After changing a drawing, run

    python3 native/tools/expand_icon_strokes.py

and record the new checksums in `SOURCES.md`. Its test checks that the
bundled icons are its output.
