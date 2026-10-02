# SPDX-License-Identifier: AGPL-3.0-only
"""Package translations retain executable identity, fallback text and notices."""
from __future__ import annotations

import gettext
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from xml.etree import ElementTree

import i18n
import package_data
import package_i18n


class MetadataTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='ox-package-i18n-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_desktop_translates_display_fields_and_never_command_or_identity(self) -> None:
        source = '[Desktop Entry]\nName=Files\nComment=Browse\nExec=Files\nIcon=Files\n'
        source += 'Keywords=files;folders;\n\n[Desktop Action New]\nName=New\nExec=New\n'
        catalogue = {'Files': 'Translated Files', 'Browse': 'Line one\nExec=bad\\path',
                     'files;folders;': 'translated files;translated folders;', 'New': 'Translated New'}
        translated = package_i18n.translate_desktop(source, {'sr@latin': catalogue})
        self.assertIn('Name=Files\nName[sr@latin]=Translated Files\n', translated)
        self.assertIn('Name=New\nName[sr@latin]=Translated New\nExec=New\n', translated)
        self.assertIn('Comment[sr@latin]=Line one\\nExec=bad\\\\path\n', translated)
        self.assertIn('Keywords[sr@latin]=translated files;translated folders;\n', translated)
        self.assertNotIn('\nExec=bad', translated)
        self.assertNotIn('Exec[', translated)
        self.assertNotIn('Icon[', translated)

    def test_metainfo_translates_text_with_xml_escaping_and_keeps_licences(self) -> None:
        source = '<?xml version="1.0"?><!-- SPDX-License-Identifier: CC0-1.0 -->'
        source += '<component><id>Files</id><name>Files</name><summary>Browse</summary>'
        source += '<project_license>AGPL-3.0-only</project_license>'
        source += '<description><p>Browse</p></description>'
        source += '<keywords><keyword>Files</keyword></keywords></component>'
        translated = package_i18n.translate_metainfo(source, {'fr': {'Files': 'A & B',
                                                                 'Browse': '<open>'}})
        root = ElementTree.fromstring(translated)
        language = '{http://www.w3.org/XML/1998/namespace}lang'
        self.assertEqual(root.findtext('id'), 'Files')
        self.assertEqual(root.findtext('project_license'), 'AGPL-3.0-only')
        self.assertEqual([(e.get(language), e.text) for e in root.findall('name')],
                         [(None, 'Files'), ('fr', 'A & B')])
        self.assertEqual(root.find('description/p[@' + language + '="fr"]').text, '<open>')
        self.assertEqual(root.find('keywords/keyword[@' + language + '="fr"]').text, 'A & B')
        self.assertIn('SPDX-License-Identifier: CC0-1.0', translated)
        self.assertNotIn('<open>', translated)

    def test_all_channels_extract_their_visible_package_text(self) -> None:
        for channel in package_data.Channel:
            for suffix in ('.desktop', '.metainfo.xml'):
                source = package_data.PACKAGING_DATA / f'{channel.app_id}{suffix}'
                messages = list(package_i18n.messages(source))
                self.assertTrue(messages)
                self.assertNotIn(channel.app_id, messages)
                self.assertNotIn('AGPL-3.0-only', messages)
                self.assertEqual(package_i18n.translate(source, {}), source.read_text())

    def test_invalid_locale_cannot_inject_a_desktop_key(self) -> None:
        path = self.root / 'test.desktop'
        path.write_text('[Desktop Entry]\nName=Files\n')
        with self.assertRaisesRegex(ValueError, 'Invalid catalogue locale'):
            package_i18n.translate(path, {'fr]\nExec=bad': {'Files': 'Files'}})

    # parity: INT-031
    def test_every_layout_installs_the_same_catalogue_and_translated_metadata(self) -> None:
        po = self.root / 'po'
        po.mkdir()
        (po / 'fr.po').write_text('msgid ""\nmsgstr "Content-Type: text/plain; charset=UTF-8\\n"\n\n'
                                  'msgid "Settings"\nmsgstr "Test settings"\n\n'
                                  'msgid "Explorer-style local and SMB file manager"\n'
                                  'msgstr "Test description"\n\n'
                                  '#, fuzzy\nmsgid "File Explorer"\nmsgstr "Do not ship"\n')
        with patch.object(i18n, 'PO_FOLDER', po):
            for layout in package_data.Layout:
                with self.subTest(layout=layout.value):
                    paths = package_data.installed_paths(package_data.Channel.STABLE, layout)
                    staging = package_data.Staging(self.root / layout.value)
                    package_data.install_desktop_data(staging, package_data.Channel.STABLE, paths)
                    package_data.install_translations(staging, paths.share)
                    desktop = staging.path_of(paths.share / 'applications/io.winspace.Development.desktop')
                    self.assertIn('Name[fr]=Test settings', desktop.read_text())
                    self.assertNotIn('Do not ship', desktop.read_text())
                    metadata = staging.path_of(paths.share / 'metainfo/io.winspace.Development.metainfo.xml')
                    self.assertIn('<summary xml:lang="fr">Test description</summary>', metadata.read_text())
                    catalogue = staging.path_of(paths.share / 'locale/fr/LC_MESSAGES/openxplorer.mo')
                    with catalogue.open('rb') as stream:
                        self.assertEqual(gettext.GNUTranslations(stream).gettext('Settings'), 'Test settings')


if __name__ == '__main__':
    unittest.main()
