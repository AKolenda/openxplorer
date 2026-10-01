// SPDX-License-Identifier: AGPL-3.0-only
//! The check a typed name passes before any file operation sees it
//! (OPS-006).
//!
//! Ports `validateName` in `v2.0.0:desktop/ui/app.js`. The dialog shows its
//! message and stays open; the operation then checks the name again with
//! ox-core's own rules ([`ox_core::location::validate_name`]), whose
//! messages the dialog shows the same way.

/// A name the dialogs refuse before asking the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Use a name without slashes or control characters.")]
pub(super) struct InvalidName;

/// `name`, when it is not empty, `.` or `..`, and has no slash,
/// backslash or control character (`[\/\\\x00-\x1f]` in app.js).
///
/// # Errors
///
/// [`InvalidName`] otherwise.
pub(super) fn check_typed_name(name: &str) -> Result<&str, InvalidName> {
    let has_forbidden_character = name
        .chars()
        .any(|character| matches!(character, '/' | '\\' | '\0'..='\u{1f}'));
    if name.is_empty() || name == "." || name == ".." || has_forbidden_character {
        return Err(InvalidName);
    }
    Ok(name)
}

/// What the name dialogs say, while the user types, about a valid `name`
/// that may surprise: that it is `taken` in the folder, that a leading
/// dot hides the item, or that a leading space or tilde is unusual
/// (OPS-007, Dolphin's New folder dialog).
pub(super) fn name_warning(name: &str, taken: bool) -> Option<String> {
    if taken {
        return Some(format!("An item named “{name}” already exists here."));
    }
    let warning = if name.starts_with('.') {
        "A name starting with a dot hides the item."
    } else if name.starts_with(char::is_whitespace) {
        "A name starting with a space is unusual."
    } else if name.starts_with('~') {
        "A name starting with a tilde is unusual."
    } else {
        return None;
    };
    Some(warning.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-007
    #[test]
    fn surprising_names_are_warned_about_while_typing() {
        assert_eq!(
            name_warning("Notes", true).as_deref(),
            Some("An item named “Notes” already exists here.")
        );
        assert_eq!(
            name_warning(".config", false).as_deref(),
            Some("A name starting with a dot hides the item.")
        );
        assert!(name_warning(" Notes", false).is_some_and(|warning| warning.contains("space")));
        assert!(name_warning("~Notes", false).is_some_and(|warning| warning.contains("tilde")));
        assert_eq!(name_warning("Notes", false), None);
    }

    /// Ported from the name rules of `v2.0.0:desktop/tests/test_core.py::CoreTests::test_names`,
    /// as `validateName` applies them before the backend.
    ///
    /// parity: OPS-006
    #[test]
    fn names_with_slashes_control_characters_or_dots_only_are_refused() {
        for refused in ["", ".", "..", "a/b", "a\\b", "tab\there", "line\nbreak", "\0"] {
            assert_eq!(check_typed_name(refused), Err(InvalidName), "{refused:?}");
        }
        for accepted in ["New folder", ".hidden", "...", "Résumé.txt", "a\u{7f}b"] {
            assert_eq!(check_typed_name(accepted), Ok(accepted), "{accepted:?}");
        }
        assert_eq!(
            InvalidName.to_string(),
            "Use a name without slashes or control characters."
        );
    }
}
