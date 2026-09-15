# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""No GI installation needed: exercises the production presentation classifier.
Fixtures mirror GVfs smb-browse metadata, not a connection to a real NAS.
"""
import unittest
from entry_model import classify_entry

class EntryModelTests(unittest.TestCase):
    def test_smb_share_is_navigable_not_a_mutable_regular_directory(self):
        e=classify_entry('mountable','smb://nas/work','inode/directory','smb://NAS/work')
        self.assertTrue(e['isDir']); self.assertTrue(e['isVirtual'])
        self.assertFalse(e['canOperate']); self.assertEqual(e['targetUri'],'smb://nas/work')
        self.assertEqual(e['folderType'],'Network share')
    def test_shortcut_server_uses_real_target(self):
        e=classify_entry('shortcut','smb://workgroup/ALPHA','inode/directory','smb://ALPHA/')
        self.assertTrue(e['isDir']); self.assertEqual(e['targetUri'],'smb://alpha/')
        self.assertEqual(e['folderType'],'Network location')
    def test_normal_smb_directory(self):
        e=classify_entry('directory','smb://nas/work/Design','inode/directory')
        self.assertTrue(e['isDir']); self.assertFalse(e['isVirtual']); self.assertTrue(e['canOperate'])
        self.assertIsNone(e['targetUri'])
    def test_local_directory(self):
        self.assertTrue(classify_entry('directory','file:///tmp/Test')['isDir'])
    def test_extensionless_smb_file_is_not_a_folder(self):
        e=classify_entry('file','smb://nas/work/README','text/plain')
        self.assertFalse(e['isDir']); self.assertTrue(e['canOperate'])
    def test_empty_regular_file_with_directory_mime_does_not_become_folder(self):
        self.assertFalse(classify_entry('file','file:///tmp/file','inode/directory')['isDir'])
    def test_unknown_directory_mime(self):
        self.assertTrue(classify_entry('unknown','smb://nas/work/dir','inode/directory')['isDir'])
    def test_unknown_without_metadata_is_not_falsely_a_directory(self):
        self.assertFalse(classify_entry('unknown','smb://nas/work/unknown')['isDir'])
    def test_mountable_share_without_optional_metadata(self):
        self.assertTrue(classify_entry('mountable','smb://nas/work')['isDir'])
    def test_non_smb_mountable_not_assumed_to_be_directory(self):
        self.assertFalse(classify_entry('mountable','file:///tmp/thing')['isDir'])
    def test_mtp_directory_with_bracketed_usb_identifier(self):
        entry = classify_entry('directory', 'mtp://[usb:001,010]/Internal%20storage')
        self.assertTrue(entry['isDir'])
        self.assertTrue(entry['canOperate'])
    def test_file_shortcut_is_not_a_folder(self):
        self.assertFalse(classify_entry('shortcut','file:///tmp/shortcut','text/plain','file:///tmp/file.txt')['isDir'])
    def test_virtual_bad_scheme_is_not_followed(self):
        e=classify_entry('shortcut','smb://group/evil','inode/directory','javascript:alert(1)')
        self.assertFalse(e['isDir']); self.assertIsNone(e['targetUri'])
    def test_credentials_in_backend_target_not_used(self):
        e=classify_entry('mountable','smb://nas/work','inode/directory','smb://u:secret@nas/work')
        self.assertFalse(e['isDir']); self.assertIsNone(e['targetUri'])
    def test_unicode_target_and_spaces(self):
        e=classify_entry('mountable','smb://nas/Team%20files','inode/directory','smb://nas/Team%20files/%C3%89t%C3%A9')
        self.assertEqual(e['targetUri'],'smb://nas/Team%20files/%C3%89t%C3%A9')
    def test_metadata_does_not_override_a_real_file_target(self):
        e=classify_entry('file','file:///tmp/file','text/plain','smb://nas/other')
        self.assertIsNone(e['targetUri']); self.assertFalse(e['isDir'])

if __name__=='__main__':unittest.main()
