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
| `options.svg` | `ox-options-24-symbolic.svg` | Options in the same menu |
| `add-network-location.svg` | `ox-add-network-location-24-symbolic.svg` | Add a network location in This PC's "…" |
| `map-network-drive.svg` | `ox-map-network-drive-24-symbolic.svg` | Map network drive in the same menu |
| `disconnect-network-drive.svg` | `ox-disconnect-network-drive-24-symbolic.svg` | Disconnect network drive in the same menu |

All seven are original icons made for OpenXplorer to match Windows 11 File
Explorer's command-bar glyphs: Microsoft publishes no Fluent design of
them, and Windows' own bitmaps cannot be shipped. They have no third-party
source and are under the project's licence, AGPL-3.0-only.

These drawings are the form to edit. The first three are rounded
rectangles; the other four are traced as filled paths, on a 600 by 600
canvas (Options) and a 340 by 320 one shared by the three network
commands. GTK 4.14 recolours a symbolic icon by its fills only, so
`native/tools/expand_icon_strokes.py` writes each icon from its drawing:
strokes become the areas they paint, which keeps every shape and size, and
a traced drawing keeps its paths as drawn, framed in the icon's 24 units
without cropping (the three network drawings together, at one scale). The
grey (#3b3b3b and #4a4a4a outlines, #424242 fills) then follows the menu's text colour
in both themes, and the blue takes Windows' blue through GTK's icon palette
(`resources/skin/menus.css`): #0078d4 as drawn in the light theme, and
Windows' lighter dark-theme accent, #60cdff, in the dark one. After
changing a drawing, run

    python3 native/tools/expand_icon_strokes.py

and record the new checksums in `SOURCES.md`. Its test checks that the
bundled icons are its output.
