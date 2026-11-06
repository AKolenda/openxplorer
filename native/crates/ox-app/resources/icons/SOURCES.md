<!-- SPDX-License-Identifier: AGPL-3.0-only -->
# Icon sources

Every icon the native app shows is one of these files, copied byte for byte
from its upstream set; none is drawn by the app or edited here. The product
owner approved the set and its mapping on 2026-09-27.

Added on 2026-11-03 and 2026-11-04 to match Windows 11 File Explorer's
"…" and View menus (CMD-006, CMD-007), from the same pinned package and
awaiting the owner's review: `wrench_20_regular` (Properties), `rectangle_landscape_20_regular` (Extra large and Large
icons), `text_column_two_20_regular` (List), `panel_left_20_regular`
(Navigation pane), `arrow_autofit_height_20_regular` (Compact view),
`checkbox_checked_20_regular` (Item check boxes),
`panel_right_gallery_20_regular` (Preview pane) and `circle_small_20_filled`
(the dot that marks a chosen view, sort or open pane). Added on 2026-11-06,
also awaiting the owner's review: `apps_list_detail_20_regular` (Tiles) and
`content_view_20_regular` (Content). Undo, Redo and Pin
are drawn in Windows' blue (#0078d4, or #60cdff in the dark theme) by the menu's CSS, and Redo shows
`arrow_undo_20_regular` mirrored by it (`-gtk-icon-transform`), as Windows
11 draws them; the files are not changed. Microsoft publishes no colour
(`_color`) Fluent design of these, in 1.1.343 or in 1.1.345 (the newest on
2026-11-04).

**Not Fluent:** `ox-select-all-24-symbolic.svg`,
`ox-select-none-24-symbolic.svg`, `ox-invert-selection-24-symbolic.svg`,
`ox-options-24-symbolic.svg`, `ox-add-network-location-24-symbolic.svg`,
`ox-map-network-drive-24-symbolic.svg` and
`ox-disconnect-network-drive-24-symbolic.svg` (Select all, Select none,
Invert selection and Options in the "…" menu, and This PC's Add a network
location, Map network drive and Disconnect network drive) are original icons
made for OpenXplorer to match Windows 11's command-bar glyphs, with no
third-party source, under the project's licence (AGPL-3.0-only). Their
drawings are in `native/icon-sources/` (see the README there), and each file
here is `native/tools/expand_icon_strokes.py`'s output for its drawing: the
same shapes, with strokes written as fills and grey as the foreground, so
GTK can recolour them, and the blue kept through its icon palette. As
CONTRIBUTING.md asks for Fluent icons only, they await the owner's
acceptance.

The files use the `hicolor/scalable/<context>/` layout of a GTK icon theme and
are compiled into the application as a GResource (`icons.gresource.xml`, built
by `build.rs`). Every name starts with `ox-`, so no desktop icon theme can
replace one. Monochrome glyphs end in `-symbolic`, which makes GTK paint them
in the CSS `color` of their widget; colour art keeps its own colours.
`src/icons/icon.rs` is the only place in the code that names these files.

| Set | Version | Licence | Licence text |
|---|---|---|---|
| [Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons) | npm `@fluentui/svg-icons` 1.1.343 (tarball SHA-1 `2b74740b18af25923a00061d80a00245a207f5f9`) | MIT, © 2020 Microsoft Corporation | `licenses/Fluent-UI-System-Icons-MIT.txt` |
| [Fluent Emoji](https://github.com/microsoft/fluentui-emoji) | commit `1ffb34c752ecf5d402f04cfb4b392c77f57c54bc` | MIT, © Microsoft Corporation | `licenses/Fluent-Emoji-MIT.txt` |

The npm package ships no licence file, so the Fluent UI System Icons licence
text is the repository's
([LICENSE](https://github.com/microsoft/fluentui-system-icons/blob/main/LICENSE)),
copied unedited; the Fluent Emoji text is `LICENSE` at the commit above.

An app file name is the upstream file name with `ox-` in front and dashes for
underscores, with the style in GTK's convention: `_regular` becomes
`-symbolic`, `_filled` becomes `-filled-symbolic`, and `_color` and `_flat`
stay. Upstream paths are relative to the npm package for Fluent UI System
Icons and to the repository root for Fluent Emoji. The SHA-256 is of the file
here, which equals the upstream file's; a test in `src/icons/icon.rs` checks
it.

| File (under `hicolor/scalable/`) | Set | Upstream file | SHA-256 |
|---|---|---|---|
| `actions/ox-add-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/add_20_regular.svg` | eace2d0d89e66e50d5c93cf82d21eb9e73a452dc370a76846d1f1802788dd71c |
| `actions/ox-add-network-location-24-symbolic.svg` | OpenXplorer, original | `native/icon-sources/add-network-location.svg` | d9eed91ea0f0973b2c3724e1c6f73a040a19371ebe2be4b455ba8e12b7f373b6 |
| `actions/ox-apps-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/apps_20_regular.svg` | 0ad15fd4cca3d66e5221e3533d1fafd4d705ebdbc68e74e0674a72106fe627ba |
| `actions/ox-apps-list-detail-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/apps_list_detail_20_regular.svg` | 89dfd154ac8e78a6b166f67bd5f076a63c10b4d00f2a80c44e8832fb1643a1d6 |
| `actions/ox-arrow-autofit-height-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_autofit_height_20_regular.svg` | 6c520e2e07a8b86ced98036b6b5f9cf3018a1a153d80c8be9842770f78990552 |
| `actions/ox-arrow-clockwise-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_clockwise_20_regular.svg` | e216c5859ca5368c3ce795d1ffaf19b4be5e5ba2df6d09e7ff544d3c2fef0466 |
| `actions/ox-arrow-counterclockwise-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_counterclockwise_20_regular.svg` | 932290054023ca03b2f567d1c351d5d53fc1b73a312cc8df9902c6dcc09aea1e |
| `actions/ox-arrow-down-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_down_20_regular.svg` | c1548448de9cc56a3569bb51e54e954337eff72fa6d6e68a910c1586033780f8 |
| `actions/ox-arrow-eject-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_eject_20_regular.svg` | c308ecfb2513a7def3bcd991c233b3f1e32106adfda35ea048eb95323540269b |
| `actions/ox-arrow-left-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_left_20_regular.svg` | d94a8b83b6764b02c7e80a80d7854b37eaa2ce0b505f289ed2a0e3b2a1d2f82c |
| `actions/ox-arrow-redo-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_redo_20_regular.svg` | 8f73a88da11b8732934b8136c55e8bab7470f81dedaf3aa11019ac5f472d7564 |
| `actions/ox-arrow-reset-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_reset_20_regular.svg` | 0b1fff894154d44bdf4a6e2645f3a2dfa9d1389ff8b1deb8f973095fe37532a1 |
| `actions/ox-arrow-right-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_right_20_regular.svg` | be74746338292cc2bb120ab6fa119bf8d8065e4bdcd67baef406c3444368b08d |
| `actions/ox-arrow-sort-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_sort_20_regular.svg` | ecfbd11d50c36da825902f00639c0cc02a3990a3d4c5fe7d185537935a92505d |
| `actions/ox-arrow-swap-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_swap_20_regular.svg` | e7e6de1e4d107a047e20cb8952d048643cb954ea7fe4e55d556ea894387eafd2 |
| `actions/ox-arrow-undo-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_undo_20_regular.svg` | 6ddfab37900746fb49d4acae7296e154274fd12f4faad3bf41fdeab66ae0bb39 |
| `actions/ox-arrow-up-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_up_20_regular.svg` | 895cf1471c758a2e3ed183ac90f3f58566ae6c792228b742c2303eac34cfdf6e |
| `actions/ox-bluetooth-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/bluetooth_20_regular.svg` | 8aca45dde436287bce2568ab6ecc281250606502a33565872e7098af3efef2d7 |
| `actions/ox-braces-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/braces_20_regular.svg` | 831e9dbe8a711887b0212ccaedca58056f72121d16a8c0fc2b7061dac770d29b |
| `actions/ox-checkbox-checked-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/checkbox_checked_20_regular.svg` | e283886c405bba6ffc78d236a38e211360e7d02b16e446d2eba71e9f8697bcc3 |
| `actions/ox-checkmark-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/checkmark_20_regular.svg` | 9ad90a13e5d3dd6309544d09a14817b7e609015db6e6c60ecd44ef120d2b3470 |
| `actions/ox-chevron-double-left-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/chevron_double_left_16_regular.svg` | 9bb0f038fd84313dce771fdcd12500fb54bed2dc4e94916b5101a8e0d7a3b10c |
| `actions/ox-chevron-double-right-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/chevron_double_right_16_regular.svg` | 67f59dacb708a2662ffedef5c63e360e08ec225af5959ca87a9ca0cfc862c2ba |
| `actions/ox-chevron-down-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/chevron_down_16_regular.svg` | 33efa668d236d70a945c00cbbd3fda019994794a5b5a2e17f5e66aac19abd199 |
| `actions/ox-chevron-down-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/chevron_down_20_regular.svg` | 2584741754fe0e27641d7423ff96e675167119889fbc1e5129516973cbfb136a |
| `actions/ox-chevron-left-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/chevron_left_16_regular.svg` | 51d4eeb4a620aec50aa0aed834830f5d7e035203c0cf15f87cf11a8e76f56bab |
| `actions/ox-chevron-right-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/chevron_right_16_regular.svg` | 44acaeb0e0f9c84a14c7ad635e9d11424e524a4f1b140893281e2329e94767ab |
| `actions/ox-chevron-up-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/chevron_up_16_regular.svg` | 540041d671dfb1c7815bde83212ad161f64100007296adb1246bafe109048fb9 |
| `actions/ox-clipboard-paste-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/clipboard_paste_20_regular.svg` | ec9306a57faababc7c2ea24526b65109d68337b71ef2b3af8edf5eb54e04f386 |
| `actions/ox-code-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/code_20_regular.svg` | 856f7cf4747d46ede53514aac276388bbfd2a09c08dea33e64781ca381f2b152 |
| `actions/ox-content-view-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/content_view_20_regular.svg` | 9aeafda6180a243f57fcfdaad1b6b3bf9121a9103243a401e8a11869f5a7f2f9 |
| `actions/ox-copy-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/copy_20_regular.svg` | 069105eaa258ab181db8e66234677d149d26d3e8caed4e034ed6cfdf0c9511f9 |
| `actions/ox-cut-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/cut_20_regular.svg` | 22e7e4781d227d384403a319309f10bfb5c73776bd1eb2804a4692689f44e753 |
| `actions/ox-delete-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/delete_20_regular.svg` | 117dd937dfea155b95a157489089b658bb0611c736cbad4e5339c8470d5a3b93 |
| `actions/ox-delete-dismiss-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/delete_dismiss_20_regular.svg` | 7b5d57fe5c168ce7cff9429e561043c0e4c4edc6806aeab9b408e45ce811bd56 |
| `actions/ox-disconnect-network-drive-24-symbolic.svg` | OpenXplorer, original | `native/icon-sources/disconnect-network-drive.svg` | 621fb43678342ff95809cda41aa8b68db7e644df7f0c34af46acfa153d410811 |
| `actions/ox-dismiss-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/dismiss_16_regular.svg` | 16ad1ebc24e77252d69c5b2a8c036ce428621d4045cc3ac64599267c604dd0c6 |
| `actions/ox-dismiss-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/dismiss_20_regular.svg` | 3d95fdce4bb3a29b983377648e5528d73d965ed90409bce5fa096ef790b75ccb |
| `actions/ox-document-add-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/document_add_20_regular.svg` | 291dad1da68908c2a3eb190f2fb95be01281e82744a27c1b0f72a941ad1a1ca3 |
| `actions/ox-document-copy-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/document_copy_20_regular.svg` | 3cd4ddb3473b836b3cf8954e3ec6592a149cd9680a201ee8bf719bf5d60d8433 |
| `actions/ox-document-text-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/document_text_20_regular.svg` | 4c1e89ff1d85152da67997db13c68af58a9b930de199e9a91922aa7b6a8dc7cb |
| `actions/ox-eye-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/eye_20_regular.svg` | 453fff6c012ee96342c6de537404ee2927f4b6f40515c26dca890eb2f3b36327 |
| `actions/ox-folder-add-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/folder_add_20_regular.svg` | ce74be6268ef151cc9accc0452df91f8164c3f8389c14fa7473a66fd15389661 |
| `actions/ox-grid-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/grid_20_regular.svg` | e39560fcf33db8035a28269765f079779c6938dbc8a51bc80f00a7293878c825 |
| `actions/ox-history-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/history_20_regular.svg` | 44499869f85aa37f46046e9652d5912a3f9a1ce2c4b835dd3c556d23cb664ef7 |
| `actions/ox-invert-selection-24-symbolic.svg` | OpenXplorer, original | `native/icon-sources/invert-selection.svg` | da9aa32ff3d024f44bd9ea1b1177889f331bc52c1717bbcd9acb2a5b79f11655 |
| `actions/ox-link-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/link_20_regular.svg` | 0d7ca18928ccb48ddc7dc5eb69102c49e1a83cc4aa8d7a455bf600ecdc3b5b69 |
| `actions/ox-mail-20-color.svg` | Fluent UI System Icons 1.1.343 | `icons/mail_20_color.svg` | 7a75cf8f2aa390adeeb5836ee8a95eb1ab9d1a5084d6c5eab51b408cc9afda12 |
| `actions/ox-map-network-drive-24-symbolic.svg` | OpenXplorer, original | `native/icon-sources/map-network-drive.svg` | a40b3c0149867e82b1d1e8d85ba32c317aa0de3da79b76318763b597945f09b4 |
| `actions/ox-markdown-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/markdown_20_regular.svg` | d403eefaecab68909090c42910f354cfeb7a63d9085437d8cbf9f7b333e149fe |
| `actions/ox-maximize-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/maximize_20_regular.svg` | 7f3aec5319b7e66c5f088f26bb8b0c1135496757f999aa421b1860414ce0e84f |
| `actions/ox-more-horizontal-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/more_horizontal_20_regular.svg` | d700fdb05f2b48ed931352413ce2fd110fbbaa77f661858568312d31aadd2cdf |
| `actions/ox-open-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/open_20_regular.svg` | ac1788bb4ce4e3fa50668a6569ac0021a5da1d27c4c3035cceb1ba26eda99584 |
| `actions/ox-options-24-symbolic.svg` | OpenXplorer, original | `native/icon-sources/options.svg` | 56e1eb45a9c7f62dc6a21245e606e3e843a99834f3a37bfe3e04066053c79545 |
| `actions/ox-paint-brush-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/paint_brush_20_regular.svg` | 208ec8628a886146383d3a520f1beff702d3b904cd9eabe2152570b6e8d172a0 |
| `actions/ox-panel-left-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/panel_left_20_regular.svg` | f31a8664092ca1bfb4fdb6809c936ab3e4109063641f8548f013bbe869a65461 |
| `actions/ox-panel-right-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/panel_right_20_regular.svg` | a4830810b33a540cc4e7d78a355b7bafbe43f1893e344efd3961589c91a59625 |
| `actions/ox-panel-right-gallery-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/panel_right_gallery_20_regular.svg` | 97f8ee2e1e8ecc0be7cb7ee9293a4087213266b2ea5873e6a4d28527ea445ce5 |
| `actions/ox-pause-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/pause_16_regular.svg` | f7dca0915417f45c18b04d0cef2f70138d8a3670117d9a651bd080d7359ca410 |
| `actions/ox-play-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/play_16_regular.svg` | 14deab264d598f2942eb0c7df63f7fc5e51a9ee0f5347408e9b385344143c1db |
| `actions/ox-rectangle-landscape-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/rectangle_landscape_20_regular.svg` | 0ff1b572b52110eebe5f13c6ee3fa7b0ad8b0edbba55f4bde353f221ac304f87 |
| `actions/ox-rename-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/rename_20_regular.svg` | e4cd064a10bd0345430f78eccb012c966502bc6285ff70c9ee23962d52836f47 |
| `actions/ox-search-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/search_20_regular.svg` | bdcaed7e5348ba774acf854b840ef8bcf30c024bf5d3bf50b1e08d3aac8aab17 |
| `actions/ox-select-all-24-symbolic.svg` | OpenXplorer, original | `native/icon-sources/select-all.svg` | db931e919e6ce6933dbed59895ad40b935869f17af47ce68d1270a98f7dbaad4 |
| `actions/ox-select-all-off-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/select_all_off_20_regular.svg` | bb1a786c70234a30f5f26f92c3d885a7a91159d9b31c1a0c4fa6a9505b2b910f |
| `actions/ox-select-all-on-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/select_all_on_20_regular.svg` | 199464932860850ba995faf317487f035e08707fd062dc682607deb69ec261f9 |
| `actions/ox-select-none-24-symbolic.svg` | OpenXplorer, original | `native/icon-sources/select-none.svg` | d5c479e248d083537d2c60fa4272956f7b33826bc39ae99d9853da2ad679552b |
| `actions/ox-settings-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/settings_20_regular.svg` | 082803991d9af2fdb934957c5c8cddb04885665c7669d73d0c3d3dfee7ac7e6a |
| `actions/ox-share-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/share_20_regular.svg` | c0f5d5c7dd6ea4bb52da116b5809256f349384b24339945891164b6854f98c5a |
| `actions/ox-square-multiple-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/square_multiple_20_regular.svg` | de601d733420494510b8129cc413f76463435f7366a727faefde9b9e9a457a9f |
| `actions/ox-subtract-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/subtract_20_regular.svg` | a724aae5ef54510875cf0b1dda4b49e6bb39dcb9f44dad450cdb1f64d5feb85b |
| `actions/ox-table-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/table_20_regular.svg` | 01abb91aad998f162fd39dd5989844c43afd428597b9824dcb7ba1e2cd965572 |
| `actions/ox-text-bullet-list-ltr-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/text_bullet_list_ltr_20_regular.svg` | e3e8e7fc40d3b9a39d731dc8c6886551d489329f792df8f20c80ffed78f3bcee |
| `actions/ox-text-column-two-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/text_column_two_20_regular.svg` | 8b692956fd7b4e8a1cb26ec17fe0cd1d5976914752e673d8d1d08e205b34fcfd |
| `actions/ox-window-console-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/window_console_20_regular.svg` | 6d198e13d313e59254a22ccc759fcf47bb32a9f73ae68a075004e0d6ca9aa579 |
| `actions/ox-window-multiple-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/window_multiple_20_regular.svg` | 6f885abfd54d205612317a62c5bd8cf9017783a817eca5e141fdc7ba5136968f |
| `actions/ox-window-new-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/window_new_20_regular.svg` | c64f483ea659f4446541cbf77e6712ba8e28673cdc7594e11a56afa44d263273 |
| `actions/ox-wrench-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/wrench_20_regular.svg` | 35032e75227053783c0a35d87fdd3e99a8287b57175876e4ea04304e305ce554 |
| `devices/ox-hard-drive-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/hard_drive_20_regular.svg` | 04551457489e67a9b568cc7d2f710a26776d3675ee4dd92b4535692085104b35 |
| `devices/ox-laptop-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/laptop_20_regular.svg` | e0ff6f7f08ec6ed92b4264dab67f248627f5b12abbbec7eb231b1029f7469085 |
| `devices/ox-phone-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/phone_20_regular.svg` | 3ee7532f1327fcd58e57b51fd523ef4bc781780bd9cd59b60fc67e02f10554f0 |
| `devices/ox-server-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/server_20_regular.svg` | 13fb53a15a383d600d24062d14a92b6df8e47d65f01b81af23cc47c54009b7a7 |
| `emblems/ox-circle-small-20-filled-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/circle_small_20_filled.svg` | 2c66febb23b8b5f7233500418e110ecb83c3b3afccefd6f86cc784f99a184bfa |
| `emblems/ox-dismiss-circle-16-filled-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/dismiss_circle_16_filled.svg` | 9e28ebc2b04e1a528968f227ba83d5a54ea2657305ef0cafd44440a93917a3bf |
| `emblems/ox-folder-zip-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/folder_zip_20_regular.svg` | 7fa45a1da754ab612939a3806d6a5079642c0a2d897db59da2f9be5034e8870c |
| `emblems/ox-pin-16-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/pin_16_regular.svg` | 4294f30fef80b1ad90e746cb161321be97ff3ea40b4316812619624aab6a12a2 |
| `mimetypes/ox-code-20-color.svg` | Fluent UI System Icons 1.1.343 | `icons/code_20_color.svg` | 67ccde98fd9e13659526f6a1bc34b9da75aeec62f383c245982fec2485a2f354 |
| `mimetypes/ox-code-24-color.svg` | Fluent UI System Icons 1.1.343 | `icons/code_24_color.svg` | b3b2f8ab097efd98de249f1a3878fb71d4b76b5fc8005823c62733d8ac80c7e1 |
| `mimetypes/ox-document-20-color.svg` | Fluent UI System Icons 1.1.343 | `icons/document_20_color.svg` | 0cb9dc43eb3d8035cfae7cc231846a1d534fbe3b205218514551b3e307339bb7 |
| `mimetypes/ox-document-32-color.svg` | Fluent UI System Icons 1.1.343 | `icons/document_32_color.svg` | 9a221f4e3a7d4eadeb1246738d112f603b7b4a3fe51c3c0c4aa26fa4c1efd6e5 |
| `mimetypes/ox-document-48-color.svg` | Fluent UI System Icons 1.1.343 | `icons/document_48_color.svg` | 1fae3603f9a4455a56dab69ef95c7e56b8960e96a5f392d6f09b48f17e808f59 |
| `mimetypes/ox-document-text-20-color.svg` | Fluent UI System Icons 1.1.343 | `icons/document_text_20_color.svg` | ad4ebf18e4821533f0b344d0285d0476be70defb2fb92522cf03954da80bcacf |
| `mimetypes/ox-document-text-32-color.svg` | Fluent UI System Icons 1.1.343 | `icons/document_text_32_color.svg` | b8e540e01c44f222f9548a698ddc0c7747d2a22875aa5e7c0b2e4404a2b9a3d4 |
| `mimetypes/ox-document-text-48-color.svg` | Fluent UI System Icons 1.1.343 | `icons/document_text_48_color.svg` | 2f24ece3d9297b3a3af63aefa649f0f6c884f3d155d580b6d5b0ab48908a56a1 |
| `mimetypes/ox-headphones-20-color.svg` | Fluent UI System Icons 1.1.343 | `icons/headphones_20_color.svg` | c483604d4aef17cfa21bc3817b4cb42e5ac0802292e4553b4947039330cba267 |
| `mimetypes/ox-headphones-32-color.svg` | Fluent UI System Icons 1.1.343 | `icons/headphones_32_color.svg` | 68efdfb61c052e96a965c22684fb4c65bd1d8e42d726853c7a09e95e6e0916ba |
| `mimetypes/ox-headphones-48-color.svg` | Fluent UI System Icons 1.1.343 | `icons/headphones_48_color.svg` | 097c85b072ca3d2eefe75ea25b78bcee05d8b533c2da75d9a7751786e3f7d7c6 |
| `mimetypes/ox-image-20-color.svg` | Fluent UI System Icons 1.1.343 | `icons/image_20_color.svg` | 90f913d66f42c521156d20605239ef17eb928917745eff1543fcc5086a06189f |
| `mimetypes/ox-image-32-color.svg` | Fluent UI System Icons 1.1.343 | `icons/image_32_color.svg` | 0f90593b22e8abfe427f7f60a797c9dfc038fa272898238b2fef0b85bd944988 |
| `mimetypes/ox-image-48-color.svg` | Fluent UI System Icons 1.1.343 | `icons/image_48_color.svg` | 094d17f2c293ab9fdc104f57d258c4500366659012c1fa977dfb69be9779ef0e |
| `mimetypes/ox-table-20-color.svg` | Fluent UI System Icons 1.1.343 | `icons/table_20_color.svg` | 4ac5d73babe5565e7c684244e7cc808fdc2754ef5730e761df505a543b09eb67 |
| `mimetypes/ox-table-32-color.svg` | Fluent UI System Icons 1.1.343 | `icons/table_32_color.svg` | 951b7fd400e8c77dba19bfecd9f0bc186db5c93f6ec9ef207daba3eb7b54bca4 |
| `mimetypes/ox-table-48-color.svg` | Fluent UI System Icons 1.1.343 | `icons/table_48_color.svg` | 88d5dc3d99d6ed460da082af03f8681b1267b615d36d63635babb1b36f48db21 |
| `mimetypes/ox-video-20-color.svg` | Fluent UI System Icons 1.1.343 | `icons/video_20_color.svg` | 35229cc6d92621f9d26137be6af04e88430ac1207f1f18ea6361f7374a8ab630 |
| `mimetypes/ox-video-32-color.svg` | Fluent UI System Icons 1.1.343 | `icons/video_32_color.svg` | 912cc61ef867de0a02b09a1342e55249db7d29b3287af0700619b3917bcd9873 |
| `mimetypes/ox-video-48-color.svg` | Fluent UI System Icons 1.1.343 | `icons/video_48_color.svg` | bb04990432790ab4281e5e423ddf2f69df3b7a752a50459fc1291e5664e81f90 |
| `places/ox-arrow-download-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/arrow_download_20_regular.svg` | cc965b2de3fca74d76e48d93fb2a2c68756e1fab748bb06de904a8e17ca4e5c2 |
| `places/ox-desktop-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/desktop_20_regular.svg` | 941d16c07f8f5468d0e2461710b7806affd20e6604f9be15374548370bcdd9e1 |
| `places/ox-document-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/document_20_regular.svg` | 5f5416510d8a774d427a05e8102dd878aa7317478644480a57a9ab50a1104650 |
| `places/ox-file-folder-flat.svg` | Fluent Emoji 1ffb34c | `assets/File folder/Flat/file_folder_flat.svg` | 870727aa9758f76359181c035456aba47e87d622a619a0ee80fb60b40070fbd1 |
| `places/ox-folder-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/folder_20_regular.svg` | d144cc84fe8997e44aede97843ba009454b5302e9d9827e01c6540e8eef4aaa8 |
| `places/ox-home-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/home_20_regular.svg` | dac78c102bf3fbd572d213673fc45c4cee38208fd80d42a12fc34ae94016b069 |
| `places/ox-image-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/image_20_regular.svg` | 0e57f2cad97fe254f56e0997f877e1bb902efc9e2fc9b14f2091d1ce7301226c |
| `places/ox-music-note-2-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/music_note_2_20_regular.svg` | e23841dcf78e78a5b5f624ab8cd702836e485a21216ba8e5058265f1f3522c69 |
| `places/ox-organization-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/organization_20_regular.svg` | b996c1ee7354c2b5bbed026cf8cb6e51298876381954cfb2d63372f1719937bd |
| `places/ox-video-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/video_20_regular.svg` | 0d08b2e62ce110c379e9f726a954a81f8ee8eeb279d60e2eae4ebcea8a025365 |
| `status/ox-clock-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/clock_20_regular.svg` | fd5202fb96544171c260e8541138dfb49c29a7db3c1801cdc5a15333f673e067 |
| `status/ox-info-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/info_20_regular.svg` | b2171313057fcd3cffca9c20cf905a05977e63eebe5275bab4371b1f0d4a1ff8 |
| `status/ox-shield-lock-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/shield_lock_20_regular.svg` | 1ee2491675efa4f0882313130035a5ae4bcf7b3a0634fe33f60ddf30390dbc2b |
| `status/ox-star-20-filled-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/star_20_filled.svg` | cc9978158a2dc62250a2321e1ba54d6f96108e3ec739eef52ed48c628604cbac |
| `status/ox-star-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/star_20_regular.svg` | ed9f5a8c58c4a2b0f240467fe5f63d5849425004971214bee6ba8afbced27e7d |
| `status/ox-weather-moon-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/weather_moon_20_regular.svg` | b2d3062ea927710445bb892b35988e106978fc57fe37ede9cf37549eb0d58cd7 |
| `status/ox-weather-sunny-20-symbolic.svg` | Fluent UI System Icons 1.1.343 | `icons/weather_sunny_20_regular.svg` | 488eef0eb62101ae3350dc2ae0c8ca77f4caacdcd56946dd0b503e63940b9b02 |
