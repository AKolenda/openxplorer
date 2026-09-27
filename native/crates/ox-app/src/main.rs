// SPDX-License-Identifier: AGPL-3.0-only
//! OpenXplorer native: a GTK4 file manager with the Explorer skin.

fn main() -> gtk::glib::ExitCode {
    ox_app::application::run()
}
