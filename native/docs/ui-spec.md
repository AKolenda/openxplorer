# OpenXplorer native UI specification

The visual contract for the GTK 4.14 skin in `native/crates/ox-app/resources/`.
The native app must look like the current Python/WebKit app
(`v2.0.0:desktop/ui/style.css`), moved closer to Windows 11 File Explorer (24H2)
and WinUI 3 where a sourced Windows value exists.

- **Units.** Logical pixels at 100 % text size. Every font size and every
  text-dependent height scales with the text-size setting (80–200 %), as in
  `v2.0.0:desktop/ui/text-size.js` and `native/crates/ox-app/src/theme/fonts.rs`.
- **"Current"** means `v2.0.0:desktop/ui/style.css` and `v2.0.0:desktop/ui/app.js` at this
  commit, plus the screenshots `apps/web/public/assets/screenshots/explorer-{light,dark}.png`
  and `/tmp/ox-work/gtk4-spike/shots/current-*.png`.
- **"Native"** means `native/crates/ox-app/resources/{style,light,dark}.css`
  and the captures `/tmp/ox-native-captures/native-browsing-{light,dark,narrow}.png`.
- **Tags.** *safe*: a change of no more than 2 px or a few luminance steps,
  which users will not notice side by side; apply it without review.
  *visible*: noticeable. Needs product-owner sign-off with before/after
  screenshots at 1440 × 900, light and dark.
- **"(observed, unverified)"** marks Explorer facts that no Microsoft document
  states. Measure them on a Windows 11 24H2 machine at 100 % scaling before
  relying on them.
- Measurements of the PNGs come from scanning pixel runs along fixed rows
  and columns, for example column x = 700 of `explorer-light.png`.

## Contents

1. [Principles](#1-principles)
2. [Sources](#2-sources)
3. [Tokens](#3-tokens) (colour, typography, icon sizes, radii, spacing, elevation, motion)
4. [Regions](#4-regions)
5. [Deviations](#5-deviations) (current app, native capture)
6. [GTK 4.14 CSS notes](#6-gtk-414-css-notes)
7. [Fonts, icons and licensing](#7-fonts-icons-and-licensing)
8. [GNOME integration and the Dolphin bar](#8-gnome-integration-and-the-dolphin-bar)
9. [Verification](#9-verification)

---

## 1. Principles

### 1.1 Identity to keep exactly

These define the product's look. The native app reproduces them to the pixel
and does not "improve" them away.

| Area | Keep |
|---|---|
| Layout order | Tab strip in the title bar → navigation row (Back, Forward, Up, Refresh, breadcrumb address, search) → command bar (New ▾ │ Cut, Copy, Paste, Rename, Share, Delete │ Sort ▾, View ▾, More ··· ⟶ Appearance, Settings, Details) → sidebar │ content │ details pane → status bar. |
| Band heights | Title bar 42 (no border), navigation row 62, command bar 55, column header 38, status bar 30 (each of these four includes its 1 px border); row pitch 38. |
| Widths | Tab 215, sidebar 210 plus a 6 px resizer, details pane 262, search 235, caption buttons 46. |
| Palette character | Blue-grey tab strip (`#eff1f4` / `#191919`) standing in for Mica Alt; the active tab merges into a light chrome band; white (or `#202020`) content; soft blue selection `#e8f1fb` / `#263c50` with a faint blue edge. |
| Artwork | The project's own colour art (`folderIcon`, `appendFolderArt`, `zipFolderIcon`, `fileIcon`, `networkIcon` in `app.js`) and its own stroke glyphs (the `paths` table). Folder art: `#d99a22` back, `#ffce56` / `#f7bd40` front, `#fff0bd` paper. Network pipe: `#23873f` with `#62cf7c` / `#35a854` highlights. |
| Sizes | Row icons 21, sidebar icons 19, tiles 56, details preview 83. |
| Behaviour you can see | Breakpoints at 1190 / 960 / 680 px, text sizes 80–200 %, the classic (Windows 10, default) or Windows 11 context menu, the amber snapshot-tab marker, type-to-select feedback in the status bar, drag-to-pin highlighting, reduced-motion support. |

### 1.2 What "true to spec" means

1. If WinUI or Windows 11 has a sourced value for an element the app already
   has, converge on it when the change is *safe*. Queue *visible* changes for
   sign-off in the order given in §5.
2. Never remove an element or behaviour to match Windows. Rule 1 ("only gain
   functionality") outranks rule 3.
3. Where Windows gives no value, keep the current one. Do not invent a
   Windows-looking number.
4. New UI such as Dolphin-parity features uses the 4 px grid (§3.5), the
   WinUI state colours (§3.1) and the type ramp (§3.2).
5. Nothing Windows-proprietary is shipped: no Segoe fonts, no Segoe Fluent
   Icons, no Explorer artwork (§7).

---

## 2. Sources

WinUI files are pinned to microsoft-ui-xaml commit `25e7dbb156afb1a7605d7b21d68305efab0738a3`
(main, 2026-09-26). Base URL:
`https://github.com/microsoft/microsoft-ui-xaml/blob/25e7dbb156afb1a7605d7b21d68305efab0738a3/`.
A citation such as `CT#L243` means the base URL, then the path below, then `#L243`.

| Key | Path or URL |
|---|---|
| CT | `controls/dev/CommonStyles/Common_themeresources_any.xaml` (dark = the `Default` dictionary, lines 4–206; light = lines 208–413) |
| CC | `controls/dev/CommonStyles/Common_themeresources.xaml` |
| CR | `controls/dev/CommonStyles/CornerRadius_themeresources.xaml` |
| TV | `controls/dev/TabView/TabView_themeresources.xaml` |
| TVX | `controls/dev/TabView/TabView.xaml` |
| BB | `controls/dev/Breadcrumb/BreadcrumbBar_themeresources.xaml` |
| CB | `controls/dev/CommonStyles/CommandBar_themeresources.xaml` |
| ABB | `controls/dev/CommonStyles/AppBarButton_themeresources.xaml` |
| ABS | `controls/dev/CommonStyles/AppBarSeparator_themeresources.xaml` |
| LV | `controls/dev/CommonStyles/ListViewItem_themeresources.xaml` |
| GV | `controls/dev/CommonStyles/GridViewItem_themeresources.xaml` |
| TR | `controls/dev/TreeView/TreeView_themeresources.xaml` |
| TRI | `controls/dev/TreeView/TreeViewItem.xaml` |
| NV | `controls/dev/NavigationView/NavigationView_themeresources.xaml` |
| MF | `controls/dev/CommonStyles/MenuFlyout_themeresources.xaml` |
| TT | `controls/dev/CommonStyles/ToolTip_themeresources.xaml` |
| TB | `controls/dev/CommonStyles/TextBox_themeresources.xaml` |
| SB | `controls/dev/CommonStyles/ScrollBar_themeresources.xaml` |
| CD | `controls/dev/CommonStyles/ContentDialog_themeresources.xaml` |
| BT | `controls/dev/CommonStyles/Button_themeresources.xaml` |
| TBK | `controls/dev/CommonStyles/TextBlock_themeresources.xaml` |
| TTL | `controls/dev/TitleBar/TitleBar_themeresources.xaml` |
| AC | `controls/dev/Materials/Acrylic/AcrylicBrush_themeresources.xaml` |
| GX | `dxaml/xcp/dxaml/themes/generic.xaml` (dark 7–1969, light 3932–) |
| L-TYPE | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography |
| L-GEO | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry |
| L-SPACE | https://learn.microsoft.com/en-us/windows/apps/design/basics/content-basics |
| L-COLOR | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/color |
| L-LAYER | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/layering |
| L-MICA | https://learn.microsoft.com/en-us/windows/apps/design/style/mica |
| L-MAT | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/materials |
| L-MOTION | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion |
| L-ICON | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/iconography |
| L-SFI | https://learn.microsoft.com/en-us/windows/apps/design/style/segoe-fluent-icons-font |
| L-TITLE | https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design |
| L-FOCUS | https://learn.microsoft.com/en-us/windows/apps/develop/input/guidelines-for-visualfeedback |
| L-FONTFAQ | https://learn.microsoft.com/en-us/typography/fonts/font-faq |
| FT | `@fluentui/tokens` 1.0.0-alpha.24 (MIT): https://unpkg.com/@fluentui/tokens@1.0.0-alpha.24/lib/ plus `utils/shadows.js`, `alias/lightColor.js` (lines 179–180), `alias/darkColor.js` (179–180), `global/durations.js`, `global/curves.js`, `global/borderRadius.js`, `global/spacings.js`, `global/fonts.js` |
| WA | https://valer100.github.io/winaccent/colors/accent-color-and-shades/ ("Accent color and shades": the palette Windows 11 generates for the default accent `#0078D4`, read on a Windows 11 machine: dark 1 `#0067C0`, light 2 `#4CC2FF`). The README of https://github.com/Valer100/winaccent shows the same values only as demo console output. Third-party readout, not a Microsoft document. |
| GTK-SRC | GTK 4.14.5 sources, `https://gitlab.gnome.org/GNOME/gtk/-/blob/4.14.5/` + `gtk/gtkwindow.c`, `gtk/gtkcolumnviewtitle.c`, `gtk/theme/Default/_common.scss` |
| SEL | https://github.com/microsoft/Selawik (`LICENSE.txt`, `README.md`) |
| FSI | https://github.com/microsoft/fluentui-system-icons (licence: MIT) |
| GTK-NEWS | https://gitlab.gnome.org/GNOME/gtk/-/blob/main/NEWS |
| GTK-CSS | https://docs.gtk.org/gtk4/css-properties.html |

Local facts checked on the development machine (Zorin OS 18.1, GTK
4.14.5, Pango 1.52.1):

- The current font stack `"Segoe UI Variable","Segoe UI","Noto Sans"`
  resolves to **Noto Sans** in Pango (checked with `PangoCairo.FontMap.load_font`).
- `"Segoe UI"` alone resolves to Selawik, through Zorin's
  `/etc/fonts/conf.d/31-croscore-zorin-os.conf` alias (package `fonts-selawik-zorin-os`).
  The alias is a `<default>` alias, which fontconfig binds weakly, so a stack
  that names Noto Sans explicitly gets Noto Sans.
- The desktop font is `Inter 10`.
- `org.gnome.desktop.interface` has no `accent-color` key.
- `gtk-decoration-layout` is `menu:minimize,maximize,close` (from
  `org.gnome.desktop.wm.preferences button-layout`).
- Zorin's Selawik (`fonts-selawik-zorin-os` 18.0) has no kerning (no `kern`
  GPOS feature and no `kern` table) and no `tnum` feature. Its default figures
  are already equal-width, as are Noto Sans's (every digit advance is 539 font
  units in Selawik and 572 in Noto Sans).
- The user's `~/.config/gtk-4.0/gtk.css` sets
  `window.csd, window.csd decoration, window.csd headerbar { border-radius: 0; }`.
  GTK loads that file at `GTK_STYLE_PROVIDER_PRIORITY_USER` (800), above the
  skin's `APPLICATION` providers (600–602, `theme.rs`). So window rules in
  the skin lose to it on this machine. Headless captures use a throwaway HOME
  and do not show this.

---

## 3. Tokens

Composite values such as "#616161 on white" are the WinUI alpha colour
flattened onto the named surface. Native targets are written as GTK 4.14
`@define-color` values, and §3.8 has the files ready to paste.

### 3.1 Colour

| ID | Token (`@ox_…`) | Current app, light / dark | Windows 11, light / dark (source) | Native target, light / dark | Tag, note |
|---|---|---|---|---|---|
| C01 | `title`: tab strip | `#eff1f4` / `#191919` | Mica Alt, tinted by the wallpaper (L-MICA); solid fallback SolidBackgroundFillColorBaseAlt `#DADADA` / `#0A0A0A` (CT#L279 / CT#L75) | `#eff1f4` / `#191919` | Keep. GTK cannot sample the wallpaper (§6.2), and the fallback greys are much darker than the identity. |
| C02 | `title_backdrop`: tab strip, inactive window | same as C01 | When the window deactivates, Mica falls back to SolidBackgroundFillColorBase and **Mica Alt to SolidBackgroundFillColorBaseAlt** (L-MICA). The tab strip is Mica Alt (C01), so its fallback is `#DADADA` / `#0A0A0A` (CT#L279 / CT#L75). | `@ox_title` (`#eff1f4` / `#191919`) | Keep, for the same reason as C01. The inactive state shows through the labels instead (§4.1). |
| C03 | `title_hover`: inactive tab, caption, new-tab and windows buttons | `#ffffff55` / `#ffffff0c` on tabs; `#f0f0f0` / `#343434` on caption buttons | TabViewItemHeaderBackgroundPointerOver = LayerOnMicaBaseAltFillColorSecondary `#0A000000` / `#0FFFFFFF` (TV#L9 / TV#L87; CT#L269 / CT#L65) | `alpha(black, .04)` / `alpha(white, .06)` | visible in light: hover darkens instead of lightening. Fixes the caption hover, which is nearly invisible today (`#f0f0f0` on `#eff1f4`). |
| C04 | `chrome`: active tab, navigation row, command bar | `#f7f7f7` / `#262626` | TabViewItemHeaderBackgroundSelected = SolidBackgroundFillColorTertiary `#F9F9F9` / `#282828` (TV#L7 / TV#L85; CT#L274 / CT#L70) | `#f9f9f9` / `#282828` | safe |
| C05 | `bg`: content, details pane, status bar | `#ffffff` / `#202020` | LayerFillColorAlt `#FFFFFF` (CT#L265); SolidBackgroundFillColorBase `#202020` (CT#L68). Explorer content is white, and near `#191919` in dark (observed, unverified). | `#ffffff` / `#202020` | Keep. |
| C06 | `sidebar` | `#fafafa` / `#252525` | Explorer's navigation pane shares the content surface (observed, unverified); LayerFillColorDefault `#80FFFFFF` on Mica ≈ `#f9f9f9` (CT#L264) | `#fafafa` / `#252525` | Keep. |
| C07 | `flyout`: menus and popovers (new; currently `chrome`) | `#f7f7f7` / `#262626` | Acrylic fallback AcrylicBackgroundFillColorDefault `#F9F9F9` / `#2C2C2C` (AC#L95 / AC#L43); MenuFlyoutPresenterBackground is desktop acrylic (MF#L202) | `#f9f9f9` / `#2c2c2c` | visible in dark: menus lift off the content. |
| C08 | `tooltip` (new) | desktop tooltip (not styled); native `@ox_chrome` | The default ToolTip style paints ToolTipBackgroundBrush (TT#L44) = AcrylicInAppFillColorDefaultBrush, fallback `#F9F9F9` / `#2C2C2C` (TT#L38 / TT#L14; AC#L96 / AC#L44). The `ToolTipBackground` key (SystemChromeMediumLowColor, TT#L32) is not used by that style. | `#f9f9f9` / `#2c2c2c` (same as `flyout`) | safe |
| C09 | `card`: drive cards, settings sections | drive card `#ffffff` / `#202020` | CardBackgroundFillColorDefault `#B3FFFFFF` / `#0DFFFFFF` → `#ffffff` on white / `#2b2b2b` on `#202020` (CT#L260 / CT#L56) | `#ffffff` / `#2b2b2b` | visible in dark, P3 |
| C10 | `text` | `#242424` / `#f1f1f1` | TextFillColorPrimary `#E4000000` / `#FFFFFF` → `#1b1b1b` on white (CT#L209 / CT#L5) | `#1b1b1b` / `#ffffff` | light safe; dark visible (subtle, 14 steps brighter), P3 |
| C11 | `muted`: secondary text, header labels, other columns | `#686b70` / `#aaaaaa` | TextFillColorSecondary `#9E000000` / `#C5FFFFFF` → `#616161` on white / `#cccccc` on `#202020` (CT#L210 / CT#L6) | `#616161` / `#cccccc` | light safe; dark visible |
| C12 | `text_tertiary`: breadcrumb dividers, pin glyph, backdrop labels (new) | dividers `var(--muted)` at 70 % opacity; pin `#92979b` in both themes (native dark `#80858a`) | TextFillColorTertiary `#72000000` / `#87FFFFFF` → `#8d8d8d` / `#969696` (CT#L211 / CT#L7) | `#8d8d8d` / `#969696` | safe; replaces `crumb_divider` and `pin` |
| C13 | `text_disabled` (new) | `opacity: .34` on buttons | TextFillColorDisabled `#5C000000` / `#5DFFFFFF` → `#a3a3a3` / `#717171` (CT#L212 / CT#L8); AppBarButtonForegroundDisabled (ABB#L12) | `#a3a3a3` / `#717171` for glyphs and labels; colour art keeps the current `opacity: .34` | safe |
| C14 | `accent` | `#0067c0` / `#74beff` | AccentFillColorDefault = SystemAccentColorDark1 in light, SystemAccentColorLight2 in dark (CT#L329 / CT#L125); for the default blue these are `#0067C0` / `#4CC2FF` (WA) | `#0067c0` / `#4cc2ff` | light already matches; dark visible, P3 |
| C15 | `accent_hover`, `accent_pressed` (new) | none | AccentFillColorSecondary and Tertiary are the accent at 0.9 and 0.8 opacity (CT#L330–331 / CT#L126–127) | `alpha(@ox_accent, .9)`, `alpha(@ox_accent, .8)` | safe |
| C16 | `on_accent`: text on primary buttons | `#ffffff` / `#142535` (auth dialog `#112438`) | TextOnAccentFillColorPrimary `#FFFFFF` / `#000000` (CT#L216 / CT#L12) | `#ffffff` / `#000000` | safe |
| C17 | `hover`: rows, sidebar, commands, menu items | `#f0f0f0` / `#343434` | SubtleFillColorSecondary `#09000000` / `#0FFFFFFF` → `#f0f0f0` on `#f9f9f9`, `#f6f6f6` on white / `#353535` on `#282828` (CT#L230 / CT#L26) | `#f0f0f0` / `#343434` on every surface | Keep. Equals WinUI on the chrome, slightly stronger on white. |
| C18 | `pressed` | web `filter: brightness(.96)`; native `#e9e9e9` / `#3a3a3a` | SubtleFillColorTertiary `#06000000` / `#0AFFFFFF` → `#f3f3f3` / `#303030` on chrome (CT#L231 / CT#L27), with the foreground at TextFillColorSecondary (ABB#L83, BT#L121, TTL#L20) | `#f3f3f3` / `#303030` plus `color: @ox_muted` | safe. Windows makes pressed *lighter* than hover. |
| C19 | `selected` | `#e8f1fb` / `#263c50` | WinUI ListView: SubtleFillColorSecondary plus an accent selection pill (LV#L172, LV#L227; the pill's size is drawn in code, not in LV: 3 × 16 (unverified)). Explorer's item view uses a blue tint instead (observed, unverified). | `#e8f1fb` / `#263c50` | Keep. |
| C20 | `selected_hover` (new) | none | ListViewItemBackgroundSelectedPointerOver = SubtleFillColorTertiary (LV#L173); Explorer deepens the blue (observed, unverified) | `mix(@ox_selected, @ox_accent, .06)` = `#dae9f7` light (GTK 4.14.5 weights the second colour by the factor; measured) / ≈ `#28445a` dark with `#4cc2ff` | visible (subtle) |
| C21 | `selected_edge` | rows `#80b4e028`, tiles `#80b4e060` | none in WinUI; GridViewItem draws a 2 px selected border (GV#L111) | rows `alpha(#80b4e0, .16)`, tiles `alpha(#80b4e0, .38)` | Keep. |
| C22 | `border`: pane dividers, band borders | `#e8e8e8` / `#373737` | DividerStrokeColorDefault `#0F000000` / `#15FFFFFF` → `#eaeaea` on `#f9f9f9` / `#3a3a3a` on `#282828` (CT#L257 / CT#L53) | `#e8e8e8` / `#373737` | Keep (within 2 steps). |
| C23 | `control_stroke`: bordered buttons and fields (new) | `var(--border)` | ControlStrokeColorDefault `#0F000000` / `#12FFFFFF` (CT#L243 / CT#L39) | `alpha(black, .06)` / `alpha(white, .07)` | safe |
| C24 | `control_stroke_edge`: bottom edge in light, top edge in dark (new) | none | ControlElevationBorderBrush: ControlStrokeColorSecondary `#29000000` at the bottom in light (flipped gradient, CT#L382–389), `#18FFFFFF` at the top in dark (CT#L186–190; CT#L244 / CT#L40) | `alpha(black, .16)` / `alpha(white, .09)` | safe |
| C25 | `control_fill`, `_hover`, `_pressed` (new) | `var(--chrome)` on bordered buttons | ControlFillColorDefault `#B3FFFFFF` / `#0FFFFFFF`, Secondary `#80F9F9F9` / `#15FFFFFF`, Tertiary `#4DF9F9F9` / `#08FFFFFF` (CT#L219–221 / CT#L15–17; BT#L30–31) | `alpha(white, .7)`, `alpha(#f9f9f9, .5)`, `alpha(#f9f9f9, .3)` / `alpha(white, .06)`, `alpha(white, .08)`, `alpha(white, .03)` | visible (subtle) |
| C26 | `field_bg`: address and search | `#ffffff` / `#202020` | TextControlBackground = ControlFillColorDefault → `#fdfdfd` / `#353535` on chrome; focused ControlFillColorInputActive `#FFFFFF` / `#B31E1E1E` (TB#L130–132; CT#L225 / CT#L21) | `#ffffff` / `#202020` | Keep: the recessed dark field is part of the identity. |
| C27 | `field_bottom`: address and search bottom edge | `#c9c9c9` in **both** themes (a bug in dark) | TextControlElevationBorderBrush uses ControlStrongStrokeColorDefault at the bottom: `#72000000` / `#8BFFFFFF` → `#8d8d8d` / `#9a9a9a` (TB#L155–162; CT#L252 / CT#L48) | `#c9c9c9` / `#5a5a5a` | Softened identity; dark fixes D-A01. |
| C28 | `input_bottom`: dialog text inputs (new) | `#8e8e8e` (light and dark); auth and network inputs use `var(--muted)` | same as C27: `#8d8d8d` / `#9a9a9a` | `#8d8d8d` / `#9a9a9a` | safe |
| C29 | `focus_outer` (new) | 2 px `@accent`, drawn inside | FocusStrokeColorOuter `#E4000000` / `#FFFFFF`, 2 px (CT#L258 / CT#L54; L-FOCUS) | `alpha(black, .89)` / `#ffffff` | visible, P2 |
| C30 | `focus_inner` (new) | none | FocusStrokeColorInner `#B3FFFFFF` / `#B3000000`, 1 px (CT#L259 / CT#L55; L-FOCUS) | `alpha(white, .7)` / `alpha(black, .7)` | visible, P2 |
| C31 | `text_selection_bg`, `text_selection_fg` (new) | WebKit default | AccentFillColorSelectedTextBackground = SystemAccentColor, `#0078D4` for the default blue (CT#L328 / CT#L124; WA); TextOnAccentFillColorSelectedText `#FFFFFF` (CT#L215 / CT#L11) | `#0078d4`, `#ffffff` in both themes | safe |
| C32 | `critical`: error text | `#b32323` / `#ff9b96` | SystemFillColorCritical `#C42B1C` / `#FF99A4` (CT#L282 / CT#L78) | `#c42b1c` / `#ff99a4` | safe |
| C33 | `critical_fill`: close-button hover, danger buttons | close `#c42b1c`, danger `#b52a26` | `#C42B1C` (CT#L282). Caption close hover is the same red in both themes (observed, unverified). | `#c42b1c` in both themes; pressed `alpha(#c42b1c, .9)` (unverified: WinUI has no caption-button resource) | safe |
| C34 | `success`: "connected" dot | `#16873c` (offline `#929292`) | SystemFillColorSuccess `#0F7B0F` / `#6CCB5F` (CT#L280 / CT#L76) | `#0f7b0f` / `#6ccb5f`; offline `@ox_text_tertiary` | safe |
| C35 | `caution_*`: snapshot banner, badge and tab marker | banner `#fff7e8` / `#775314` / `#dfcba5`, dark `#302a20` / `#ecc993` / `#51422a`; badge `#fff2d6` / `#714a08`, dark `#443721` / `#f4cc82`; tab top `#c48c2f` | SystemFillColorCautionBackground `#FFF4CE` / `#433519` (CT#L287 / CT#L83) | current values | Keep (identity); within a few steps of WinUI. |
| C36 | `smoke`: modal scrim | `#0004`, auth `#0006` | SmokeFillColorDefault `#4D000000` (CT#L263 / CT#L59) | `alpha(black, .3)` for both dialogs | safe |
| C37 | `tab_text_inactive` (new) | same as the active tab (`@text`) | TabViewItemHeaderForeground = TextFillColorSecondary (TV#L12 / TV#L90) | `@ox_muted` | visible, P3 |
| C38 | `tab_separator` (new) | none | TabViewItemSeparator = DividerStrokeColorDefault (TV#L46 / TV#L124) | `alpha(black, .06)` / `alpha(white, .08)` | visible, P3 |
| C39 | `tab_strip_line` (new) | none | TabViewBorderBrush = CardStrokeColorDefault `#0F000000` / `#19000000` (TV#L68 / TV#L146). The strip draws it left of the tabs and right of the add button (TVX#L36–37); each tab draws its own bottom line (TVX#L546), hidden on the selected tab (TVX#L337). The selected tab also gets a 1 px outline of the same colour on its top and sides (TV#L72–80, TV#L268; TVX#L344–345). | `alpha(black, .06)` / `alpha(black, .10)` | visible, P3. §1.1 fixes the title bar as having no border, so this ships only if sign-off amends §1.1. |
| C40 | `scrollbar_thumb` | web `#aaa8`; native `alpha(@ox_text, .35)` | ScrollBarThumbFill = ControlStrongFillColorDefault `#72000000` / `#8BFFFFFF` (SB#L138; CT#L226 / CT#L22) | `alpha(black, .45)` / `alpha(white, .55)` | safe |
| C41 | `window_stroke` (new) | framed preview `#adb7c54a` | Window stroke 1 px (L-LAYER); SurfaceStrokeColorDefault `#66757575` (CT#L254 / CT#L50) | `alpha(#757575, .4)` | safe |
| C42 | `flyout_stroke` (new; currently `@border`) | `var(--border)` | SurfaceStrokeColorFlyout `#0F000000` / `#33000000` (MF#L203, TT#L33; CT#L255 / CT#L51) | `alpha(black, .06)` / `alpha(black, .2)` | safe |
| C43 | `dialog_stroke` (new) | `var(--border)` | ContentDialogBorderBrush = SurfaceStrokeColorDefault (CD#L43) | `@ox_window_stroke` | safe |
| C44 | `shadow_ambient`, `shadow_key` (new) | `#0002`, `#0003`, `#0005`, `#20304e26` | Fluent neutral shadows `rgba(0,0,0,.12)` and `.14` in light, `.24` and `.28` in dark (FT `alias/lightColor.js` and `darkColor.js`, lines 179–180) | `alpha(black, .12)`, `alpha(black, .14)` / `alpha(black, .24)`, `alpha(black, .28)` | safe; used in §3.6 |
| C45 | `rubberband` | none (web) | Explorer: translucent accent rectangle (observed, unverified) | border 1 px `@ox_accent`, fill `alpha(@ox_accent, .15)` | Keep (native). |
| C46 | `doc_paper`, `doc_fold`, `doc_stroke`: file art | `#fafcfe`, `#e5ecf5`, `#c9d3df` / `#dbe3ec`, `#b7c8dc`, `#94a6bb` | none (Explorer artwork is proprietary) | current values | Keep (identity). |
| C47 | `capacity`: drive bar | 4 px `@accent` on a `@hover` track | Explorer: accent bar that turns red when nearly full (observed, unverified) | 4 px `@ox_accent` on `@ox_hover`, radius 2; `@ox_critical_fill` at ≥ 90 % full | Keep; red is a gain. Native now uses 6 px `#26a0da` (D-N20). |

### 3.2 Typography

Windows 11 type ramp, in size/line-height epx (L-TYPE; TBK#L3–9): Caption
12/16, Body 14/20, Body Strong 14/20 semibold, Body Large 18/24, Subtitle 20/28,
Title 28/36, Title Large 40/52, Display 68/92. Use Regular (400) for text and
Semibold (600) for titles. The minimum sizes are "14px Semibold, 12px Regular",
and all UI text is sentence case (L-TYPE). Explorer's item view, navigation
pane and status bar use the 9 pt system font, 12 px at 96 dpi (observed,
unverified). That is why the app's 12 px base is correct.

| ID | Role | Current | Windows 11 (source) | Native target | Tag |
|---|---|---|---|---|---|
| T01 | Family | `"Segoe UI Variable","Segoe UI","Noto Sans",Arial,sans-serif`; renders **Noto Sans** on Zorin 18.1 | Segoe UI Variable (L-TYPE); the XAML default is `XamlAutoFontFamily` (GX#L14) | Keep the rendered face: `"Segoe UI Variable", "Segoe UI", "Noto Sans", sans-serif` (Noto Sans on Zorin, as today). Candidate after sign-off: insert `"Selawik"` before `"Noto Sans"`. | Keep. Selawik is closer to Segoe, but Zorin's copy has no kerning and is Latin-only (§2, §7.1), and it would restyle every label on Zorin, which changes the look users know rather than refining it. It is a visible P2 candidate that needs a side-by-side at 12 px and 13 px. Offer "Use desktop font" as a gain (§8). |
| T02 | UI text: rows, sidebar, commands, tabs, menus, search, crumbs | 12 px / 400 | Caption 12/16 (TBK#L3); TabView header 12 (TV#L245); AppBarButton label 12 (ABB#L33) | 12 px / 400 | Keep. |
| T03 | Window base, address entry, detail header, section titles | 13 px (body `line-height: 1.45`) | Body 14/20 (TBK#L4); BreadcrumbBar item 14 = ControlContentThemeFontSize (BB#L63; GX#L36) | 13 px / 400 | Keep. A 14 px address bar would match WinUI but is visible. |
| T04 | Small text: status bar, detail properties, notes, search-info bar | 11 px | 12 px minimum (L-TYPE) | 12 px | visible, P2 |
| T05 | Tiny text: menu accelerators, sidebar headings, quick-access heading, "connected", cache and version metadata, status mode | 9–10 px | Menu accelerator uses CaptionTextBlockStyle, 12 px (MF#L392); 12 px minimum (L-TYPE) | 12 px | visible, P2 |
| T06 | Sidebar section headings | 10 px, uppercase, `letter-spacing: .06em` | sentence case, no tracking (L-TYPE) | 12 px, sentence case, `letter-spacing: 0`, `@ox_muted` | visible, P3 |
| T07 | Section titles (landing, details "Properties") | 13 / 600 and 12 / 600 | Body Strong 14/20 semibold (L-TYPE) | 13 / 600 and 12 / 600 | Keep. |
| T08 | Details-pane name | 16 / 600, line-height 1.4 | nearest: Body Large 18/24 (L-TYPE) | 16 / 600 | Keep; 18 is optional (visible, P3). |
| T09 | Empty-state title | 16 / 500 | Semibold for titles (L-TYPE) | 16 / 600 | visible (subtle), P3 (T13) |
| T10 | Dialog title | 21 / 600, `letter-spacing: -.3px` (auth 22, `-.45px`) | ContentDialog title 20, SemiBold (CD#L238) | 20 / 600, `letter-spacing: 0` | safe |
| T11 | Page title (landing h1) | 24 / 600, `-.4px` | Title 28/36 (TBK#L7) | 24 / 600, `letter-spacing: 0` | tracking safe; 28 px optional (visible, P3) |
| T12 | Settings page h1 / h2 / h3 | 27 / 19 / 15 (modal section titles) and 19 (settings-page section titles) | Title 28, Subtitle 20, Body Strong 14 (TBK#L4–7) | 28 / 20 / 14 for the modal h3; 20 for the settings-page h3, which works as a subtitle; all 600 | safe (±1 px). Mapping the 19 px settings-page h3 to 14 would be a 5 px drop (visible), so it maps to Subtitle 20. |
| T13 | Weights in use | 400, 500, 600 | 400 and 600 only (L-TYPE) | 400 and 600. Replace every 500 (drive-card name, cache-folder name, property-file name, app-choice name, versions heading and version names, browser-profile name, empty-state title) with 600. | visible (subtle), P3: Zorin installs Noto Sans Medium (500), so these labels render Medium today and SemiBold would be one step bolder. Selawik has no 500. |
| T14 | Figures in the Size, Date and version columns | the font's default figures, which are already equal-width in Noto Sans and Selawik (§2); `tabular-nums` only in `.version-date` | Segoe UI figures (not specified) | `font-feature-settings: "tnum" 1` on those cells, for fonts whose default figures are proportional | safe (no visible change with Noto Sans; Selawik has no `tnum`) |
| T15 | Line height | body 1.45; tile names 1.35; notes 1.65–1.75 | Body 20/14 = 1.43 (L-TYPE) | natural for single lines; tile names `line-height: 1.35`; notes `1.65` | Keep. `line-height` needs GTK ≥ 4.6 (GTK-CSS). |

### 3.3 Icon sizes and glyph stroke

| ID | Where | Current | Windows 11 (source) | Native target | Tag |
|---|---|---|---|---|---|
| I01 | Command-bar glyphs | 18 (icon only), 17 (with label) | Segoe Fluent Icons: a 16 epx font size = a 16 × 16 icon (L-ICON); AppBarButton icons are 16 (observed, unverified) | 16 | safe |
| I02 | Navigation buttons | 16 | 16 | 16 | Keep. |
| I03 | Tab icon | 17 (colour art) | TabViewItemHeaderIconSize 16 (TV#L246) | 16 colour art | safe |
| I04 | Caption glyphs | 12 | ChromeMinimize E921, ChromeMaximize E922, ChromeRestore E923, ChromeClose E8BB; the maximize and restore glyphs have rounded corners (L-TITLE); drawn at 10 (observed, unverified) | 10, with a dedicated 10 × 10 path and a 1 px stroke | visible (crisper) |
| I05 | Menu glyphs | 15 (classic) / 16 (Windows 11) | MenuFlyoutItem icon 16 × 16 (MF#L388); icon column 28 = 16 + 12 (MF#L212) | 16 in both styles | safe |
| I06 | Sidebar icons | 19 in a 20 × 24 slot | NavigationView icon 16 (NV#L219) | 19 | Keep (identity). |
| I07 | Row icons, details view | 21 | Explorer details view uses 16 (observed, unverified) | 21 | Keep (identity). |
| I08 | Tile icons | 56 ("Large icons") | Explorer: Small 16, Medium 48, Large 96, Extra large 256 (observed, unverified) | 56 for Large (identity); native adds 28 / 40 / 96 as a gain | Keep. |
| I09 | Status-bar view toggles | 15 | 16 (observed, unverified) | 16 | safe |
| I10 | Glyph stroke | `stroke-width: 1.35` in a 24-unit box: 0.68 px at 12, 0.9 px at 16, 1.0 px at 18 | Monoline, a single 1 epx stroke (L-ICON) | 1 logical px at every size (stroke width in path units = 24 / size) | safe at 16 px and above (0.9 → 1.0 px); visible below 16 px (0.68 → 1.0 px at 12 px is about 50 % bolder), P3. It fixes faint 12 px glyphs. |

### 3.4 Corner radii

| ID | Element | Current | Windows 11 (source) | Native target | Tag |
|---|---|---|---|---|---|
| R01 | Window | framed preview 10; native CSD 8 | 8 for top-level windows; 0 when snapped or maximized (L-GEO) | 8; 0 for `.maximized`, `.fullscreen` and `.tiled*` | safe. On this machine the user's own `gtk.css` forces 0 at a higher priority (§2); leave that choice winning. |
| R02 | Menus (Windows 11 style) and popovers | 10 (`.menu.win11`), 8 (base `.menu`) | 8, OverlayCornerRadius (L-GEO; CR#L13–14) | 8 | safe |
| R03 | Menus (classic style) | 2 | Windows 10 menus are square (observed, unverified) | 2 | Keep (a deliberate option). |
| R04 | Dialogs | 9 | 8 (L-GEO; CD `CornerRadius` = OverlayCornerRadius) | 8 | safe |
| R05 | Tab top corners | 8 8 0 0 | TabViewItem CornerRadius = OverlayCornerRadius, top only (TVX#L297, TVX#L562) | 8 8 0 0 | Keep. |
| R06 | Selected-tab bottom flare | none | 4 × 4 outward arcs at both bottom corners of the selected tab (TVX#L547–548) | 4 px concave arcs in `@ox_chrome` | visible, P3 (§6.2) |
| R07 | Controls: buttons, rows, sidebar entries, crumbs, fields, list backplates | buttons 4, rows 4, fields **5** | 4, ControlCornerRadius (L-GEO; CR#L13; LV#L210) | 4 everywhere | safe (fields 5 → 4) |
| R08 | Tiles and quick cards | 6 | 4 (GV#L109) | 4 | safe |
| R09 | Large panels: details preview, drive card, network banner, settings section, integration card, transfer panel, toast | 7 / 7 / 7 / 9 / 7 / 7 / 7 | L-GEO gives 8 only to top-level containers and transient or overlay UI (windows, flyouts, dialogs). In-page elements get 4 (L-GEO). | Overlays (transfer panel, toast): 8. In-page panels (details preview, drive card, network banner, settings section, integration card): keep 7, with the settings section 9 → 7. | Overlays safe. In-page panels: keep. 4 would match L-GEO but a 3 px change is visible, P3. |
| R10 | Small boxes: notice, modal note, auth target, versions list, archive list | 5 | 4 (L-GEO) | 4 | safe |
| R11 | Tooltip | desktop default; native 4 | 4 (L-GEO; TT#L52) | 4 | Keep. |
| R12 | Scrollbar thumb | 8 (web, 9 px bar) | 3 (SB#L190) | 3 | Native already (§4.12). |
| R13 | Selection pill | 4 | 2 (NV#L222; TRI#L130); 1.5 (LV#L212) | 2 | safe |
| R14 | Bars: capacity, progress, loading line | 4 / 3 / 0 | 4 for bar-shaped elements (L-GEO) | 2 (half of the 4 px height) / 2 / 0 | Keep; a 4 px-tall bar is already a pill. |

### 3.5 Spacing

The Fluent 4 px grid has 2 px sub-steps: 2, 4, 6, 8, 10, 12, 16, 20, 24, 32
(FT `global/spacings.js`). The Windows rules are 8 between buttons, 8 between a
button and its flyout, 8 between a control and its header, 12 between a control
and its label, 12 between content areas, and 16 from a surface's edge to its text
(L-SPACE).

| ID | Rule | Current | Native target | Tag |
|---|---|---|---|---|
| S01 | Grid for **new** UI | none | 4 px, with 2 px sub-steps | n/a |
| S02 | Existing metrics | odd values (9, 11, 13, 15, 22, …) | Keep, except where §4 lists a snap of 2 px or less. | Keep |
| S03 | Row inset from the pane edge | 12 / 12 (`#file-canvas .file-row` overrides the base `.file-row` left 13) | 12 / 12 | Keep. |
| S04 | Dialog button gap | 9 | 8 (ContentDialogButtonSpacing, CD#L50) | safe |
| S05 | Dialog padding | 28 (auth 28–32) | 24 (ContentDialogPadding, CD#L52) | visible (subtle), P3 |
| S06 | Tab icon to title | 9 | 10 (TV#L247) | safe |
| S07 | Command glyph to label | 9 | 8 | safe |
| S08 | Tab strip top | 7: the tab renders 35 high because `.tab{min-height:max(35px,…)}` overrides `height:34px` (measured: `explorer-light.png` column x = 150, tab rows y = 7–41) | 7 (35-high tabs in the 42 bar). WinUI's TabViewHeaderPadding is `0,8,0,0` (TV#L239). | Keep. 8 would need 34-high tabs (1 px, safe, optional). |
| S09 | Menu item insets | item margin 0; menu padding 3 (classic) / 5 (Windows 11) | Windows 11 style: menu padding `2px 0`, item margin `2px 4px`, item padding `0 11px` (WinUI: presenter padding `0,2`, MF#L255; item margin `4,2`, MF#L259; item padding `11,4,11,5` for mouse, pen and keyboard, MF#L261); classic unchanged | safe |

### 3.6 Elevation

Windows elevation values (L-LAYER): window 128, dialog 128, flyout 32,
tooltip 16, card 8, control 2 (1 when pressed), layer 1; every one has a 1 px
stroke. Fluent's CSS shadow ramp (FT `utils/shadows.js`):
`shadowN = 0 0 2px ambient, 0 (N/2)px Npx key` for N = 2, 4, 8, 16;
`shadow28 = 0 0 8px ambient, 0 14px 28px key`;
`shadow64 = 0 0 8px ambient, 0 32px 64px key`.

| ID | Surface | Current | Native target (GTK CSS) | Tag |
|---|---|---|---|---|
| E01 | Window (CSD) | framed preview `0 16px 54px #20304e26`; native uses GTK's own | Leave `box-shadow` to GTK, so the window matches every other GTK window on GNOME (§8); set only `border-radius: 8px`. For `.solid-csd` (no compositor): `border: 1px solid @ox_window_stroke; padding: 4px; box-shadow: none; border-radius: 0`. Do not set `padding: 0`. Without a compositor, GTK 4.14 uses the window's border plus padding as the resize area (GTK-SRC `gtkwindow.c`, `get_edge_for_coordinates` and `get_box_border`), so `padding: 0` would leave a 1 px edge that cannot be grabbed to resize (a rule 1 loss). The 5 px band (1 px stroke plus 4 px padding, the same size GTK's theme uses) shows the window's `@ox_chrome` background instead of GTK's grey frame. | safe |
| E02 | Dialog (in-window) | `0 18px 65px #0003`; auth `0 24px 90px #0006` | `box-shadow: 0 0 8px @ox_shadow_ambient, 0 32px 64px @ox_shadow_key;` `border: 1px solid @ox_dialog_stroke` | safe |
| E03 | Menus and popovers | `0 7px 26px #0002` (classic `0 5px 16px #0003`); native `0 7px 26px @ox_menu_shadow` | Windows 11 style: `box-shadow: 0 0 8px @ox_shadow_ambient, 0 14px 28px @ox_shadow_key; border: 1px solid @ox_flyout_stroke`. Classic style: keep `0 5px 16px alpha(black, .2)`. | safe |
| E04 | Tooltip | desktop; native `0 2px 8px alpha(black, .15)` | `box-shadow: 0 0 2px @ox_shadow_ambient, 0 8px 16px @ox_shadow_key; border: 1px solid @ox_flyout_stroke` | safe |
| E05 | Cards and panels | 1 px `@border`, no shadow | Same: stroke only. L-LAYER lists cards at elevation 8 with a 1 px stroke. WinUI's card-style controls draw only CardStrokeColorDefault (CT#L250) over CardBackgroundFillColorDefault, without a shadow (observed, unverified). | Keep. shadow8 would be the L-LAYER value (visible, P3). |
| E06 | Bordered buttons (dialogs, details "Open", empty-state action) | 1 px `@border`, flat | `border: 1px solid @ox_control_stroke;` then `border-bottom-color: @ox_control_stroke_edge` in light or `border-top-color: @ox_control_stroke_edge` in dark (CT#L382–389 / CT#L186–190) | safe |
| E07 | Accent buttons | 1 px `@accent` | `border: 1px solid alpha(white, .08); border-bottom-color: alpha(black, .4)` in light; `border-bottom-color: alpha(black, .14)` in dark (ControlStrokeColorOnAccentDefault / Secondary, CT#L245–246 / CT#L41–42) | safe |
| E08 | Toast, transfer panel, drag badge, tab-drag hint | `0 4px 18px #0002` to `#0004` | shadow16 (as E04) | safe |
| E09 | Band separation (title / navigation / commands / content) | 1 px `@border`, no shadow | Same. Windows' layering is two-tone, not shadowed (L-LAYER). | Keep. |

### 3.7 Motion

| ID | Use | Current | Windows 11 (source) | Native target | Tag |
|---|---|---|---|---|---|
| M01 | Hover, pressed and selected colour changes | instant | BrushTransition 83 ms (BT#L174); ControlFasterAnimationDuration 83 ms (CT#L606); "Fade – In + Out, Linear, 83ms" (L-MOTION) | `transition: background-color 83ms linear, color 83ms linear, box-shadow 83ms linear;` | safe |
| M02 | Entrance (surfaces, expanding elements) | none | Fast-in `cubic-bezier(0,0,0,1)`, 167 / 250 / 333 ms (L-MOTION); ControlFastOutSlowInKeySpline `0,0,0,1` (CT#L602); ControlFastAnimationDuration 167 ms, Normal 250 ms (CT#L603–604) | `167ms cubic-bezier(0,0,0,1)` for anything the app animates | n/a |
| M03 | Exit | none | Fast-out `cubic-bezier(0,0,0,1)`, 167 ms, always with a fade (L-MOTION) | `167ms cubic-bezier(0,0,0,1)` plus `opacity` | n/a |
| M04 | Point to point (moving selection pill) | instant | `cubic-bezier(0.55,0.55,0,1)`, 167 / 250 / 333 ms (L-MOTION) | optional, 167 ms | visible, P3 |
| M05 | Scrollbar expand and contract | web: none | expand 167 ms after 400 ms; contract 167 ms after 500 ms (SB#L173–176, SB#L188–189); spline `0,0,0,1` (SB#L484) | `transition: min-width 167ms cubic-bezier(0,0,0,1), min-height 167ms cubic-bezier(0,0,0,1);` GTK controls the delays. | safe |
| M06 | Loading line | 25 % bar, `1s ease-in-out infinite` | none; Fluent duration and curve tokens are in FT `global/durations.js` and `global/curves.js` | Keep. Show it only after 150 ms (Fluent `durationFast`) so fast folders never flash it. | safe |
| M07 | Popover open | instant (web) | Direct entrance (M02) | GTK 4.14 popovers do not animate; keep them instant. | n/a |
| M08 | Reduced motion | `@media (prefers-reduced-motion: reduce)` → 0.01 ms | n/a | GTK disables CSS transitions and animations when `gtk-enable-animations` is false (GNOME "Reduce animation" / `enable-animations`). The loading line must then show a static 25 % bar. | must keep (rule 1) |

### 3.8 Target palette files

Paste these over `light.css` and `dark.css`. Base rules refer only to these
names.

```css
/* light.css */
@define-color ox_title #eff1f4;
@define-color ox_title_backdrop @ox_title;
@define-color ox_title_hover alpha(black, .04);
@define-color ox_chrome #f9f9f9;
@define-color ox_bg #ffffff;
@define-color ox_sidebar #fafafa;
@define-color ox_flyout #f9f9f9;
@define-color ox_tooltip #f9f9f9;
@define-color ox_card #ffffff;
@define-color ox_text #1b1b1b;
@define-color ox_muted #616161;
@define-color ox_text_tertiary #8d8d8d;
@define-color ox_text_disabled #a3a3a3;
@define-color ox_accent #0067c0;
@define-color ox_accent_hover alpha(@ox_accent, .9);
@define-color ox_accent_pressed alpha(@ox_accent, .8);
@define-color ox_on_accent #ffffff;
@define-color ox_hover #f0f0f0;
@define-color ox_pressed #f3f3f3;
@define-color ox_selected #e8f1fb;
@define-color ox_selected_hover mix(@ox_selected, @ox_accent, .06);
@define-color ox_selected_edge alpha(#80b4e0, .16);
@define-color ox_tile_selected_edge alpha(#80b4e0, .38);
@define-color ox_border #e8e8e8;
@define-color ox_control_stroke alpha(black, .06);
@define-color ox_control_stroke_edge alpha(black, .16);
@define-color ox_control_fill alpha(white, .7);
@define-color ox_control_fill_hover alpha(#f9f9f9, .5);
@define-color ox_control_fill_pressed alpha(#f9f9f9, .3);
@define-color ox_field_bg #ffffff;
@define-color ox_field_bottom #c9c9c9;
@define-color ox_input_bottom #8d8d8d;
@define-color ox_focus_outer alpha(black, .89);
@define-color ox_focus_inner alpha(white, .7);
@define-color ox_text_selection_bg #0078d4;
@define-color ox_text_selection_fg #ffffff;
@define-color ox_critical #c42b1c;
@define-color ox_critical_fill #c42b1c;
@define-color ox_success #0f7b0f;
@define-color ox_smoke alpha(black, .3);
@define-color ox_tab_separator alpha(black, .06);
@define-color ox_tab_strip_line alpha(black, .06);
@define-color ox_scrollbar_thumb alpha(black, .45);
@define-color ox_window_stroke alpha(#757575, .4);
@define-color ox_flyout_stroke alpha(black, .06);
@define-color ox_dialog_stroke alpha(#757575, .4);
@define-color ox_shadow_ambient alpha(black, .12);
@define-color ox_shadow_key alpha(black, .14);
```

```css
/* dark.css */
@define-color ox_title #191919;
@define-color ox_title_backdrop @ox_title;
@define-color ox_title_hover alpha(white, .06);
@define-color ox_chrome #282828;
@define-color ox_bg #202020;
@define-color ox_sidebar #252525;
@define-color ox_flyout #2c2c2c;
@define-color ox_tooltip #2c2c2c;
@define-color ox_card #2b2b2b;
@define-color ox_text #ffffff;
@define-color ox_muted #cccccc;
@define-color ox_text_tertiary #969696;
@define-color ox_text_disabled #717171;
@define-color ox_accent #4cc2ff;
@define-color ox_accent_hover alpha(@ox_accent, .9);
@define-color ox_accent_pressed alpha(@ox_accent, .8);
@define-color ox_on_accent #000000;
@define-color ox_hover #343434;
@define-color ox_pressed #303030;
@define-color ox_selected #263c50;
@define-color ox_selected_hover mix(@ox_selected, @ox_accent, .06);
@define-color ox_selected_edge alpha(#80b4e0, .16);
@define-color ox_tile_selected_edge alpha(#80b4e0, .38);
@define-color ox_border #373737;
@define-color ox_control_stroke alpha(white, .07);
@define-color ox_control_stroke_edge alpha(white, .09);
@define-color ox_control_fill alpha(white, .06);
@define-color ox_control_fill_hover alpha(white, .08);
@define-color ox_control_fill_pressed alpha(white, .03);
@define-color ox_field_bg #202020;
@define-color ox_field_bottom #5a5a5a;
@define-color ox_input_bottom #9a9a9a;
@define-color ox_focus_outer #ffffff;
@define-color ox_focus_inner alpha(black, .7);
@define-color ox_text_selection_bg #0078d4;
@define-color ox_text_selection_fg #ffffff;
@define-color ox_critical #ff99a4;
@define-color ox_critical_fill #c42b1c;
@define-color ox_success #6ccb5f;
@define-color ox_smoke alpha(black, .3);
@define-color ox_tab_separator alpha(white, .08);
@define-color ox_tab_strip_line alpha(black, .10);
@define-color ox_scrollbar_thumb alpha(white, .55);
@define-color ox_window_stroke alpha(#757575, .4);
@define-color ox_flyout_stroke alpha(black, .2);
@define-color ox_dialog_stroke alpha(#757575, .4);
@define-color ox_shadow_ambient alpha(black, .24);
@define-color ox_shadow_key alpha(black, .28);
```

Staging: *safe* rows can land at once. Until sign-off, the *visible* rows keep
their current values: `ox_accent` stays `#74beff` in dark, `ox_text` stays
`#f1f1f1` in dark, `ox_muted` stays `#aaaaaa` in dark, `ox_card` stays
`#202020` in dark, and the focus colours stay the accent. `ox_title_hover` in
light is also *visible*. The file art
colours (C46) and snapshot colours (C35) live in the art code, not these
files.

---

## 4. Regions

Every value is a native target. A value in parentheses is the current app
where it differs; ✓ means the target equals the current app.

### 4.1 Title bar and tab strip

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Bar | height 42 (min `max(42, ceil(25·s + 12))`), `padding-left: 9`, background `@ox_title`, **no bottom border** under the active tab ✓ | WinUI TitleBar is 32 (compact) or 48 (tall) (TTL#L77–78); tabs use the title-bar space and keep the caption buttons on the right (L-TITLE) | Keep. |
| Tab | 35 high (`max(35, ceil(25·s + 7))`; the current app's `height: 34px` loses to this min-height), 7 below the bar top ✓ (S08), natural width 215, shrinking to **100** (80) when crowded, radius 8 8 0 0, padding `0 10 0 13`, 2 px apart | TabView: min height 32, width 100–240, header padding `8,3,4,3` (TV#L240–244) | min-width safe |
| Active tab | background `@ox_chrome`, label `@ox_text`, joins the navigation row with no seam; 4 px outward flares at the bottom corners (P3) | TabViewItemHeaderBackgroundSelected (TV#L7); flares: the selected background path extends 4 px past each side (TVX#L551) and 4 × 4 arcs stroke its edge in TabViewBorderBrush (TVX#L547–548). WinUI also sets the selected title SemiBold (TVX#L351); not adopted, because the current app keeps the active tab at 400. | flare visible |
| Inactive tab | transparent; label `@ox_tab_text_inactive`; hover `@ox_title_hover`; pressed `alpha(white, .7)` / `alpha(#3a3a3a, .45)` | Foreground TextFillColorSecondary (TV#L12); pressed LayerOnMicaBaseAltFillColorDefault `#B3FFFFFF` / `#733A3A3A` (TV#L10; CT#L268 / CT#L64) | visible, P3 |
| Separators | 1 × 19 px `@ox_tab_separator` (the 35 px tab minus WinUI's 8 px top and bottom margins), centred in the gap between two inactive tabs; hidden next to the active or a hovered tab | TabViewItemSeparator, margin `0,8,0,8` (TV#L46, TV#L266) | visible, P3 |
| Tab icon | 16 colour art (folder, drive, network, device), then 10 px to the title | 16 icon, 10 margin (TV#L246–247) | safe |
| Tab title | 12 px, ellipsis at the end | 12 (TV#L245) | Keep. |
| Close button | 22 × 22, radius 4, glyph 12, hover `@ox_hover` on the active tab and `@ox_title_hover` elsewhere | 32 × 24, glyph 12 (TV#L248–251); hover SubtleFillColorSecondary (TV#L49) | Keep size. |
| Snapshot tab | width 330, min 285, `border-top: 2px solid #c48c2f`, badge 10 → 12 px (T05) ✓ | — | Keep. |
| New tab (+) | 38 × 34, glyph 14, margin `0 3 0 5`, **directly after the last tab**; hover `@ox_title_hover` | 32 × 24, glyph 12 (TV#L261–263) | Keep size. |
| Drag area | fills the remaining width, **min 48** (16) | TitleBarMinDragRegionWidth 48 (TTL#L85) | safe |
| Open-windows button | 35 × 30, glyph 16, `@ox_muted`, before the caption buttons ✓ | — | Keep. |
| Caption buttons | 46 wide × full bar height (42), radius 0, glyph 10 (I04); hover `@ox_title_hover`; close hover `@ox_critical_fill` with a white glyph; close pressed `alpha(@ox_critical_fill, .9)` with `alpha(white, .7)` | full-bleed backplates; glyphs E921, E922, E923, E8BB (L-TITLE); 46 px width and the close pressed colours (observed, unverified) | glyph visible (crisper) |
| Inactive window (`:backdrop`) | bar unchanged (`@ox_title_backdrop` = `@ox_title`, C02); inactive tab labels and caption glyphs `@ox_text_tertiary`; active tab unchanged | inactive title-bar elements are semi-transparent (L-TITLE); TitleBarDeactivatedOpacity 0.5, deactivated foreground TextFillColorTertiary (TTL#L86, TTL#L33); Mica Alt's inactive fallback is `#DADADA` / `#0A0A0A` (C02) | safe |
| Maximized or tiled | window radius 0; caption buttons flush to the top | L-GEO | safe |
| Tab strip line | 1 px `@ox_tab_strip_line` at the bar bottom, everywhere except under the active tab | TVX#L36–37, TVX#L546, TVX#L337 (C39) | visible, P3; needs §1.1 amended ("no border") |
| Narrow (≤ 960) | tab width 180 ✓ | — | Keep. |
| Narrow (≤ 680) | tab width 150; caption buttons 32 wide; bar `padding-left: 5` ✓ | — | Keep. |

### 4.2 Navigation row, breadcrumbs and search

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Row | 62 including a 1 px bottom border `@ox_border`; padding `11 16 11 13`; group gap 14; background `@ox_chrome` ✓. Children are **vertically centred**, not stretched (D-N08). | Explorer's address row is about 48 (observed, unverified) | Keep. |
| Back, Forward, Up, Refresh | 34 × 34, radius 4, 5 apart, glyph 16; disabled `@ox_text_disabled` | AppBarButton hover and pressed (ABB#L77–79) | safe |
| Address box | 34 high including borders; radius **4** (5); `border: 1px solid @ox_border; border-bottom-color: @ox_field_bottom`; background `@ox_field_bg`; padding `0 8 0 11`; gap 10; location icon **16** (17) | TextBox: min height 32 (GX#L96), padding `10,5,6,6` (CC#L26), strong bottom stroke (TB#L155–162) | safe |
| Address focus or edit | bottom edge 2 px `@ox_accent`, drawn as `border-bottom-color: @ox_accent; box-shadow: inset 0 -1px @ox_accent` (no height change) | Focused border `1,1,1,2` with an accent bottom (CC#L25; TB#L164–171) | Keep. |
| Crumb | 28 high, padding `0 9`, radius 4, 12 px, `border: 1px solid transparent`; hover `@ox_hover` with a `@ox_border` edge; keyboard focus is the focus ring (§4.13) | BreadcrumbBar: normal weight, hover and pressed change only the text colour (BB#L39–42). Explorer shows a hover backplate (observed, unverified). | Keep. |
| Crumb divider | chevron glyph 11 in `@ox_text_tertiary`, 1 px padding each side | E974 at 12 px, padding `2,0` (BB#L36–38, BB#L64) | safe (colour) |
| Crumb overflow | a « button at the start that lists hidden ancestors in a popover (native gain); the web app scrolls instead | BreadcrumbBar ellipsis flyout (BB#L29 / BB#L60) | gain |
| History chevron | 22 × 24, glyph 12, `@ox_muted`, right end | — | Keep. |
| Edit mode | an entry replaces the crumbs; 13 px; all text selected on entry; selection `@ox_text_selection_bg` / `_fg` | CT#L328 | safe |
| Search box | 235 × 34 (195 at ≤ 1190, 175 at ≤ 960, hidden at ≤ 680); radius 4; padding `0 35 0 12`; 12 px; placeholder "Search *folder*" in `@ox_muted`; search glyph 15 in `@ox_muted`, **11 px from the right edge**; focus as the address box | placeholder TextFillColorSecondary (TB#L142); Explorer puts the glyph at the right end (observed, unverified) | Keep. |

### 4.3 Command bar

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Bar | 55 including a 1 px bottom border (min `max(53, ceil(30·s + 12))`); padding `0 15`; gap 4; background `@ox_chrome`; scrolls horizontally when too narrow ✓ | CommandBar compact height 48 (CB#L72); Explorer is about 48 (observed, unverified) | Keep. |
| Icon command | 34 high, min 38 wide, padding `0 10`, radius 4, glyph **16** (18) | I01 | safe |
| Text command | padding `0 11`, 12 px label, glyph **16** (17), gap **8** (9); dropdown chevron 12 in `@ox_muted`, 4 px after the label | AppBarButton label 12 (ABB#L33) | safe |
| "New" | padding `0 10 0 8` ✓ | — | Keep. |
| Separator | 1 × 20, `margin: 0 10`, `@ox_border` | AppBarSeparator 1 px, DividerStrokeColorDefault, margin `2,8,2,8` (ABS#L8, ABS#L14–16) | Keep. |
| States | hover `@ox_hover`; pressed `@ox_pressed` with `@ox_muted` text; disabled `@ox_text_disabled`; toggled (Details open) `@ox_selected` | ABB#L77–79, ABB#L12 | safe |
| Right group | Appearance (sun or moon glyph with a "Light" / "Dark" label, `@ox_muted`, text `@ox_text` on hover, label hidden at ≤ 1050), Settings (gear, `@ox_muted`), Details toggle ✓ | — | Keep. |
| Responsive | ≤ 1190: gap 1, padding `0 10`, separator margin `0 6`; ≤ 960: commands padding `0 8`, min width 32; ≤ 680: hide Copy path, Cut, Rename and the Details toggle (they stay in More) ✓ | CommandBar overflow (CB#L5–8) | Keep. |

### 4.4 Sidebar

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Pane | width 210 by default (185 at ≤ 1190, 175 at ≤ 960, 150 at ≤ 680), user-resizable; background `@ox_sidebar`; padding **`12 8 0`** (`13 7 0`) | Explorer pane default width about 200–220 (observed, unverified) | safe |
| Resizer | 6 px wide, background `@ox_sidebar`, 1 px `@ox_border` right edge; hover or focus: `@ox_selected` with a 2 px `@ox_accent` centre line | — | Keep. |
| Entry | 35 high (min `ceil(20·s + 15)`), margin `1 0` (37 pitch), radius 4, padding `0 11`, gap 12, 12 px text; icon slot 20 × 24 holding a 19 px icon | NavigationViewItem min height 36 with margin `4,2` (NV#L217, NV#L228); TreeViewItem 28 with margin `4,2` (TR#L79, TR#L82) | Keep. |
| States | hover `@ox_hover`; pressed `@ox_pressed`; current location `@ox_selected` plus the pill | TreeViewItem hover and selected fills (TR#L47, TR#L50) | safe |
| Selection pill | 3 × **16**, radius **2** (3 × 15, radius 4), `@ox_accent`, at the left edge, centred vertically | NavigationView indicator 3 × 16, radius 2 (NV#L220–222); TreeViewItem 3 × 16, radius 2 (TRI#L130) | safe |
| Indent | child rows `padding-left: 32`; expander chevron 9 in `@ox_muted` | TreeView chevron (TRI#L143–145) | Keep. |
| Pin glyph | 11 in `@ox_text_tertiary`, right-aligned, on pinned Quick access rows | Explorer shows a pin on pinned items (observed, unverified) | safe |
| Group separator | 1 px `@ox_border`, margin `12 13` | NavigationView separator margin `0,3,0,4` (NV#L247) | Keep. |
| Headings | T06 | L-TYPE casing | visible, P3 |
| Quick access drop | block `border: 1px solid @ox_accent` (2 px in high contrast), background `@ox_selected`, heading in `@ox_accent`; insertion line 2 px `@ox_accent`, radius 2; dashed 30 px tail row when empty ✓ | — | Keep (rule 1). |
| Bottom area | `border-top: 1px solid @ox_border`; padding `9 0 11`; button 34 high, glyph 17 in `@ox_accent`, "Map network location" at 12 px ✓ | — | Keep. |
| Network rows | network art (folder or server glyph on a green pipe, `networkIcon`) ✓ | — | Keep. |

### 4.5 Details view

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Header | 38 including a 1 px bottom border `@ox_border` (min `max(38, ceil(24·s + 10))`); 12 px `@ox_muted`, weight 400; column padding `0 12`; header row inset `0 14` so labels line up with cell text | Explorer header labels are 12 px secondary text (observed, unverified) | Keep. |
| Header hover | `@ox_hover` plus a 1 px `@ox_border` right edge | Explorer shows column dividers on hover (observed, unverified) | Keep. |
| Size header | label **right-aligned**, 17 px right padding | Explorer left-aligns every header (observed, unverified) | Keep (identity). |
| Resize handle | 8 px hit area centred on the column edge; hover `@ox_selected` with a 2 px `@ox_accent` line; column-resize cursor | — | Keep. |
| Sort indicator | caret glyph 10, `@ox_muted`, 6 px after the label, **primary sort column only** | Explorer draws a small chevron centred above the label (observed, unverified) | Keep; see D-N05. |
| Columns | Name (flexible, min 170), Date modified 152, Type 135, Size **78** (native 90); at ≤ 1190: 140 / 132 / 112 / 63; at ≤ 680: Name and Size only; user-resized widths persist | — | Keep. |
| Search-results columns | Name (min 160), Folder (min 210, 1.2×), Type 110, Size 74; folder text 11 → **12** px `@ox_muted` | — | Keep, except T04. |
| Row, normal | pitch **38** = 36 row + 2 gap (`row = max(38, ceil(24·s + 14)) − 2`); first row 4 below the header (5) | ListViewItem min height 40 (LV#L166); Explorer normal pitch about 30–32 (observed, unverified) | Keep. |
| Row, compact (new) | pitch **28** = 26 + 2 (`max(28, ceil(20·s + 8)) − 2`), behind a "Compact view" toggle in View | Explorer "Compact view" (`UseCompactMode`); compact pitch about 22–24 (observed, unverified) | gain |
| Row backplate | inset 12 from both pane edges ✓ (S03); radius 4; inner padding `0 2`; cells padding `0 12` | ListViewItemCornerRadius 4 (LV#L210) | Keep. |
| Name cell | icon 21, gap 11, `@ox_text`, ellipsis at the end | — | Keep. |
| Other cells | `@ox_muted`; Size right-aligned with 12 px right padding and tabular figures (T14) | — | Keep. |
| Row states | hover `@ox_hover`; selected `@ox_selected` + `inset 0 0 0 1px @ox_selected_edge`; selected and hovered `@ox_selected_hover`; keyboard focus: inset ring (§4.13); cut: `opacity: .5`; drag source: `opacity: .5`; drop target: `outline: 2px solid @ox_accent` inset with `@ox_selected` | LV#L170–173, LV#L183 | see §3.1 |
| Rubberband | 1 px `@ox_accent`, fill `alpha(@ox_accent, .15)`, radius 0 | — | Keep (native). |

### 4.6 Icon view

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Cell | width `max(135, ceil(90·s + 45))`, height `130 + max(0, ceil((s − 1)·46))`; tiles are 4 px apart (cell minus 4 wide, minus 2 high); the grid starts 10 px from the left and 5 px from the top | — | Keep. |
| Tile | radius **4** (6); padding `12 10`; gap 8; icon 56 (Large), top-aligned; name 12 px, centred, at most two lines, line height 1.35, ellipsis | GridViewItem radius 4 (GV#L109); grid items use centred caption text (L-SPACE) | safe |
| States | as rows; selected edge `inset 0 0 0 1px @ox_tile_selected_edge` | GV#L111 | Keep. |
| Thumbnails (gain) | images and video show the freedesktop thumbnail in the icon box, radius 4, 1 px `@ox_border` | Dolphin and Explorer both preview (§8) | gain |
| Other sizes (native gain) | Small 28, Medium 40, Extra large 96; cell width `max(grid_width, icon + 79)` | Explorer sizes 16 / 48 / 96 / 256 (observed, unverified) | gain |

### 4.7 Status bar

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Bar | 30 including a 1 px top border (min `max(29, ceil(15·s + 9))` + 1); padding `0 13 0 19`; gap 18; background `@ox_bg`; text **12** px (11) in `@ox_muted` | 12 px minimum (L-TYPE) | text visible, P2 |
| Left | "N items", "N selected, size" as separate spans 18 apart (native joins them with " · "); type-to-select hint in `@ox_accent`, or `@ox_muted` when nothing matches; ellipsis at 48 % width | — | Keep; D-N13. |
| Right | status mode (**12** px, hidden at ≤ 680), update button 24 × 24, then the Details view and Large icons toggles, 24 × 24 with glyph **16** (15); active toggle `@ox_selected` with a `@ox_accent` glyph | Explorer puts Details and Large icons toggles at the right of its status bar (observed, unverified) | safe |

### 4.8 Details pane

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Pane | width **262** (235 at ≤ 1190, hidden at ≤ 960); `border-left: 1px solid @ox_border`; padding `22 22 16`; background `@ox_bg` | Explorer details pane about 260–320 (observed, unverified) | Keep. |
| Header | "Details" at 13 px; close button 24 × 24, glyph 12; 20 px below | — | Keep. |
| Preview | min height 148; background `@ox_chrome`; 1 px `@ox_border`; radius 7 ✓ (R09); icon 83 with `drop-shadow(0 4px 4px alpha(black, .04))`; 18 px below; thumbnails when available (gain) | — | safe |
| Name | 16 / 600, line height 1.4, selectable, wraps anywhere; 4 px below | — | Keep. |
| Type | 12 px `@ox_muted`; 17 px below | — | Keep. |
| Action button | "Open", or "Pin to Quick access" for folders: full width, 32 high, E06 border, background `@ox_control_fill`, 12 px, glyph 14; 24 px below | ButtonPadding `11,5,11,6` (BT#L152) | safe |
| Properties | heading 12 / 600, 17 px below; grid with a 73 px key column, row gap 15, column gap 8; keys `@ox_muted`; values selectable; text **12** px (11) | — | text visible, P2 |
| Note | 12 px (11) `@ox_muted`, line height 1.65, `border-top: 1px solid @ox_border`, `padding-top: 17`, `margin-top: 23`; info glyph 14 in `@ox_accent` | — | text visible, P2 |

### 4.9 Menus and context menus

Both styles stay, because the preference is behaviour (rule 1).
**Classic** is the default.

| Element | Classic target | Windows 11 style target | Windows reference | Tag |
|---|---|---|---|---|
| Surface | width `max(264, 235·s)`; padding 3; radius 2; 1 px `@ox_flyout_stroke`; background `@ox_flyout`; shadow `0 5px 16px alpha(black, .2)` | width `max(276, 235·s)`; padding `2 0`; radius 8; 1 px `@ox_flyout_stroke`; background `@ox_flyout`; shadow E03 | MF#L202–203, MF#L255; L-GEO; L-LAYER | safe |
| Item | min height `ceil(22·s + 11)` (33); padding `0 9`; radius 0; gap 9; glyph 16 (15) | min height 29 plus a `2 4` margin (33 pitch); padding `0 11`; radius 4; icon column 16 + 12 | MF#L259–261, MF#L212, MF#L388 | safe |
| Label | 12 px `@ox_text`, ellipsis | same | MenuFlyoutItemForeground (MF#L177) | Keep. |
| Accelerator | 12 px (10) `@ox_muted`, right-aligned, at least 12 px after the label | 12 px `@ox_muted`, at least 24 px after the label | Caption style, margin 24 (MF#L392); TextFillColorSecondary (MF#L197) | visible, P2 |
| Hover, keyboard highlight | `@ox_hover` | `@ox_hover` | SubtleFillColorSecondary (MF#L173) | Keep. |
| Pressed | `@ox_pressed` | `@ox_pressed` | SubtleFillColorTertiary (MF#L174) | safe |
| Disabled | `@ox_text_disabled` | `@ox_text_disabled` | TextFillColorDisabled (MF#L180) | safe |
| Separator | 1 px `@ox_border`, margin `4 5` | 1 px `@ox_border`, **full width** (margin `1 0`) | MenuFlyoutSeparatorBackground, padding `-4,1,-4,1` (MF#L171, MF#L258) | visible (subtle), P3 |
| Heading | 12 px (10) `@ox_muted`, padding `7 12 5` | same | L-TYPE minimum | visible, P2 |
| Check and radio | 16 glyph in `@ox_accent`, in the icon column | same | — | Keep. |
| Submenu arrow | chevron 12 `@ox_muted`, right end | same | chevron margin 24 (MF#L257) | Keep. |
| Quick-action strip (Windows 11 style only) | — | five 34-high icon buttons at 20 % width each, 2 px gap and padding, above the list ✓ | Windows 11 context menu (observed, unverified) | Keep. |

### 4.10 Tooltips

Background `@ox_tooltip`; text `@ox_text` at 12 px; `border: 1px solid @ox_flyout_stroke`;
radius 4; padding `6px 9px 8px 9px`; maximum width 320 (wrap with
`max-width-chars: 48` in code); shadow E04. Sources: ToolTip font size 12,
border 1, padding `9,6,9,8`, max width 320, ControlCornerRadius (TT#L29–33,
TT#L52, TT#L76–77). Tag: safe.

### 4.11 Dialogs

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Scrim | `@ox_smoke` over the window (tab-scoped dialogs cover only the tab's content, as now) | SmokeFillColorDefault (CT#L263) | safe |
| Surface | width 510 (settings 790, properties 600, versions 860, archive 720, auth 535) within `calc(100vw − 30px)`; radius **8** (9); 1 px `@ox_dialog_stroke`; shadow E02 | ContentDialog width 320–548, height 184–756, border 1, SurfaceStrokeColorDefault (CD#L43–49) | safe |
| Layout | content padding **24** (28); title 20 / 600 (T10), 12 px below; body 12 px `@ox_muted`, line height 1.75 | ContentDialogPadding 24, title margin 12 (CD#L51–52); title 20 SemiBold (CD#L238) | padding visible (P3) |
| Footer (P3) | the auth dialog's footer applied to every dialog: 1 px top border `@ox_border`, background `@ox_chrome` (`#f3f3f3` / `#202020` if WinUI-exact), padding `18 24`, buttons right-aligned | ContentDialog command space: Background = SolidBackgroundFillColorBase under a LayerFillColorAlt content area with a 1 px separator (CD#L40–45, CD#L233, CD#L248) | visible, P3 |
| Buttons | 32 high (33), min width 92, padding `5 17`, 8 apart (9), E06 border, background `@ox_control_fill`; hover `@ox_control_fill_hover`; pressed `@ox_control_fill_pressed` with `@ox_muted` text | ButtonPadding `11,5,11,6`; button fills (BT#L30–31, BT#L152); spacing 8 (CD#L50) | safe |
| Primary | background `@ox_accent`, text `@ox_on_accent`, E07 border (transparent while pressed); hover `@ox_accent_hover`; pressed `@ox_accent_pressed` with text `alpha(@ox_on_accent, .7)` in light and `alpha(@ox_on_accent, .5)` in dark | AccentButton (BT#L5–16); TextOnAccentFillColorSecondary `#B3FFFFFF` / `#80000000` (CT#L217 / CT#L13); pressed border ControlFillColorTransparent (BT#L15) | safe |
| Danger | background `@ox_critical_fill`, white text | SystemFillColorCritical (CT#L282) | safe |
| Text input | 35 high (keep), radius 4, background `@ox_chrome`, `border: 1px solid @ox_border; border-bottom-color: @ox_input_bottom`; focus: bottom 2 px `@ox_accent` | TextBox (§4.2) | safe |
| Checkbox | 16 × 16, accent fill when checked | — | Keep. |
| Auth caption bar | 43 high; close button 44 × 42 with a `@ox_critical_fill` hover ✓ | — | Keep. |

### 4.12 Scrollbars

| Mode | Target | Windows reference |
|---|---|---|
| Overlay (GNOME `overlay-scrolling` true, GTK's default) | slider 3 px thick when idle, **8** px under the pointer or while dragging (6 now); 2 px from the edge; radius 3; `@ox_scrollbar_thumb`; minimum length 30; transitions per M05; trough transparent when idle, `alpha(@ox_flyout, .85)` while `.hovering` | panning indicator 2 px (SB#L714); thumb min width 8, min length 30 (SB#L181–182); offset 2 (SB#L177); radius 3 (SB#L190); expanded track fill AcrylicInAppFillColorDefault (SB#L143) |
| Always visible (`overlay-scrolling` false) | trough 12 px, slider 8 px centred, radius 3, `@ox_scrollbar_thumb`, no arrow buttons | ScrollBarSize 12 (SB#L180) |
| Web app (legacy) | 9 px bar, thumb `#aaa8` with a 2 px transparent border ✓ | — |

Tag: safe.

### 4.13 Focus visuals

A focus ring shows for keyboard focus only (`:focus-visible`), never after a
pointer click. That already holds in both apps.

| Element | Target | Windows reference | Tag |
|---|---|---|---|
| Free-standing buttons (navigation, command bar, dialog, details "Open"), cards | `outline: 2px solid @ox_focus_outer; outline-offset: 1px; box-shadow: 0 0 0 1px @ox_focus_inner;` (inner 1 px ring against the edge, then a 2 px ring). Current: `outline: 2px solid accent; outline-offset: -2px`. | A 2 px primary border outside a 1 px secondary border (L-FOCUS). L-FOCUS gives the default margin as 1 px, but Button and GridViewItem set FocusVisualMargin −3 (BT#L167, GV#L149), which puts the secondary ring flush against the edge, as here. Colours CT#L258–259 / CT#L54–55. | visible, P2 |
| Rows, tiles, sidebar entries, menu items, and tabs, crumbs and caption buttons (these sit edge to edge, inside a ScrolledWindow's viewport or at the window edge, and those parents clip with `overflow: hidden`, so an outside ring would be cut off, §6.2) | `box-shadow: inset 0 0 0 2px @ox_focus_outer, inset 0 0 0 3px @ox_focus_inner;` (current: 1 px accent inside); on a selected row, add the selected edge after these | ListViewItemFocusBorderBrush = FocusStrokeColorOuter (LV#L183) | visible, P2 |
| Fields | none; the 2 px accent bottom edge is the focus indicator ✓ | TB#L164–171 | Keep. |
| High contrast (GNOME `high-contrast`) | `outline-width: 2px` everywhere; sidebar current row `outline: 1px solid @ox_accent`; pin drop block 2 px border (the current `prefers-contrast: more` rules) | — | Keep (rule 1). |

### 4.14 States summary

| Surface | Rest | Hover | Pressed | Selected | Disabled |
|---|---|---|---|---|---|
| Subtle button (bars, rows, sidebar, menus) | transparent | `@ox_hover` | `@ox_pressed` + `@ox_muted` text | `@ox_selected` | `@ox_text_disabled` |
| Title-strip button (+, caption, windows; inactive tabs per §4.1) | transparent | `@ox_title_hover` | `alpha(black, .02)` / `alpha(white, .04)` (SubtleFillColorTertiary, TV#L23 / TV#L101) | active tab `@ox_chrome` | `@ox_text_disabled` |
| Bordered button | `@ox_control_fill` + E06 | `@ox_control_fill_hover` | `@ox_control_fill_pressed` + `@ox_muted` text, border `@ox_control_stroke` only | — | fill ControlFillColorDisabled (`@ox_control_fill_pressed` in light, `alpha(white, .04)` in dark), border `@ox_control_stroke`, text `@ox_text_disabled`, no opacity (BT#L131, BT#L135, BT#L139; CT#L223 / CT#L19) |
| Accent button | `@ox_accent` | `@ox_accent_hover` | `@ox_accent_pressed` | — | `alpha(black, .22)` / `alpha(white, .16)` fill (AccentFillColorDisabled, CT#L242 / CT#L38) |
| Row or tile | transparent | `@ox_hover` | `@ox_pressed` | `@ox_selected` + edge; hover `@ox_selected_hover` | — |

All transitions follow M01.

### 4.15 Empty, loading and error states

| State | Target | Tag |
|---|---|---|
| Empty folder or no results | Centred block starting 30 % down the pane, 20 px from each side, gap 12: glyph 44 at `opacity: .6` in `@ox_muted`; title 16 / **600** in `@ox_text`; message 12 px `@ox_muted`, max width 460, wrapping; optional action as a bordered button, padding `8 20`. | visible (subtle), P3 (weight, T13) |
| Error (unreachable share, denied) | The same block with the error text and a Retry button. | Keep. |
| Loading | 2 px line across the top of the content pane: track `@ox_selected`, 25 % bar `@ox_accent` moving left to right in `1s ease-in-out infinite`; appears after 150 ms (M06); static when animations are off (M08). | safe |
| Search in progress or cached | Search-info bar under the header area: min height 40, padding `7 14`, `@ox_chrome`, 1 px bottom border, **12** px text (11), accent glyph; cache freshness on the right in `@ox_muted`. | text visible, P2 |
| Folder-size scan | Bottom bar: min height 44, padding `8 18`, `@ox_chrome`, 1 px top border, **12** px text (11), "Cancel scan" as a secondary button 27 high. | text visible, P2 (T04) |
| Transfer | Floating panel 43 px above the bottom, between the sidebar and the details pane, radius 8, shadow E08, 4 px progress track `@ox_border` with a `@ox_accent` fill. | safe |
| Toast | 78 px above the bottom, centred, background `@ox_text`, text `@ox_bg`, radius 8, padding `12 20`, 12 px, shadow E08, max width 650. | safe |

### 4.16 Landing pages (Home, This PC, Network, Settings)

Keep the current metrics:

- Page padding `25 28`; title T11; subtitle 12 px `@ox_muted`, 27 px below.
- Section title 13 / 600, margins `24 0 15`.
- Quick cards 78 high, padding 12, radius **4** (6), glyph 43, hover `@ox_hover` with a `@ox_border` edge.
- Drive cards min height 103 (native: 67), padding 18, radius 7 (R09), 1 px `@ox_border`, background `@ox_card`, glyph 46; capacity bar per C47.
- Network banner padding 20, radius 7 (R09).
- Recent table: 12 px rows with 10 px padding and 1 px `@ox_border` dividers.
- Settings: left navigation 244 wide on `@ox_sidebar`; sections max 1000 wide, radius **7** (9) (R09), padding 24.

---

## 5. Deviations

Priorities: P0 breaks the identity or is a bug; P1 is a visible gap in parity;
P2 moves the look to the Windows spec and needs sign-off if tagged *visible*;
P3 is polish.

### 5.1 Current app (`v2.0.0:desktop/ui/style.css`)

The native app is the product going forward. Fix these in the web app only
where marked, so both apps stay comparable during parity testing.

| ID | P | Tag | Deviation | Exact fix |
|---|---|---|---|---|
| D-A01 | P0 | safe | The address and search bottom edges are `#c9c9c9` in dark as well, drawing a bright line under both fields (visible at y = 89 in `explorer-dark.png`). | Add `--field-bottom:#c9c9c9` to `:root` and `--field-bottom:#5a5a5a` to `:root[data-theme="dark"]`, then use `border-bottom-color:var(--field-bottom)` in `.address` and `.search-wrap`. |
| D-A02 | P0 | safe | `--selected-border` is used by `.app-choice.chosen` but never defined, so the border computes to `currentcolor`: a text-coloured frame around the chosen app in "Open with". | Define `--selected-border:#80b4e060` in both themes. |
| D-A03 | P1 | safe | Caption, new-tab and windows-button hover is `#f0f0f0` on `#eff1f4`, so it is nearly invisible in light. | `.titlebar button:not(.close-window):hover{background:#0000000a}` and `.dark .titlebar button:not(.close-window):hover{background:#ffffff0f}`. The `:not()` is required: `.dark .titlebar button:hover` (specificity 0,3,1) would beat `.window-buttons .close-window:hover` (0,3,0) and remove the red close hover in dark mode, which would be a rule 1 loss. |
| D-A04 | P2 | visible | Text is below the 12 px Regular minimum (L-TYPE): status bar 11, menu shortcuts and headings 10, sidebar and quick-access headings 10, details properties and notes 11, search-info bar 11, cache and version metadata 9–10. | Change each `calc(10px …)` and `calc(11px …)` to `calc(12px * var(--text-scale,1))` (T04, T05). |
| D-A05 | P2 | visible | The focus ring is a 2 px accent ring drawn inside the control; Windows uses a black-and-white two-ring visual. | §4.13. |
| D-A06 | P2 | safe | Pressed state darkens (`filter: brightness(.96)`); Windows lightens and dims the text. | `button:not(.primary):not(.danger):not(.close-window):active{filter:none;background:#f3f3f3;color:var(--muted)}`, and `#303030` in dark. |
| D-A07 | P2 | safe | Radii do not match the ramp: fields 5, modal 9, Windows 11 menu 10, overlays (transfer panel, toast) 7, settings section 9, tiles 6. | §3.4, rows R02, R04, R07–R10. In-page panels stay 7 (R09). |
| D-A08 | P2 | safe | The sidebar pill is 3 × 15 with radius 4. | `.side-entry.selected:before{height:16px;top:50%;transform:translateY(-50%);border-radius:2px}`, which stays centred at every text size. |
| D-A09 | P2 | safe | Modal scrims are `#0004` and `#0006`. | `#0000004d` for both (C36). |
| D-A10 | P2 | safe | Weight 500 is not on the ramp, and headings use negative tracking. | T10–T13. |
| D-A11 | P3 | visible | Inactive tabs use primary text; tab hover lightens instead of darkening; there are no tab separators, strip line or bottom flares. | C03, C37–C39, R06. |
| D-A12 | P3 | visible | The dark accent `#74beff` is not the Windows default (`#4CC2FF`), and text on accent is `#142535` rather than black. | C14, C16. |
| D-A13 | P3 | safe | Dark menus sit on the same `#262626` as the chrome and do not lift off the content. | C07. |

### 5.2 Native capture (`native-browsing-*.png`, 1310 × 800)

**Capture caveats.** The captures were taken under Xvfb without a compositor,
so GTK used its `.solid-csd` frame, which is the 4 px grey border on every
side. Some 1 px lines are split across two half-strength pixels (the header
border at y = 203/204, the status border at y = 765/766) while others are
crisp. That suggests the image was resampled. Re-capture at the exact window
size before judging hairlines.

| ID | P | Tag | Deviation, as measured | Exact fix |
|---|---|---|---|---|
| D-N01 | P0 | visible | The title bar is 45 px (spec 42), and a 2 px grey line (`#d8d6d4`/`#dcdad6`; dark `#141414`/`#181818`) runs under the whole strip, **including under the active tab**, so the tab floats instead of joining the navigation row. GTK gives the `WindowHandle` passed to `set_titlebar()` the class `.titlebar`, and the built-in theme styles `.titlebar:not(headerbar)` as a header bar: `min-height: 46px`, a 1 px bottom border and a gradient `background` (GTK-SRC `_common.scss`). `window.csd > .titlebar` resets the border and background but not the height. The capture window was not composited, so it had `.solid-csd` instead of `.csd` (GTK-SRC `gtkwindow.c`, `gtk_window_enable_csd`), and nothing was reset. Measured in the headless session: the handle is 46 px even under `.csd`, and 42 px with the fix. | `window.ox > .titlebar { min-height: 42px; padding: 0; border: none; box-shadow: none; background: @ox_title; }` (the `background` shorthand also clears the theme's gradient image) and `.ox-titlebar { min-height: 42px; padding-left: 9px; }`. Put the 9 px padding only on the inner box; putting it on both nodes insets the tabs by 18 px. |
| D-N02 | P0 | visible | The caption buttons come from `gtk::WindowControls`, so they use the **desktop icon theme** (bold minimize, a four-arrow maximize, a heavy ×) without the 46 px full-bleed backplates or the red close hover. CSS cannot override a `GtkImage` icon name, and an app icon path loses to the desktop theme. | Replace `WindowControls` with three `gtk::Button`s (class `caption` plus `minimize`, `maximize` or `close`) holding `icons::glyph("minus" / "maximize" or "restore" / "close", 10)` and the built-in actions `window.minimize`, `window.toggle-maximized` and `window.close`. Take their order and presence from `GtkSettings:gtk-decoration-layout`, and re-read it on `notify`. Buttons named after `:` go at the end of the title bar. Buttons named before `:` go at its start, before the tabs. Ignore `menu` and `icon`. If only the part after `:` were read, a left-hand layout such as `close,minimize,maximize:` would show no caption buttons at all. Style per §4.1. |
| D-N03 | P0 | visible | The + button sits at the far right beside the caption buttons, because the tab `ScrolledWindow` is `hexpand(true)`. | `tab_scroll.set_hexpand(false); tab_scroll.set_propagate_natural_width(true);` and change its `hscrollbar_policy` from `Never` to `External`. With `Never`, the content sets the size, so the tab strip's minimum width would grow with every tab. Append `new_tab`, then a `gtk::Box` drag spacer with `hexpand(true)` and `set_size_request(48, -1)`, then the windows button and the caption buttons. |
| D-N04 | P0 | visible | The tab icon is the line glyph `folderline`; the current app uses the colour folder art. | Use `icons::art_image(&ArtKind::Folder …, 16, …)` (the drive, network or device art for those locations). |
| D-N05 | P0 | visible | **Two sort arrows** (Name ▲ and Date modified ▲). Only the primary sort column may show one. GTK 4.14.5 gives every non-primary title's indicator the class `.unsorted` (GTK-SRC `gtkcolumnviewtitle.c`, `gtk_column_view_title_update_sort`), and the built-in theme draws an arrow only for `.ascending` and `.descending`. So the second arrow comes from a stale class or another stylesheet (cause unverified). | Inspect the Date title's `sort-indicator` classes with `GTK_DEBUG=interactive` inside `/tmp/ox-headless/run.sh`, and check that `ColumnViewSorter::primary_sort_column()` is the only sorted column after `sort_by`. As a guard: `columnview.files > header > button sort-indicator:not(.ascending):not(.descending) { -gtk-icon-source: none; min-width: 0; margin: 0; }` Add a test that reads the header widgets' CSS classes. |
| D-N06 | P1 | visible | Row backplates run edge to edge across the pane (x = 226–1066) instead of inset 12 px, and the Name column starts at x = 252 (current: 242). | §6.4 recipe: `columnview.files { margin: 0 12px; }`; drop the header's own bottom border and draw the full-width header line on the parent `scrolledwindow.files-scroll` as a background gradient generated by `fonts.rs` at the header height; first cell `padding-left: 14px` (28). |
| D-N07 | P1 | visible | The Size header is left-aligned; the current app right-aligns it. | GTK 4.14 has no API for header alignment (§6.2). After the view is realized, find the last `header > button > box` and call `set_halign(gtk::Align::End)`, behind a small helper documented as depending on GTK 4.14's title layout and covered by a test. Otherwise accept left alignment, which is closer to Explorer (observed, unverified). |
| D-N08 | P1 | visible | The address and search fields are **40** px high (y = 61–100; spec 34), because GtkBox stretches its children to the row's 40 px content height. | `address_box.set_valign(gtk::Align::Center); search.set_valign(gtk::Align::Center);` Set `.navrow { min-height: 39px; padding: 11px 16px 11px 13px; }` (was 40). 11 + 39 + 11 + 1 px border = 62, as in the current app, whose content box is also 39. |
| D-N09 | P1 | visible | The search glyph is at the left and the placeholder reads "Filter this folder"; the current app reads "Search *folder*" with the glyph on the right. | Use `gtk::Entry` with `set_secondary_icon_paintable(glyph("search", 15))` and set the placeholder to `format!("Search {}", folder_name)`, following the wording rule in `native/README.md`. |
| D-N10 | P1 | visible | Command bar content differs: "New tab / Open / Sort / View … Details / Appearance ▾" against "New ▾ │ Cut Copy Paste Rename Share Delete │ Sort ▾ View ▾ ··· ⟶ Light, Settings, Details". The Details button is not highlighted while the pane is open. | Build the current command set (rule 1). Make Details a `gtk::ToggleButton`, styled `:checked` as `@ox_selected`. Appearance is a button showing the current appearance label, like `#theme-toggle`. |
| D-N11 | P1 | visible | The sidebar has no pin glyphs, no expandable "This PC" (drives) and "Network" (shares) groups, and no "Map network location" footer; the order differs (Home, This PC and Network come first). | §4.4 layout and the Python sidebar model (`v2.0.0:desktop/ui/app.js` sidebar rendering). |
| D-N12 | P1 | visible | The details pane is **238** wide (spec 262), has no Open or Pin button, no "Properties" heading or key/value grid, and shows a raw path. | §4.8. |
| D-N13 | P1 | visible | The status bar joins its parts with " · "; the view toggles are not highlighted; there is no status mode or update button. | §4.7; add class `active` to the current view's toggle. |
| D-N14 | P1 | visible | No responsive behaviour: at 990 px the capture still shows a 238 px details pane and a 218 px sidebar. The current app shows 235 / 185 there and hides the pane at ≤ 960. | Follow `.workspace` width with a `notify::width` handler (or `GtkConstraintLayout`) that applies the §4 breakpoints: 1190, 960 and 680. |
| D-N15 | P1 | visible | The open-windows button is missing. | §4.1. |
| D-N16 | P2 | safe | The sidebar default width is 220 (`window.rs`); the current app uses 210 plus a 6 px resizer. | `unwrap_or(210)`; draw the resizer per §4.4. |
| D-N17 | P2 | safe | The Size column is 90 wide (spec 78). | `SortColumn::Size => Some(78)` in `folder_view/details.rs`. |
| D-N18 | P2 | safe | `ox_pressed` is darker than hover (`#e9e9e9` / `#3a3a3a`). | C18. |
| D-N19 | P2 | safe | Menus have min width 215 (current 264 classic / 276 Windows 11), only one style, and a 25 px accelerator gap. | §4.9: add the `classic` class from `preferences.contextMenu`. |
| D-N20 | P2 | visible | Drive capacity bars are 6 px `#26a0da` with `#da2626` when full; drive cards have min height 67 (current 103). | C47; §4.16. |
| D-N21 | P2 | safe | `window.ox.solid-csd` keeps GTK's thick frame. | E01. |
| D-N22 | P3 | — | Date format "09/26/2026" against "31/08/2026". Both follow the locale, and the capture ran under a US locale. | Confirm the native code formats with the user's `LC_TIME` (GLib `%x`), as `toLocaleDateString` does. Not a style change. |
| D-N23 | P3 | safe | The tooltip, menu and details preview use `@ox_chrome`, radius 7 and min heights 146 and 30 instead of the §4 values. | §4.8–4.10. |

---

## 6. GTK 4.14 CSS notes

### 6.1 Selectors (GTK 4.14 CSS nodes)

| Widget | Selector | Notes |
|---|---|---|
| Window | `window.ox`, `.csd`, `.solid-csd`, `.maximized`, `.fullscreen`, `.tiled`, `.tiled-top`/`-left`/`-right`/`-bottom`, `:backdrop` | `:backdrop` = inactive window |
| Title bar | `window.ox > .titlebar` (the `WindowHandle`), `.ox-titlebar` (its box) | GTK adds `.titlebar` to the widget given to `set_titlebar()` |
| Custom tabs | `.ox-titlebar .tabs > .tab`, `.tab.active`, `.tab:hover`, `.tab:focus-visible`, `.tab > button.tab-close`, `.tabs:drop(active)` | A `gtk::Box` gets `:hover` without extra code. Set `gtk::StateFlags::ACTIVE` on press to get `:active`. |
| Caption buttons | `.ox-titlebar button.caption.minimize`, `.maximize`, `.close` | our own buttons (D-N02) |
| ColumnView | `columnview.files`, `> header`, `> header > button` (column title), `> header > button sort-indicator.ascending`/`.descending`, `> listview`, `> listview > row`, `row:hover`, `row:selected`, `row:focus-visible`, `row > cell`, `> listview > header` (Group by headings, VIEW-022), `rubberband` | `sort-indicator` is a built-in icon, so `-gtk-icon-source` and `-gtk-icon-size` apply |
| GridView | `gridview.files`, `> child`, `child:selected`, `rubberband` | tile size via generated `min-width`/`min-height` (`fonts.rs`) |
| Sidebar (ListBox) | `.sidebar list > row`, `row:selected`, `.current`, `.pill` | the pill is a real child widget (no `::before` in GTK) |
| Tree expander | `expander` (built-in icon) inside `treeexpander` | set the chevron with `-gtk-icon-source: -gtk-recolor(url("resource:///…/chevron-symbolic.svg"))` |
| PopoverMenu | `popover.menu.ox-menu > contents`, `modelbutton`, `modelbutton:hover`, `modelbutton:selected`, `modelbutton > check`/`radio`, `modelbutton > accelerator`, `modelbutton > arrow`, `popover.menu separator`, `popover > arrow` | set `has-arrow = false` in code |
| Entry | `entry`, `entry > text`, `entry > text > placeholder`, `entry > text > selection`, `entry > image.left`/`.right`, `entry:focus-within` | `SearchEntry` is `entry.search` |
| Scrollbar | `scrollbar.vertical`/`.horizontal`, `.overlay-indicator`, `.hovering`, `.dragging`, `> range > trough > slider` | |
| Tooltip | `tooltip.background` | |
| Paned | `paned.workspace > separator` | |
| Progress | `progressbar > trough > progress` | loading line, capacity bar |

### 6.2 What GTK 4.14 cannot do, and the workaround

| Limitation (source) | Workaround |
|---|---|
| No custom properties (`--x`, `var()`); they arrive in 4.15.1 (stable 4.16) (GTK-CSS; GTK-NEWS) | `@define-color` in per-theme providers (already done); size-dependent rules generated in Rust (`theme/fonts.rs`). |
| No `color-mix()`, relative colours or `color()`; these arrive in 4.15.2 (GTK-NEWS) | Use `alpha()`, `mix()` and `shade()`. They are only deprecated from 4.16 on. |
| No media queries: `prefers-color-scheme` and `prefers-contrast` arrive in 4.19.3 (stable 4.20), reduced motion in 4.21.1 (GTK-NEWS) | Swap palette providers from `theme::system` (already done); add a `.high-contrast` window class from `org.gnome.desktop.a11y.interface high-contrast`; rely on `gtk-enable-animations` for motion. |
| No `text-align`, `width`, `height`, `max-width`, `margin: auto`, `display`, `position` or `cursor` (GTK-CSS property list) | Label `xalign`, `set_size_request`, `max-width-chars` and `set_cursor` in code; `border-spacing` for Box gaps. |
| No `backdrop-filter`, so no Mica or Acrylic (it arrives in 4.21.2, GTK-NEWS) | Static tints: `@ox_title` stands in for Mica Alt and `@ox_flyout` for Acrylic, using the documented fallbacks where they fit (C01, C07). |
| No concave (outward) corners for the tab flare | A 4 × 4 child widget on each side of the active tab that draws the WinUI arc path (TVX#L547–548) with `gsk::PathBuilder` (GskPath is in GTK 4.14; the glyphs already use it). |
| No `::before` or `::after` | Real child widgets (sidebar pill, drop insertion line). |
| No `:has()` | Toggle state classes in code (`.active`, `.current`, `.classic`, `.editing`). |
| Negative margins are allowed: they parse, and GTK's own theme uses them (for example `margin-top: -1px` on `.solid-csd` header bars, GTK-SRC `_common.scss`). But they push a widget outside its allocation, and `overflow: hidden` parents clip it. | Restructure instead where clipping matters (§6.4). |
| Parents with `overflow: hidden` clip anything drawn outside a child. GtkWindow, GtkViewport, GtkListBase and GtkColumnView all set it (GTK-SRC `gtkwindow.c`, `gtkviewport.c`, `gtklistbase.c`, `gtkcolumnview.c`). A ScrolledWindow wraps non-scrollable children, such as the tab and crumb boxes, in a GtkViewport. | Use the inset focus ring (§4.13) for widgets that touch such a parent's edge: tabs, crumbs, caption buttons, rows. |
| A user's `~/.config/gtk-4.0/gtk.css` loads at `USER` priority (800), above the skin's `APPLICATION` providers (§2) | Leave it winning. It is the user's own GNOME customisation (rule 2). Verification captures use a throwaway HOME (§9). |
| No custom widget for column titles; alignment is fixed | D-N07 helper, or accept left alignment. |
| No shrink-to-fit tab widths (no `flex`) | A small `gtk::LayoutManager` for `.tabs` that gives each tab `clamp(avail / n, 100, 215)`. |
| `WindowControls` icons cannot be restyled | Own caption buttons (D-N02). |
| Popovers cannot animate open or closed | Accept instant (M07). |
| Shadows widen popup surfaces and are not drawn without a compositor | Keep E03 and E04 moderate. `.solid-csd` and non-composited sessions draw no shadow; the 1 px stroke still separates. |
| One `outline` per widget | Two-ring focus = `outline` (outer) + `box-shadow` spread (inner), §4.13. |
| Pango 1.52 cannot register an app font file (`pango_font_map_add_font_file` needs 1.56), and fontconfig registration needs FFI (`unsafe`, which the README forbids) | Do not bundle fonts at runtime. If Selawik is adopted after sign-off (T01), depend on the distribution package (§7.1). |

Supported and worth using: `line-height` and `text-transform` (since 4.6),
`letter-spacing`, `font-feature-settings`, `font-variation-settings`,
`filter`, multiple `box-shadow`s including inset, `outline-offset`, per-corner
`border-radius`, `transition-*`, `animation-*`, `-gtk-icon-size`,
`-gtk-recolor()`, `:drop(active)` (GTK-CSS).

### 6.3 Snippets

```css
/* 4.1 Title bar. The handle needs the `background` shorthand to clear the
   built-in theme's gradient image; the 9 px inset goes on the inner box only. */
window.ox > .titlebar {
  min-height: 42px; padding: 0;
  border: none; box-shadow: none; background: @ox_title;
}
.ox-titlebar { min-height: 42px; padding-left: 9px; background-color: @ox_title; }
window.ox:backdrop .ox-titlebar { background-color: @ox_title_backdrop; }
.ox-titlebar .tabs { margin-top: 7px; border-spacing: 2px; }
.ox-titlebar .tab {
  min-height: 35px; padding: 0 10px 0 13px; border-radius: 8px 8px 0 0;
  color: @ox_muted;
  transition: background-color 83ms linear, color 83ms linear;
}
.ox-titlebar .tab:hover { background-color: @ox_title_hover; }
.ox-titlebar .tab.active { background-color: @ox_chrome; color: @ox_text; }
window.ox:backdrop .ox-titlebar .tab:not(.active),
window.ox:backdrop .ox-titlebar button.caption { color: @ox_text_tertiary; }
.ox-titlebar button.caption { min-width: 46px; min-height: 42px; border-radius: 0; }
.ox-titlebar button.caption:hover,
.ox-titlebar button.newtab:hover,
.ox-titlebar button.windows-button:hover { background-color: @ox_title_hover; }
.ox-titlebar button.caption.close:hover { background-color: @ox_critical_fill; color: white; }
.ox-titlebar button.caption.close:active { background-color: alpha(@ox_critical_fill, .9); color: alpha(white, .7); }

/* 4.2 Fields: 32 + 2 px border = 34; valign centre in code */
.address, entry.search {
  min-height: 32px; border-radius: 4px;
  border: 1px solid @ox_border; border-bottom-color: @ox_field_bottom;
  background-color: @ox_field_bg;
}
.address.editing, entry.search:focus-within {
  border-bottom-color: @ox_accent; box-shadow: inset 0 -1px @ox_accent;
}
entry > text > selection { background-color: @ox_text_selection_bg; color: @ox_text_selection_fg; }

/* 4.3 Buttons */
window.ox button {
  transition: background-color 83ms linear, color 83ms linear, box-shadow 83ms linear;
}
window.ox button:hover { background-color: @ox_hover; }
window.ox button:active { background-color: @ox_pressed; color: @ox_muted; }
window.ox button:disabled { color: @ox_text_disabled; }
window.ox button:focus-visible {
  outline: 2px solid @ox_focus_outer; outline-offset: 1px;
  box-shadow: 0 0 0 1px @ox_focus_inner;
}
/* Clipped by a viewport or the window edge: draw the ring inside (§4.13). */
.ox-titlebar .tab:focus-visible, .ox-titlebar button.caption:focus-visible,
.address button.crumb:focus-visible {
  outline: none;
  box-shadow: inset 0 0 0 2px @ox_focus_outer, inset 0 0 0 3px @ox_focus_inner;
}

/* 4.5 Rows */
columnview.files > listview > row {
  margin: 1px 0; border-radius: 4px; transition: background-color 83ms linear;
}
columnview.files > listview > row:hover { background-color: @ox_hover; }
columnview.files > listview > row:selected {
  background-color: @ox_selected; box-shadow: inset 0 0 0 1px @ox_selected_edge;
}
columnview.files > listview > row:selected:hover { background-color: @ox_selected_hover; }
columnview.files > listview > row:focus-visible {
  box-shadow: inset 0 0 0 2px @ox_focus_outer, inset 0 0 0 3px @ox_focus_inner;
}
columnview.files > listview > row:selected:focus-visible {
  box-shadow: inset 0 0 0 2px @ox_focus_outer, inset 0 0 0 3px @ox_focus_inner,
              inset 0 0 0 4px @ox_selected_edge;
}
columnview.files > header > button sort-indicator:not(.ascending):not(.descending) {
  -gtk-icon-source: none; min-width: 0; margin: 0;
}
columnview.files > listview > row > cell { font-feature-settings: "tnum" 1; }

/* 4.9 Menus (Windows 11 style; add .classic overrides) */
popover.menu.ox-menu > contents {
  background-color: @ox_flyout; color: @ox_text;
  border: 1px solid @ox_flyout_stroke; border-radius: 8px; padding: 2px 0;
  box-shadow: 0 0 8px @ox_shadow_ambient, 0 14px 28px @ox_shadow_key;
}
popover.menu.ox-menu modelbutton { min-height: 29px; margin: 2px 4px; padding: 0 11px; border-radius: 4px; }
popover.menu.ox-menu modelbutton accelerator { color: @ox_muted; margin-left: 24px; }
popover.menu.ox-menu separator { margin: 1px 0; min-height: 1px; background-color: @ox_border; }
popover.menu.ox-menu.classic > contents {
  border-radius: 2px; padding: 3px; box-shadow: 0 5px 16px alpha(black, .2);
}
popover.menu.ox-menu.classic modelbutton { margin: 0; padding: 0 9px; border-radius: 0; }

/* 4.10 Tooltip */
tooltip.background {
  background-color: @ox_tooltip; color: @ox_text;
  border: 1px solid @ox_flyout_stroke; border-radius: 4px; padding: 6px 9px 8px 9px;
  box-shadow: 0 0 2px @ox_shadow_ambient, 0 8px 16px @ox_shadow_key;
}

/* 4.12 Overlay scrollbar */
window.ox scrollbar slider {
  border-radius: 3px; margin: 2px; background-color: @ox_scrollbar_thumb;
  transition: min-width 167ms cubic-bezier(0,0,0,1), min-height 167ms cubic-bezier(0,0,0,1);
}
window.ox scrollbar.vertical slider { min-width: 3px; min-height: 30px; }
window.ox scrollbar.horizontal slider { min-height: 3px; min-width: 30px; }
window.ox scrollbar.vertical.hovering slider, window.ox scrollbar.vertical.dragging slider { min-width: 8px; }
window.ox scrollbar.horizontal.hovering slider, window.ox scrollbar.horizontal.dragging slider { min-height: 8px; }

/* E01 Window */
window.ox.csd { border-radius: 8px; }
window.ox.maximized, window.ox.fullscreen, window.ox.tiled, window.ox.tiled-top,
window.ox.tiled-left, window.ox.tiled-right, window.ox.tiled-bottom { border-radius: 0; }
/* Keep a 5 px resize band without a compositor: border + padding (E01). */
window.ox.solid-csd {
  border-radius: 0; border: 1px solid @ox_window_stroke; padding: 4px; box-shadow: none;
}
```

### 6.4 Inset rows with a full-width header line

GTK gives the header and the rows the same column widths, so horizontal
padding inside the `listview` pushes the last column out of view, as the
comment in `style.css` notes. Instead:

1. Put the inset on the column view itself: `columnview.files { margin: 0 12px; }`.
   The ScrolledWindow still reaches the pane edges, so the scrollbar stays at
   the edge.
2. Remove `border-bottom` from `columnview.files > header`. Give
   `columnview.files`, its `header` and its `listview` `background: none`, so
   the ScrolledWindow (add the class `files-scroll` in code) paints `@ox_bg`
   and the line.
3. Have `fonts.rs` emit the header height `h` for the text size
   (`h = max(38, ceil(24·s + 10)) − 1`, set as the header button
   `min-height`) and this rule:
   `scrolledwindow.files-scroll { background-color: @ox_bg; background-image: linear-gradient(to bottom, transparent {h}px, @ox_border {h}px, @ox_border {h+1}px, transparent {h+1}px); }`
   The header stays fixed while scrolling vertically, and so does this
   background. The listview's 4 px top padding keeps rows off the line.
4. Set the first cell and the first header button to `padding-left: 14px`
   (2 row padding + 12 cell padding), so icons land 26 px from the pane edge,
   as in the current app.

---

## 7. Fonts, icons and licensing

The project is AGPL-3.0-only. Everything shipped must be compatible with it
and listed in `THIRD_PARTY_NOTICES.md`.

### 7.1 Fonts

| Font | Licence facts (source) | Decision |
|---|---|---|
| Segoe UI Variable, Segoe UI | Proprietary Windows fonts. "the redistribution of fonts supplied with Windows is generally not allowed" (L-FONTFAQ). Naming a Windows font in a font stack is allowed: "you don't even need to be a Windows licensee to include a Windows font name in a CSS font stack" (L-FONTFAQ). | Never bundle or download. Keep the names first in the stack so a legitimately installed copy is used. |
| Selawik | Microsoft, SIL OFL 1.1, **Reserved Font Name "Selawik"**; "an open source replacement for Segoe UI"; known issues: "missing kerning to match Segoe UI", "needs improved hinting" (SEL). Covers Latin only (fc-query on Zorin's copy: no Greek or Cyrillic). Zorin's copy has no kerning at all and no `tnum` (§2). Weights: Light, Semilight, Regular, Semibold, Bold. | Not in the default stack (T01). It is a sign-off candidate, **visible** on Zorin, which ships it (`fonts-selawik-zorin-os`); elsewhere text would stay Noto Sans. Judge the missing kerning at 12 px before adopting it. Do not bundle at runtime (§6.2). Packages may recommend the distribution's Selawik; a bundled copy must stay unmodified (any subset must be renamed, per the RFN), with `OFL.txt` in `licenses/` and a notice entry. Non-Latin text falls back to Noto per glyph. |
| Noto Sans | SIL OFL 1.1, a system font | Stays the face that actually renders on Zorin, which is today's look (T01). |
| Desktop font (Inter on Zorin) | system | Optional setting "Use desktop font" (gain, §8). Not the default, because it would change the identity. |

### 7.2 Icons

| Set | Licence facts (source) | Decision |
|---|---|---|
| Segoe Fluent Icons, Segoe MDL2 Assets | "You can download the font for use in design and development, but you may not ship it to another platform." (L-SFI). WinUI's symbol font is `Segoe Fluent Icons,Segoe MDL2 Assets` (GX#L20). | Never bundle or reference. Where the spec names glyph code points (E921 and so on), it names shapes to imitate, not glyphs to use. |
| Fluent System Icons | MIT (FSI). Microsoft's Fluent 2 product icons: SVGs drawn as filled shapes; sizes vary per icon (for example, Dismiss comes at 12/16/20/24/28/32/48), in regular and filled styles and a few light ones. The same visual family as Segoe Fluent Icons, but not the same glyphs. | Compatible with AGPL-3.0 (keep the MIT notice). **Not adopted wholesale.** The project's own stroke glyphs are the identity and already match the 1 epx monoline rule (L-ICON) once I10 is applied. Individual glyphs may be replaced where the own drawing is weak (settings gear, share/open-in, rename, pin): vendor only the SVGs used, as `-symbolic` resources recoloured by GTK, and add an MIT notice. visible, P3. |
| Own stroke glyphs (`paths` in `app.js`, `icons/glyphs.rs`) | project-original, AGPL | Keep as the source of truth. Apply I10 (1 px stroke). Add dedicated 10 × 10 caption glyphs (I04) and a 10 px sort caret. |
| Own colour art (`folderIcon`, `zipFolderIcon`, `fileIcon`, `networkIcon`) | project-original; the folder art comes from the upstream Winspace project (MIT, `licenses/Winspace-MIT.txt`) | Keep exactly. Explorer's artwork is proprietary and must not be copied. |
| Desktop icon theme | system | For application icons only ("Open with" lists, from `GAppInfo`), and file types the own art does not cover, as an option (§8). |

---

## 8. GNOME integration and the Dolphin bar

Rule 2 asks for GNOME's own mechanisms. These change what appears on screen:

| Mechanism | Visual effect | Status |
|---|---|---|
| `org.gnome.desktop.interface color-scheme`, or the portal's `org.freedesktop.appearance color-scheme` | Light and dark palette | done (`theme/system.rs`) |
| Portal `org.freedesktop.appearance accent-color` (GNOME 47+; absent on Zorin 18.1) | Optional "Use desktop accent": derive `ox_accent` for light (darker) and dark (lighter) from it | gain, off by default |
| `org.gnome.desktop.a11y.interface high-contrast` | `.high-contrast` window class: §4.13 rules, 2 px strokes | must keep (rule 1: the current `prefers-contrast: more` rules) |
| `gtk-enable-animations` (GNOME "Reduce animation") | M08 | must keep |
| `gtk-overlay-scrolling` | §4.12 overlay or always-visible scrollbars | automatic |
| `gtk-decoration-layout` | Which caption buttons appear, and their order (D-N02) | to do |
| `org.gnome.desktop.wm.preferences action-double-click-titlebar` | Title-bar double-click | automatic through `WindowHandle` |
| `org.gnome.desktop.interface font-name`, `text-scaling-factor` | Optional desktop font; default text size seeded from the scaling factor when the user has never set one | gain |
| Freedesktop thumbnail cache, GNOME thumbnailers | Thumbnails in tiles and the details preview (§4.6, §4.8) | gain (Dolphin parity) |
| `GAppInfo` icons | Real application icons in "Open with" | gain |
| xdg-desktop-portal `org.freedesktop.impl.portal.FileChooser` backend | Other applications' Open and Save dialogs drawn as this window in picker mode: the caller's title in place of the tabs, and a bar under the status bar on `@ox_chrome` with right-aligned labels ("File name:", "Save as type:"), 30 px fields, and 80 px accent and bordered buttons at the bottom right, as Windows' common file dialog (`skin/picker.css`) | done, opt-in (INT-032) |

Dolphin features the app does not have yet must still fit the skin. When they
are built, use these rules:

| Dolphin feature | Where and how it looks |
|---|---|
| Split view | Two content panes separated by 1 px `@ox_border`. The inactive pane's header labels use `@ox_text_tertiary`; the active pane keeps them `@ox_muted`. No extra chrome. |
| Filter bar | A strip below the column header, styled as the search-info bar (§4.15), with a 34 px field. |
| Zoom | Ctrl + wheel and Ctrl + +/−, already mapped to text size (80–200 %) and icon size. There is no status-bar slider, because Explorer has none. |
| Free space | "N GB free" at 12 px in the status bar's left group, after the selection text. |
| Selection checkboxes | Optional 16 × 16 checkbox before the row icon, shown on hover and for selected rows (Explorer "Item check boxes", observed, unverified). |
| Hidden files | View ▾ toggle. Hidden entries at `opacity: .6`, like the cut style. |
| Terminal panel | Bottom pane with the status-bar treatment: 1 px `@ox_border` top, `@ox_bg`. |

---

## 9. Verification

Rule 1 needs proof that nothing is lost. Rule 3 needs proof that the look
holds.

1. **Band metrics** (automatable with the pixel-run method used here): at
   1440 × 900 and 100 % text size, the title bar is 42, the navigation row 62
   and the command bar 55; the column header is 38; the row pitch is 38
   (compact 28); the status bar is 30; the sidebar is 210 + 6; the details
   pane is 262. Fields are 34, tabs 35 × 215 at y = 7, caption buttons 46 × 42. Check
   the same at 1190, 960 and 680 against the §4 breakpoint values, and at
   150 % and 200 % text size against the formulas.
2. **Colour probes**: sample C01, C04, C05, C06, C19 and C22 at fixed points
   in light and dark. Allow ±1 per channel for the solid tokens.
3. **Side-by-side captures** of the current app and the native app, from
   the same fixture folder (fictional names, per `AGENTS.md`). Views: details,
   icons, context menu (both styles), a dialog, empty folder, loading, and an
   inactive window. Capture with `/tmp/ox-headless/run.sh`, which never uses
   the live desktop. Capture at the exact window size, with no resampling
   (§5.2 caveat).
4. **State checklist**: hover, pressed, selected, selected + hover, keyboard
   focus, disabled, cut, drag source, drop target and rubberband, for rows,
   tiles, sidebar entries, tabs, crumbs and menu items.
5. **Integration checklist**: flip `color-scheme`, `high-contrast`,
   `enable-animations`, `overlay-scrolling` and `gtk-decoration-layout` in the
   headless session's private GSettings backend (never the user's) and
   re-capture.
6. Every *visible* item in §5 ships with its before/after pair for sign-off.
