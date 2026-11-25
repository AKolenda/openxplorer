// SPDX-License-Identifier: AGPL-3.0-only
//! Appearance: the theme, the text size and font, the compact view, the
//! Details view's column header style, the navigation pane's arrows, the
//! app's extra navigation pane rows and tab menu items, the right-click
//! menu, the pane widths and the icons of kinds of item
//! ([`super::file_icons`], LOOK-029), including icons from Windows icon
//! files the user chooses (LOOK-030).
//!
//! Ports the "Appearance & layout" section of `renderSettingsPage`,
//! `textSizeControls` and `menuPreferenceControls` in
//! `v2.0.0:desktop/ui/app.js` (SET-005). The theme is chosen from three preview
//! cards, as in the settings mockup. Each card's radio button, one group
//! of three, runs the window's `win.theme` action, as the Appearance menu
//! does, so every window changes at once; screen readers hear one choice
//! of three, and the arrow keys move between them. Text size changes at
//! once too, through the shared skin.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::{ColumnHeaderStyle, ContextMenu, PreferencesUpdate, StripPosition, Theme};

use super::bindings::{position_u32, Choice, PreferenceBinding};
use super::choice_list::ChoiceButton;
use super::group::SettingsGroup;
use super::pages::Category;
use super::parts;
use super::radio_cards::RadioCards;
use super::row::{ControlName, RowLayout, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::SettingsPage;
use crate::icons::{user_glyphs, user_icons, Icon};
use crate::text_size::TextSize;
use crate::window::ButtonStyle;
use crate::window::WindowAction;

const THEME: RowText = RowText {
    title: "Theme",
    description: "The app, menus, and network sign-in use the same theme.",
    keywords: "appearance dark light system colour color mode",
};

const TEXT_SIZE: RowText = RowText {
    title: "Text size",
    description: "Ctrl + and Ctrl − change it anywhere; Ctrl 0 resets. Desktop scaling is unchanged.",
    keywords: "zoom font larger smaller ctrl plus minus reset accessibility scale. Ctrl + makes \
               text larger, Ctrl − smaller, and Ctrl 0 resets it. Saved for all windows; desktop \
               scaling is unchanged.",
};

const DESKTOP_FONT: RowText = RowText {
    title: "Use the desktop font",
    description: "Text uses your desktop's interface font and size instead of Segoe UI.",
    keywords: "font typeface family desktop system interface noto segoe size accessibility",
};

const LANGUAGE: RowText = RowText {
    title: "Language",
    description: "OpenXplorer shows its menus, buttons and messages in your desktop's language when a \
                  translation for it is installed, and in English otherwise. Change the language in your \
                  desktop's language or region settings, then start OpenXplorer again.",
    keywords: "language languages translation translations locale localization interface display \
               chinese english region",
};

const RIGHT_CLICK_MENU: RowText = RowText {
    title: "Right-click menu",
    description: "Windows 10 is compact and shows familiar text commands.",
    keywords: "context menu style windows 10 11 classic compact",
};

const MENU_BUTTONS: RowText = RowText {
    title: "Buttons in the right-click menu",
    description: "Where the Cut, Copy, Rename and Delete buttons appear in the right-click menu. \
                  Near the pointer puts them at the top of a menu that opens downward and at the \
                  bottom of one that opens upward, as Windows does.",
    keywords: "context menu right-click strip buttons icons cut copy rename share delete paste top \
               bottom pointer position windows 11 compact",
};

const SHIFT_F10_CLASSIC: RowText = RowText {
    title: "Shift+F10 opens the classic menu",
    description: "When on, Shift+F10 opens the Windows 10 menu. When off, it opens the same menu \
                  as a right-click and the Menu key, as in Windows 11.",
    keywords: "shift f10 keyboard key context menu right-click classic windows 10 11 compact \
               show more options",
};

const F6_LIKE_WINDOWS: RowText = RowText {
    title: "F6 moves like Windows",
    description: "When on, F6 moves only between the tabs, the navigation pane, the file list, the \
                  Details pane and the status bar, as in Windows 11, and Tab reaches the address bar, \
                  search box and command bar. When off, F6 visits those three too.",
    keywords: "f6 shift+f6 keyboard focus panes regions cycle windows 11 address bar search box \
               command bar tab",
};

const HIDE_EXPAND_ARROWS: RowText = RowText {
    title: "Hide expand arrows in the sidebar",
    description: "As in Windows, the navigation pane's arrows (This PC, Network and the folder tree) \
                  show only while you point at the pane or move through it with the keyboard.",
    keywords: "hide expand arrows chevron expander triangle sidebar navigation pane tree folders \
               windows",
};

const MORE_NAVIGATION_PANE_ITEMS: RowText = RowText {
    title: "Show more items in the navigation pane",
    description: "Adds Recent files, Recent locations, the Recycle Bin and a bar under each drive that \
                  shows how full it is. Off, the navigation pane looks like Windows 11's.",
    keywords: "navigation pane sidebar recent files locations recycle bin trash drive space capacity \
               usage bar extra more rows windows",
};

const MORE_TAB_MENU_ITEMS: RowText = RowText {
    title: "Show more items in the tab menu",
    description: "Adds Move tab to new window, Move tab to window… and Open windows… to the menu you \
                  get when you right-click a tab. Off, the tab menu looks like Windows 11's.",
    keywords: "tab menu right-click move tab new window open windows extra more items windows",
};

const COLUMN_HEADER_STYLE: RowText = RowText {
    title: "Column header style",
    description: "How the Details view marks the sorted column. OpenXplorer puts an arrow after its \
                  title and colors a sort other than Name ascending; Windows shows only a small gray \
                  arrow above the title, as Windows 11 does.",
    keywords: "column header title sort sorted arrow chevron caret accent underline details view \
               windows 11 style look",
};

const ICON_FILES: RowText = RowText {
    title: "Icons from Windows icon files",
    description: "Shows folders, the standard folders, drives, Home, Gallery, This PC, Network, the \
                  Recycle Bin, Settings and common file types with the icons of Windows icon files you choose, for example \
                  imageres.dll.mun and shell32.dll.mun from a Windows installation. OpenXplorer includes \
                  no Windows icons: it reads the files you choose while it runs and keeps their pictures \
                  only in your cache folder. Anything the files do not have keeps OpenXplorer's own icon.",
    keywords: "windows icons icon files imageres shell32 dll mun exe resources folder drive recycle bin \
               this pc home gallery look classic original",
};

const CHOSEN_ICON_FILES: RowText = RowText {
    title: "Windows icon files",
    description: "The .dll, .mun or .exe files the icons are read from. Only where they are is saved, \
                  never the icons. Choosing files again replaces the list.",
    keywords: "windows icons icon files choose browse imageres shell32 dll mun exe clear remove",
};

const SYMBOL_FONT: RowText = RowText {
    title: "Windows symbol font",
    description: "The Segoe Fluent Icons font (SegoeIcons.ttf) or Segoe MDL2 Assets (segmdl2.ttf) from a \
                  Windows installation, whose glyphs then draw the command bar, menus, navigation buttons \
                  and address bar, in your theme's colors. Only where the font is is saved; it is read, \
                  never installed. Glyphs it lacks keep OpenXplorer's own.",
    keywords: "windows symbol font glyphs segoe fluent icons mdl2 assets segoeicons segmdl2 ttf command bar \
               menu buttons",
};

const PANE_WIDTHS: RowText = RowText {
    title: "Sidebar and column widths",
    description: "Restore the default widths in every window.",
    keywords: "reset layout widths sidebar resize columns",
};

/// The right-click menu styles, as `menuPreferenceControls` offers them.
const MENU_STYLES: [Choice<ContextMenu>; 2] = [
    Choice {
        value: ContextMenu::Win10,
        label: crate::i18n::message_id("Windows 10 · Classic"),
    },
    Choice {
        value: ContextMenu::Win11,
        label: crate::i18n::message_id("Windows 11 · Compact actions (default)"),
    },
];

/// The column header styles, the app's own first, as the default.
const HEADER_STYLES: [Choice<ColumnHeaderStyle>; 2] = [
    Choice {
        value: ColumnHeaderStyle::OpenXplorer,
        label: crate::i18n::message_id("OpenXplorer (default)"),
    },
    Choice {
        value: ColumnHeaderStyle::Windows,
        label: crate::i18n::message_id("Windows"),
    },
];

/// Where the compact menu's buttons go, the pointer's side first, as
/// Windows 11 draws them.
const STRIP_POSITIONS: [Choice<StripPosition>; 3] = [
    Choice {
        value: StripPosition::Pointer,
        label: crate::i18n::message_id("Near the pointer (default)"),
    },
    Choice {
        value: StripPosition::Top,
        label: crate::i18n::message_id("Always at the top"),
    },
    Choice {
        value: StripPosition::Bottom,
        label: crate::i18n::message_id("Always at the bottom"),
    },
];

/// The class of the chosen theme's card.
const CHOSEN_CLASS: &str = "chosen";

/// A theme card: the choice it stands for, its name and its CSS class.
struct ThemeCard {
    theme: Theme,
    name: &'static str,
    css_class: &'static str,
}

/// The cards, in the mockup's order.
const THEME_CARDS: [ThemeCard; 3] = [
    ThemeCard {
        theme: Theme::System,
        name: crate::i18n::message_id("System"),
        css_class: "system",
    },
    ThemeCard {
        theme: Theme::Light,
        name: crate::i18n::message_id("Light"),
        css_class: "light",
    },
    ThemeCard {
        theme: Theme::Dark,
        name: crate::i18n::message_id("Dark"),
        css_class: "dark",
    },
];

/// The Appearance page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::Appearance;
    let appearance = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    appearance.append_group(&theme_group(page));
    appearance.append_group(&super::file_icons::group(page));
    appearance.append_group(&layout_group(page));
    appearance
}

/// The theme, the text size and the font.
fn theme_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Theme"));
    let row = SettingRow::new(THEME);
    row.add_control(&theme_cards(), ControlName::OwnLabel);
    row.set_roomy_layout(RowLayout::ControlsBelow);
    group.add_row(&row);
    let text_size = SettingRow::new(TEXT_SIZE);
    text_size.add_control(&text_size_choice(page), ControlName::RowTitle);
    group.add_row(&text_size);
    group.add_row(&desktop_font_row(page));
    group.add_row(&language_row());
    group
}

/// "Language": the desktop's language, which the app follows, under the
/// title, as Windows sets the display language in its own settings and
/// not in File Explorer's.
fn language_row() -> SettingRow {
    let row = SettingRow::new(LANGUAGE);
    let asked: Vec<String> = glib::language_names().iter().map(ToString::to_string).collect();
    row.set_description(&language_status(ox_core::i18n::interface_language(), &asked));
    row
}

/// What the Language row says, given the language of the catalogue the
/// app picked (`None` while it shows English) and the languages the
/// desktop asks for, in order.
fn language_status(picked: Option<&str>, asked: &[String]) -> String {
    if let Some(language) = picked {
        let language = language.split(['.', '@']).next().unwrap_or(language);
        return ox_core::i18n::format_message(
            "Your desktop's language ({language})",
            &[("language", language)],
        );
    }
    // The first language asked for before the C locale, encoded or not
    // (`C.UTF-8` comes before `C` with LANG=C.UTF-8), unless it is English.
    let wanted = asked
        .iter()
        .map(|name| name.split(['.', '@']).next().unwrap_or_default())
        .filter(|name| !name.is_empty())
        .take_while(|name| !matches!(*name, "C" | "POSIX"))
        .next()
        .filter(|name| name.split('_').next() != Some("en"));
    match wanted {
        Some(language) => ox_core::i18n::format_message(
            "English: no translation for {language} is installed",
            &[("language", language)],
        ),
        None => ox_core::i18n::gettext("English"),
    }
}

/// The three theme cards, in a row that wraps in a narrow window, their
/// radio buttons one group and one Tab stop ([`RadioCards`]).
fn theme_cards() -> RadioCards {
    let cards = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .column_spacing(14)
        .row_spacing(14)
        .homogeneous(true)
        .css_classes(["theme-cards"])
        .build();
    let mut first_radio: Option<gtk::CheckButton> = None;
    let mut radios = Vec::new();
    for card in THEME_CARDS {
        let radio = theme_radio(&card, first_radio.as_ref());
        let child = gtk::FlowBoxChild::builder()
            .child(&theme_card(&card, &radio))
            .focusable(false)
            .build();
        cards.append(&child);
        first_radio.get_or_insert_with(|| radio.clone());
        radios.push(radio);
    }
    RadioCards::new(&cards, radios)
}

/// The radio button under a card, "System", "Light" or "Dark", in the
/// group of `first`. It runs `win.theme` with the card's choice and is on
/// while that is the window's theme; screen readers hear it as one choice
/// of the group, and the arrow keys move to the next and choose it.
fn theme_radio(card: &ThemeCard, first: Option<&gtk::CheckButton>) -> gtk::CheckButton {
    let radio = gtk::CheckButton::builder()
        .label(ox_core::i18n::gettext(card.name))
        .accessible_role(gtk::AccessibleRole::Radio)
        .css_classes(["theme-radio"])
        .build();
    radio.set_group(first);
    let target = card.theme.as_str().to_variant();
    WindowAction::Theme.assign_with_target_to(&radio, &target);
    radio
}

/// A card: the preview in the card's colours over its `radio`. Clicking
/// the preview chooses the card too, and the chosen card is outlined.
fn theme_card(card: &ThemeCard, radio: &gtk::CheckButton) -> gtk::Box {
    let theme_card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .css_classes(["theme-card", card.css_class])
        .build();
    let preview = theme_preview();
    let click = gtk::GestureClick::new();
    click.connect_released(glib::clone!(
        #[weak]
        radio,
        move |_, _, _, _| {
            radio.activate();
        }
    ));
    preview.add_controller(click);
    theme_card.append(&preview);
    theme_card.append(radio);
    radio.connect_active_notify(glib::clone!(
        #[weak]
        theme_card,
        move |radio| outline_when_chosen(&theme_card, radio)
    ));
    theme_card
}

/// Outlines `theme_card` while its `radio` is on.
fn outline_when_chosen(theme_card: &gtk::Box, radio: &gtk::CheckButton) {
    if radio.is_active() {
        theme_card.add_css_class(CHOSEN_CLASS);
    } else {
        theme_card.remove_css_class(CHOSEN_CLASS);
    }
}

/// A small window in the card's colours: a title strip, a sidebar and
/// three lines of content, drawn by `resources/skin/settings.css`.
fn theme_preview() -> gtk::Box {
    let preview = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["theme-preview"])
        .build();
    preview.append(&gtk::Box::builder().css_classes(["preview-title"]).build());
    let body = gtk::Box::builder().css_classes(["preview-body"]).build();
    body.append(&gtk::Box::builder().css_classes(["preview-sidebar"]).build());
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .css_classes(["preview-content"])
        .build();
    for line_class in ["preview-accent-line", "preview-line", "preview-short-line"] {
        content.append(&gtk::Box::builder().css_classes([line_class]).build());
    }
    body.append(&content);
    preview.append(&body);
    preview
}

/// The rows of icons from Windows icon files the user chooses, off until
/// turned on (LOOK-030), which close the File icons group as one more
/// source of the same icons: a choice above wins over them. The first
/// row's switch turns them on and says how many of the icons the files
/// gave; the second chooses the files.
pub(super) fn icon_files_rows(page: &SettingsPage) -> [SettingRow; 3] {
    let use_files = SettingRow::new(ICON_FILES);
    let switch = page.preference_switch(PreferenceBinding {
        read: |preferences| preferences.use_icon_files,
        write: |on| PreferencesUpdate {
            use_icon_files: Some(on),
            ..PreferencesUpdate::default()
        },
    });
    use_files.add_control(&switch, ControlName::RowTitle);
    let context = page.context().clone();
    let row = use_files.downgrade();
    user_icons::follow_status(move |status| {
        let Some(row) = row.upgrade() else {
            return false;
        };
        row.set_description(&icon_files_status(&context.settings_data().preferences, status));
        true
    });
    page.follow_preferences(glib::clone!(
        #[weak]
        use_files,
        move |preferences| use_files.set_description(&icon_files_status(preferences, user_icons::status()))
    ));
    [use_files, chosen_icon_files_row(page), symbol_font_row(page)]
}

/// "Windows symbol font": the chosen font's name and how many of the
/// glyphs it gave, Choose font… to pick it in a file dialog and Clear to
/// forget it (LOOK-031). The switch above turns it on with the icon files.
fn symbol_font_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(SYMBOL_FONT);
    let choose = parts::button(&ox_core::i18n::gettext("Choose font…"), ButtonStyle::Bordered);
    let clear = parts::button(&ox_core::i18n::gettext("Clear"), ButtonStyle::Bordered);
    choose.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| choose_symbol_font(&page)
    ));
    clear.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| {
            page.save_preferences(PreferencesUpdate {
                symbol_font: Some(String::new()),
                ..PreferencesUpdate::default()
            });
        }
    ));
    let context = page.context().clone();
    let weak_row = row.downgrade();
    user_glyphs::follow_status(move |status| {
        let Some(row) = weak_row.upgrade() else {
            return false;
        };
        row.set_description(&symbol_font_text(&context.settings_data().preferences, status));
        true
    });
    page.follow_preferences(glib::clone!(
        #[weak]
        row,
        #[weak]
        clear,
        move |preferences| {
            row.set_description(&symbol_font_text(preferences, user_glyphs::status()));
            clear.set_sensitive(preferences.symbol_font.is_some());
        }
    ));
    row.add_control(&choose, ControlName::OwnLabel);
    row.add_control(&clear, ControlName::OwnLabel);
    row
}

/// What the symbol font's row says: the font's name and whether its
/// glyphs are in use, being read, or how many of them it has.
fn symbol_font_text(preferences: &ox_core::settings::Preferences, status: user_glyphs::Status) -> String {
    let Some(font) = &preferences.symbol_font else {
        return ox_core::i18n::gettext("No font chosen");
    };
    let name = font.rsplit('/').next().unwrap_or(font);
    let state = match status {
        _ if !preferences.use_icon_files => {
            ox_core::i18n::gettext("used while Icons from Windows icon files is on")
        }
        user_glyphs::Status::Off | user_glyphs::Status::Reading => ox_core::i18n::gettext("reading…"),
        user_glyphs::Status::Ready { found: 0, .. } => {
            ox_core::i18n::gettext("it has none of these glyphs, so OpenXplorer's own are shown")
        }
        user_glyphs::Status::Ready { found, wanted } => ox_core::i18n::format_message(
            "{found} of {wanted} glyphs",
            &[("found", &found.to_string()), ("wanted", &wanted.to_string())],
        ),
    };
    ox_core::i18n::format_message("{name} · {state}", &[("name", name), ("state", &state)])
}

/// Asks for the Windows symbol font and saves its path.
fn choose_symbol_font(page: &SettingsPage) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&ox_core::i18n::gettext("Fonts")));
    for pattern in ["*.ttf", "*.otf", "*.TTF", "*.OTF"] {
        filter.add_pattern(pattern);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let picker = gtk::FileDialog::builder()
        .title(ox_core::i18n::gettext("Choose the Windows symbol font"))
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    let window = page.root().and_downcast::<gtk::Window>();
    picker.open(
        window.as_ref(),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            page,
            move |chosen| {
                let Some(path) = chosen.ok().and_then(|file| file.path()) else {
                    return;
                };
                if let Some(path) = path.to_str() {
                    page.save_preferences(PreferencesUpdate {
                        symbol_font: Some(path.to_owned()),
                        ..PreferencesUpdate::default()
                    });
                }
            }
        ),
    );
}

/// What the switch's row says: whether the icons are on, being read, or
/// how many the chosen files gave.
fn icon_files_status(preferences: &ox_core::settings::Preferences, status: user_icons::Status) -> String {
    if !preferences.use_icon_files {
        return ox_core::i18n::gettext("Off: OpenXplorer's own icons");
    }
    match status {
        _ if preferences.icon_files.is_empty() => ox_core::i18n::gettext("On, but no file is chosen yet"),
        user_icons::Status::Off | user_icons::Status::Reading => {
            ox_core::i18n::gettext("Reading the chosen files…")
        }
        user_icons::Status::Ready { found: 0 } => ox_core::i18n::gettext(
            "The chosen files have none of these icons, so OpenXplorer's own are shown",
        ),
        user_icons::Status::Ready { found } => ox_core::i18n::format_message(
            "{found} of {total} icons come from the chosen files",
            &[
                ("found", &found.to_string()),
                ("total", &ox_core::windows_icons::IconRole::ALL.len().to_string()),
            ],
        ),
    }
}

/// "Windows icon files": the chosen files' names, Choose files… to pick
/// them in a file dialog and Clear to forget them.
fn chosen_icon_files_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(CHOSEN_ICON_FILES);
    let choose = parts::button(&ox_core::i18n::gettext("Choose files…"), ButtonStyle::Bordered);
    let clear = parts::button(&ox_core::i18n::gettext("Clear"), ButtonStyle::Bordered);
    choose.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| choose_icon_files(&page)
    ));
    clear.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| {
            page.save_preferences(PreferencesUpdate {
                icon_files: Some(Vec::new()),
                ..PreferencesUpdate::default()
            });
        }
    ));
    page.follow_preferences(glib::clone!(
        #[weak]
        row,
        #[weak]
        clear,
        move |preferences| {
            row.set_description(&chosen_icon_files_text(&preferences.icon_files));
            clear.set_sensitive(!preferences.icon_files.is_empty());
        }
    ));
    row.add_control(&choose, ControlName::OwnLabel);
    row.add_control(&clear, ControlName::OwnLabel);
    row
}

/// The chosen files' names, or that none is chosen.
fn chosen_icon_files_text(files: &[String]) -> String {
    if files.is_empty() {
        return ox_core::i18n::gettext("No files chosen");
    }
    let names: Vec<&str> = files
        .iter()
        .map(|path| path.rsplit('/').next().unwrap_or(path))
        .collect();
    names.join(", ")
}

/// Asks for Windows icon files and saves the paths of those chosen.
fn choose_icon_files(page: &SettingsPage) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&ox_core::i18n::gettext("Windows icon files")));
    for pattern in ["*.dll", "*.mun", "*.exe", "*.DLL", "*.MUN", "*.EXE"] {
        filter.add_pattern(pattern);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let picker = gtk::FileDialog::builder()
        .title(ox_core::i18n::gettext("Choose Windows icon files"))
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    let window = page.root().and_downcast::<gtk::Window>();
    picker.open_multiple(
        window.as_ref(),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            page,
            move |chosen| {
                let Ok(chosen) = chosen else {
                    return;
                };
                let paths: Vec<String> = chosen
                    .iter::<gio::File>()
                    .filter_map(Result::ok)
                    .filter_map(|file| file.path())
                    .filter_map(|path| path.to_str().map(str::to_owned))
                    .collect();
                if !paths.is_empty() {
                    page.save_preferences(PreferencesUpdate {
                        icon_files: Some(paths),
                        ..PreferencesUpdate::default()
                    });
                }
            }
        ),
    );
}

/// "Right-click menu": the menu's style.
fn right_click_menu_row(page: &SettingsPage) -> SettingRow {
    let menu = SettingRow::new(RIGHT_CLICK_MENU);
    let binding = PreferenceBinding {
        read: |preferences| preferences.context_menu,
        write: |style| PreferencesUpdate {
            context_menu: Some(style),
            ..PreferencesUpdate::default()
        },
    };
    menu.add_control(
        &page.preference_choice(&MENU_STYLES, binding),
        ControlName::RowTitle,
    );
    // The folder views' context menus follow the choice when they open
    // (CMD-008, src/window/context_menu.rs).
    menu
}

/// "Buttons in the right-click menu": where the compact menu's strip sits.
/// Every compact menu reads it when it opens (`src/window/menu_popover`).
fn menu_buttons_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(MENU_BUTTONS);
    let binding = PreferenceBinding {
        read: |preferences| preferences.strip_position,
        write: |position| PreferencesUpdate {
            strip_position: Some(position),
            ..PreferencesUpdate::default()
        },
    };
    row.add_control(
        &page.preference_choice(&STRIP_POSITIONS, binding),
        ControlName::RowTitle,
    );
    row
}

/// "Column header style": how the Details view draws its sorted column's
/// title, which every window follows at once
/// (`src/window/column_header_style.rs`).
fn column_header_style_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(COLUMN_HEADER_STYLE);
    let binding = PreferenceBinding {
        read: |preferences| preferences.column_header_style,
        write: |style| PreferencesUpdate {
            column_header_style: Some(style),
            ..PreferencesUpdate::default()
        },
    };
    row.add_control(
        &page.preference_choice(&HEADER_STYLES, binding),
        ControlName::RowTitle,
    );
    row
}

/// "Shift+F10 opens the classic menu": the switch saves the choice, which
/// the keyboard menus read when Shift+F10 opens one
/// (`src/window/context_menu.rs`).
fn shift_f10_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(SHIFT_F10_CLASSIC);
    let switch = page.preference_switch(PreferenceBinding {
        read: |preferences| preferences.shift_f10_classic,
        write: |on| PreferencesUpdate {
            shift_f10_classic: Some(on),
            ..PreferencesUpdate::default()
        },
    });
    row.add_control(&switch, ControlName::RowTitle);
    row
}

/// "F6 moves like Windows": the switch saves the choice, which F6 reads
/// each time it moves the keyboard (`src/window/focus_regions.rs`).
fn f6_like_windows_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(F6_LIKE_WINDOWS);
    let switch = page.preference_switch(PreferenceBinding {
        read: |preferences| preferences.f6_like_windows,
        write: |on| PreferencesUpdate {
            f6_like_windows: Some(on),
            ..PreferencesUpdate::default()
        },
    });
    row.add_control(&switch, ControlName::RowTitle);
    row
}

/// "Use the desktop font": the switch saves the choice and the skin draws
/// it in every window at once.
fn desktop_font_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(DESKTOP_FONT);
    let switch = page.preference_switch(PreferenceBinding {
        read: |preferences| preferences.desktop_font,
        write: |on| PreferencesUpdate {
            desktop_font: Some(on),
            ..PreferencesUpdate::default()
        },
    });
    // Text follows the saved choice, not the switch: told at once, the
    // shared skin redrew text and showed the page the preference not yet
    // saved, which turned the switch straight back off.
    let skin = page.context().skin().clone();
    page.follow_preferences(move |preferences| skin.set_uses_desktop_font(preferences.desktop_font));
    row.add_control(&switch, ControlName::RowTitle);
    row
}

/// The text sizes of text-size.js, 100% marked as the default. The skin
/// draws the chosen size in every window at once, and the choice is
/// saved; a size changed with Ctrl+plus shows here too.
fn text_size_choice(page: &SettingsPage) -> gtk::MenuButton {
    let sizes: Vec<TextSize> = TextSize::all().collect();
    let labels: Vec<String> = sizes.iter().map(|size| text_size_label(*size)).collect();
    let drop_down = ChoiceButton::new(&labels);
    let list = drop_down.choices.clone();
    let skin = page.context().skin().clone();
    let shown_sizes = sizes.clone();
    page.follow_preferences(glib::clone!(
        #[weak]
        list,
        #[weak]
        skin,
        move |_| {
            let position = shown_sizes.iter().position(|size| *size == skin.text_size());
            list.set_selected(position_u32(position.unwrap_or_default()));
        }
    ));
    list.connect_selected_notify(glib::clone!(
        #[weak]
        page,
        move |list| {
            if !page.is_user_change() {
                return;
            }
            if let Some(size) = sizes.get(list.selected() as usize) {
                choose_text_size(&page, *size);
            }
        }
    ));
    drop_down.button
}

/// "125%", or "100% (default)" (`textSizeControls`).
fn text_size_label(size: TextSize) -> String {
    let percent = size.percent();
    if size == TextSize::DEFAULT {
        ox_core::i18n::format_message("{percent}% (default)", &[("percent", &percent.to_string())])
    } else {
        format!("{percent}%")
    }
}

/// Draws text at `size` in every window and saves it, as
/// `changeTextSize` does.
fn choose_text_size(page: &SettingsPage, size: TextSize) {
    page.context().skin().set_text_size(size);
    page.save_preferences(PreferencesUpdate {
        text_size: Some(size.percent()),
        ..PreferencesUpdate::default()
    });
}

/// The compact view, the column header style, the navigation pane's
/// arrows, the app's extra navigation pane rows and tab menu items, the
/// right-click menu, Shift+F10's menu and the pane widths.
fn layout_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Layout"));
    group.add_row(&super::window_decorations::row(page));
    group.add_row(&super::files_folders::compact_view_row(page));
    group.add_row(&column_header_style_row(page));
    let arrows = SettingRow::new(HIDE_EXPAND_ARROWS);
    let binding = PreferenceBinding {
        read: |preferences| preferences.hide_expand_arrows,
        write: |hidden| PreferencesUpdate {
            hide_expand_arrows: Some(hidden),
            ..PreferencesUpdate::default()
        },
    };
    arrows.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    group.add_row(&arrows);
    let pane_items = SettingRow::new(MORE_NAVIGATION_PANE_ITEMS);
    let binding = PreferenceBinding {
        read: |preferences| preferences.more_navigation_pane_items,
        write: |on| PreferencesUpdate {
            more_navigation_pane_items: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    pane_items.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    group.add_row(&pane_items);
    let tab_menu_items = SettingRow::new(MORE_TAB_MENU_ITEMS);
    let binding = PreferenceBinding {
        read: |preferences| preferences.more_tab_menu_items,
        write: |on| PreferencesUpdate {
            more_tab_menu_items: Some(on),
            ..PreferencesUpdate::default()
        },
    };
    tab_menu_items.add_control(&page.preference_switch(binding), ControlName::RowTitle);
    group.add_row(&tab_menu_items);
    group.add_row(&right_click_menu_row(page));
    group.add_row(&menu_buttons_row(page));
    group.add_row(&shift_f10_row(page));
    group.add_row(&f6_like_windows_row(page));
    let row = SettingRow::new(PANE_WIDTHS);
    let reset = parts::button_with_glyph(&ox_core::i18n::gettext("Reset"), Icon::ArrowClockwise);
    WindowAction::ResetLayout.assign_to(&reset);
    row.add_control(&reset, ControlName::OwnLabel);
    group.add_row(&row);
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_sizes_read_as_percentages_with_the_default_marked() {
        let labels: Vec<String> = TextSize::all().map(text_size_label).collect();
        assert_eq!(
            labels,
            [
                "80%",
                "90%",
                "100% (default)",
                "110%",
                "125%",
                "150%",
                "175%",
                "200%"
            ]
        );
    }

    #[test]
    fn the_menu_styles_are_the_python_choices_classic_first() {
        let values = MENU_STYLES.map(|choice| choice.value);
        assert_eq!(values, ContextMenu::ALL);
        assert_eq!(MENU_STYLES[0].label, "Windows 10 · Classic");
    }

    /// parity: CMD-008
    #[test]
    fn the_strip_positions_offer_the_pointers_side_first_as_the_default() {
        let values = STRIP_POSITIONS.map(|choice| choice.value);
        assert_eq!(values, StripPosition::ALL);
        assert_eq!(StripPosition::default(), StripPosition::Pointer);
        assert_eq!(STRIP_POSITIONS[0].label, "Near the pointer (default)");
    }
}

#[cfg(test)]
mod language_tests {
    use super::language_status;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The Language row names the language the interface is in: the one
    /// whose catalogue was picked, after any the desktop asked for first
    /// that has none, and says English when no catalogue was found,
    /// naming the language that has no translation.
    ///
    /// parity: SET-004
    #[test]
    fn the_language_row_names_the_language_the_interface_is_in() {
        let french_then_chinese = names(&["fr_FR.UTF-8", "fr_FR", "fr", "zh_CN.UTF-8", "zh_CN", "zh", "C"]);
        assert_eq!(
            language_status(Some("zh_CN"), &french_then_chinese),
            "Your desktop's language (zh_CN)"
        );
        let french = names(&["fr_FR.UTF-8", "fr_FR", "fr", "C"]);
        assert_eq!(
            language_status(None, &french),
            "English: no translation for fr_FR is installed"
        );
        let english = names(&["en_US.UTF-8", "en_US", "en", "C"]);
        assert_eq!(language_status(None, &english), "English");
        assert_eq!(language_status(None, &names(&["C"])), "English");
        // GLib lists an encoded C locale before the plain one (LANG=C.UTF-8).
        assert_eq!(language_status(None, &names(&["C.UTF-8", "C"])), "English");
        assert_eq!(
            language_status(None, &names(&["POSIX.UTF-8", "POSIX"])),
            "English"
        );
        assert_eq!(
            language_status(None, &names(&["C.UTF-8", "fr_FR", "C"])),
            "English",
            "the C locale ends the search"
        );
    }
}
