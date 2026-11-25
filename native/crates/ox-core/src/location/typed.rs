// SPDX-License-Identifier: AGPL-3.0-only
//! Text typed or pasted into the address bar, made ready to be read as an
//! address.
//!
//! Windows 11 File Explorer's address bar takes a path between double
//! quotes, as "Copy as path" puts it on the clipboard, and expands
//! environment variables written as `%NAME%`. [`typed_address`] does the
//! same, and also expands the `$NAME` and `${NAME}` forms Linux users
//! type. A variable that is not set is left as typed, as Windows leaves
//! it. `%USERPROFILE%`, Windows' name for the home folder, stands for
//! `HOME` when it is not set itself.

use std::borrow::Cow;

/// The text `typed` with one pair of surrounding double quotes removed
/// and its environment variables expanded through `variable`, which gives
/// a variable's value or `None` when it is not set.
///
/// `%NAME%` is expanded anywhere; `$NAME` only where the name ends the
/// text or is followed by `/`, so a name such as `$HOME notes` stays as
/// typed; `${NAME}` anywhere. An empty value expands to nothing.
pub fn typed_address(typed: &str, variable: impl Fn(&str) -> Option<String>) -> Cow<'_, str> {
    let unquoted = without_quotes(typed);
    if !unquoted.contains(['%', '$']) {
        return unquoted;
    }
    let lookup = |name: &str, any_case: bool| {
        let value = variable(name).or_else(|| any_case.then(|| variable(&name.to_uppercase())).flatten());
        let is_home_alias = any_case && name.eq_ignore_ascii_case("USERPROFILE");
        value.or_else(|| is_home_alias.then(|| variable("HOME")).flatten())
    };
    let mut expanded = String::with_capacity(unquoted.len());
    let mut rest: &str = &unquoted;
    while let Some(at) = rest.find(['%', '$']) {
        expanded.push_str(&rest[..at]);
        let marked = &rest[at..];
        if let Some((value, used)) = expand_one(marked, &lookup) {
            expanded.push_str(&value);
            rest = &marked[used..];
        } else {
            expanded.push_str(&marked[..1]);
            rest = &marked[1..];
        }
    }
    expanded.push_str(rest);
    if expanded == *unquoted {
        unquoted
    } else {
        Cow::Owned(expanded)
    }
}

/// `typed` without one pair of double quotes around all of it, spaces
/// outside them ignored; `typed` itself otherwise.
fn without_quotes(typed: &str) -> Cow<'_, str> {
    let trimmed = typed.trim();
    let inner = trimmed.strip_prefix('"').and_then(|rest| rest.strip_suffix('"'));
    match inner {
        Some(inner) => Cow::Owned(inner.to_owned()),
        None => Cow::Borrowed(typed),
    }
}

/// The value of the variable `marked` starts with (`%NAME%`, `${NAME}` or
/// `$NAME` before a `/` or the end) and how many bytes it takes; `None`
/// when it starts with none, or names one that is not set.
fn expand_one(marked: &str, lookup: &impl Fn(&str, bool) -> Option<String>) -> Option<(String, usize)> {
    if let Some(after) = marked.strip_prefix('%') {
        let end = after.find('%')?;
        let name = &after[..end];
        if !is_variable_name(name) {
            return None;
        }
        return Some((lookup(name, true)?, end + 2));
    }
    let after = marked.strip_prefix('$')?;
    if let Some(braced) = after.strip_prefix('{') {
        let end = braced.find('}')?;
        let name = &braced[..end];
        if !is_variable_name(name) {
            return None;
        }
        return Some((lookup(name, false)?, end + 3));
    }
    let end = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(after.len());
    let name = &after[..end];
    let ends_well = after[end..].is_empty() || after[end..].starts_with('/');
    if !is_variable_name(name) || !ends_well {
        return None;
    }
    Some((lookup(name, false)?, end + 1))
}

/// A name a variable may have: letters, digits and `_`, not starting
/// with a digit, and the `(`, `)` of Windows' `ProgramFiles(x86)`.
fn is_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    let starts_well = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
    starts_well && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '(' | ')'))
}
