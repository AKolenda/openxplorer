# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Opt-in real GIO local checks: /usr/bin/python3 -m unittest tests.gio_integration -v.
Never connects to SMB; never operates outside a TemporaryDirectory.
"""
import tempfile
import unittest
import os
import stat
from unittest.mock import patch
from pathlib import Path
try:
    from gio_backend import GioCancellation, GioNode, create_item, enumerate_folder, rename_item
    AVAILABLE = True
except (ImportError, ValueError):
    AVAILABLE = False
from operations import TransferEngine
from previous_versions import PreviousVersions

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
    def test_replace_existing_file(self):
        src=self.root/'payload.txt';src.write_text('new')
        dest=self.root/'dest';dest.mkdir();(dest/'payload.txt').write_text('old')
        result=TransferEngine(GioNode).run('copy',[src.as_uri()],dest.as_uri(),'replace',self.cancel)
        self.assertEqual(result.errors, [])
        self.assertEqual((dest/'payload.txt').read_text(), 'new')
        self.assertEqual(src.read_text(), 'new')
    def test_copy_preserves_private_directory_modes(self):
        src=self.root/'private';src.mkdir(mode=0o700)
        child=src/'restricted';child.mkdir(mode=0o750)
        (child/'payload.txt').write_text('private through parent permissions')
        (child/'payload.txt').chmod(0o644)
        dest=self.root/'dest';dest.mkdir()
        previous_umask=os.umask(0o022)
        try:
            result=TransferEngine(GioNode).run('copy',[src.as_uri()],dest.as_uri(),'skip',self.cancel)
        finally:os.umask(previous_umask)
        self.assertEqual(result.errors,[])
        self.assertEqual(stat.S_IMODE((dest/'private').stat().st_mode),0o700)
        self.assertEqual(stat.S_IMODE((dest/'private'/'restricted').stat().st_mode),0o750)
        self.assertEqual((dest/'private'/'restricted'/'payload.txt').read_text(),'private through parent permissions')

    def test_copy_read_only_directory_preserves_mode(self):
        src=self.root/'read-only';src.mkdir();(src/'payload').write_text('contents');src.chmod(0o500)
        dest=self.root/'dest';dest.mkdir()
        try:
            result=TransferEngine(GioNode).run('copy',[src.as_uri()],dest.as_uri(),'skip',self.cancel)
            self.assertEqual(result.errors,[])
            self.assertEqual(stat.S_IMODE((dest/'read-only').stat().st_mode),0o500)
            self.assertEqual((dest/'read-only'/'payload').read_text(),'contents')
        finally:
            src.chmod(0o700)
            if (dest/'read-only').exists():(dest/'read-only').chmod(0o700)

    def test_merge_read_only_source_keeps_existing_destination_permissions(self):
        src=self.root/'project';src.mkdir();(src/'incoming').write_text('new');src.chmod(0o500)
        dest=self.root/'dest';target=dest/'project';target.mkdir(parents=True);target.chmod(0o700)
        (target/'keep').write_text('existing')
        try:
            result=TransferEngine(GioNode).run('copy',[src.as_uri()],dest.as_uri(),'replace',self.cancel)
            self.assertEqual(result.errors,[])
            self.assertEqual(stat.S_IMODE(target.stat().st_mode),0o700)
            self.assertEqual((target/'incoming').read_text(),'new')
            self.assertEqual((target/'keep').read_text(),'existing')
            self.assertFalse(list(dest.glob('.winspace-transfer-*')))
        finally:src.chmod(0o700)

    def test_failed_publish_cleans_restricted_staging_tree(self):
        src=self.root/'project';src.mkdir();(src/'payload').write_text('contents');src.chmod(0o500)
        dest=self.root/'dest';dest.mkdir()
        try:
            with patch.object(GioNode,'move_native',side_effect=OSError('Simulated rename failure')):
                result=TransferEngine(GioNode).run('copy',[src.as_uri()],dest.as_uri(),'skip',self.cancel)
            self.assertTrue(result.errors)
            self.assertEqual(list(dest.iterdir()),[])
            self.assertEqual((src/'payload').read_text(),'contents')
        finally:src.chmod(0o700)

    def test_recursive_replace_and_delete_preserve_backup_descendant(self):
        versions=PreviousVersions(self.root/'config')
        source=self.root/'source'/'project';target=self.root/'dest'/'project'
        for directory,contents in [(source,'incoming'),(target,'backup')]:
            (directory/'.snapshot').mkdir(parents=True)
            (directory/'.snapshot'/'version.txt').write_text(contents)
        engine=TransferEngine(GioNode,assert_writable=versions.assert_writable)
        replaced=engine.run('copy',[source.as_uri()],target.parent.as_uri(),'replace',self.cancel)
        removed=engine.run('delete',[target.as_uri()],None,'skip',self.cancel)
        self.assertTrue(replaced.errors);self.assertTrue(removed.errors)
        self.assertEqual((target/'.snapshot'/'version.txt').read_text(),'backup')

    def test_rename_parent_of_backup_is_rejected(self):
        versions=PreviousVersions(self.root/'config')
        source=self.root/'project';(source/'.snapshot').mkdir(parents=True)
        (source/'.snapshot'/'version.txt').write_text('backup')
        with self.assertRaisesRegex(ValueError,'read-only'):
            rename_item(source.as_uri(),'renamed',self.cancel,assert_writable=versions.assert_writable)
        self.assertEqual((source/'.snapshot'/'version.txt').read_text(),'backup')
        self.assertFalse((self.root/'renamed').exists())

if __name__ == '__main__':
    unittest.main(verbosity=2)
