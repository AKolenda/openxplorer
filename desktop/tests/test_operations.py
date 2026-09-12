# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
from pathlib import Path
import os
import tempfile
import unittest
from operations import TransferEngine
from tests.local_provider import LocalNode, Cancellation

class TransferTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.root=Path(self.tmp.name)
        self.src=self.root/'source'; self.src.mkdir()
        self.dst=self.root/'destination'; self.dst.mkdir()
        self.cancel=Cancellation()
        self.engine=TransferEngine(LocalNode)
    def tearDown(self): self.tmp.cleanup()
    def run_op(self, paths, mode='copy', policy='skip', target=None, engine=None):
        return (engine or self.engine).run(mode,[p.as_uri() for p in paths],(target or self.dst).as_uri(),policy,self.cancel)
    def no_stage(self): self.assertFalse(list(self.dst.glob('.winspace-transfer-*')))
    def test_copy_file(self):
        p=self.src/'data.bin'; p.write_bytes(os.urandom(35000))
        r=self.run_op([p]); self.assertEqual(len(r.done),1); self.assertFalse(r.errors)
        self.assertEqual((self.dst/p.name).read_bytes(),p.read_bytes()); self.no_stage()
    def test_recursive_copy_includes_hidden(self):
        p=self.src/'tree'; (p/'deep').mkdir(parents=True)
        (p/'deep'/'a.txt').write_text('alpha'); (p/'.hidden').write_text('secret')
        r=self.run_op([p]); self.assertEqual(len(r.done),1); self.assertFalse(r.errors)
        self.assertEqual((self.dst/'tree'/'deep'/'a.txt').read_text(),'alpha')
        self.assertEqual((self.dst/'tree'/'.hidden').read_text(),'secret'); self.no_stage()
    def test_never_overwrite(self):
        p=self.src/'file.txt'; p.write_text('new'); (self.dst/'file.txt').write_text('precious')
        r=self.run_op([p]); self.assertEqual(r.skipped,[p.as_uri()]); self.assertEqual((self.dst/'file.txt').read_text(),'precious'); self.no_stage()
    def test_keep_both(self):
        p=self.src/'file.txt'; p.write_text('new'); (self.dst/'file.txt').write_text('old')
        (self.dst/'file (copy 2).txt').write_text('also old')
        r=self.run_op([p],policy='keep-both'); self.assertEqual(len(r.done),1)
        self.assertEqual((self.dst/'file (copy 3).txt').read_text(),'new')
        self.assertEqual((self.dst/'file.txt').read_text(),'old'); self.no_stage()
    def test_symlink_copied_not_followed(self):
        outside=self.root/'external';outside.mkdir();(outside/'keep').write_text('keep')
        p=self.src/'link';p.symlink_to(outside,target_is_directory=True)
        r=self.run_op([p]); self.assertEqual(len(r.done),1)
        self.assertTrue((self.dst/'link').is_symlink()); self.assertEqual(os.readlink(self.dst/'link'),str(outside)); self.no_stage()
    def test_nested_symlink_loop_not_traversed(self):
        p=self.src/'tree';p.mkdir();(p/'loop').symlink_to(p,target_is_directory=True)
        r=self.run_op([p]); self.assertFalse(r.errors);self.assertTrue((self.dst/'tree'/'loop').is_symlink());self.no_stage()
    def test_reject_self_descendant(self):
        p=self.src/'tree'; p.mkdir(); sub=p/'inside';sub.mkdir()
        r=self.run_op([p],target=sub);self.assertTrue(r.errors);self.assertFalse(list(sub.iterdir()))
    def test_reject_symlink_destination_inside_source(self):
        p=self.src/'tree';p.mkdir();inside=p/'inside';inside.mkdir();alias=self.root/'alias';alias.symlink_to(inside,target_is_directory=True)
        r=self.run_op([p],target=alias);self.assertTrue(r.errors);self.assertFalse(list(inside.iterdir()))
    def test_copy_cancel_removes_partial_stage(self):
        p=self.src/'big'; p.write_bytes(os.urandom(100000))
        engine=TransferEngine(LocalNode,lambda data:self.cancel.cancel() if data.get('fraction',0)>0 else None)
        r=self.run_op([p],engine=engine);self.assertTrue(r.cancelled);self.assertFalse((self.dst/'big').exists());self.assertEqual(p.stat().st_size,100000);self.no_stage()
    def test_cancel_before_start(self):
        p=self.src/'a';p.write_text('a');self.cancel.cancel()
        # Destination preflight deliberately propagates cancellation before a
        # result exists. No data is touched.
        with self.assertRaises(Exception):self.run_op([p])
        self.assertFalse(list(self.dst.iterdir()));self.assertTrue(p.exists())
    def test_move_native(self):
        p=self.src/'a';p.write_text('a');r=self.run_op([p],mode='move')
        self.assertEqual(len(r.done),1);self.assertFalse(p.exists());self.assertEqual((self.dst/'a').read_text(),'a')
    def test_move_collision_keeps_source(self):
        p=self.src/'a';p.write_text('new');(self.dst/'a').write_text('old')
        r=self.run_op([p],mode='move');self.assertEqual(len(r.skipped),1);self.assertEqual(p.read_text(),'new');self.assertEqual((self.dst/'a').read_text(),'old')
    def test_move_same_directory_is_noop(self):
        p=self.src/'a';p.write_text('a');r=self.run_op([p],mode='move',target=self.src,policy='keep-both')
        self.assertEqual(len(r.skipped),1);self.assertTrue(p.exists());self.assertEqual(len(list(self.src.iterdir())),1)
    def test_trash_unsupported_no_delete(self):
        p=self.src/'important';p.write_text('keep');r=self.run_op([p],mode='trash')
        self.assertTrue(r.errors);self.assertEqual(p.read_text(),'keep');self.assertEqual(r.done,[])
    def test_special_file_rejected_cleanup(self):
        p=self.src/'pipe';os.mkfifo(p);r=self.run_op([p]);self.assertTrue(r.errors);self.assertTrue(p.exists());self.no_stage()
    def test_failure_inside_tree_leaves_source(self):
        p=self.src/'tree';p.mkdir();(p/'a').write_text('hello');os.mkfifo(p/'pipe')
        r=self.run_op([p]);self.assertTrue(r.errors);self.assertFalse((self.dst/'tree').exists());self.assertEqual((p/'a').read_text(),'hello');self.no_stage()
    def test_preflight_race_never_overwrites(self):
        p=self.src/'a';p.write_text('new');dst=self.dst
        class RacingNode(LocalNode):
            def move_native(self,target,cancel=None):
                if self.name=='payload': (dst/'a').write_text('racing writer')
                return super().move_native(target,cancel)
        r=self.run_op([p],engine=TransferEngine(RacingNode));self.assertTrue(r.errors)
        self.assertEqual((self.dst/'a').read_text(),'racing writer');self.assertEqual(p.read_text(),'new');self.no_stage()
    def test_move_failure_does_not_copy_delete(self):
        p=self.src/'a';p.write_text('keep')
        class RefuseMove(LocalNode):
            def move_native(self,*_):raise OSError('native move unsupported')
        r=self.run_op([p],mode='move',engine=TransferEngine(RefuseMove));self.assertTrue(r.errors);self.assertEqual(p.read_text(),'keep');self.assertFalse(list(self.dst.iterdir()))
    def test_duplicate_sources_deduplicated(self):
        p=self.src/'a';p.write_text('a');r=self.run_op([p,p]);self.assertEqual(len(r.done),1)
    def test_delete_removes_tree_permanently(self):
        p=self.src/'tree';p.mkdir();(p/'a').write_text('a');(p/'sub').mkdir();(p/'sub'/'b').write_text('b')
        f=self.src/'loose';f.write_text('x')
        r=self.run_op([p,f],mode='delete')
        self.assertEqual(len(r.done),2);self.assertEqual(r.errors,[])
        self.assertFalse(p.exists());self.assertFalse(f.exists())
    def test_delete_does_not_need_a_destination(self):
        p=self.src/'a';p.write_text('a')
        r=self.engine.run('delete',[p.as_uri()],None,'skip',self.cancel)
        self.assertEqual(len(r.done),1);self.assertFalse(p.exists())
    def test_unknown_operation_rejected(self):
        p=self.src/'a';p.write_text('a')
        with self.assertRaises(ValueError): self.run_op([p],mode='erase')
        with self.assertRaises(ValueError): self.run_op([p],policy='overwrite')
        self.assertTrue(p.exists())

if __name__ == '__main__':unittest.main()
