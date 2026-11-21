// SPDX-License-Identifier: AGPL-3.0-only
//! The warnings GTK logs while a test runs. GTK logs through the
//! structured log, whose writer a process may set only once, so the first
//! [`LogRecorder::start`] installs a writer that records each warning and
//! hands every message on to the default writer, which prints it as
//! before.

use std::sync::{Mutex, Once};

use gtk::glib;

/// The warnings logged since the recorder started, or `None` while no
/// recorder runs.
static RECORDED: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// Installs the recording writer, once per test process.
static INSTALL: Once = Once::new();

/// Records the warnings logged while it lives.
pub(crate) struct LogRecorder(());

impl LogRecorder {
    /// Starts recording warnings, critical messages and errors.
    pub(crate) fn start() -> Self {
        INSTALL.call_once(|| {
            glib::log_set_writer_func(|level, fields| {
                if matches!(
                    level,
                    glib::LogLevel::Warning | glib::LogLevel::Critical | glib::LogLevel::Error
                ) {
                    let message = fields
                        .iter()
                        .find(|field| field.key() == "MESSAGE")
                        .and_then(glib::LogField::value_str)
                        .unwrap_or_default()
                        .to_owned();
                    if let Some(recorded) = RECORDED
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .as_mut()
                    {
                        recorded.push(message);
                    }
                }
                glib::log_writer_default(level, fields)
            });
        });
        *RECORDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Vec::new());
        Self(())
    }

    /// The warnings logged so far.
    #[expect(
        clippy::unused_self,
        reason = "the warnings are read from a live recorder, which keeps recording"
    )]
    pub(crate) fn warnings(&self) -> Vec<String> {
        RECORDED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_default()
    }
}

impl Drop for LogRecorder {
    fn drop(&mut self) {
        *RECORDED.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}
