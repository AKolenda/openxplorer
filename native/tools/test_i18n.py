# SPDX-License-Identifier: AGPL-3.0-only
"""Tests for i18n.py: message extraction and the .mo files it compiles."""
from __future__ import annotations

import gettext
import io
import unittest

import i18n

SOURCE = r'''
let menu = gettext("Folder tree");
let items = ngettext("{count} item", "{count} items", count);
let open = pgettext("menu", "Open \"here\"");
let ignored = catalog.gettext("A test lookup");
'''

PO = r'''
msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"
"Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);\n"

msgid "Folder tree"
msgstr "Drzewo "
"folderów"

msgid "{count} item"
msgid_plural "{count} items"
msgstr[0] "{count} element"
msgstr[1] "{count} elementy"
msgstr[2] "{count} elementów"

msgctxt "menu"
msgid "Open \"here\""
msgstr "Otwórz \"tutaj\""

#, fuzzy
msgid "Terminal"
msgstr "Terminal?"
'''


class I18nTest(unittest.TestCase):
    def test_template_messages_are_explicit_and_xml_entities_are_decoded(self) -> None:
        source = '''<interface><object class="GtkLabel" id="label">
                    <property name="label" translatable="yes">Windows &amp; tabs</property>
                    <property name="text">A user's Settings filename</property>
                    </object></interface>'''
        properties = list(i18n.template_properties(source))
        self.assertEqual(properties, [('label', 'label', False, 'Windows & tabs')])

    def test_messages_are_extracted_and_a_compiled_catalogue_reads_as_gettext_reads_it(self) -> None:
        messages = list(i18n.messages_in(SOURCE, 'example.rs'))
        self.assertEqual(
            [(message.context, message.msgid, message.plural) for message in messages],
            [(None, 'Folder tree', None), ('menu', 'Open "here"', None),
             (None, '{count} item', '{count} items')])

        catalogue = gettext.GNUTranslations(io.BytesIO(i18n.mo_bytes(i18n.parse_po(PO))))

        self.assertEqual(catalogue.gettext('Folder tree'), 'Drzewo folderów')
        self.assertEqual([catalogue.ngettext('{count} item', '{count} items', n) for n in (1, 3, 5)],
                         ['{count} element', '{count} elementy', '{count} elementów'])
        self.assertEqual(catalogue.pgettext('menu', 'Open "here"'), 'Otwórz "tutaj"')
        self.assertEqual(catalogue.gettext('Terminal'), 'Terminal', 'fuzzy messages stay English')


if __name__ == '__main__':
    unittest.main()
