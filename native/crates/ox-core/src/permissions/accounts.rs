// SPDX-License-Identifier: AGPL-3.0-only
//! The owners and groups the Permissions tab offers, as Dolphin's
//! Permissions tab does: the groups the user belongs to, and every user
//! only for the superuser, who alone may give an item away.
//!
//! Names come from `/etc/passwd` and `/etc/group`; an id without a line
//! there is shown as its number.

use std::fs;

/// A user or group: its id and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The numeric id.
    pub id: u32,
    /// The name, or the id written out when no name is known.
    pub name: String,
}

/// The account database of users.
const PASSWD: &str = "/etc/passwd";
/// The account database of groups.
const GROUP: &str = "/etc/group";

/// Whether this process may change an item's owner.
pub fn current_user_is_superuser() -> bool {
    rustix::process::geteuid().is_root()
}

/// The groups the user may give an item: their primary group and every
/// supplementary group, by name, without repeats.
pub fn group_choices() -> Vec<Account> {
    let mut ids = vec![rustix::process::getegid().as_raw()];
    if let Ok(groups) = rustix::process::getgroups() {
        ids.extend(groups.into_iter().map(rustix::process::Gid::as_raw));
    }
    let database = fs::read_to_string(GROUP).unwrap_or_default();
    named(&ids, &parse_database(&database))
}

/// Every user of the account database, for the superuser's owner choice.
pub fn user_choices() -> Vec<Account> {
    let database = fs::read_to_string(PASSWD).unwrap_or_default();
    parse_database(&database)
}

/// The accounts `ids` names, in order, without repeats.
fn named(ids: &[u32], known: &[Account]) -> Vec<Account> {
    let mut accounts: Vec<Account> = Vec::new();
    for &id in ids {
        if accounts.iter().any(|account| account.id == id) {
            continue;
        }
        let name = known
            .iter()
            .find(|account| account.id == id)
            .map_or_else(|| id.to_string(), |account| account.name.clone());
        accounts.push(Account { id, name });
    }
    accounts
}

/// The `name:password:id:…` lines of `/etc/passwd` or `/etc/group`.
fn parse_database(text: &str) -> Vec<Account> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let id = fields.nth(1)?.parse().ok()?;
            (!name.is_empty()).then(|| Account {
                id,
                name: name.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: PROP-007
    #[test]
    fn groups_are_named_from_the_database_and_listed_once() {
        let known = parse_database("# comment\nwheel:x:10:ann\nann:x:1000:\nbroken\nstaff:x:50:\n");

        let groups = named(&[1000, 10, 1000, 4242], &known);

        let names: Vec<&str> = groups.iter().map(|group| group.name.as_str()).collect();
        assert_eq!(names, ["ann", "wheel", "4242"]);
        assert!(!group_choices().is_empty(), "the user's own group is offered");
    }
}
