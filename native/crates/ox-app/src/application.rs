// SPDX-License-Identifier: AGPL-3.0-only
//! Application lifetime, command-line locations and shared display appearance.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::{Settings, SettingsData};

use crate::theme::{system::SystemScheme, Skin, ThemePreference};
use crate::window::BrowserWindow;

struct Desktop {
    settings: SettingsData,
    warning: Option<String>,
    skin: Rc<Skin>,
    system_scheme: Rc<SystemScheme>,
    windows: RefCell<Vec<Rc<BrowserWindow>>>,
}

impl Desktop {
    fn new(app: &gtk::Application) -> Option<Rc<Self>> {
        let display = gtk::gdk::Display::default()?;
        let settings = Settings::open_default();
        let data = settings.data().clone();
        let system_scheme = SystemScheme::new();
        let system_dark = system_scheme.is_dark();
        let skin = Rc::new(Skin::install(&display));
        let theme = ThemePreference::parse(data.preferences.theme.as_str());
        skin.set_preference(theme, system_dark);
        skin.set_text_size(data.preferences.text_size);
        crate::window::install_accelerators(app);
        let desktop = Rc::new(Self {
            settings: data,
            warning: settings.warning().map(str::to_string),
            skin,
            system_scheme,
            windows: RefCell::new(Vec::new()),
        });
        let weak = Rc::downgrade(&desktop);
        app.connect_window_removed(move |_, window| {
            if let Some(desktop) = weak.upgrade() {
                desktop
                    .windows
                    .borrow_mut()
                    .retain(|browser| browser.widget() != window);
            }
        });
        Some(desktop)
    }

    fn open(self: &Rc<Self>, app: &gtk::Application, locations: &[String]) {
        let browser = BrowserWindow::new(
            app,
            self.settings.clone(),
            Rc::clone(&self.skin),
            Rc::clone(&self.system_scheme),
        );
        for uri in locations {
            if let Err(error) = browser.add_tab(uri) {
                eprintln!("{error}");
            }
        }
        if browser.tab_count() == 0 {
            if let Err(error) = browser.add_tab("home:") {
                eprintln!("{error}");
            }
        }
        if let Some(warning) = &self.warning {
            browser.notify(warning);
        }
        self.windows.borrow_mut().push(Rc::clone(&browser));
        browser.widget().present();
    }
}

/// Runs the preview under its isolated application ID, preserving installed
/// file-manager defaults and the production application's DBus identity.
pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(crate::config::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    let desktop: Rc<RefCell<Option<Rc<Desktop>>>> = Rc::new(RefCell::new(None));
    let state = Rc::clone(&desktop);
    app.connect_startup(move |app| {
        state.replace(Desktop::new(app));
    });
    let state = Rc::clone(&desktop);
    app.connect_activate(move |app| {
        if let Some(desktop) = state.borrow().as_ref() {
            desktop.open(app, &["home:".to_string()]);
        }
    });
    let state = Rc::clone(&desktop);
    app.connect_open(move |app, files, _| {
        if let Some(desktop) = state.borrow().as_ref() {
            let locations: Vec<String> = files.iter().map(|file| file.uri().to_string()).collect();
            desktop.open(app, &locations);
        }
    });
    app.connect_shutdown(move |_| {
        desktop.borrow_mut().take();
    });
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_one_window_releases_its_controller_while_another_stays_open() {
        gtk::init().expect("native checks require a private display");
        let app = gtk::Application::builder()
            .application_id("io.winspace.Development.Native.LifetimeTest")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>)
            .expect("register lifetime test");
        let desktop = Desktop::new(&app).expect("test display");
        desktop.open(&app, &["home:".to_string()]);
        desktop.open(&app, &["home:".to_string()]);
        let first = Rc::downgrade(&desktop.windows.borrow()[0]);
        let second = Rc::downgrade(&desktop.windows.borrow()[1]);
        let window = desktop.windows.borrow()[0].widget().clone();
        window.close();
        assert!(
            first.upgrade().is_none(),
            "closed window controller must be released"
        );
        assert!(second.upgrade().is_some());
        let window = desktop.windows.borrow()[0].widget().clone();
        window.close();
        assert!(second.upgrade().is_none());
        assert!(desktop.windows.borrow().is_empty());
    }
}
