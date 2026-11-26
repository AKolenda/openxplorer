// SPDX-License-Identifier: AGPL-3.0-only
//! The test process's own widgets read back through the AT-SPI bus, as
//! Orca reads them.
//!
//! GTK keeps more about a widget than it sends to screen readers: GTK
//! 4.14 sends no column-header role, no sort order and no level, so a
//! check on GTK's own accessible properties can pass while Orca hears
//! nothing. These checks ask the accessibility bus instead, over a
//! connection of their own, the way a screen reader does: the registry
//! lists the applications, and each node answers for its role, name,
//! description and states.
//!
//! GTK answers on the main loop of the test thread, so every call waits
//! by iterating that loop. Where no accessibility bus runs (GTK then
//! gives widgets its test context, which reaches no screen reader),
//! [`AtSpi::connect`] returns `None`; with `OX_REQUIRE_ATSPI=1` that is a
//! failure instead, so a run that should check the bus never passes by
//! quietly checking nothing.

use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gio, glib};

/// GTK's accessibility context that reaches no screen reader.
const TEST_CONTEXT: &str = "GtkTestATContext";

/// The variable that makes a missing accessibility bus a failure.
const REQUIRE_VARIABLE: &str = "OX_REQUIRE_ATSPI";

/// The registry's root: the desktop, whose children are the applications.
const REGISTRY: (&str, &str) = ("org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root");

/// AT-SPI's role names, by the number `GetRole` answers (`AtspiRole` in
/// at-spi2-core, as `atspi_role_get_name` names them). GTK's own
/// `GetRoleName` answers with GTK's role names instead, which no screen
/// reader uses.
const ROLE_NAMES: [&str; 130] = [
    "invalid",
    "accelerator label",
    "alert",
    "animation",
    "arrow",
    "calendar",
    "canvas",
    "check box",
    "check menu item",
    "color chooser",
    "column header",
    "combo box",
    "date editor",
    "desktop icon",
    "desktop frame",
    "dial",
    "dialog",
    "directory pane",
    "drawing area",
    "file chooser",
    "filler",
    "focus traversable",
    "font chooser",
    "frame",
    "glass pane",
    "html container",
    "icon",
    "image",
    "internal frame",
    "label",
    "layered pane",
    "list",
    "list item",
    "menu",
    "menu bar",
    "menu item",
    "option pane",
    "page tab",
    "page tab list",
    "panel",
    "password text",
    "popup menu",
    "progress bar",
    "push button",
    "radio button",
    "radio menu item",
    "root pane",
    "row header",
    "scroll bar",
    "scroll pane",
    "separator",
    "slider",
    "spin button",
    "split pane",
    "status bar",
    "table",
    "table cell",
    "table column header",
    "table row header",
    "tearoff menu item",
    "terminal",
    "text",
    "toggle button",
    "tool bar",
    "tool tip",
    "tree",
    "tree table",
    "unknown",
    "viewport",
    "window",
    "extended",
    "header",
    "footer",
    "paragraph",
    "ruler",
    "application",
    "autocomplete",
    "editbar",
    "embedded",
    "entry",
    "chart",
    "caption",
    "document frame",
    "heading",
    "page",
    "section",
    "redundant object",
    "form",
    "link",
    "input method window",
    "table row",
    "tree item",
    "document spreadsheet",
    "document presentation",
    "document text",
    "document web",
    "document email",
    "comment",
    "list box",
    "grouping",
    "image map",
    "notification",
    "info bar",
    "level bar",
    "title bar",
    "block quote",
    "audio",
    "video",
    "definition",
    "article",
    "landmark",
    "log",
    "marquee",
    "math",
    "rating",
    "timer",
    "static",
    "math fraction",
    "math root",
    "subscript",
    "superscript",
    "description list",
    "description term",
    "description value",
    "footnote",
    "content deletion",
    "content insertion",
    "mark",
    "suggestion",
    "push button menu",
];

/// How long one call may take before the check fails.
const CALL_LIMIT: Duration = Duration::from_secs(10);

/// AT-SPI's states, by their bit in `GetState`'s two words
/// (`AtspiStateType` in at-spi2-core).
const STATE_NAMES: [(u32, &str); 22] = [
    (1, "active"),
    (4, "checked"),
    (5, "collapsed"),
    (7, "editable"),
    (9, "expandable"),
    (10, "expanded"),
    (11, "focusable"),
    (12, "focused"),
    (16, "modal"),
    (18, "multiselectable"),
    (20, "pressed"),
    (22, "selectable"),
    (23, "selected"),
    (24, "sensitive"),
    (25, "showing"),
    (30, "visible"),
    (32, "indeterminate"),
    (33, "required"),
    (36, "invalid-entry"),
    (41, "checkable"),
    (42, "has-popup"),
    (43, "read-only"),
];

/// One node of the accessibility tree: its bus name and object path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeRef {
    bus: String,
    path: String,
}

/// What a screen reader hears of one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Heard {
    /// AT-SPI's role name, such as "push button" or "tree item", as Orca
    /// says it.
    pub(crate) role: String,
    /// The accessible name.
    pub(crate) name: String,
    /// The accessible description.
    pub(crate) description: String,
    /// The states set, by AT-SPI's names, such as "expanded".
    pub(crate) states: Vec<&'static str>,
    /// The object attributes, such as "level".
    pub(crate) attributes: Vec<(String, String)>,
}

impl Heard {
    /// Whether the node has `state`.
    pub(crate) fn has(&self, state: &str) -> bool {
        self.states.contains(&state)
    }

    /// The object attribute `name`, if the node has it.
    pub(crate) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A connection to the accessibility bus, rooted at the test process's
/// own application.
#[derive(Debug)]
pub(crate) struct AtSpi {
    connection: gio::DBusConnection,
    application: NodeRef,
}

impl AtSpi {
    /// Connects to the accessibility bus that `widget`'s context talks
    /// to, or returns `None` when it reaches none.
    ///
    /// # Panics
    ///
    /// When `OX_REQUIRE_ATSPI=1` is set and the bus cannot be reached in
    /// any way (no bus, the session bus, the bus's address or the
    /// connection to it failing), or when the bus does not list this
    /// process: a run that requires the screen-reader tests never passes
    /// by skipping them.
    pub(crate) fn connect(widget: &impl IsA<gtk::Accessible>) -> Option<Self> {
        let required = std::env::var(REQUIRE_VARIABLE).is_ok_and(|value| value == "1");
        if widget.at_context().type_().name() == TEST_CONTEXT {
            assert!(
                !required,
                "{REQUIRE_VARIABLE}=1, but GTK reaches no accessibility bus: install at-spi2-core"
            );
            return None;
        }
        match Self::connect_to_bus() {
            Ok(bus) => Some(bus),
            Err(why) => {
                assert!(!required, "{REQUIRE_VARIABLE}=1, but {why}");
                None
            }
        }
    }

    /// Connects to the accessibility bus the session bus names, or says
    /// which step failed.
    fn connect_to_bus() -> Result<Self, String> {
        let session = wait(gio::bus_get_future(gio::BusType::Session))
            .map_err(|error| format!("the session bus cannot be reached: {error}"))?;
        let reply = wait(session.call_future(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            "org.a11y.Bus",
            "GetAddress",
            None,
            Some(glib::VariantTy::new("(s)").expect("a valid type")),
            gio::DBusCallFlags::NONE,
            -1,
        ))
        .map_err(|error| format!("org.a11y.Bus gives no address: {error}"))?;
        let (address,) = reply
            .get::<(String,)>()
            .ok_or_else(|| format!("org.a11y.Bus answered {reply}, not an address"))?;
        let flags = gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
        let connection = wait(gio::DBusConnection::for_address_future(&address, flags, None))
            .map_err(|error| format!("the accessibility bus at {address} cannot be reached: {error}"))?;
        let application = own_application(&connection);
        Ok(Self {
            connection,
            application,
        })
    }

    /// Every node of the application that shows, with what a screen
    /// reader hears of it.
    pub(crate) fn showing(&self) -> Vec<Heard> {
        self.walk(true)
    }

    /// Every node of the application, shown or not.
    pub(crate) fn everything(&self) -> Vec<Heard> {
        self.walk(false)
    }

    /// The nodes of the application in tree order, leaving out those that
    /// do not show, and what is inside them, when `only_showing`.
    fn walk(&self, only_showing: bool) -> Vec<Heard> {
        let mut heard = Vec::new();
        let mut pending = vec![self.application.clone()];
        while let Some(node) = pending.pop() {
            let facts = self.hear(&node);
            let shows = node == self.application || facts.has("showing");
            if only_showing && !shows {
                continue;
            }
            let mut children = self.children(&node);
            children.reverse();
            pending.extend(children);
            if node != self.application {
                heard.push(facts);
            }
        }
        heard
    }

    /// The shown nodes with `role` and `name`.
    pub(crate) fn find(&self, role: &str, name: &str) -> Vec<Heard> {
        self.showing()
            .into_iter()
            .filter(|heard| heard.role == role && heard.name == name)
            .collect()
    }

    /// What a screen reader hears of `node`.
    fn hear(&self, node: &NodeRef) -> Heard {
        let role = self.call(node, "GetRole", "(u)").get::<(u32,)>().map(|(role,)| {
            let known = usize::try_from(role).ok().and_then(|role| ROLE_NAMES.get(role));
            known.map_or_else(|| format!("role {role}"), |name| (*name).to_owned())
        });
        let states = self.call(node, "GetState", "(au)").get::<(Vec<u32>,)>();
        let attributes = self
            .call(node, "GetAttributes", "(a{ss})")
            .get::<(std::collections::HashMap<String, String>,)>()
            .map(|(map,)| {
                let mut pairs: Vec<_> = map.into_iter().collect();
                pairs.sort();
                pairs
            });
        Heard {
            role: role.unwrap_or_default(),
            name: self.property(node, "Name"),
            description: self.property(node, "Description"),
            states: states.map(|(words,)| state_names(&words)).unwrap_or_default(),
            attributes: attributes.unwrap_or_default(),
        }
    }

    /// The children of `node`.
    fn children(&self, node: &NodeRef) -> Vec<NodeRef> {
        let reply = self.call(node, "GetChildren", "(a(so))");
        let children = reply
            .get::<(Vec<(String, glib::variant::ObjectPath)>,)>()
            .map(|(children,)| children)
            .unwrap_or_default();
        children
            .into_iter()
            .map(|(bus, path)| NodeRef {
                bus,
                path: path.as_str().to_owned(),
            })
            .collect()
    }

    /// A string property of the Accessible interface of `node`.
    fn property(&self, node: &NodeRef, name: &str) -> String {
        let reply = wait(self.connection.call_future(
            Some(&node.bus),
            &node.path,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&("org.a11y.atspi.Accessible", name).to_variant()),
            Some(glib::VariantTy::new("(v)").expect("a valid type")),
            gio::DBusCallFlags::NONE,
            -1,
        ));
        reply
            .ok()
            .and_then(|reply| reply.child_value(0).as_variant())
            .and_then(|value| value.get::<String>())
            .unwrap_or_default()
    }

    /// Calls `method` of the Accessible interface of `node`, which
    /// answers with `reply`; an empty tuple when it fails.
    fn call(&self, node: &NodeRef, method: &str, reply: &str) -> glib::Variant {
        wait(self.connection.call_future(
            Some(&node.bus),
            &node.path,
            "org.a11y.atspi.Accessible",
            method,
            None,
            Some(glib::VariantTy::new(reply).expect("a valid type")),
            gio::DBusCallFlags::NONE,
            -1,
        ))
        .unwrap_or_else(|_| ().to_variant())
    }
}

/// The application node of this process, as the registry lists it.
fn own_application(connection: &gio::DBusConnection) -> NodeRef {
    let deadline = Instant::now() + CALL_LIMIT;
    let own_process = std::process::id();
    loop {
        let desktop = NodeRef {
            bus: REGISTRY.0.to_owned(),
            path: REGISTRY.1.to_owned(),
        };
        let probe = AtSpi {
            connection: connection.clone(),
            application: desktop.clone(),
        };
        for application in probe.children(&desktop) {
            let process = wait(connection.call_future(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetConnectionUnixProcessID",
                Some(&(application.bus.as_str(),).to_variant()),
                Some(glib::VariantTy::new("(u)").expect("a valid type")),
                gio::DBusCallFlags::NONE,
                -1,
            ));
            if process.ok().and_then(|reply| reply.get::<(u32,)>()) == Some((own_process,)) {
                return application;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the accessibility registry does not list this process"
        );
        crate::test_support::harness::wait_for(Duration::from_millis(50));
    }
}

/// The names of the states set in AT-SPI's two state words.
fn state_names(words: &[u32]) -> Vec<&'static str> {
    STATE_NAMES
        .iter()
        .filter(|(bit, _)| {
            let word = words
                .get(usize::try_from(bit / 32).unwrap_or(usize::MAX))
                .copied()
                .unwrap_or(0);
            word & (1 << (bit % 32)) != 0
        })
        .map(|(_, name)| *name)
        .collect()
}

/// Runs the main loop of the test thread until `future` finishes, so
/// GTK can answer the calls the future waits for.
///
/// # Panics
///
/// When it takes longer than [`CALL_LIMIT`].
fn wait<T: 'static>(future: impl std::future::Future<Output = T> + 'static) -> T {
    let context = glib::MainContext::default();
    let result = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = result.clone();
    context.spawn_local(async move {
        let value = future.await;
        slot.replace(Some(value));
    });
    let deadline = Instant::now() + CALL_LIMIT;
    loop {
        if let Some(value) = result.borrow_mut().take() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "an accessibility bus call took too long"
        );
        context.iteration(true);
    }
}
