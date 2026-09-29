// SPDX-License-Identifier: AGPL-3.0-only
//! The choices of the Permissions tab, each of which says what the user
//! changed. When several items differ, a choice shows "Varying (No
//! Change)", as Dolphin's does, and keeps each item's own value until the
//! user picks one.

use gtk::prelude::*;
use ox_core::permissions::{Access, Account};

use crate::dialog_layer::check_row;

/// Shown by a choice whose items differ.
const VARYING: &str = "Varying (No Change)";

/// A choice of the three accesses.
#[derive(Debug, Clone)]
pub(super) struct AccessChoice {
    pub(super) choice: gtk::DropDown,
    /// The access every item has, if they share one.
    initial: Option<Access>,
}

impl AccessChoice {
    /// The choice showing `initial`, or "Varying (No Change)".
    pub(super) fn new(label: &str, initial: Option<Access>) -> Self {
        let mut labels: Vec<&str> = Access::ALL.iter().map(|access| access.label()).collect();
        if initial.is_none() {
            labels.push(VARYING);
        }
        let choice = gtk::DropDown::from_strings(&labels);
        let index = initial
            .and_then(|initial| Access::ALL.iter().position(|access| *access == initial))
            .unwrap_or(Access::ALL.len());
        choice.set_selected(u32::try_from(index).unwrap_or_default());
        choice.set_halign(gtk::Align::Start);
        choice.update_property(&[gtk::accessible::Property::Label(label)]);
        Self { choice, initial }
    }

    /// The access chosen, unless it is "Varying (No Change)".
    pub(super) fn chosen(&self) -> Option<Access> {
        let index = usize::try_from(self.choice.selected()).ok()?;
        Access::ALL.get(index).copied()
    }

    /// The access to apply: the one chosen when the user changed it, or
    /// when a folder's change reaches its content, where it may differ.
    pub(super) fn change(&self, recursive: bool) -> Option<Access> {
        let chosen = self.chosen()?;
        (recursive || Some(chosen) != self.initial).then_some(chosen)
    }
}

/// A check box whose items may differ, shown half ticked until the user
/// ticks or clears it.
#[derive(Debug, Clone)]
pub(super) struct StateCheck {
    pub(super) check: gtk::CheckButton,
    initial: Option<bool>,
}

impl StateCheck {
    /// The check box labelled `label`, showing `initial`.
    pub(super) fn new(label: &str, initial: Option<bool>) -> Self {
        let check = check_row(label, initial == Some(true));
        check.set_inconsistent(initial.is_none());
        check.connect_toggled(|check| check.set_inconsistent(false));
        Self { check, initial }
    }

    /// The new state, when the user changed it.
    pub(super) fn change(&self) -> Option<bool> {
        if self.check.is_inconsistent() {
            return None;
        }
        let active = self.check.is_active();
        (Some(active) != self.initial).then_some(active)
    }
}

/// A choice of accounts, showing the items' one, or "Varying (No
/// Change)" when they differ or it is unknown.
#[derive(Debug, Clone)]
pub(super) struct AccountChoice {
    pub(super) choice: gtk::DropDown,
    accounts: Vec<Account>,
    current: Option<u32>,
}

impl AccountChoice {
    /// `accounts`, with `current` first when it is not among them.
    pub(super) fn new(label: &str, current: Option<&Account>, mut accounts: Vec<Account>) -> Self {
        if let Some(current) = current.filter(|current| !accounts.contains(current)) {
            accounts.insert(0, current.clone());
        }
        let mut names: Vec<&str> = accounts.iter().map(|account| account.name.as_str()).collect();
        let index = match current {
            Some(current) => accounts
                .iter()
                .position(|account| account.id == current.id)
                .unwrap_or_default(),
            None => {
                names.push(VARYING);
                accounts.len()
            }
        };
        let choice = gtk::DropDown::from_strings(&names);
        choice.set_selected(u32::try_from(index).unwrap_or_default());
        choice.set_halign(gtk::Align::Start);
        choice.update_property(&[gtk::accessible::Property::Label(label)]);
        Self {
            choice,
            accounts,
            current: current.map(|account| account.id),
        }
    }

    /// The chosen id, when it differs from the current one.
    pub(super) fn changed(&self) -> Option<u32> {
        let index = usize::try_from(self.choice.selected()).ok()?;
        let chosen = self.accounts.get(index)?.id;
        (Some(chosen) != self.current).then_some(chosen)
    }
}
