// SPDX-License-Identifier: AGPL-3.0-only
//! The interface's translations (INT-031): gettext message catalogues in
//! the desktop's language.
//!
//! Every user-visible string goes through [`gettext`], [`ngettext`] or
//! [`pgettext`], with its English text as the message id, as GNOME and
//! KDE apps do. `native/tools/i18n.py` extracts those calls into the
//! template `native/po/openxplorer.pot`; a translator's `<lang>.po` is
//! compiled to `<prefix>/share/locale/<lang>/LC_MESSAGES/openxplorer.mo`
//! by the package build.
//!
//! At start-up [`install`] picks the catalogue of the first language the
//! desktop asks for that has one (`LANGUAGE`, then `LC_ALL`, `LC_MESSAGES`
//! and `LANG`, as `g_get_language_names` orders them), searching the
//! `locale` folder of every XDG data directory, so `/usr/share`,
//! `/usr/local/share` and a Flatpak's `/app/share` all work. Without one
//! the interface stays in English. The catalogue is read by this module
//! ([`mo`], [`plural`]) rather than by the C library, so no FFI is needed.

mod mo;
mod plural;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use plural::Plural;

/// The gettext domain: the catalogues' file name without `.mo`.
pub const DOMAIN: &str = "openxplorer";

/// What separates a message's context from its id in a catalogue.
const CONTEXT_SEPARATOR: char = '\u{4}';

/// The catalogue the interface uses, once [`install`]ed.
static INSTALLED: OnceLock<Catalog> = OnceLock::new();

/// One language's translations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    /// Each message id, with its context in front, and its translations:
    /// one, or one per plural form.
    messages: HashMap<String, Vec<String>>,
    /// Which plural form a count takes.
    plural: Plural,
}

impl Catalog {
    /// Reads a compiled catalogue (`.mo`); `None` when `bytes` is not one.
    pub fn from_mo(bytes: &[u8]) -> Option<Self> {
        let messages = mo::read(bytes)?;
        let plural = messages
            .get("")
            .and_then(|header| header.first())
            .and_then(|header| Plural::from_header(header))
            .unwrap_or_default();
        Some(Self { messages, plural })
    }

    /// The catalogue of `domain` for the first of `languages` that has one
    /// in a `<dir>/<language>/LC_MESSAGES` folder of `locale_dirs`.
    /// Languages are tried in order (`pt_BR` before `pt`); `C` and
    /// `POSIX` stand for untranslated English and end the search.
    pub fn find(domain: &str, locale_dirs: &[PathBuf], languages: &[String]) -> Option<Self> {
        let file = format!("{domain}.mo");
        let file = file.as_str();
        languages
            .iter()
            .take_while(|language| !matches!(language.as_str(), "C" | "POSIX"))
            .flat_map(|language| {
                locale_dirs
                    .iter()
                    .map(move |dir| dir.join(language).join("LC_MESSAGES").join(file))
            })
            .find_map(|path| read_catalog(&path))
    }

    /// The translation of `msgid`, or `msgid` itself.
    pub fn gettext(&self, msgid: &str) -> String {
        self.lookup(msgid, 0).unwrap_or(msgid).to_owned()
    }

    /// The translation of `msgid` in `context`, or `msgid` itself.
    pub fn pgettext(&self, context: &str, msgid: &str) -> String {
        let key = format!("{context}{CONTEXT_SEPARATOR}{msgid}");
        self.lookup(&key, 0).unwrap_or(msgid).to_owned()
    }

    /// The form of `msgid` (singular) or `plural` that `count` takes in
    /// the catalogue's language; English's when untranslated.
    pub fn ngettext(&self, msgid: &str, plural: &str, count: u64) -> String {
        let english = if count == 1 { msgid } else { plural };
        self.lookup(msgid, self.plural.form(count))
            .unwrap_or(english)
            .to_owned()
    }

    /// The translation `form` of `key`, if translated.
    fn lookup(&self, key: &str, form: usize) -> Option<&str> {
        let forms = self.messages.get(key)?;
        forms
            .get(form)
            .or_else(|| forms.first())
            .map(String::as_str)
            .filter(|text| !text.is_empty())
    }
}

/// The catalogue at `path`, if it exists and is one.
fn read_catalog(path: &Path) -> Option<Catalog> {
    let bytes = std::fs::read(path).ok()?;
    let catalog = Catalog::from_mo(&bytes);
    if catalog.is_none() {
        glib::g_warning!(crate::LOG_DOMAIN, "Not a message catalogue: {}", path.display());
    }
    catalog
}

/// The folders searched for catalogues: `locale` in the user's and every
/// system XDG data directory.
pub fn locale_dirs() -> Vec<PathBuf> {
    std::iter::once(glib::user_data_dir())
        .chain(glib::system_data_dirs())
        .map(|dir| dir.join("locale"))
        .collect()
}

/// Uses the catalogue of the desktop's language for the rest of the run,
/// if one is installed; called once at start-up, before any text is
/// shown. Returns whether a catalogue was found.
pub fn install() -> bool {
    let languages: Vec<String> = glib::language_names().iter().map(ToString::to_string).collect();
    let Some(catalog) = Catalog::find(DOMAIN, &locale_dirs(), &languages) else {
        return false;
    };
    INSTALLED.set(catalog).is_ok()
}

/// The translation of `msgid` in the interface's language.
pub fn gettext(msgid: &str) -> String {
    INSTALLED
        .get()
        .map_or_else(|| msgid.to_owned(), |catalog| catalog.gettext(msgid))
}

/// The translation of `msgid` in `context`, for an id that needs telling
/// apart from the same English text elsewhere.
pub fn pgettext(context: &str, msgid: &str) -> String {
    INSTALLED
        .get()
        .map_or_else(|| msgid.to_owned(), |catalog| catalog.pgettext(context, msgid))
}

/// The singular `msgid` or the `plural` form for `count`, in the
/// interface's language. The text may hold `{count}`, which the caller
/// replaces.
pub fn ngettext(msgid: &str, plural: &str, count: u64) -> String {
    match INSTALLED.get() {
        Some(catalog) => catalog.ngettext(msgid, plural, count),
        None if count == 1 => msgid.to_owned(),
        None => plural.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compiled catalogue of `messages` (id, translations) and a
    /// `Plural-Forms` header, as msgfmt writes it.
    fn mo_file(plural_forms: &str, messages: &[(&str, &[&str])]) -> Vec<u8> {
        let header = format!("Content-Type: text/plain; charset=UTF-8\nPlural-Forms: {plural_forms}\n");
        let mut entries: Vec<(String, String)> = vec![(String::new(), header)];
        entries.extend(
            messages
                .iter()
                .map(|(id, forms)| ((*id).to_owned(), forms.join("\0"))),
        );
        entries.sort();
        mo::write(&entries)
    }

    /// parity: INT-031
    #[test]
    fn the_catalogue_of_the_desktop_language_is_picked_up_with_its_plurals() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let messages = dir.path().join("pl").join("LC_MESSAGES");
        std::fs::create_dir_all(&messages).expect("catalogue folder");
        let catalog = mo_file(
            "nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
            &[
                ("Folder tree", &["Drzewo folderów"]),
                (
                    "{count} item",
                    &["{count} element", "{count} elementy", "{count} elementów"],
                ),
                ("menu\u{4}Open", &["Otwórz"]),
            ],
        );
        std::fs::write(messages.join("openxplorer.mo"), catalog).expect("catalogue");
        let languages = ["pl_PL.UTF-8", "pl_PL", "pl", "C"].map(String::from);

        let catalog = Catalog::find(DOMAIN, &[dir.path().to_owned()], &languages).expect("found");

        assert_eq!(catalog.gettext("Folder tree"), "Drzewo folderów");
        assert_eq!(catalog.gettext("Not translated"), "Not translated");
        assert_eq!(catalog.pgettext("menu", "Open"), "Otwórz");
        let items = |count| catalog.ngettext("{count} item", "{count} items", count);
        assert_eq!(
            [items(1), items(3), items(5), items(22)],
            [
                "{count} element",
                "{count} elementy",
                "{count} elementów",
                "{count} elementy"
            ]
        );
        let english = ["C".to_owned(), "pl".to_owned()];
        assert_eq!(Catalog::find(DOMAIN, &[dir.path().to_owned()], &english), None);
    }
}
