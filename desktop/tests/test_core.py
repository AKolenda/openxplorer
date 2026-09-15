# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
import json
import os
from pathlib import Path
import tempfile
import unittest
from core import normalise_location, require_item_uri, require_share, split_location, validate_name, new_copy_name, Settings

class CoreTests(unittest.TestCase):
    def test_unc(self):
        self.assertEqual(normalise_location(r'\\NAS\Team files\Q3 #1'), 'smb://nas/Team%20files/Q3%20%231')
    def test_forward_unc(self):
        self.assertEqual(normalise_location('//NAS/Projects'), 'smb://nas/Projects')
    def test_smb_encoded(self):
        self.assertEqual(normalise_location('smb://NAS/Team%20files/100%25.pdf'), 'smb://nas/Team%20files/100%25.pdf')
    def test_local_spaces_unicode(self):
        self.assertEqual(normalise_location('/tmp/Été #1?.txt'), Path('/tmp/Été #1?.txt').as_uri())
    def test_relative_local(self):
        self.assertEqual(normalise_location('Plans', 'file:///home/a'), 'file:///home/a/Plans')
    def test_relative_smb(self):
        self.assertEqual(normalise_location('Next plan', 'smb://nas/share'), 'smb://nas/share/Next%20plan')
    def test_connected_phone_locations(self):
        cases = {
            'mtp://[usb:001,010]/Internal storage/DCIM': 'mtp://[usb:001,010]/Internal%20storage/DCIM',
            'gphoto2://[usb:001,002]/DCIM': 'gphoto2://[usb:001,002]/DCIM',
            'afc://00008020-001C/': 'afc://00008020-001C/',
        }
        for value, expected in cases.items():
            with self.subTest(value=value):
                normalized = normalise_location(value)
                self.assertEqual(normalized, expected)
                self.assertEqual(split_location(normalized).scheme, expected.split(':', 1)[0])
    def test_relative_connected_phone_path(self):
        self.assertEqual(normalise_location('DCIM/Camera', 'mtp://[usb:001,010]/Internal%20storage'),
                         'mtp://[usb:001,010]/Internal%20storage/DCIM/Camera')
    def test_reject_unsafe_connected_device_addresses(self):
        for value in ('mtp://user@device/DCIM', 'mtp://[usb:001,002/DCIM',
                      'mtp://[usb:001,002]/DCIM?mode=write', 'afc:///DCIM',
                      'mtp://[usb:001,002]/a%00b'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                normalise_location(value)
    def test_device_root_is_not_a_clipboard_item(self):
        with self.assertRaises(ValueError):
            require_item_uri('mtp://[usb:001,010]/')
        self.assertEqual(require_item_uri('mtp://[usb:001,010]/Internal%20storage'),
                         'mtp://[usb:001,010]/Internal%20storage')
    def test_home(self):
        self.assertEqual(normalise_location('~/Docs', home=Path('/home/test')), 'file:///home/test/Docs')
    def test_file_localhost(self):
        self.assertEqual(normalise_location('file://localhost/tmp/x'), 'file:///tmp/x')
    def test_reject_passwords(self):
        for uri in ['smb://u:p@nas/share','smb://u@nas/share',r'\\u:p@nas\share','smb://u%40nas/share']:
            with self.subTest(uri=uri), self.assertRaises(ValueError): normalise_location(uri)
    def test_reject_unsafe_addresses(self):
        for value in ['', 'http://example.org', 'javascript:alert(1)', 'file://nas/share', 'smb:///share', 'smb://nas/a%00b', 'smb://nas/a#b', 'smb://nas/a?b', r'C:\Windows', 'smb://nas%0a/share']:
            with self.subTest(value=value), self.assertRaises(ValueError): normalise_location(value)
    def test_require_actual_share(self):
        for value in ['smb://nas/', '/tmp']:
            with self.assertRaises(ValueError): require_share(value)
        self.assertEqual(require_share(r'\\nas\share'), 'smb://nas/share')
    def test_names(self):
        self.assertEqual(validate_name('Résumé 2026.txt'), 'Résumé 2026.txt')
        for name in ['', '.', '..', '../bad', 'a/b', 'a\\b', 'x\0', 'x\n', 'é'*200]:
            with self.subTest(name=name), self.assertRaises(ValueError): validate_name(name)
    def test_copy_names(self):
        self.assertEqual(new_copy_name('file.pdf',2,False),'file (copy 2).pdf')
        self.assertEqual(new_copy_name('.env',2,False),'.env (copy 2)')
        self.assertEqual(new_copy_name('Folder.v1',3,True),'Folder.v1 (copy 3)')
        self.assertLessEqual(len(new_copy_name('é'*120+'.txt',2,False).encode()),255)
    def test_settings_private_and_atomic(self):
        with tempfile.TemporaryDirectory() as tmp:
            s=Settings(Path(tmp)/'settings')
            s.bookmark('add','share',r'\\NAS\Projects','Projects (Z:)')
            s.update_preferences({'theme':'dark','password':'not-stored','view':'bogus'})
            s2=Settings(Path(tmp)/'settings')
            self.assertEqual(s2.data['shares'][0]['uri'],'smb://nas/Projects')
            self.assertEqual(s2.data['preferences']['theme'],'dark')
            self.assertEqual(s2.data['preferences']['view'],'details')
            self.assertNotIn('not-stored',s.path.read_text())
            self.assertEqual(s.path.stat().st_mode&0o777,0o600)
            self.assertEqual(s.directory.stat().st_mode&0o777,0o700)
            self.assertFalse(list(s.directory.glob('.settings-*')))
    def test_credential_bookmark_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            s=Settings(Path(tmp))
            with self.assertRaises(ValueError): s.bookmark('add','share','smb://u:secret@nas/share')
            self.assertFalse(s.path.exists())
    def test_corrupt_settings(self):
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp,'settings.json').write_text('{bad')
            s=Settings(Path(tmp))
            self.assertTrue(s.warning)
            self.assertEqual(s.data['shares'],[])
    def test_hidden_builtin_pin(self):
        with tempfile.TemporaryDirectory() as tmp:
            s=Settings(Path(tmp))
            s.bookmark('remove','pin','file:///home/a/Desktop')
            self.assertIn('file:///home/a/Desktop',s.data['hiddenQuick'])
            s.bookmark('add','pin','file:///home/a/Desktop','Desktop')
            self.assertNotIn('file:///home/a/Desktop',s.data['hiddenQuick'])

if __name__ == '__main__': unittest.main()
