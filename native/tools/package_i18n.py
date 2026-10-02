# SPDX-License-Identifier: AGPL-3.0-only
"""Translate package descriptions without changing application identifiers.

Desktop entries keep their unqualified English keys and gain Name[locale]
and similar values (the freedesktop Desktop Entry specification). AppStream
metainfo gains sibling text elements with xml:lang, including paragraphs
and keywords as required by the metainfo format. No translated command,
icon, URI, licence identifier or D-Bus name is ever emitted.
"""
from __future__ import annotations

from collections.abc import Iterator, Mapping
from pathlib import Path
import re
from xml.dom import Node, minidom

DESKTOP_KEYS = frozenset(('Name', 'GenericName', 'Comment', 'Keywords'))
METAINFO_TAGS = frozenset(('name', 'summary', 'p', 'li', 'caption', 'keyword'))
LOCALE = re.compile(r'[a-zA-Z]{2,3}(?:_[a-zA-Z0-9]+)?(?:@[a-zA-Z0-9_-]+)?\Z')
XML_NAMESPACE = 'http://www.w3.org/XML/1998/namespace'
Catalogues = Mapping[str, Mapping[str, str]]


def desktop_unescape(text: str) -> str:
    """Read the Desktop Entry string escapes, leaving list separators intact."""
    escapes = {'s': ' ', 'n': '\n', 't': '\t', 'r': '\r', '\\': '\\'}
    return re.sub(r'\\([sntr\\])', lambda match: escapes[match.group(1)], text)


def desktop_escape(text: str) -> str:
    """Keep translation newlines and backslashes inside one key's value."""
    return (text.replace('\\', '\\\\').replace('\n', '\\n')
            .replace('\r', '\\r').replace('\t', '\\t'))


def text_elements(document: minidom.Document) -> Iterator[minidom.Element]:
    """Yield English display text in the plain-text metainfo shipped here."""
    for element in document.getElementsByTagName('*'):
        if element.tagName not in METAINFO_TAGS or element.hasAttribute('xml:lang'):
            continue
        # Inline markup needs translator-aware markup handling; it must not
        # be flattened silently if future metadata introduces it.
        if any(child.nodeType == Node.ELEMENT_NODE for child in element.childNodes):
            raise ValueError(f'Inline markup in translatable <{element.tagName}> is unsupported')
        if element_text(element):
            yield element


def element_text(element: minidom.Element) -> str:
    """Get a metadata element's message, without its XML formatting indent."""
    return ''.join(child.data for child in element.childNodes
                   if child.nodeType in (Node.TEXT_NODE, Node.CDATA_SECTION_NODE)).strip()


def messages(path: Path) -> Iterator[str]:
    """Yield package messages for the common English gettext template."""
    text = path.read_text(encoding='utf-8')
    if path.suffix == '.desktop':
        for line in text.splitlines():
            key, separator, value = line.partition('=')
            if separator and key in DESKTOP_KEYS:
                yield desktop_unescape(value)
    else:
        with minidom.parseString(text) as document:
            for element in text_elements(document):
                yield element_text(element)


def translate(path: Path, catalogues: Catalogues) -> str:
    """Return metadata with available translations, preserving English fallback."""
    for locale in catalogues:
        if not LOCALE.fullmatch(locale):
            raise ValueError(f'Invalid catalogue locale: {locale!r}')
    text = path.read_text(encoding='utf-8')
    if not catalogues:
        return text
    if path.suffix == '.desktop':
        return translate_desktop(text, catalogues)
    return translate_metainfo(text, catalogues)


def translate_desktop(text: str, catalogues: Catalogues) -> str:
    """Add localised strings beside their unqualified Desktop Entry keys."""
    lines = []
    for line in text.splitlines():
        lines.append(line)
        key, separator, value = line.partition('=')
        if not separator or key not in DESKTOP_KEYS:
            continue
        original = desktop_unescape(value)
        for locale, catalogue in sorted(catalogues.items()):
            translated = catalogue.get(original)
            if translated and '\0' not in translated:
                lines.append(f'{key}[{locale}]={desktop_escape(translated)}')
    return '\n'.join(lines) + '\n'


def translate_metainfo(text: str, catalogues: Catalogues) -> str:
    """Add xml:lang siblings, escaping translations as text rather than markup."""
    with minidom.parseString(text) as document:
        for element in list(text_elements(document)):
            original = element_text(element)
            anchor = element.nextSibling
            for locale, catalogue in sorted(catalogues.items()):
                translated = catalogue.get(original)
                if not translated or '\0' in translated:
                    continue
                sibling = document.createElement(element.tagName)
                sibling.setAttributeNS(XML_NAMESPACE, 'xml:lang', locale)
                sibling.appendChild(document.createTextNode(translated))
                element.parentNode.insertBefore(sibling, anchor)
        return document.toxml(encoding='UTF-8').decode('UTF-8') + '\n'
