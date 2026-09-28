// SPDX-License-Identifier: AGPL-3.0-only
//! The `openxplorer-native` executable: a GTK4 file manager with the
//! Explorer skin.
//!
//! The native counterpart of `main()` in `desktop/winspace.py`; everything
//! it does lives in [`ox_app::application::run`].

fn main() -> gtk::glib::ExitCode {
    ox_app::application::run()
}
