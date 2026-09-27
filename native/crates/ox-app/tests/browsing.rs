// SPDX-License-Identifier: AGPL-3.0-only
//! Native GTK regression scenario. Run with a display and disposable HOME.

use std::cell::Cell;
use std::fs;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_app::theme::{system::SystemScheme, Appearance, Skin};
use ox_app::window::BrowserWindow;
use ox_core::settings::SettingsData;

fn wait_until(description: &str, predicate: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if predicate() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {description}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn names(browser: &BrowserWindow) -> Vec<String> {
    let model = browser.folder_model();
    (0..model.n_items())
        .map(|position| model.name_at(position))
        .collect()
}

fn action(browser: &BrowserWindow, name: &str, value: Option<&str>) {
    let value = value.map(ToVariant::to_variant);
    gtk::prelude::WidgetExt::activate_action(browser.widget(), name, value.as_ref())
        .expect("window action exists");
}

fn descendants<T: IsA<gtk::Widget> + glib::types::StaticType>(widget: &impl IsA<gtk::Widget>) -> Vec<T> {
    let mut matches = Vec::new();
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Ok(found) = current.clone().downcast::<T>() {
            matches.push(found);
        }
        matches.extend(descendants::<T>(&current));
        child = current.next_sibling();
    }
    matches
}

fn capture(browser: &BrowserWindow, filename: &str) {
    let Some(directory) = std::env::var_os("OX_NATIVE_CAPTURE_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    fs::create_dir_all(&directory).expect("capture directory");
    let window = browser.widget();
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let frames = Rc::new(Cell::new(0));
    let counter = Rc::clone(&frames);
    window.add_tick_callback(move |_, _| {
        counter.set(counter.get() + 1);
        if counter.get() >= 3 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    wait_until("native capture frame", || frames.get() >= 3);
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(window.width()), f64::from(window.height()));
    let node = snapshot.to_node().expect("mapped window has a render node");
    let renderer = window.renderer().expect("mapped window has a renderer");
    let texture = renderer.render_texture(&node, None);
    texture
        .save_to_png(directory.join(filename))
        .expect("save native capture");
}

#[test]
fn browsing_preserves_native_models_and_tab_lifetimes() {
    gtk::init().expect("native browsing test requires a display; use native/tools/check.py");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("test owns GTK main context");
    let application = gtk::Application::builder()
        .application_id("io.winspace.Development.Native.Test")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gio::Cancellable>)
        .expect("register test app");
    let display = gtk::gdk::Display::default().expect("test display");
    let skin = Rc::new(Skin::install(&display));
    let system = SystemScheme::new();
    let browser = BrowserWindow::new(
        &application,
        SettingsData::default(),
        Rc::clone(&skin),
        Rc::clone(&system),
    );
    let selected_while_loading = Rc::new(Cell::new(false));
    let selected = Rc::clone(&selected_while_loading);
    let weak = Rc::downgrade(&browser);
    browser
        .folder_model()
        .sorted()
        .connect_items_changed(move |_, _, _, _| {
            let Some(browser) = weak.upgrade() else { return };
            if !selected.get() && browser.is_loading() && browser.folder_model().n_items() > 1 {
                selected.set(true);
                browser.folder_model().select_only(1);
            }
        });
    let fixture = tempfile::tempdir().expect("synthetic folder");
    let root = fixture.path().join("Example projects");
    fs::create_dir_all(root.join("Documents")).expect("fixture folder");
    for filename in ["Notes 10.txt", "Notes 2.txt", "Résumé.txt", ".private"] {
        fs::write(root.join(filename), b"Synthetic test data\n").expect("fixture file");
    }
    let root_uri = gio::File::for_path(&root).uri().to_string();
    let child_uri = gio::File::for_path(root.join("Documents")).uri().to_string();
    browser.add_tab(&root_uri).expect("open folder");
    browser.widget().present();
    wait_until("first folder", || {
        !browser.is_loading() && browser.widget().is_mapped()
    });
    assert_eq!(browser.load_error(), None);
    assert_eq!(
        names(&browser),
        ["Documents", "Notes 2.txt", "Notes 10.txt", "Résumé.txt"]
    );
    assert!(
        selected_while_loading.get(),
        "selection made before listing completion"
    );
    assert_eq!(
        browser.folder_model().selected_items()[0].entry().name,
        "Notes 2.txt"
    );

    let details = descendants::<gtk::ColumnView>(browser.widget())
        .into_iter()
        .next()
        .expect("details view");
    let modified = details
        .columns()
        .item(1)
        .and_downcast::<gtk::ColumnViewColumn>()
        .expect("modified column");
    details.sort_by_column(Some(&modified), gtk::SortType::Descending);
    let state = |name: &str| {
        browser
            .widget()
            .lookup_action(name)
            .and_then(|action| action.state())
            .and_then(|value| value.get::<String>())
    };
    assert_eq!(state("sort").as_deref(), Some("modified"));
    assert_eq!(state("direction").as_deref(), Some("descending"));
    action(&browser, "win.direction", Some("ascending"));
    action(&browser, "win.sort", Some("name"));

    browser.folder_model().select_only(1);
    assert_eq!(browser.folder_model().summary().count, 1);
    action(&browser, "win.view", Some("large"));
    assert_eq!(
        browser.folder_model().selected_items()[0].entry().name,
        "Notes 2.txt"
    );
    action(&browser, "win.view", Some("details"));
    capture(&browser, "native-browsing-light.png");
    browser.widget().set_default_size(1000, 720);
    capture(&browser, "native-browsing-narrow.png");
    browser.widget().set_default_size(1320, 810);
    action(&browser, "win.hidden", None);
    assert!(names(&browser).contains(&".private".to_string()));
    action(&browser, "win.hidden", None);

    let search = descendants::<gtk::SearchEntry>(browser.widget())
        .into_iter()
        .next()
        .expect("folder filter");
    search.set_text("notes 10");
    wait_until("name filter", || names(&browser) == ["Notes 10.txt"]);
    browser.navigate(&child_uri).expect("navigate child");
    wait_until("empty child", || !browser.is_loading());
    assert_eq!(names(&browser), Vec::<String>::new());
    browser.go_history(-1);
    wait_until("history back", || !browser.is_loading());
    assert_eq!(names(&browser).len(), 4);
    assert_eq!(search.text().as_str(), "");

    browser.folder_model().select_only(2);
    browser.refresh();
    wait_until("refresh selection", || !browser.is_loading());
    assert_eq!(
        browser.folder_model().selected_items()[0].entry().name,
        "Notes 10.txt"
    );
    browser.folder_model().select_uris(&[]);
    assert_eq!(browser.folder_model().summary().count, 0);

    browser.add_tab(&child_uri).expect("second tab");
    wait_until("second tab", || !browser.is_loading());
    action(&browser, "win.previous-tab", None);
    assert_eq!(browser.current_uri().as_deref(), Some(root_uri.as_str()));
    search.set_text("Notes 2");
    wait_until("active filter", || names(&browser) == ["Notes 2.txt"]);
    let close_background = descendants::<gtk::Button>(browser.widget())
        .into_iter()
        .find(|button| button.tooltip_text().as_deref() == Some("Close Documents"))
        .expect("background tab close button");
    close_background.emit_clicked();
    assert_eq!(browser.tab_count(), 1);
    assert_eq!(search.text().as_str(), "Notes 2");
    assert_eq!(names(&browser), ["Notes 2.txt"]);

    browser.navigate(&root_uri).expect("clear filter");
    wait_until("root reload", || !browser.is_loading());
    // Change monitoring performs a real GIO reload without a simulated bridge.
    fs::write(root.join("New document.txt"), b"New synthetic entry").expect("new file");
    wait_until("directory monitor", || {
        names(&browser).contains(&"New document.txt".to_string())
    });
    browser.navigate(&child_uri).expect("first quick navigation");
    browser.navigate(&root_uri).expect("second quick navigation");
    wait_until("latest navigation", || !browser.is_loading());
    assert_eq!(browser.current_uri().as_deref(), Some(root_uri.as_str()));
    assert_eq!(names(&browser).len(), 5);

    let before_invalid = browser.current_uri();
    assert!(browser.navigate("https://example.invalid").is_err());
    assert_eq!(browser.current_uri(), before_invalid);
    browser
        .navigate(root.join("Missing folder").to_str().expect("fixture UTF-8"))
        .expect("valid missing path");
    wait_until("missing folder error", || !browser.is_loading());
    assert!(browser.load_error().is_some());
    browser.navigate(&root_uri).expect("recover after error");
    wait_until("error recovery", || !browser.is_loading());

    let second = BrowserWindow::new(&application, SettingsData::default(), Rc::clone(&skin), system);
    second.add_tab(&child_uri).expect("second window");
    second.widget().present();
    wait_until("second window", || !second.is_loading());
    action(&browser, "win.theme", Some("dark"));
    assert_eq!(skin.appearance(), Appearance::Dark);
    assert_eq!(
        second
            .widget()
            .lookup_action("theme")
            .and_then(|action| action.state())
            .and_then(|value| value.get::<String>())
            .as_deref(),
        Some("dark")
    );
    capture(&browser, "native-browsing-dark.png");
    second.widget().close();
    browser.widget().close();
    drop(second);
    drop(browser);
    while context.pending() {
        context.iteration(false);
    }
}
