// SPDX-License-Identifier: AGPL-3.0-only
//! The mount assistant: a plan's command, its Linux folder, its removal
//! command and the assistant's refusals. Nothing is mounted or run.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;

use super::*;
use crate::test_support::harness::descendants;

/// Clicks the button labelled `label` inside `assistant`.
fn press(assistant: &gtk::Expander, label: &str) {
    let button = descendants::<gtk::Label>(assistant)
        .into_iter()
        .find(|text| text.label() == label)
        .and_then(|text| text.ancestor(gtk::Button::static_type()))
        .and_downcast::<gtk::Button>()
        .unwrap_or_else(|| panic!("a {label} button"));
    button.emit_clicked();
}

/// The texts of the labels inside `assistant`.
fn texts(assistant: &gtk::Expander) -> Vec<String> {
    descendants::<gtk::Label>(assistant)
        .iter()
        .map(|label| label.label().to_string())
        .collect()
}

/// Prepare setup command shows the steps, the command in a read-only text
/// area, the Linux folder and the removal command; Use this path hands
/// that folder to the Location tab; a share the assistant cannot handle
/// replaces the plan with the reason.
///
/// parity: NET-027
#[gtk::test]
fn the_assistant_prepares_a_command_and_hands_over_the_linux_folder() {
    let used = Rc::new(RefCell::new(Vec::new()));
    let assistant = mount_assistant(glib::clone!(
        #[strong]
        used,
        move |path: &str| used.borrow_mut().push(path.to_owned())
    ));
    assert_eq!(assistant.label().as_deref(), Some("Set up network mount (SMB)"));
    assert!(!assistant.is_expanded(), "the assistant starts collapsed");
    // A collapsed expander holds its content outside the widget tree.
    assistant.set_expanded(true);
    let address = descendants::<gtk::Entry>(&assistant)[0].clone();
    assert_eq!(
        address.placeholder_text().as_deref(),
        Some("\\\\archive-nas\\Shared")
    );

    address.set_text("\\\\nas\\Downloads\\Browser");
    press(&assistant, "Prepare setup command");

    for section in descendants::<gtk::Expander>(&assistant) {
        section.set_expanded(true);
    }
    let command = descendants::<gtk::TextView>(&assistant)[0].clone();
    let buffer = command.buffer();
    let shown = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    assert_eq!(
        shown,
        "sudo /usr/bin/openxplorer-mount-share --share //nas/Downloads"
    );
    assert!(!command.is_editable());
    let plan = mount_plan("//nas/Downloads/Browser", DesktopUser::current()).expect("a plan");
    let target = plan.target_path.display().to_string();
    let labels = texts(&assistant);
    assert!(labels.contains(&STEPS.to_owned()), "{labels:?}");
    assert!(labels.contains(&format!("Linux folder: {target}")), "{labels:?}");
    assert!(labels.contains(&plan.remove_command), "{labels:?}");
    assert!(target.ends_with("/Browser"), "{target}");

    press(&assistant, "Use this path");
    assert_eq!(*used.borrow(), [target]);

    address.set_text("smb://nas:1445/share");
    press(&assistant, "Prepare setup command");
    let labels = texts(&assistant);
    assert!(
        labels.contains(
            &"The persistent mount assistant supports a hostname or IPv4 address without a port.".to_owned()
        ),
        "{labels:?}"
    );
    assert!(
        descendants::<gtk::TextView>(&assistant).is_empty(),
        "the old plan is gone"
    );
}
