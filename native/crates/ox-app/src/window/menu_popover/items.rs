// SPDX-License-Identifier: AGPL-3.0-only
//! What a menu shows: its items and dividers, the action each item runs,
//! and whether an item is checked or disabled.
//!
//! Ports the item objects `openMenu` in `v2.0.0:desktop/ui/app.js` takes
//! (`{label, icon, fn, shortcut, disabled}` and `'-'`).

use gtk::glib;
use gtk::prelude::*;

use crate::application::AppAction;
use crate::icons::Icon;
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// Whether an item shows a check mark.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::window) enum ItemCheck {
    /// Never checked.
    Plain,
    /// Checked or not, decided when the menu is built.
    Fixed(bool),
    /// Checked while the action's state equals the item's target (a
    /// choice), or is `true` for an item without a target (a toggle).
    FollowsAction,
}

/// The action a menu item runs: the window's, or the application's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum MenuAction {
    /// A window action (`win.*`).
    Window(WindowAction),
    /// An application action (`app.*`).
    Application(AppAction),
}

impl From<WindowAction> for MenuAction {
    fn from(action: WindowAction) -> Self {
        MenuAction::Window(action)
    }
}

impl From<AppAction> for MenuAction {
    fn from(action: AppAction) -> Self {
        MenuAction::Application(action)
    }
}

impl MenuAction {
    /// The name a row runs the action by, such as `win.sort`.
    pub(in crate::window) fn detailed_name(self) -> String {
        match self {
            MenuAction::Window(action) => action.detailed_name(),
            MenuAction::Application(action) => action.detailed_name(),
        }
    }

    /// The action's state as `widget`'s window sees it.
    pub(super) fn state(self, widget: &gtk::Widget) -> Option<glib::Variant> {
        let window = widget.root().and_downcast::<gtk::ApplicationWindow>()?;
        match self {
            MenuAction::Window(action) => window.action_state(action.name()),
            MenuAction::Application(action) => window.application()?.action_state(action.name()),
        }
    }

    /// Whether `widget`'s window or application has the action enabled.
    pub(super) fn is_enabled(self, widget: &gtk::Widget) -> bool {
        let Some(window) = widget.root().and_downcast::<gtk::ApplicationWindow>() else {
            return false;
        };
        match self {
            MenuAction::Window(action) => window.is_action_enabled(action.name()),
            MenuAction::Application(action) => window
                .application()
                .is_some_and(|app| app.is_action_enabled(action.name())),
        }
    }
}

/// Whether an item can be chosen, beyond its action being enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum ItemAvailability {
    /// It follows its action: enabled while the action is.
    FollowsAction,
    /// Disabled in this menu, such as Open with several items selected
    /// (`disabled: true` in app.js).
    Disabled,
    /// It follows its action, and the compact menu's strip leaves it out
    /// while the action is off, as Windows 11 shows Paste there only
    /// while there is something to paste.
    ShownWhileEnabled,
}

/// One menu item.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::window) struct MenuItem {
    /// The visible and accessible name.
    pub label: String,
    /// The glyph before the label.
    pub glyph: Icon,
    /// The action it runs.
    pub action: MenuAction,
    /// The action's parameter.
    pub target: Option<glib::Variant>,
    /// The keyboard shortcut shown at the right, such as `Ctrl+N`.
    pub shortcut: Option<&'static str>,
    /// How the item shows that it is chosen.
    pub check: ItemCheck,
    /// Whether it can be chosen in this menu.
    pub availability: ItemAvailability,
    /// Shown in bold, as a crumb's subfolder menu shows the folder the
    /// address goes on to (NAV-020).
    pub emphasised: bool,
    /// Why it is disabled in this menu, which its tooltip says.
    pub disabled_reason: Option<&'static str>,
    /// The icon of the application it opens
    /// ([`ox_core::integration::ApplicationInfo::icon`]), which replaces
    /// the glyph.
    pub application_icon: Option<String>,
    /// The entries of the submenu it opens beside the menu, such as a
    /// Templates subfolder's in the New menu (OPS-003); empty for an item
    /// that runs its action.
    pub submenu: Vec<MenuEntry>,
    /// How its glyph is drawn: as it is, mirrored or in the accent colour.
    pub glyph_look: GlyphLook,
    /// Marks it with a dot rather than a check mark while it is on, as
    /// Windows 11 marks a choice such as the view or the open pane.
    pub dot_mark: bool,
    /// Draws its glyph; false for a row with only its mark and label, as
    /// Windows 11's Sort menu draws every row.
    pub shows_glyph: bool,
}

/// How a menu item's glyph is drawn: as the bundled file draws it, or
/// mirrored, or in Windows' blue, or both, as Windows 11 draws Redo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::window) struct GlyphLook {
    /// Mirrored left to right, as Windows 11 draws Redo with Undo's arrow.
    pub mirrored: bool,
    /// In Windows' blue, as Windows 11 draws Undo, Redo and Pin in its "…".
    pub accent: bool,
}

impl MenuItem {
    /// An item that runs `action` without a parameter.
    pub(in crate::window) fn new(label: &str, glyph: Icon, action: impl Into<MenuAction>) -> Self {
        Self {
            label: label.to_owned(),
            glyph,
            action: action.into(),
            target: None,
            shortcut: None,
            check: ItemCheck::Plain,
            availability: ItemAvailability::FollowsAction,
            emphasised: false,
            disabled_reason: None,
            application_icon: None,
            submenu: Vec::new(),
            glyph_look: GlyphLook::default(),
            dot_mark: false,
            shows_glyph: true,
        }
    }

    /// An item that opens `entries` in a submenu beside the menu. It is enabled
    /// while `action` is, which its entries share.
    pub(in crate::window) fn submenu(
        label: &str,
        glyph: Icon,
        action: impl Into<MenuAction>,
        entries: Vec<MenuEntry>,
    ) -> Self {
        Self {
            submenu: entries,
            ..Self::new(label, glyph, action)
        }
    }

    /// An item that runs the window action `action` with `target`, such as
    /// a tab's id.
    pub(in crate::window) fn with_target(
        label: &str,
        glyph: Icon,
        action: WindowAction,
        target: glib::Variant,
    ) -> Self {
        Self {
            target: Some(target),
            ..Self::new(label, glyph, action)
        }
    }

    /// An item that runs the window action `action` with the string
    /// `target`, such as a location.
    pub(in crate::window) fn with_text_target(
        label: &str,
        glyph: Icon,
        action: WindowAction,
        target: &str,
    ) -> Self {
        Self::with_target(label, glyph, action, target.to_variant())
    }

    /// A choice of the string action `action`, checked while it is chosen.
    pub(in crate::window) fn choice(label: &str, glyph: Icon, action: WindowAction, value: &str) -> Self {
        Self {
            check: ItemCheck::FollowsAction,
            dot_mark: true,
            ..Self::with_text_target(label, glyph, action, value)
        }
    }

    /// An item for the boolean action `action`, checked while it is on.
    pub(in crate::window) fn toggle(label: &str, glyph: Icon, action: WindowAction) -> Self {
        Self {
            check: ItemCheck::FollowsAction,
            ..Self::new(label, glyph, action)
        }
    }

    /// The same item with its glyph mirrored left to right.
    pub(in crate::window) fn mirrored(self) -> Self {
        Self {
            glyph_look: GlyphLook {
                mirrored: true,
                ..self.glyph_look
            },
            ..self
        }
    }

    /// The same item with its glyph in the accent colour.
    pub(in crate::window) fn with_accent_glyph(self) -> Self {
        Self {
            glyph_look: GlyphLook {
                accent: true,
                ..self.glyph_look
            },
            ..self
        }
    }

    /// The same item without its glyph: its mark and label alone.
    pub(in crate::window) fn without_glyph(self) -> Self {
        Self {
            shows_glyph: false,
            ..self
        }
    }

    /// The same item, marked with a dot while it is on.
    pub(in crate::window) fn with_dot_mark(self) -> Self {
        Self {
            dot_mark: true,
            ..self
        }
    }

    /// The same item showing `shortcut`.
    pub(in crate::window) fn with_shortcut(self, shortcut: &'static str) -> Self {
        Self {
            shortcut: Some(shortcut),
            ..self
        }
    }

    /// The same item, left out of the compact menu's strip while its
    /// action is off ([`ItemAvailability::ShownWhileEnabled`]).
    pub(in crate::window) fn shown_while_enabled(self) -> Self {
        Self {
            availability: ItemAvailability::ShownWhileEnabled,
            ..self
        }
    }

    /// The same item, disabled in this menu when `disabled` holds.
    pub(in crate::window) fn disabled_when(self, disabled: bool) -> Self {
        let availability = if disabled {
            ItemAvailability::Disabled
        } else {
            self.availability
        };
        Self { availability, ..self }
    }

    /// The same item, disabled in this menu when `disabled` holds, with
    /// `reason` in its tooltip.
    pub(in crate::window) fn disabled_because(self, disabled: bool, reason: &'static str) -> Self {
        if !disabled {
            return self;
        }
        Self {
            disabled_reason: Some(reason),
            ..self.disabled_when(true)
        }
    }

    /// The same item showing an application's `icon` instead of its glyph.
    pub(in crate::window) fn with_application_icon(self, icon: Option<&str>) -> Self {
        Self {
            application_icon: icon.map(str::to_owned),
            ..self
        }
    }
}

impl MenuAction {
    /// Why `widget`'s window has the action disabled, when it says.
    pub(super) fn disabled_reason(self, widget: &gtk::Widget) -> Option<&'static str> {
        let MenuAction::Window(action) = self else {
            return None;
        };
        let window = widget.root().and_downcast::<BrowserWindow>()?;
        window.disabled_reason(action)
    }
}

/// A menu line: an item or a divider.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::window) enum MenuEntry {
    /// A clickable item.
    Item(MenuItem),
    /// A thin line between groups.
    Divider,
}

impl From<MenuItem> for MenuEntry {
    fn from(item: MenuItem) -> Self {
        MenuEntry::Item(item)
    }
}

/// How a menu looks: the Settings choice "Right-click menu" (CMD-008).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::window) enum MenuStyle {
    /// Windows 10 classic: a square text menu, the default.
    #[default]
    Classic,
    /// Windows 11 compact: rounded, with a strip of icon buttons on top.
    Compact,
}

impl MenuStyle {
    /// The CSS class the skin draws the style with.
    pub(super) const fn css_class(self) -> &'static str {
        match self {
            MenuStyle::Classic => "classic",
            MenuStyle::Compact => "compact",
        }
    }

    /// The menu's accessible name, as `openMenu` sets `aria-label`.
    pub(super) fn accessible_name(self) -> &'static str {
        match self {
            MenuStyle::Classic => ox_core::i18n::gettext_static("Windows 10 style menu"),
            MenuStyle::Compact => ox_core::i18n::gettext_static("Windows 11 style menu"),
        }
    }
}
