# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Opt-in real GIO local checks: /usr/bin/python3 -m unittest tests.gio_integration -v.
Never connects to SMB; never operates outside a TemporaryDirectory.
"""
import tempfile
import unittest
from pathlib import Path
try:
    from gio_backend import GioCancellation, GioNode, create_item, enumerate_folder, rename_item
    AVAILABLE = True
except (ImportError, ValueError):
    AVAILABLE = False
from operations import TransferEngine

@unittest.skipUnless(AVAILABLE, 'System GI/GIO not available; native test not executed')
class GioLocalIntegration(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='winspace-gio-test-')
        self.root = Path(self.temp.name)
        self.cancel = GioCancellation()
    def tearDown(self):
        self.temp.cleanup()
    def test_enumeration_and_creation(self):
        create_item(self.root.as_uri(), 'Created folder', 'folder', self.cancel)
        create_item(self.root.as_uri(), 'Unicode café.txt', 'file', self.cancel)
        rows=[]
        result=enumerate_folder(self.root.as_uri(), True, self.cancel, rows.extend)
        self.assertEqual(result['count'], 2)
        self.assertEqual({r['name'] for r in rows}, {'Created folder', 'Unicode café.txt'})
    def test_recursive_copy_preserves_link_and_source(self):
        src=self.root/'source';src.mkdir();(src/'nested').mkdir()
        (src/'nested'/'payload.txt').write_text('GIO verification')
        (src/'link').symlink_to('nested/payload.txt')
        dest=self.root/'destination';dest.mkdir()
        result=TransferEngine(GioNode).run('copy',[src.as_uri()],dest.as_uri(),'skip',self.cancel)
        self.assertEqual(result.errors, [])
        self.assertEqual((dest/'source'/'nested'/'payload.txt').read_text(), 'GIO verification')
        self.assertTrue((dest/'source'/'link').is_symlink())
        self.assertTrue((src/'nested'/'payload.txt').exists())
    def test_rename_does_not_overwrite(self):
        a=self.root/'a.txt';a.write_text('original')
        b=self.root/'b.txt';b.write_text('competing')
        with self.assertRaises(Exception):
            rename_item(a.as_uri(), 'b.txt', self.cancel)
        self.assertEqual(a.read_text(),'original')
        self.assertEqual(b.read_text(),'competing')
    def test_keep_both(self):
        src=self.root/'payload.txt';src.write_text('new')
        dest=self.root/'dest';dest.mkdir();(dest/'payload.txt').write_text('old')
        result=TransferEngine(GioNode).run('copy',[src.as_uri()],dest.as_uri(),'keep-both',self.cancel)
        self.assertEqual(result.errors, [])
        self.assertEqual((dest/'payload.txt').read_text(), 'old')
        self.assertEqual((dest/'payload (copy 2).txt').read_text(), 'new')

if __name__ == '__main__':
    unittest.main(verbosity=2)
