// SPDX-License-Identifier: AGPL-3.0-only
//! The check a typed name passes before any file operation sees it
//! (OPS-006).
//!
//! Ports `validateName` in `desktop/ui/app.js`. The dialog shows its
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from the name rules of `desktop/tests/test_core.py::CoreTests::test_names`,
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
