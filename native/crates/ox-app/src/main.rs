// SPDX-License-Identifier: AGPL-3.0-only
//! The `openxplorer-native` executable: a GTK4 file manager with the
//! Explorer skin.

fn main() -> gtk::glib::ExitCode {
    ox_app::application::run()
}
