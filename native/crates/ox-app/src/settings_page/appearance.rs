// SPDX-License-Identifier: AGPL-3.0-only
//! Appearance: the theme, the text size, the right-click menu and the
//! pane widths.
//!
//! Ports the "Appearance & layout" section of `renderSettingsPage`,
//! `textSizeControls` and `menuPreferenceControls` in
//! `desktop/ui/app.js` (SET-005). The theme is chosen from three preview
//! cards, as in the settings mockup. Each card's radio button, one group
//! of three, runs the window's `win.theme` action, as the Appearance menu
//! does, so every window changes at once; screen readers hear one choice
//! of three, and the arrow keys move between them. Text size changes at
//! once too, through the shared skin.

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::{ContextMenu, PreferencesUpdate};

use super::bindings::{position_u32, Choice, PreferenceBinding};
use super::choice_list::ChoiceButton;
use super::group::SettingsGroup;
use super::pages::Category;
use super::parts;
use super::row::{Availability, ControlName, RowLayout, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::SettingsPage;
use crate::icons::Icon;
use crate::text_size::TextSize;
use crate::theme::ThemePreference;
use crate::window::{Milestone, WindowAction};

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

const RIGHT_CLICK_MENU: RowText = RowText {
    title: "Right-click menu",
    description: "Windows 10 is compact and shows familiar text commands.",
    keywords: "context menu style windows 10 11 classic compact",
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
        label: "Windows 10 · Classic (default)",
    },
    Choice {
        value: ContextMenu::Win11,
        label: "Windows 11 · Compact actions",
    },
];

/// The class of the chosen theme's card.
const CHOSEN_CLASS: &str = "chosen";

/// A theme card: the choice it stands for, its name and its CSS class.
struct ThemeCard {
    preference: ThemePreference,
    name: &'static str,
    css_class: &'static str,
}

/// The cards, in the mockup's order.
const THEME_CARDS: [ThemeCard; 3] = [
    ThemeCard {
        preference: ThemePreference::System,
        name: "System",
        css_class: "system",
    },
    ThemeCard {
        preference: ThemePreference::Light,
        name: "Light",
        css_class: "light",
    },
    ThemeCard {
        preference: ThemePreference::Dark,
        name: "Dark",
        css_class: "dark",
    },
];

/// The Appearance page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::Appearance;
    let appearance = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    appearance.append_group(&theme_group());
    appearance.append_group(&text_and_menus_group(page));
    appearance.append_group(&layout_group());
    appearance
}

fn theme_group() -> SettingsGroup {
    // The row is called Theme already.
    let group = SettingsGroup::new("");
    let row = SettingRow::new(THEME);
    row.add_control(&theme_cards(), ControlName::OwnLabel);
    row.set_roomy_layout(RowLayout::ControlsBelow);
    group.add_row(&row);
    group
}

/// The three theme cards, in a row that wraps in a narrow window, their
/// radio buttons one group.
fn theme_cards() -> gtk::FlowBox {
    let cards = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(3)
        .column_spacing(14)
        .row_spacing(14)
        .homogeneous(true)
        .css_classes(["theme-cards"])
        .build();
    let mut first_radio: Option<gtk::CheckButton> = None;
    for card in THEME_CARDS {
        let radio = theme_radio(&card, first_radio.as_ref());
        let child = gtk::FlowBoxChild::builder()
            .child(&theme_card(&card, &radio))
            .focusable(false)
            .build();
        cards.append(&child);
        first_radio.get_or_insert(radio);
    }
    cards
}

/// The radio button under a card, "System", "Light" or "Dark", in the
/// group of `first`. It runs `win.theme` with the card's choice and is on
/// while that is the window's theme; screen readers hear it as one choice
/// of the group, and the arrow keys move to the next and choose it.
fn theme_radio(card: &ThemeCard, first: Option<&gtk::CheckButton>) -> gtk::CheckButton {
    let radio = gtk::CheckButton::builder()
        .label(card.name)
        .accessible_role(gtk::AccessibleRole::Radio)
        .css_classes(["theme-radio"])
        .build();
    radio.set_group(first);
    let target = card.preference.key().to_variant();
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

fn text_and_menus_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new("Text and menus");
    let text_size = SettingRow::new(TEXT_SIZE);
    text_size.add_control(&text_size_choice(page), ControlName::RowTitle);
    group.add_row(&text_size);
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
    // The native context menu has one style until file operations bring
    // the compact one (CMD-008); the choice is saved for the Python app.
    menu.set_availability(Availability::SavedForLater(Milestone::FileOperations));
    group.add_row(&menu);
    group
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
        format!("{percent}% (default)")
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

fn layout_group() -> SettingsGroup {
    let group = SettingsGroup::new("Layout");
    let row = SettingRow::new(PANE_WIDTHS);
    let reset = parts::button_with_glyph("Reset", Icon::ArrowClockwise);
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
        assert_eq!(MENU_STYLES[0].label, "Windows 10 · Classic (default)");
    }
}
