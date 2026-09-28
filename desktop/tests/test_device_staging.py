# SPDX-License-Identifier: AGPL-3.0-only
"""Sibling staging for device backends whose native move cannot rename across
folders (GVfs MTP).

``DeviceNode`` follows the adapter contract that ``GioNode`` provides for
``mtp://`` after the fix, as measured on a Pixel 9 with GVfs 1.54.4:
a same-folder move is a non-overwriting rename (set_display_name); a
cross-folder move keeps the item's name; a cross-folder move under a
different name is refused; one-step overwrite is unsupported.
"""
from pathlib import Path
import os
import re
import tempfile
import unittest
from operations import ReplaceUnsupported, TransferEngine

STAGING_NAME = re.compile(r'\.winspace-transfer-[0-9a-f]{32}\.part')


def is_own_staging_name(name):
    return isinstance(name, str) and STAGING_NAME.fullmatch(name) is not None
from tests.local_provider import LocalNode, Cancellation

CALLS = []


class DeviceNode(LocalNode):
    stage_as_sibling = True

    def move_native(self, target, cancel=None):
        CALLS.append(('move', self.p, target.p))
        if cancel:
            cancel.check()
        if self.p.parent == target.p.parent:
            if os.path.lexists(target.p):
                raise FileExistsError(f'An item named “{target.name}” already exists.')
            return super().move_native(target, cancel)
        if self.name != target.name:
            raise ValueError('This device can move an item to another folder or rename it, but not both in one step.')
        return super().move_native(target, cancel)

    def replace_native(self, target, cancel=None):
        raise ReplaceUnsupported('This device cannot replace an item in one step.')

    def refresh_listing(self, cancel=None):
        CALLS.append(('refresh', self.p))


class DeviceStagingTests(unittest.TestCase):
    def setUp(self):
        CALLS.clear()
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.src = root / 'source'; self.src.mkdir()
        self.dst = root / 'phone'; self.dst.mkdir()
        self.cancel = Cancellation()
        self.sleeps = []

    def tearDown(self):
        self.tmp.cleanup()

    def engine(self, node=DeviceNode, emit=None):
        return TransferEngine(node, emit, sleep=self.sleeps.append)

    def run_op(self, paths, mode='copy', policy='skip', target=None, engine=None):
        return (engine or self.engine()).run(mode, [p.as_uri() for p in paths],
                                             (target or self.dst).as_uri(), policy, self.cancel)

    def leftovers(self, root=None):
        return sorted(str(p.relative_to(self.dst)) for p in (root or self.dst).rglob('*')
                      if p.name.startswith('.winspace-') or p.name == 'payload')

    def assert_only_same_folder_renames_across_names(self):
        for call in CALLS:
            if call[0] == 'move':
                _, a, b = call
                self.assertTrue(a.parent == b.parent or a.name == b.name, call)

    def test_file_copy_publishes_by_same_folder_rename(self):
        p = self.src / 't3code.apk'; p.write_bytes(os.urandom(5000))
        r = self.run_op([p])
        self.assertEqual((r.done, r.errors), ([p.as_uri()], []))
        self.assertEqual((self.dst / p.name).read_bytes(), p.read_bytes())
        self.assertEqual(self.leftovers(), [])
        (move,) = [c for c in CALLS if c[0] == 'move']
        self.assertTrue(is_own_staging_name(move[1].name))
        self.assertEqual(move[1].parent, self.dst)
        self.assertEqual(move[2], self.dst / p.name)

    def test_partial_copy_never_visible_under_final_name(self):
        tree = self.src / 'tree'; (tree / 'sub').mkdir(parents=True)
        (tree / 'sub' / 'b.txt').write_text('b'); (tree / 'a.txt').write_text('a')
        dst = self.dst
        seen = []
        class Watching(DeviceNode):
            def copy_file(self, target, cancel, progress):
                seen.append((os.path.lexists(dst / 'tree'), target.p))
                return super().copy_file(target, cancel, progress)
        r = self.run_op([tree], engine=self.engine(Watching))
        self.assertEqual(r.errors, [])
        self.assertTrue(seen)
        for final_visible, staged in seen:
            self.assertFalse(final_visible)
            self.assertTrue(is_own_staging_name(staged.relative_to(dst).parts[0]))
        self.assertEqual((dst / 'tree' / 'sub' / 'b.txt').read_text(), 'b')
        self.assertEqual(self.leftovers(), [])
        self.assert_only_same_folder_renames_across_names()

    def test_second_copy_into_same_folder_succeeds(self):
        # Regression: the old engine left the first copy as "payload" and the
        # second failed with "libmtp error: could not move object".
        a = self.src / 'a.apk'; a.write_text('a'); b = self.src / 'b.apk'; b.write_text('b')
        self.assertEqual(self.run_op([a]).errors, [])
        self.assertEqual(self.run_op([b]).errors, [])
        self.assertEqual(sorted(os.listdir(self.dst)), ['a.apk', 'b.apk'])

    def test_replace_file_uses_reversible_renames(self):
        p = self.src / 'a'; p.write_text('new'); (self.dst / 'a').write_text('old')
        r = self.run_op([p], policy='replace')
        self.assertEqual((r.done, r.errors), ([p.as_uri()], []))
        self.assertEqual((self.dst / 'a').read_text(), 'new')
        self.assertEqual(self.leftovers(), [])
        self.assert_only_same_folder_renames_across_names()

    def test_replace_install_failure_restores_original_and_cleans_stage(self):
        class FailInstall(DeviceNode):
            def move_native(self, target, cancel=None):
                if is_own_staging_name(self.name) and target.name == 'a':
                    raise OSError('simulated device refusal')
                return super().move_native(target, cancel)
        p = self.src / 'a'; p.write_text('new'); (self.dst / 'a').write_text('old')
        r = self.run_op([p], policy='replace', engine=self.engine(FailInstall))
        self.assertTrue(r.errors); self.assertEqual(r.done, [])
        self.assertEqual((self.dst / 'a').read_text(), 'old')
        self.assertEqual(self.leftovers(), [])

    def test_replace_merges_folders_and_keeps_destination_only_items(self):
        s = self.src / 'd'; s.mkdir(); (s / 'same').write_text('new'); (s / 'added').write_text('added')
        d = self.dst / 'd'; d.mkdir(); (d / 'same').write_text('old'); (d / 'keep').write_text('keep')
        r = self.run_op([s], policy='replace')
        self.assertEqual(r.errors, [])
        self.assertEqual({n: (d / n).read_text() for n in os.listdir(d)},
                         {'same': 'new', 'added': 'added', 'keep': 'keep'})
        self.assertEqual(self.leftovers(), [])
        self.assert_only_same_folder_renames_across_names()

    def test_skip_never_touches_existing(self):
        p = self.src / 'a'; p.write_text('new'); (self.dst / 'a').write_text('old')
        r = self.run_op([p], policy='skip')
        self.assertEqual(r.skipped, [p.as_uri()]); self.assertEqual((self.dst / 'a').read_text(), 'old')
        self.assertEqual([c for c in CALLS if c[0] == 'move'], [])

    def test_cancel_mid_copy_leaves_no_stage_and_no_final_name(self):
        p = self.src / 'big'; p.write_bytes(os.urandom(100000))
        engine = self.engine(emit=lambda d: self.cancel.cancel() if d.get('fraction', 0) > 0 else None)
        r = self.run_op([p], engine=engine)
        self.assertTrue(r.cancelled); self.assertEqual(r.errors, [])
        self.assertFalse((self.dst / 'big').exists()); self.assertEqual(os.listdir(self.dst), [])

    def test_publish_race_never_overwrites(self):
        dst = self.dst
        class Racing(DeviceNode):
            def move_native(self, target, cancel=None):
                if is_own_staging_name(self.name):
                    (dst / 'a').write_text('racing writer')
                return super().move_native(target, cancel)
        p = self.src / 'a'; p.write_text('new')
        r = self.run_op([p], engine=self.engine(Racing))
        self.assertTrue(r.errors)
        self.assertEqual((dst / 'a').read_text(), 'racing writer'); self.assertEqual(self.leftovers(), [])

    def test_success_report_without_rename_is_an_error(self):
        # GVfs MTP reported success while keeping the old name.
        class SilentNoop(DeviceNode):
            def move_native(self, target, cancel=None):
                return None
        p = self.src / 'a'; p.write_text('new')
        r = self.run_op([p], engine=self.engine(SilentNoop))
        self.assertEqual(r.done, []); self.assertIn('not at', r.errors[0])
        self.assertEqual(os.listdir(self.dst), [])

    def test_cleanup_retries_a_transient_device_error(self):
        state = {'fail': 1}
        class Flaky(DeviceNode):
            def delete(self):
                if is_own_staging_name(self.name) and state['fail']:
                    state['fail'] -= 1
                    raise OSError('libmtp error: could not get object handles.')
                return super().delete()
            def copy_file(self, target, cancel, progress):
                super().copy_file(target, cancel, progress)
                raise OSError('simulated transfer error after writing')
        p = self.src / 'a'; p.write_text('data')
        r = self.run_op([p], engine=self.engine(Flaky))
        self.assertEqual(len(r.errors), 1); self.assertIn('simulated transfer error', r.errors[0])
        self.assertEqual(self.sleeps, [.5]); self.assertEqual(os.listdir(self.dst), [])

    def test_missing_stage_after_aborted_upload_is_not_reported_as_leftover(self):
        class Discarded(DeviceNode):
            def copy_file(self, target, cancel, progress):
                raise OSError('device discarded the aborted upload')
        p = self.src / 'a'; p.write_text('data')
        r = self.run_op([p], engine=self.engine(Discarded))
        self.assertEqual(len(r.errors), 1); self.assertNotIn('Incomplete staging', r.errors[0])

    def test_not_found_for_an_existing_stage_is_confirmed_by_listing(self):
        # GVfs MTP reports NOT_FOUND when it fails to look up an uncached path.
        class Unsure(DeviceNode):
            def info(self, cancel=None):
                if is_own_staging_name(self.name) and state['lookups']:
                    state['lookups'] -= 1
                    raise FileNotFoundError('File not found')
                return super().info(cancel)
            def copy_file(self, target, cancel, progress):
                super().copy_file(target, cancel, progress)
                raise OSError('LIBMTP_Send_File_From_File_Descriptor(): Could not retrieve updated metadata.')
        state = {'lookups': 1}
        p = self.src / 'a'; p.write_text('data')
        r = self.run_op([p], engine=self.engine(Unsure))
        self.assertEqual(len(r.errors), 1); self.assertNotIn('Incomplete staging', r.errors[0])
        self.assertEqual(self.sleeps, [.5]); self.assertEqual(os.listdir(self.dst), [])

    def test_persistent_cleanup_failure_reports_exact_location(self):
        class Stuck(DeviceNode):
            def delete(self):
                if is_own_staging_name(self.name):
                    raise OSError('device busy')
                return super().delete()
            def copy_file(self, target, cancel, progress):
                super().copy_file(target, cancel, progress)
                raise OSError('simulated transfer error')
        p = self.src / 'a'; p.write_text('data')
        r = self.run_op([p], engine=self.engine(Stuck))
        (stage,) = os.listdir(self.dst)
        self.assertTrue(is_own_staging_name(stage))
        self.assertTrue(any(f'Incomplete staging item left at {(self.dst / stage).as_uri()}' in e for e in r.errors))
        self.assertEqual(self.sleeps, [.5, 1.5])

    def test_same_device_file_copy_is_built_inside_a_private_folder(self):
        # MTP CopyObject keeps the SOURCE name whatever target is requested.
        writes = []
        class SameDevice(DeviceNode):
            def native_copy_keeps_name(self, _target_dir): return True
            def copy_file(self, target, cancel, progress):
                landed = type(self)(path=target.p.parent / self.name)
                writes.append((landed.p, os.path.lexists(dst / 'a.txt')))
                return super().copy_file(landed, cancel, progress)
        dst = self.dst
        p = self.src / 'a.txt'; p.write_text('data')
        r = self.run_op([p], engine=self.engine(SameDevice))
        self.assertEqual((r.done, r.errors), ([p.as_uri()], []))
        ((landed, final_visible),) = writes
        self.assertTrue(is_own_staging_name(landed.parent.name))
        self.assertEqual(landed.parent.parent, dst)
        self.assertFalse(final_visible)
        self.assertEqual((dst / 'a.txt').read_text(), 'data')
        self.assertEqual(self.leftovers(), [])
        self.assertIn(('refresh', landed.parent), CALLS)
        self.assert_only_same_folder_renames_across_names()

    def test_same_device_keep_both_renames_inside_the_private_folder(self):
        class SameDevice(DeviceNode):
            def native_copy_keeps_name(self, _target_dir): return True
            def copy_file(self, target, cancel, progress):
                return super().copy_file(type(self)(path=target.p.parent / self.name), cancel, progress)
        p = self.src / 'a.txt'; p.write_text('new'); (self.dst / 'a.txt').write_text('old')
        r = self.run_op([p], policy='keep-both', engine=self.engine(SameDevice))
        self.assertEqual(r.errors, [])
        self.assertEqual((self.dst / 'a.txt').read_text(), 'old')
        (copy,) = [n for n in os.listdir(self.dst) if n != 'a.txt']
        self.assertEqual((self.dst / copy).read_text(), 'new')
        self.assertEqual(self.leftovers(), [])
        self.assert_only_same_folder_renames_across_names()

    def test_same_device_copy_failure_leaves_nothing_under_the_final_name(self):
        class SameDevice(DeviceNode):
            def native_copy_keeps_name(self, _target_dir): return True
            def copy_file(self, target, cancel, progress):
                landed = type(self)(path=target.p.parent / self.name)
                landed.p.write_text('partial')
                raise OSError('device copy failed')
        p = self.src / 'a.txt'; p.write_text('data')
        r = self.run_op([p], engine=self.engine(SameDevice))
        self.assertEqual(len(r.errors), 1); self.assertIn('device copy failed', r.errors[0])
        self.assertEqual(os.listdir(self.dst), [])

    def test_local_stage_query_error_is_still_reported(self):
        # A missing or unreachable local/network stage was never cleaned up.
        class Unreachable(LocalNode):
            def info(self, cancel=None):
                if is_own_staging_name(self.name) and state['down']:
                    raise OSError(5, 'Input/output error')
                return super().info(cancel)
            def copy_file(self, target, cancel, progress):
                state['down'] = True
                raise OSError(5, 'Input/output error')
        state = {'down': False}
        p = self.src / 'a'; p.write_text('data')
        r = self.run_op([p], engine=self.engine(Unreachable))
        (stage,) = os.listdir(self.dst)
        self.assertTrue(any(f'Incomplete staging folder left at {(self.dst / stage).as_uri()}' in e for e in r.errors))
        self.assertEqual(self.sleeps, [])

    def test_device_stage_query_error_is_retried_and_reported(self):
        class Unreachable(DeviceNode):
            def info(self, cancel=None):
                if is_own_staging_name(self.name) and state['down']:
                    raise OSError(5, 'libmtp error: could not get object handles.')
                return super().info(cancel)
            def copy_file(self, target, cancel, progress):
                super().copy_file(target, cancel, progress)
                state['down'] = True
                raise OSError('simulated transfer error')
        state = {'down': False}
        p = self.src / 'a'; p.write_text('data')
        r = self.run_op([p], engine=self.engine(Unreachable))
        (stage,) = os.listdir(self.dst)
        self.assertTrue(any(f'Incomplete staging item left at {(self.dst / stage).as_uri()}' in e for e in r.errors))
        self.assertEqual(self.sleeps, [.5, 1.5])

    def test_replace_move_aside_finished_by_the_device_is_restored(self):
        # The device completed the rename but reported an error (for example a
        # cancelled wait): the original must not stay under the hidden name.
        class LateRename(DeviceNode):
            def move_native(self, target, cancel=None):
                result = super().move_native(target, cancel)
                if target.name.startswith('.winspace-replaced-') and not state['done']:
                    state['done'] = True
                    raise OSError('Operation was cancelled')
                return result
        state = {'done': False}
        p = self.src / 'a'; p.write_text('new'); (self.dst / 'a').write_text('old')
        r = self.run_op([p], policy='replace', engine=self.engine(LateRename))
        self.assertEqual(len(r.errors), 1)
        self.assertEqual((self.dst / 'a').read_text(), 'old')
        self.assertEqual(self.leftovers(), [])

    def test_replace_move_aside_is_not_cancellable(self):
        seen = []
        class Watching(DeviceNode):
            def move_native(self, target, cancel=None):
                if target.name.startswith('.winspace-replaced-'):
                    seen.append(cancel)
                return super().move_native(target, cancel)
        p = self.src / 'a'; p.write_text('new'); (self.dst / 'a').write_text('old')
        r = self.run_op([p], policy='replace', engine=self.engine(Watching))
        self.assertEqual(r.errors, []); self.assertEqual(seen, [None])
        self.assertEqual((self.dst / 'a').read_text(), 'new')

    def test_device_move_relists_each_source_folder_once(self):
        a = self.src / 'a'; a.write_text('a'); b = self.src / 'b'; b.write_text('b')
        r = self.run_op([a, b], mode='move')
        self.assertEqual(len(r.done), 2)
        self.assertEqual([c for c in CALLS if c[0] == 'refresh'], [('refresh', self.src)])

    def test_device_move_with_new_name_is_refused_not_misnamed(self):
        a = self.src / 'a'; a.write_text('new'); (self.dst / 'a').write_text('old')
        r = self.run_op([a], mode='move', policy='keep-both')
        self.assertTrue(r.errors); self.assertEqual(a.read_text(), 'new')
        self.assertEqual(os.listdir(self.dst), ['a'])

    def test_local_destinations_keep_directory_staging(self):
        p = self.src / 'a'; p.write_text('a')
        seen = []
        class Watching(LocalNode):
            def copy_file(self, target, cancel, progress):
                seen.append(target.p)
                return super().copy_file(target, cancel, progress)
        TransferEngine(Watching).run('copy', [p.as_uri()], self.dst.as_uri(), 'skip', self.cancel)
        self.assertEqual(seen[0].name, 'payload'); self.assertTrue(is_own_staging_name(seen[0].parent.name))


class StagingNameTests(unittest.TestCase):
    def test_only_exact_generated_names_match(self):
        self.assertTrue(is_own_staging_name('.winspace-transfer-' + 'a' * 32 + '.part'))
        for name in ('.winspace-transfer-' + 'A' * 32 + '.part', '.winspace-transfer-' + 'a' * 31 + '.part',
                     'x.winspace-transfer-' + 'a' * 32 + '.part', '.winspace-transfer-' + 'a' * 32 + '.part/x',
                     '.winspace-replaced-' + 'a' * 32 + '.backup', 'payload', None):
            self.assertFalse(is_own_staging_name(name), name)


try:
    import gi
    gi.require_version('Gio', '2.0')
    from gi.repository import Gio, GLib
    import gio_backend
except (ImportError, ValueError):
    gio_backend = None


class FakeGFile:
    """Just enough GFile for GioNode's MTP routing; records every call."""
    def __init__(self, uri, log, existing=()):
        self.uri, self.log, self.existing = uri, log, existing
    def get_uri(self): return self.uri
    def get_basename(self): return self.uri.rstrip('/').rsplit('/', 1)[-1]
    def get_path(self): return None
    def get_uri_scheme(self): return self.uri.split(':', 1)[0]
    def get_parent(self): return FakeGFile(self.uri.rstrip('/').rsplit('/', 1)[0], self.log, self.existing)
    def equal(self, other): return self.uri == other.uri
    def query_exists(self, _c): return self.uri in self.existing
    def move(self, *a): self.log.append(('move', self.uri, a[0].uri, a[1]))
    def set_display_name(self, name, _c):
        self.log.append(('rename', self.uri, name))
        if getattr(self, 'fail_after_rename', False):
            parent = self.uri.rstrip('/').rsplit('/', 1)[0]
            self.existing.discard(self.uri); self.existing.add(f'{parent}/{name}')
            raise GLib.Error.new_literal(Gio.io_error_quark(), 'Operation was cancelled',
                                         int(Gio.IOErrorEnum.CANCELLED))


@unittest.skipIf(gio_backend is None, 'PyGObject/GIO not available')
class GioMtpAdapterTests(unittest.TestCase):
    D = 'mtp://Pixel/Internal%20shared%20storage/Download'

    def node(self, rel, log, existing=()):
        return gio_backend.GioNode(gfile=FakeGFile(f'{self.D}/{rel}', log, existing))

    def test_same_folder_move_is_set_display_name(self):
        log = []
        self.node('.stage', log).move_native(self.node('t3code.apk', log))
        self.assertEqual(log, [('rename', f'{self.D}/.stage', 't3code.apk')])

    def test_same_folder_move_onto_taken_name_is_refused_before_device_call(self):
        log = []
        with self.assertRaises(GLib.Error) as caught:
            self.node('.stage', log).move_native(self.node('a', log, existing={f'{self.D}/a'}))
        self.assertTrue(caught.exception.matches(Gio.io_error_quark(), Gio.IOErrorEnum.EXISTS))
        self.assertEqual(log, [])

    def test_cross_folder_move_with_new_name_is_refused_without_device_call(self):
        log = []
        with self.assertRaises(ValueError):
            self.node('x/payload', log).move_native(self.node('t3code.apk', log))
        self.assertEqual(log, [])

    def test_cross_folder_move_with_same_name_uses_no_fallback_move(self):
        log = []
        self.node('x/a', log).move_native(self.node('y/a', log))
        self.assertEqual(log, [('move', f'{self.D}/x/a', f'{self.D}/y/a', gio_backend.MOVE_FLAGS)])

    def test_replace_never_uses_device_overwrite(self):
        log = []
        with self.assertRaises(ReplaceUnsupported):
            self.node('.stage', log).replace_native(self.node('a', log))
        self.assertEqual(log, [])

    def test_rename_the_device_finished_after_an_error_is_success(self):
        log, existing = [], {f'{self.D}/.stage'}
        source = gio_backend.GioNode(gfile=FakeGFile(f'{self.D}/.stage', log, existing))
        source.file.fail_after_rename = True
        source.move_native(self.node('a', log, existing))
        self.assertEqual(existing, {f'{self.D}/a'})

    def test_same_device_copies_are_detected(self):
        other = gio_backend.GioNode(gfile=FakeGFile('mtp://Other/Internal%20shared%20storage/x', []))
        local = gio_backend.GioNode(gfile=FakeGFile('file:///tmp/x', []))
        self.assertTrue(self.node('x', []).native_copy_keeps_name(self.node('y', [])))
        self.assertFalse(self.node('x', []).native_copy_keeps_name(other))
        self.assertFalse(local.native_copy_keeps_name(self.node('y', [])))
        self.assertFalse(self.node('x', []).native_copy_keeps_name(local))

    def test_device_schemes_request_sibling_staging(self):
        self.assertTrue(self.node('x', []).stage_as_sibling)
        # Untested camera (gphoto2) backends keep the previous staging path.
        self.assertFalse(gio_backend.GioNode(gfile=FakeGFile('gphoto2://cam/DCIM/x', [])).stage_as_sibling)
        self.assertFalse(gio_backend.GioNode(gfile=FakeGFile('smb://host/share/x', [])).stage_as_sibling)
        self.assertFalse(gio_backend.GioNode(gfile=FakeGFile('file:///tmp/x', [])).stage_as_sibling)


if __name__ == '__main__':
    unittest.main()
