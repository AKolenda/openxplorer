// SPDX-License-Identifier: AGPL-3.0-only
//! What an action of the progress window says it does: "Copying 1,718
//! items from Downloads to Backup", with each folder a link that opens it
//! (OPS-019).
//!
//! Windows names the action with its verb (`SPACTION_COPYING`,
//! `SPACTION_MOVING`, `SPACTION_RECYCLING`, `SPACTION_DELETING` in
//! `shobjidl_core.h`), the number of items, where they come from and,
//! for a copy or a move, where they go. Extracting and compressing are
//! not Shell actions; their wording follows the same pattern.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::transfer::TransferMode;

use super::rate::grouped as grouped_number;

/// What an action does to its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verb {
    /// Copying into a folder.
    Copying,
    /// Moving into a folder.
    Moving,
    /// Moving out of the Recycle Bin into the folders the items were
    /// deleted from, as Windows restores them.
    Restoring,
    /// Moving to the Trash.
    Recycling,
    /// Deleting permanently.
    Deleting,
    /// Extracting an archive into a folder.
    Extracting,
    /// Compressing items into an archive.
    Compressing,
}

/// The description of a running action: its verb, how many items, and
/// the locations it reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Action {
    /// What it does.
    pub(crate) verb: Verb,
    /// How many items, when known.
    pub(crate) count: Option<u64>,
    /// Where the items come from: a folder, or the archive extracted.
    pub(crate) source: String,
    /// The folder they go to, for a copy, a move or an extraction.
    pub(crate) destination: Option<String>,
}

impl Action {
    /// The action of `verb` on `uris`, from their folder (or, for an
    /// extraction, the archive itself) to `destination`.
    pub(crate) fn of(verb: Verb, uris: &[String], destination: Option<&str>) -> Option<Self> {
        let first = uris.first()?;
        let source = if verb == Verb::Extracting {
            first.clone()
        } else {
            gio::File::for_uri(first).parent()?.uri().to_string()
        };
        let count = (verb != Verb::Extracting).then(|| u64::try_from(uris.len()).unwrap_or(u64::MAX));
        Some(Self {
            verb,
            count,
            source,
            destination: destination.map(str::to_owned),
        })
    }

    /// The text, with each location a link to it, as Pango markup.
    pub(crate) fn markup(&self) -> String {
        let link = |uri: &str| {
            format!(
                "<a href=\"{}\">{}</a>",
                glib::markup_escape_text(uri),
                glib::markup_escape_text(&location_name(uri))
            )
        };
        let template = glib::markup_escape_text(&self.template()).to_string();
        let count = self.count.map(grouped_number).unwrap_or_default();
        let destination = self.destination.as_deref().map(link).unwrap_or_default();
        substitute(
            &template,
            &[
                ("count", &count),
                ("source", &link(&self.source)),
                ("destination", &destination),
            ],
        )
    }

    /// The translated sentence, with `{count}`, `{source}` and
    /// `{destination}` in it.
    fn template(&self) -> String {
        use ox_core::i18n::{gettext, ngettext};
        let goes = self.destination.is_some();
        match (self.verb, self.count) {
            (Verb::Copying, Some(count)) if goes => ngettext(
                "Copying {count} item from {source} to {destination}",
                "Copying {count} items from {source} to {destination}",
                count,
            ),
            (Verb::Moving, Some(count)) if goes => ngettext(
                "Moving {count} item from {source} to {destination}",
                "Moving {count} items from {source} to {destination}",
                count,
            ),
            (Verb::Restoring, Some(count)) if goes => ngettext(
                "Moving {count} item to {destination}",
                "Moving {count} items to {destination}",
                count,
            ),
            (Verb::Restoring, Some(count)) => ngettext(
                "Moving {count} item to its original folder",
                "Moving {count} items to their original folders",
                count,
            ),
            (Verb::Recycling, Some(count)) => ngettext(
                "Recycling {count} item from {source}",
                "Recycling {count} items from {source}",
                count,
            ),
            (Verb::Deleting, Some(count)) => ngettext(
                "Deleting {count} item from {source}",
                "Deleting {count} items from {source}",
                count,
            ),
            (Verb::Compressing, Some(count)) => ngettext(
                "Compressing {count} item from {source}",
                "Compressing {count} items from {source}",
                count,
            ),
            (Verb::Extracting, _) if goes => gettext("Extracting {source} to {destination}"),
            (verb, _) => gettext(verb.alone()),
        }
    }
}

impl Verb {
    /// The verb of a transfer in `mode`.
    pub(crate) const fn of_mode(mode: TransferMode) -> Self {
        match mode {
            TransferMode::Copy => Verb::Copying,
            TransferMode::Move => Verb::Moving,
            TransferMode::Trash => Verb::Recycling,
            TransferMode::Delete => Verb::Deleting,
        }
    }

    /// The sentence for an action without its count or destination.
    const fn alone(self) -> &'static str {
        match self {
            Verb::Copying => crate::i18n::message_id("Copying from {source}"),
            Verb::Moving | Verb::Restoring => crate::i18n::message_id("Moving from {source}"),
            Verb::Recycling => crate::i18n::message_id("Recycling from {source}"),
            Verb::Deleting => crate::i18n::message_id("Deleting from {source}"),
            Verb::Extracting => crate::i18n::message_id("Extracting {source}"),
            Verb::Compressing => crate::i18n::message_id("Compressing from {source}"),
        }
    }
}

/// The name a location is shown by: its display name, or for a root
/// such as a share, its URI.
pub(crate) fn location_name(uri: &str) -> String {
    let file = gio::File::for_uri(uri);
    if file.has_uri_scheme("trash") && file.parent().is_none() {
        return ox_core::i18n::gettext("Recycle Bin");
    }
    file.basename()
        .filter(|name| file.parent().is_some() && !name.as_os_str().is_empty())
        .map_or_else(
            || file.parse_name().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
}

/// Substitutes `{placeholders}` in `message` once, as
/// `ox_core::i18n::format_message` does, without translating it again: a
/// value is never read as another placeholder.
pub(super) fn substitute(message: &str, values: &[(&str, &str)]) -> String {
    let mut result = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('}') else { break };
        let name = &rest[1..end];
        match values.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => result.push_str(value),
            None => result.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text a label shows of `markup`: its links, which GTK's label
    /// reads and Pango does not, as their text.
    fn text(markup: &str) -> String {
        let mut plain = String::new();
        let mut rest = markup;
        while let Some(start) = rest.find("<a href=") {
            plain.push_str(&rest[..start]);
            let close = rest[start..].find('>').expect("a closed tag") + start;
            rest = &rest[close + 1..];
        }
        plain.push_str(rest);
        let plain = plain.replace("</a>", "");
        let (_, text, _) = gtk::pango::parse_markup(&plain, '\0').expect("valid markup");
        text.to_string()
    }

    fn uris(folder: &str, count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("{folder}/Item {index}.txt"))
            .collect()
    }

    /// Each kind of action has its verb, its count and its locations, as
    /// Windows writes them; the folders are links.
    ///
    /// parity: OPS-019
    #[test]
    fn each_action_says_its_verb_count_and_folders() {
        let downloads = "file:///home/sample/Downloads";
        let backup = "file:///media/sample/Backup";
        let copy = Action::of(Verb::Copying, &uris(downloads, 1718), Some(backup)).unwrap();
        let moving = Action::of(Verb::Moving, &uris(downloads, 1), Some(backup)).unwrap();
        let recycle = Action::of(Verb::Recycling, &uris("file:///home/sample/Videos", 4777), None).unwrap();
        let delete = Action::of(Verb::Deleting, &uris(downloads, 2), None).unwrap();
        let extract = Action::of(
            Verb::Extracting,
            &[format!("{downloads}/Holiday photos.zip")],
            Some("file:///home/sample/Pictures"),
        )
        .unwrap();
        let compress = Action::of(Verb::Compressing, &uris(downloads, 3), Some(downloads)).unwrap();

        assert_eq!(
            text(&copy.markup()),
            "Copying 1,718 items from Downloads to Backup"
        );
        assert_eq!(text(&moving.markup()), "Moving 1 item from Downloads to Backup");
        assert_eq!(text(&recycle.markup()), "Recycling 4,777 items from Videos");
        assert_eq!(text(&delete.markup()), "Deleting 2 items from Downloads");
        assert_eq!(
            text(&extract.markup()),
            "Extracting Holiday photos.zip to Pictures"
        );
        assert_eq!(text(&compress.markup()), "Compressing 3 items from Downloads");
        assert!(copy
            .markup()
            .contains(&format!("<a href=\"{downloads}\">Downloads</a>")));
        assert!(copy
            .markup()
            .contains(&format!("<a href=\"{backup}\">Backup</a>")));
    }

    /// A name with markup characters, or one that looks like a
    /// placeholder, is shown as it is.
    #[test]
    fn folder_names_are_shown_literally() {
        let odd = "file:///home/sample/Tom%20%26%20%7Bdestination%7D";
        let copy = Action::of(Verb::Copying, &uris(odd, 2), Some("file:///home/sample/Out")).unwrap();

        assert_eq!(
            text(&copy.markup()),
            "Copying 2 items from Tom & {destination} to Out"
        );
    }
}
