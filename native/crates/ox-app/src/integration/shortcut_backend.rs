// SPDX-License-Identifier: AGPL-3.0-only
//! Where global shortcuts are read and changed: KDE's shortcut service,
//! or, in tests, a table in memory, so no test changes the shortcuts of
//! the session it runs in (INT-033).

#[cfg(test)]
use std::sync::{Arc, Mutex, PoisonError};

use ox_core::integration::{GlobalShortcuts, KdeShortcuts, KeySequence, ShortcutAction, ShortcutError};

/// Each action of the test table and its keys.
#[cfg(test)]
pub(crate) type ShortcutTable = Arc<Mutex<Vec<(ShortcutAction, Vec<KeySequence>)>>>;

/// Where global shortcuts live.
#[derive(Debug, Clone)]
pub(crate) enum ShortcutBackend {
    /// KDE's shortcut service on the session bus.
    #[cfg_attr(test, expect(dead_code, reason = "tests keep shortcuts in memory"))]
    Desktop(KdeShortcuts),
    /// A table shared by the test and the app, which skips a key another
    /// action has, as KDE's service does.
    #[cfg(test)]
    InMemory(ShortcutTable),
}

impl ShortcutBackend {
    /// The session's shortcuts; in tests, Plasma's as it ships, where
    /// Super+E launches Dolphin.
    pub(crate) fn session() -> Self {
        #[cfg(test)]
        {
            let dolphin = ShortcutAction::launch("org.kde.dolphin.desktop", "Dolphin");
            let table = vec![(dolphin, vec![ox_core::integration::SUPER_E])];
            Self::InMemory(Arc::new(Mutex::new(table)))
        }
        #[cfg(not(test))]
        Self::Desktop(KdeShortcuts)
    }

    /// The test table.
    #[cfg(test)]
    pub(crate) fn table(&self) -> Option<ShortcutTable> {
        match self {
            Self::InMemory(table) => Some(Arc::clone(table)),
            Self::Desktop(_) => None,
        }
    }
}

impl GlobalShortcuts for ShortcutBackend {
    fn owner(&self, keys: KeySequence) -> Result<Option<ShortcutAction>, ShortcutError> {
        match self {
            Self::Desktop(kde) => kde.owner(keys),
            #[cfg(test)]
            Self::InMemory(table) => {
                let table = table.lock().unwrap_or_else(PoisonError::into_inner);
                Ok(table
                    .iter()
                    .find(|(_, owned)| owned.contains(&keys))
                    .map(|(action, _)| action.clone()))
            }
        }
    }

    fn keys(&self, action: &ShortcutAction) -> Result<Vec<KeySequence>, ShortcutError> {
        match self {
            Self::Desktop(kde) => kde.keys(action),
            #[cfg(test)]
            Self::InMemory(table) => {
                let table = table.lock().unwrap_or_else(PoisonError::into_inner);
                Ok(table
                    .iter()
                    .find(|(known, _)| known.component == action.component && known.action == action.action)
                    .map(|(_, keys)| keys.clone())
                    .unwrap_or_default())
            }
        }
    }

    fn register(&self, action: &ShortcutAction) -> Result<(), ShortcutError> {
        match self {
            Self::Desktop(kde) => kde.register(action),
            #[cfg(test)]
            Self::InMemory(table) => {
                let mut table = table.lock().unwrap_or_else(PoisonError::into_inner);
                if !table
                    .iter()
                    .any(|(known, _)| known.component == action.component && known.action == action.action)
                {
                    table.push((action.clone(), Vec::new()));
                }
                Ok(())
            }
        }
    }

    fn set_keys(&self, action: &ShortcutAction, keys: &[KeySequence]) -> Result<(), ShortcutError> {
        match self {
            Self::Desktop(kde) => kde.set_keys(action, keys),
            #[cfg(test)]
            Self::InMemory(table) => {
                let mut table = table.lock().unwrap_or_else(PoisonError::into_inner);
                let free: Vec<KeySequence> = keys
                    .iter()
                    .copied()
                    .filter(|key| {
                        !table.iter().any(|(other, owned)| {
                            (other.component != action.component || other.action != action.action)
                                && owned.contains(key)
                        })
                    })
                    .collect();
                if let Some((_, owned)) = table
                    .iter_mut()
                    .find(|(known, _)| known.component == action.component && known.action == action.action)
                {
                    *owned = free;
                }
                Ok(())
            }
        }
    }
}
