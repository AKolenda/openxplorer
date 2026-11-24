// SPDX-License-Identifier: AGPL-3.0-only
//! Mapped drive letters typed in the address bar: `Z:` and `Z:\Plans\Q3`.
//!
//! Map network drive names a share after a letter, as Windows does
//! ("Projects (Z:)"). In Windows the letter is a drive, so typing `Z:\` in
//! the address bar opens the share. [`mapped_drive_location`] gives that
//! address the same meaning here: the share mapped to the letter, and the
//! folders after it inside the share. A letter that no share is mapped to
//! is left to the usual rules, which refuse it as a Windows path.

use super::text::quote_component;

/// The location `typed` names through a mapped drive letter: `Z:` or
/// `Z:\` is the share mapped to Z, and `Z:\Plans\Q3` (or with `/`) the
/// folder inside it. `mappings` lists each letter, upper case, with the
/// share's location. `None` when `typed` is not a drive path or no share
/// is mapped to its letter.
pub fn mapped_drive_location(typed: &str, mappings: &[(char, String)]) -> Option<String> {
    let typed = typed.trim();
    let mut characters = typed.chars();
    let letter = characters.next().filter(char::is_ascii_alphabetic)?;
    if characters.next() != Some(':') {
        return None;
    }
    let rest = characters.as_str();
    if !(rest.is_empty() || rest.starts_with(['\\', '/'])) {
        return None;
    }
    let letter = letter.to_ascii_uppercase();
    let (_, share) = mappings.iter().find(|(known, _)| *known == letter)?;
    let components: Vec<String> = rest
        .split(['\\', '/'])
        .filter(|component| !component.is_empty() && *component != ".")
        .map(quote_component)
        .collect();
    if components.iter().any(|component| component == "..") {
        return None;
    }
    if components.is_empty() {
        return Some(share.clone());
    }
    Some(format!(
        "{}/{}",
        share.trim_end_matches('/'),
        components.join("/")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mappings() -> Vec<(char, String)> {
        vec![
            ('Z', "smb://harbornas.local/Projects".to_owned()),
            ('Y', "sftp://harbornas.local:2222/home/nasuser/".to_owned()),
        ]
    }

    /// A letter no share is mapped to, a path climbing out of the share
    /// and text that is not a drive path are not mapped drives.
    #[test]
    fn other_addresses_are_not_mapped_drives() {
        let mappings = mappings();
        let open = |typed: &str| mapped_drive_location(typed, &mappings);
        assert_eq!(open("C:\\Windows"), None);
        assert_eq!(open("Z:\\..\\Private"), None);
        assert_eq!(open("Z:Plans"), None);
        assert_eq!(open("Zebra"), None);
        assert_eq!(open("\\\\harbornas\\Projects"), None);
    }
}
