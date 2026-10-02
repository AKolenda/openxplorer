#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Keep the native app's message template current and compile its translations.

The app's user-visible strings go through ox_core::i18n's gettext,
ngettext and pgettext, with their English text as the message id
(crates/ox-core/src/i18n.rs). This tool does what xgettext and msgfmt do
for them, with the standard library only, so no build machine needs GNU
gettext:

  i18n.py extract   writes native/po/openxplorer.pot from the Rust sources
  i18n.py check     fails when the template is out of date (check.py runs it)
  i18n.py compile PO MO
                    compiles a translator's .po into a .mo catalogue

Translations live in native/po/<language>.po; package_data.py installs each
one compiled. Machine translations are not accepted.
"""
from __future__ import annotations

import argparse
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path
import re
import struct
import sys
import xml.etree.ElementTree as ET

import package_i18n

NATIVE = Path(__file__).resolve().parents[1]
CRATES = NATIVE / 'crates'
PO_FOLDER = NATIVE / 'po'
TEMPLATE = PO_FOLDER / 'openxplorer.pot'
DOMAIN = 'openxplorer'
UI_FOLDER = CRATES / 'ox-app/resources/ui'
UI_MESSAGES = CRATES / 'ox-app/src/i18n/template_messages.rs'

# A Rust string literal, without raw strings, which the interface does not use
# for messages.
_STRING = r'"((?:[^"\\]|\\.)*)"'
_GAP = r'\s*,\s*'
# A function name that starts there: not a method (catalog.gettext) or part
# of a longer name.
_FUNCTION = r'(?<![.\w])'
# The calls whose literal arguments are messages: (context, id, plural).
CALLS = (
    (re.compile(_FUNCTION + r'message_id\(\s*' + _STRING), None, 1, None),
    (re.compile(_FUNCTION + r'format_message\(\s*' + _STRING), None, 1, None),
    (re.compile(_FUNCTION + r'gettext(?:_static)?\(\s*' + _STRING), None, 1, None),
    (re.compile(_FUNCTION + r'pgettext\(\s*' + _STRING + _GAP + _STRING), 1, 2, None),
    (re.compile(_FUNCTION + r'ngettext\(\s*' + _STRING + _GAP + _STRING), None, 1, 2),
)
RUST_ESCAPES = {'n': '\n', 't': '\t', 'r': '\r', '0': '\0', '\\': '\\', '"': '"', "'": "'"}
PO_ESCAPES = {'\\': '\\\\', '"': '\\"', '\n': '\\n', '\t': '\\t', '\r': '\\r'}

TEMPLATE_HEADER = '''# OpenXplorer interface messages.
# SPDX-License-Identifier: AGPL-3.0-only
# Written by native/tools/i18n.py extract; do not edit by hand.
msgid ""
msgstr ""
"Project-Id-Version: OpenXplorer\\n"
"MIME-Version: 1.0\\n"
"Content-Type: text/plain; charset=UTF-8\\n"
"Content-Transfer-Encoding: 8bit\\n"
"Plural-Forms: nplurals=INTEGER; plural=EXPRESSION;\\n"
'''


@dataclass
class Message:
    """One message of the template, with where the sources use it."""

    context: str | None
    msgid: str
    plural: str | None
    places: list[str] = field(default_factory=list)


def unescape_rust(text: str) -> str:
    """Return the value of a Rust string literal's text."""
    def replace(match: re.Match[str]) -> str:
        escape = match.group(1)
        if escape.startswith('u{'):
            return chr(int(escape[2:-1], 16))
        if escape.startswith('x'):
            return chr(int(escape[1:], 16))
        if escape == '\n':
            # A line continuation: the newline and the next line's indent go.
            return ''
        return RUST_ESCAPES[escape]
    text = re.sub(r'\\\n\s*', '', text)
    return re.sub(r'\\(u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)', replace, text)


def source_files() -> Iterator[Path]:
    """Yield the Rust sources whose messages ship, leaving out test files."""
    for path in sorted(CRATES.rglob('*.rs')):
        parts = path.relative_to(CRATES).parts
        if 'tests' in parts or path.name in ('tests.rs', 'test_support.rs') or 'test_support' in parts:
            continue
        yield path


def messages_in(text: str, place: str) -> Iterator[Message]:
    """Yield the messages of one source file's text."""
    # Inline unit-test fixtures are not application messages.
    text = re.split(r'#\[cfg\(test\)\]\s*mod tests\s*\{', text, maxsplit=1)[0]
    for pattern, context, msgid, plural in CALLS:
        for match in pattern.finditer(text):
            values = [None if index is None else unescape_rust(match.group(index))
                      for index in (context, msgid, plural)]
            yield Message(values[0], values[1] or '', values[2], [place])
    # SettingRow is the one constructor for these static descriptions;
    # it translates the display and search text without changing IDs.
    for row in re.finditer(r'\bRowText\s*\{(?P<fields>.*?)\n\s*\}', text, re.DOTALL):
        for value in re.finditer(r'\b(?:title|description|keywords):\s*' + _STRING, row['fields']):
            message = unescape_rust(value.group(1))
            if message:
                yield Message(None, message, None, [place])


def template_properties(text: str) -> Iterator[tuple[str, str, bool, str]]:
    """Yield only marked object properties, with exact builder IDs.

    Runtime entry values, file labels and action identifiers are never marked.
    XML parsing decodes message entities before catalogue lookup.
    """
    root = ET.fromstring(text)
    for owner in root.iter():
        if owner.tag not in ('object', 'template'):
            continue
        for container, accessible in [(owner, False), *[(a, True) for a in owner.findall('accessibility')]]:
            for prop in container.findall('property'):
                if prop.get('translatable') != 'yes':
                    continue
                if list(prop) or not prop.text or prop.get('context'):
                    raise ValueError('Marked template properties must be plain text without context')
                identifier = '.' if owner.tag == 'template' else owner.get('id')
                if not identifier:
                    raise ValueError('Objects with marked template text need an explicit ID')
                yield identifier, prop.attrib['name'], accessible, prop.text


def ui_properties() -> Iterator[tuple[str, str, str, bool, str]]:
    """Yield template messages together with their source paths."""
    for path in sorted(UI_FOLDER.glob('*.ui')):
        for identifier, prop, accessible, message in template_properties(path.read_text(encoding='utf-8')):
            yield path.relative_to(NATIVE).as_posix(), identifier, prop, accessible, message


def template_messages_source() -> str:
    """Generate the exact properties translated when a template is constructed."""
    rows = sorted({(Path(place).name, identifier, prop, accessible, message)
                   for place, identifier, prop, accessible, message in ui_properties()})
    lines = [
        '// SPDX-License-Identifier: AGPL-3.0-only',
        '// Written by native/tools/i18n.py extract; do not edit by hand.',
        '// (template file, builder object ID, property, accessibility property, message id).',
        '#[rustfmt::skip]',
        'pub(super) const MESSAGES: &[(&str, &str, &str, bool, &str)] = &[',
    ]
    for template, identifier, prop, accessible, message in rows:
        values = [po_string(template), po_string(identifier), po_string(prop),
                  str(accessible).lower(), po_string(message)]
        lines.append('    (' + ', '.join(values) + '),')
    return '\n'.join([*lines, '];', ''])


def extract() -> list[Message]:
    """Return every message of the sources, merged and in a stable order."""
    merged: dict[tuple[str | None, str], Message] = {}
    for path in source_files():
        place = path.relative_to(NATIVE).as_posix()
        for message in messages_in(path.read_text(encoding='utf-8'), place):
            key = (message.context, message.msgid)
            if key in merged:
                if place not in merged[key].places:
                    merged[key].places.append(place)
            else:
                merged[key] = message
    for place, _, _, _, message in ui_properties():
        key = (None, message)
        if key in merged:
            if place not in merged[key].places:
                merged[key].places.append(place)
        else:
            merged[key] = Message(None, message, None, [place])
    for path in sorted((NATIVE / 'packaging/data').glob('*')):
        if path.suffix != '.desktop' and not path.name.endswith('.metainfo.xml'):
            continue
        place = path.relative_to(NATIVE).as_posix()
        for message in package_i18n.messages(path):
            key = (None, message)
            if key in merged:
                if place not in merged[key].places:
                    merged[key].places.append(place)
            else:
                merged[key] = Message(None, message, None, [place])
    return sorted(merged.values(), key=lambda message: (message.places[0], message.msgid))


def po_string(text: str) -> str:
    """Return text as a quoted PO string."""
    return '"' + ''.join(PO_ESCAPES.get(character, character) for character in text) + '"'


def template_text(messages: list[Message]) -> str:
    """Return the template for messages."""
    blocks = [TEMPLATE_HEADER]
    for message in messages:
        lines = []
        if '{' in message.msgid:
            lines.append('#. Keep each {name} as it is: the app puts a value there.')
        lines.append(f'#: {" ".join(sorted(message.places))}')
        if message.context is not None:
            lines.append(f'msgctxt {po_string(message.context)}')
        lines.append(f'msgid {po_string(message.msgid)}')
        if message.plural is None:
            lines.append('msgstr ""')
        else:
            lines += [f'msgid_plural {po_string(message.plural)}', 'msgstr[0] ""', 'msgstr[1] ""']
        blocks.append('\n'.join(lines) + '\n')
    return '\n'.join(blocks)


def parse_po(text: str) -> list[tuple[str, str]]:
    """Return a .po file's translated messages as (original, translation) pairs.

    The original is the context and '\\x04' before the id, and '\\0' and the
    plural id after it; the translation is the forms separated by '\\0', as
    a .mo file stores them. Fuzzy and untranslated messages are left out;
    the header is kept.
    """
    entries: list[tuple[str, str]] = []
    fields: dict[str, str] = {}
    current: str | None = None
    fuzzy = False

    def finish() -> None:
        nonlocal fields, fuzzy
        if 'msgid' in fields:
            forms = [fields[key] for key in sorted(fields, key=form_index) if key.startswith('msgstr')]
            is_header = fields['msgid'] == ''
            if forms and all(forms) and (is_header or not fuzzy):
                original = fields['msgid']
                if 'msgctxt' in fields:
                    original = fields['msgctxt'] + '\x04' + original
                if 'msgid_plural' in fields:
                    original += '\0' + fields['msgid_plural']
                entries.append((original, '\0'.join(forms)))
        fields, fuzzy = {}, False

    def translated() -> bool:
        return any(key.startswith('msgstr') for key in fields)

    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith('#'):
            if translated():
                finish()
            fuzzy = fuzzy or (line.startswith('#,') and 'fuzzy' in line)
            continue
        keyword = re.match(r'(msgctxt|msgid_plural|msgid|msgstr(?:\[\d+\])?)\s+(".*")$', line)
        if keyword:
            current = keyword.group(1)
            if current in ('msgctxt', 'msgid') and translated():
                finish()
            fields[current] = unquote_po(keyword.group(2))
        elif line.startswith('"') and current is not None:
            fields[current] += unquote_po(line)
    finish()
    return entries


def form_index(keyword: str) -> int:
    """Return the plural form a msgstr keyword names: 0 for msgstr and msgstr[0]."""
    found = re.search(r'\[(\d+)\]', keyword)
    return int(found.group(1)) if found else 0


def unquote_po(quoted: str) -> str:
    """Return the value of one quoted PO string."""
    escapes = {'n': '\n', 't': '\t', 'r': '\r', '"': '"', '\\': '\\'}
    return re.sub(r'\\(.)', lambda match: escapes.get(match.group(1), match.group(1)), quoted[1:-1])


def mo_bytes(entries: list[tuple[str, str]]) -> bytes:
    """Return a little-endian .mo catalogue of entries, without a hash table."""
    entries = sorted((original.encode(), translation.encode()) for original, translation in entries)
    count = len(entries)
    originals_at = 28
    translations_at = originals_at + 8 * count
    strings_at = translations_at + 8 * count
    tables = b''
    strings = b''
    for column in (0, 1):
        for entry in entries:
            text = entry[column]
            tables += struct.pack('<II', len(text), strings_at + len(strings))
            strings += text + b'\0'
    header = struct.pack('<7I', 0x950412de, 0, count, originals_at, translations_at, 0, strings_at)
    return header + tables + strings


def check() -> bool:
    """Return whether the template matches the sources; say what to run if not."""
    expected = template_text(extract())
    current = TEMPLATE.read_text(encoding='utf-8') if TEMPLATE.exists() else ''
    sources = UI_MESSAGES.read_text(encoding='utf-8') if UI_MESSAGES.exists() else ''
    stale = [path for path, matches in ((TEMPLATE, current == expected),
                                      (UI_MESSAGES, sources == template_messages_source())) if not matches]
    for path in stale:
        print(f'{path.relative_to(NATIVE.parent)} is out of date: run python3 native/tools/i18n.py extract',
              file=sys.stderr)
    return not stale


def main(argv: list[str] | None = None) -> int:
    """Run the command the arguments name."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('extract', help=f'write {TEMPLATE.name}')
    commands.add_parser('check', help=f'fail when {TEMPLATE.name} is out of date')
    compile_command = commands.add_parser('compile', help='compile a .po file into a .mo file')
    compile_command.add_argument('po', type=Path)
    compile_command.add_argument('mo', type=Path)
    arguments = parser.parse_args(argv)
    if arguments.command == 'extract':
        PO_FOLDER.mkdir(exist_ok=True)
        TEMPLATE.write_text(template_text(extract()), encoding='utf-8')
        UI_MESSAGES.parent.mkdir(exist_ok=True)
        UI_MESSAGES.write_text(template_messages_source(), encoding='utf-8')
        return 0
    if arguments.command == 'check':
        return 0 if check() else 1
    entries = parse_po(arguments.po.read_text(encoding='utf-8'))
    arguments.mo.parent.mkdir(parents=True, exist_ok=True)
    arguments.mo.write_bytes(mo_bytes(entries))
    return 0


if __name__ == '__main__':
    sys.exit(main())
