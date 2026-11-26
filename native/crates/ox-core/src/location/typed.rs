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
/// the value of the variable named, or `None` when it is not set: with
/// `any_case`, a variable whose name differs only in case counts, as
/// Windows' variables are named in any case ([`environment_variable`]).
///
/// `%NAME%` is expanded anywhere, its name in any case; `$NAME` only where
/// the name ends the text or is followed by `/`, so a name such as `$HOME
/// notes` stays as typed; `${NAME}` anywhere. The `$` forms keep the case
/// of the name, as a Linux shell does. An empty value expands to nothing.
pub fn typed_address(typed: &str, variable: impl Fn(&str, bool) -> Option<String>) -> Cow<'_, str> {
    let unquoted = without_quotes(typed);
    if !unquoted.contains(['%', '$']) {
        return unquoted;
    }
    let lookup = |name: &str, any_case: bool| {
        let value = variable(name, any_case);
        let is_home_alias = any_case && name.eq_ignore_ascii_case("USERPROFILE");
        value.or_else(|| is_home_alias.then(|| variable("HOME", false)).flatten())
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

/// The value of this process's environment variable `name`, for
/// [`typed_address`]: the one of that very name, else, with `any_case`, the
/// first whose name differs from it only in ASCII case, as Windows finds
/// `%PROJECTROOT%` for a variable set as `projectRoot`. `None` when none is
/// set, or its value is no UTF-8.
pub fn environment_variable(name: &str, any_case: bool) -> Option<String> {
    variable_in(std::env::vars_os(), name, any_case)
}

/// The value of `name` among `variables`, as [`environment_variable`]
/// finds it in the environment.
fn variable_in(
    variables: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    name: &str,
    any_case: bool,
) -> Option<String> {
    let mut same_but_case = None;
    for (key, value) in variables {
        let Some(key) = key.to_str() else {
            continue;
        };
        if key == name {
            return value.into_string().ok();
        }
        if any_case && same_but_case.is_none() && key.eq_ignore_ascii_case(name) {
            same_but_case = Some(value);
        }
    }
    same_but_case.and_then(|value| value.into_string().ok())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `%NAME%` finds a variable whatever the case of its name, as Windows
    /// does, preferring the one of that very name; `$NAME` and `${NAME}`
    /// keep their case, as a shell does. `%USERPROFILE%` stands for
    /// `HOME` when it is not set itself.
    ///
    /// parity: NAV-033
    #[test]
    fn percent_variables_are_found_in_any_case_and_dollar_ones_as_typed() {
        let variables = [
            ("projectRoot", "/srv/example/project"),
            ("PROJECTROOT", "/srv/example/upper"),
            ("teamShare", "/srv/example/team"),
            ("HOME", "/home/demo"),
        ];
        let variable = |name: &str, any_case: bool| {
            let pairs = variables
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()));
            variable_in(pairs, name, any_case)
        };
        let expand = |typed: &str| typed_address(typed, variable).into_owned();

        assert_eq!(expand("%TEAMSHARE%/Plans"), "/srv/example/team/Plans");
        assert_eq!(expand("%teamshare%"), "/srv/example/team");
        assert_eq!(
            expand("%projectRoot%"),
            "/srv/example/project",
            "its very name first"
        );
        assert_eq!(expand("%PROJECTROOT%"), "/srv/example/upper");
        assert_eq!(expand("$TEAMSHARE/Plans"), "$TEAMSHARE/Plans", "a shell's case");
        assert_eq!(expand("${teamShare}/Plans"), "/srv/example/team/Plans");
        assert_eq!(expand("%UserProfile%/Notes"), "/home/demo/Notes");
        assert_eq!(expand("%NOT_SET%/x"), "%NOT_SET%/x");
    }
}
