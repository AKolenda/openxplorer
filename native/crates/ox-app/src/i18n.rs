// SPDX-License-Identifier: AGPL-3.0-only
//! Translates explicitly marked static GtkBuilder properties after their
//! template is bound, before controls are filled with runtime file data.

mod template_messages;

use gtk::prelude::*;
use ox_core::i18n::gettext;

pub(crate) fn translate_template(root: &impl IsA<gtk::Widget>, template: &str) {
    translate_properties(root.as_ref(), template, &gettext);
}

pub(crate) fn translate_properties(root: &gtk::Widget, template: &str, translate: &impl Fn(&str) -> String) {
    fn visit(widget: &gtk::Widget, template: &str, is_root: bool, translate: &impl Fn(&str) -> String) {
        // Nested custom widgets translate their own template while being
        // constructed; builder IDs are local to each template.
        if !is_root && widget.type_().name().starts_with("Ox") {
            return;
        }
        let identifier = widget.buildable_id();
        let identifier = if is_root {
            "."
        } else {
            identifier.as_deref().unwrap_or("")
        };
        for &(file, object, property, accessible, message) in template_messages::MESSAGES {
            if file != template || object != identifier {
                continue;
            }
            let text = translate(message);
            if accessible {
                match property {
                    "label" => widget.update_property(&[gtk::accessible::Property::Label(&text)]),
                    "description" => widget.update_property(&[gtk::accessible::Property::Description(&text)]),
                    _ => unreachable!("unsupported marked accessibility property"),
                }
            } else {
                widget.set_property(property, text);
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            visit(&current, template, false, translate);
        }
    }
    visit(root, template, true, translate);
}
