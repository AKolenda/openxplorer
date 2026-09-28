// SPDX-License-Identifier: AGPL-3.0-only
//! The skin's stylesheets: the rules and the two palettes.
//!
//! Ports `desktop/ui/style.css` as `native/docs/ui-spec.md` refines it.
//! The rules live in `resources/skin/`, one file per region of the
//! window, and refer to colours only by `@ox_*` tokens (or `transparent`).
//! `light.css` and `dark.css` define every token, one palette per
//! appearance, so switching appearance swaps one small provider and never
//! touches the rules.

use super::Appearance;

/// The skin's rules, one stylesheet per region of the window (ui-spec.md
/// §4), in cascade order: later files may override earlier ones, and the
/// narrow-window rules come last.
pub(super) const RULES: &str = concat!(
    include_str!("../../resources/skin/base.css"),
    include_str!("../../resources/skin/icons.css"),
    include_str!("../../resources/skin/title-bar.css"),
    include_str!("../../resources/skin/navigation.css"),
    include_str!("../../resources/skin/command-bar.css"),
    include_str!("../../resources/skin/sidebar.css"),
    include_str!("../../resources/skin/folder-views.css"),
    include_str!("../../resources/skin/details-pane.css"),
    include_str!("../../resources/skin/status-bar.css"),
    include_str!("../../resources/skin/landing.css"),
    include_str!("../../resources/skin/menus.css"),
    include_str!("../../resources/skin/dialogs.css"),
    include_str!("../../resources/skin/integration-dialogs.css"),
    include_str!("../../resources/skin/settings.css"),
    include_str!("../../resources/skin/breakpoints.css"),
);

/// Rules added while the desktop asks for high contrast
/// ([`super::contrast`]).
pub(super) const HIGH_CONTRAST_RULES: &str = include_str!("../../resources/skin/high-contrast.css");

/// The colour tokens of [`Appearance::Light`].
const LIGHT_PALETTE: &str = include_str!("../../resources/light.css");

/// The colour tokens of [`Appearance::Dark`].
const DARK_PALETTE: &str = include_str!("../../resources/dark.css");

/// The palette that draws `appearance`.
pub(super) const fn palette(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::Light => LIGHT_PALETTE,
        Appearance::Dark => DARK_PALETTE,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::rc::Rc;

    use super::super::fonts::css_for_text_size;
    use super::*;
    use crate::text_size::TextSize;

    /// `css` without its comments, which mention tokens in prose.
    fn without_comments(css: &str) -> String {
        let mut code = String::new();
        let mut rest = css;
        while let Some(start) = rest.find("/*") {
            code.push_str(&rest[..start]);
            let comment_end = rest[start..].find("*/").map_or(rest.len(), |end| start + end + 2);
            rest = &rest[comment_end..];
        }
        code.push_str(rest);
        code
    }

    /// The `@ox_*` tokens that `css` refers to, including the ones a
    /// palette uses to define others.
    fn referenced_tokens(css: &str) -> BTreeSet<String> {
        let code = without_comments(css);
        code.match_indices("@ox_")
            .map(|(start, _)| token_at(&code, start + 1).to_owned())
            .collect()
    }

    /// The tokens `palette` defines with `@define-color`.
    fn defined_tokens(palette: &str) -> BTreeSet<String> {
        palette
            .lines()
            .filter_map(|line| line.strip_prefix("@define-color "))
            .map(|definition| token_at(definition, 0).to_owned())
            .collect()
    }

    /// The token name that starts at `start` in `css`.
    fn token_at(css: &str, start: usize) -> &str {
        let rest = &css[start..];
        let end = rest
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .unwrap_or(rest.len());
        &rest[..end]
    }

    #[test]
    fn both_palettes_define_the_same_tokens() {
        let light = defined_tokens(palette(Appearance::Light));
        let dark = defined_tokens(palette(Appearance::Dark));
        assert!(light.len() > 40, "the palette defines the spec's tokens");
        assert_eq!(light, dark);
    }

    /// GTK drops a rule whose colour names an undefined token, so a typo
    /// would silently leave a control unstyled in one appearance.
    #[test]
    fn every_token_the_skin_uses_is_defined_in_both_palettes() {
        let rules_use: BTreeSet<_> = referenced_tokens(RULES)
            .union(&referenced_tokens(HIGH_CONTRAST_RULES))
            .cloned()
            .collect();
        assert!(rules_use.contains("ox_accent"), "the scan finds tokens");
        for appearance in [Appearance::Light, Appearance::Dark] {
            let defined = defined_tokens(palette(appearance));
            let palette_uses = referenced_tokens(palette(appearance));
            let missing: Vec<_> = rules_use
                .union(&palette_uses)
                .filter(|token| !defined.contains(*token))
                .collect();
            assert!(missing.is_empty(), "{appearance:?} lacks {missing:?}");
        }
    }

    /// A colour written into a rule would not follow the appearance.
    #[test]
    fn the_rules_name_colours_only_by_token() {
        for rules in [RULES, HIGH_CONTRAST_RULES] {
            let code = without_comments(rules);
            let raw = raw_colours(&code);
            assert!(raw.is_empty(), "raw colours in the skin: {raw:?}");
        }
        let written = raw_colours("a { color: alpha(white, .7); background: #c42b1c; }");
        assert_eq!(written, ["white", "#c42b1c"], "the scan finds raw colours");
    }

    /// The colour keywords and hex colours `code` writes out instead of
    /// naming a token.
    fn raw_colours(code: &str) -> Vec<&str> {
        let words = code.split(|character: char| {
            !(character.is_ascii_alphanumeric() || character == '#' || character == '_')
        });
        words
            .filter(|word| is_colour_keyword(word) || is_hex_colour(word))
            .collect()
    }

    /// `black`, `white` or a CSS function that spells a colour out.
    fn is_colour_keyword(word: &str) -> bool {
        matches!(word, "black" | "white" | "rgb" | "rgba" | "hsl" | "hsla")
    }

    /// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    fn is_hex_colour(word: &str) -> bool {
        let Some(digits) = word.strip_prefix('#') else {
            return false;
        };
        let is_colour_length = matches!(digits.len(), 3 | 4 | 6 | 8);
        is_colour_length && digits.chars().all(|digit| digit.is_ascii_hexdigit())
    }

    #[test]
    fn every_palette_token_is_used() {
        let light = palette(Appearance::Light);
        let used: BTreeSet<_> = referenced_tokens(RULES)
            .union(&referenced_tokens(light))
            .cloned()
            .collect();
        let unused: Vec<_> = defined_tokens(light)
            .into_iter()
            .filter(|token| !used.contains(token))
            .collect();
        assert!(unused.is_empty(), "nothing uses {unused:?}");
    }

    #[test]
    fn the_palettes_keep_the_current_apps_surfaces() {
        assert!(palette(Appearance::Light).contains("@define-color ox_bg #ffffff;"));
        assert!(palette(Appearance::Dark).contains("@define-color ox_bg #202020;"));
        assert!(palette(Appearance::Light).contains("@define-color ox_title #eff1f4;"));
        assert!(palette(Appearance::Dark).contains("@define-color ox_title #191919;"));
    }

    /// Loads `css` into a provider and returns GTK's parsing errors.
    fn parsing_errors(css: &str) -> Vec<String> {
        let provider = gtk::CssProvider::new();
        let errors = Rc::new(RefCell::new(Vec::new()));
        let collected = Rc::clone(&errors);
        provider.connect_parsing_error(move |_, section, error| {
            collected.borrow_mut().push(format!("{section}: {error}"));
        });
        provider.load_from_string(css);
        errors.take()
    }

    #[gtk::test]
    fn every_stylesheet_parses_without_errors() {
        let text_sizes = TextSize::all().map(css_for_text_size);
        let sheets = [RULES, HIGH_CONTRAST_RULES, LIGHT_PALETTE, DARK_PALETTE]
            .into_iter()
            .map(str::to_owned)
            .chain(text_sizes);
        for sheet in sheets {
            let errors = parsing_errors(&sheet);
            assert!(errors.is_empty(), "{errors:#?}");
        }
    }
}
