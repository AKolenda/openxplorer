# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from core import Settings, is_smb_server, require_item_uri

class PinTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(); self.dir=Path(self.tmp.name)/'config'
        self.store=Settings(self.dir)
        self.quick=['file:///home/test/Desktop','file:///home/test/Downloads','file:///home/test/Documents']
    def tearDown(self):self.tmp.cleanup()
    def test_add_pin_preserves_file_tree(self):
        root=Path(self.tmp.name)/'actual';root.mkdir();(root/'data.txt').write_text('unchanged')
        self.store.pin_many([{'uri':root.as_uri(),'label':'Work'}],quick_order=self.quick)
        self.assertEqual((root/'data.txt').read_text(),'unchanged');self.assertTrue(root.is_dir())
    def test_insert_before_documents(self):
        self.store.pin_many([{'uri':'smb://nas/work','label':'work'}],before=self.quick[2],quick_order=self.quick)
        self.assertEqual(self.store.data['quickOrder'],self.quick[:2]+['smb://nas/work']+self.quick[2:])
    def test_bulk_pin_and_reload(self):
        pins=[{'uri':'smb://nas/work/Plans','label':'Plans'},{'uri':'smb://nas/work/Invoices','label':'Invoices'}]
        self.store.pin_many(pins,quick_order=self.quick)
        new=Settings(self.dir)
        self.assertEqual(new.data['pins'],pins); self.assertEqual(new.data['quickOrder'],self.quick+[p['uri'] for p in pins])
    def test_duplicate_batch_dedupes_canonical_uri(self):
        self.store.pin_many([{'uri':'smb://NAS/work/'},{'uri':'smb://nas/work'}])
        self.assertEqual(len(self.store.data['pins']),1)
    def test_repeat_drag_does_not_duplicate(self):
        self.store.pin_many([{'uri':'smb://nas/work'}],quick_order=self.quick)
        self.store.pin_many([{'uri':'smb://nas/work'}],quick_order=self.store.data['quickOrder'])
        self.assertEqual(len(self.store.data['pins']),1)
    def test_reorder_offline_pin_without_querying_nas(self):
        self.store.pin_many([{'uri':'smb://offline/work'}],quick_order=self.quick)
        self.store.pin_many([{'uri':'smb://offline/work'}],before=self.quick[0],quick_order=self.store.data['quickOrder'])
        self.assertEqual(self.store.data['quickOrder'][0],'smb://offline/work')
    def test_drop_on_self_keeps_order(self):
        self.store.pin_many([{'uri':'smb://nas/work'}],quick_order=self.quick)
        before=self.store.snapshot()
        self.store.pin_many([{'uri':'smb://nas/work'}],before='smb://nas/work',quick_order=self.store.data['quickOrder'])
        self.assertEqual(self.store.data['quickOrder'],before['quickOrder'])
    def test_unpin_removes_order_only_not_share(self):
        self.store.bookmark('add','share','smb://nas/work','Drive')
        self.store.pin_many([{'uri':'smb://nas/work'}],quick_order=self.quick)
        self.store.bookmark('remove','pin','smb://nas/work')
        self.assertNotIn('smb://nas/work',self.store.data['quickOrder']);self.assertEqual(len(self.store.data['shares']),1)
    def test_invalid_batch_has_no_partial_writes(self):
        before=self.store.snapshot()
        with self.assertRaises(ValueError):self.store.pin_many([{'uri':'smb://nas/work'},{'uri':'smb://u:secret@nas/work'}])
        self.assertEqual(self.store.snapshot(),before);self.assertFalse(self.store.path.exists())
    def test_bad_label_does_not_mutate(self):
        before=self.store.snapshot()
        with self.assertRaises(ValueError):self.store.pin_many([{'uri':'smb://nas/work','label':'bad\nlabel'}])
        self.assertEqual(self.store.snapshot(),before)
    def test_save_error_rolls_back_in_memory(self):
        before=self.store.snapshot()
        with patch.object(self.store,'save',side_effect=OSError('disk full')):
            with self.assertRaises(OSError):self.store.pin_many([{'uri':'smb://nas/work'}])
        self.assertEqual(self.store.snapshot(),before)
    def test_system_theme_and_legacy_migration(self):
        self.dir.mkdir();self.store.path.write_text(json.dumps({'version':1,'pins':[{'uri':'smb://nas/work','label':'work'}],'preferences':{'theme':'dark'},'shares':[{'uri':'smb://nas/work','label':'Z:'}]}))
        new=Settings(self.dir);self.assertEqual(new.data['preferences']['theme'],'dark');self.assertEqual(len(new.data['shares']),1)
        new.update_preferences({'theme':'system'});self.assertEqual(Settings(self.dir).data['preferences']['theme'],'system')
    def test_pin_cap(self):
        with self.assertRaises(ValueError):self.store.pin_many([{'uri':f'smb://nas/s/{i}'} for i in range(201)])
    def test_item_guard_does_not_apply_to_folders_inside_share(self):
        self.assertEqual(require_item_uri('smb://nas/work/Projects'),'smb://nas/work/Projects')
        for uri in ('smb://nas/','smb://nas/work','smb://nas/work/'):
            with self.assertRaises(ValueError):require_item_uri(uri)
    def test_server_detection(self):
        self.assertTrue(is_smb_server('smb://nas/'));self.assertFalse(is_smb_server('smb://nas/work'));self.assertFalse(is_smb_server('file:///'))

if __name__=='__main__':unittest.main()
