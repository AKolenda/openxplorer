// SPDX-License-Identifier: AGPL-3.0-only
//! OpenXplorer native: a GTK4 file manager with the Explorer skin.

mod config;

use gtk::prelude::*;
use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(config::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_activate(|app| {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("OpenXplorer")
            .default_width(1320)
            .default_height(810)
            .build();
        window.set_child(Some(&gtk::Label::new(Some("OpenXplorer native"))));
        window.present();
    });
    app.run()
}
