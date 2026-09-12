# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""0.6 regression tests: real metadata scans + pure app/prefs logic. No NAS/GTK."""
import math
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock
from core import Settings
from folder_sizes import scan_folder, LocalSizeProvider
from app_catalog import unique_applications, editor_shortcuts
from tests.local_provider import Cancellation


def app(identifier, name='Visual Studio Code', shown=True, files=True, uris=False):
    a=Mock();a.get_id.return_value=identifier;a.get_display_name.return_value=name
    a.should_show.return_value=shown;a.supports_files.return_value=files;a.supports_uris.return_value=uris
    return a


class CatalogTests(unittest.TestCase):
    def test_duplicate_desktop_id(self):
        a=app('code.desktop'); self.assertEqual(len(unique_applications([a,a])),1)
    def test_duplicate_visible_name_prefers_primary(self):
        a=app('code.desktop');b=app('com.visualstudio.code.desktop')
        self.assertEqual(unique_applications([b,a]),[a])
    def test_preferred_default_wins(self):
        a=app('code.desktop');b=app('com.visualstudio.code.desktop')
        self.assertEqual(unique_applications([a,b],b.get_id()),[b])
    def test_url_helper_omitted(self):
        self.assertEqual(unique_applications([app('code-url-handler.desktop')]),[])
    def test_hidden_omitted(self):
        self.assertEqual(unique_applications([app('code.desktop',shown=False)]),[])
    def test_missing_identifier_omitted(self):
        self.assertEqual(unique_applications([app(None)]),[])
    def test_self_excluded(self):
        self.assertEqual(unique_applications([app('io.winspace.Development.desktop')]),[])
    def test_no_argument_support_omitted(self):
        self.assertEqual(unique_applications([app('x.desktop',files=False,uris=False)]),[])
    def test_distinct_editors_preserved(self):
        rows=editor_shortcuts([app('code.desktop'),app('code-insiders.desktop','Code Insiders'),app('codium.desktop','VSCodium')])
        self.assertEqual(len(rows),3)
    def test_other_app_not_editor_shortcut(self):
        self.assertEqual(editor_shortcuts([app('evince.desktop','Document Viewer')]),[])
    def test_name_case_and_space_dedup(self):
        self.assertEqual(len(unique_applications([app('a.desktop',' Code '),app('b.desktop','code')])),1)


class PrefTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);self.s=Settings(self.root)
    def tearDown(self):self.temp.cleanup()
    def test_layout_persists(self):
        self.s.update_preferences({'sidebarWidth':333,'columnWidths':{'name':460,'size':100}})
        self.assertEqual(Settings(self.root).snapshot()['preferences']['sidebarWidth'],333)
        self.assertEqual(Settings(self.root).snapshot()['preferences']['columnWidths'],{'name':460,'size':100})
    def test_sidebar_width_bounds(self):
        for v in (-1,139,561,math.inf,math.nan,True,'350'):
            self.s.update_preferences({'sidebarWidth':v})
            self.assertNotIn('sidebarWidth',self.s.snapshot()['preferences'])
    def test_width_rounding(self):
        self.s.update_preferences({'sidebarWidth':280.4});self.assertEqual(self.s.snapshot()['preferences']['sidebarWidth'],280)
    def test_columns_whitelist(self):
        self.s.update_preferences({'columnWidths':{'name':150,'css':'url(bad)','size':99999,'type':True}})
        self.assertEqual(self.s.snapshot()['preferences']['columnWidths'],{'name':150})
    def test_column_reset(self):
        self.s.update_preferences({'columnWidths':{'name':700}});self.s.update_preferences({'columnWidths':{}})
        self.assertEqual(self.s.snapshot()['preferences']['columnWidths'],{})
    def test_other_preferences_retained(self):
        self.s.update_preferences({'theme':'dark','contextMenu':'win11'});self.s.update_preferences({'sidebarWidth':300})
        p=self.s.snapshot()['preferences'];self.assertEqual((p['theme'],p['contextMenu']),('dark','win11'))
    def test_partial_window_updates_do_not_remove_other_preferences(self):
        other=Settings(self.root);self.s.update_preferences({'sidebarWidth':270});other.update_preferences({'columnWidths':{'modified':200}})
        p=Settings(self.root).snapshot()['preferences'];self.assertEqual(p['sidebarWidth'],270);self.assertEqual(p['columnWidths']['modified'],200)


class FolderSizeTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name);self.uri=self.root.as_uri();self.cancel=Cancellation()
    def tearDown(self):self.temp.cleanup()
    def scan(self, **kw):return scan_folder(self.uri,self.cancel,**kw)
    def write(self,name,data):
        p=self.root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data);return p
    def test_empty_is_real_zero(self):
        r=self.scan();self.assertEqual((r['bytes'],r['status']),(0,'complete'))
    def test_recursive_file_bytes(self):
        self.write('a',b'abc');self.write('sub/b',b'12345');r=self.scan()
        self.assertEqual((r['bytes'],r['files'],r['folders']),(8,2,1))
    def test_hidden_files_counted(self):
        self.write('.hidden/private',b'1234');self.assertEqual(self.scan()['bytes'],4)
    def test_files_unchanged(self):
        p=self.write('a',b'abc');before=p.stat();self.scan();after=p.stat()
        self.assertEqual((before.st_size,before.st_mtime_ns),(after.st_size,after.st_mtime_ns));self.assertEqual(p.read_bytes(),b'abc')
    def test_symbolic_link_not_followed(self):
        self.write('actual',b'abc');(self.root/'link').symlink_to(self.root/'actual');r=self.scan()
        self.assertEqual((r['bytes'],r['skipped'],r['status']),(3,1,'partial'))
    def test_cycle_does_not_recurse(self):
        self.write('sub/a',b'xyz');(self.root/'sub/loop').symlink_to(self.root,target_is_directory=True);r=self.scan()
        self.assertEqual(r['bytes'],3);self.assertEqual(r['folders'],1)
    def test_root_symlink_refused(self):
        target=self.root/'target';target.mkdir();link=self.root/'link';link.symlink_to(target,target_is_directory=True)
        with self.assertRaises(ValueError):scan_folder(link.as_uri(),self.cancel)
    def test_regular_file_refused(self):
        p=self.write('a',b'x')
        with self.assertRaises(ValueError):scan_folder(p.as_uri(),self.cancel)
    def test_snapshot_collection_excluded(self):
        self.write('a',b'abc');self.write('.zfs/snapshot/a/secret',b'not duplicated')
        r=self.scan();self.assertEqual(r['bytes'],3);self.assertEqual(r['status'],'partial')
    def test_explicit_snapshot_root_can_be_scanned(self):
        p=self.write('.zfs/snapshot/dated/a',b'abc')
        r=scan_folder(p.parent.as_uri(),self.cancel);self.assertEqual(r['bytes'],3)
    def test_hardlinks_count_once(self):
        p=self.write('a',b'abc');os.link(p,self.root/'b');r=self.scan();self.assertEqual((r['bytes'],r['files']),(3,1))
    def test_fifo_excluded_not_opened(self):
        os.mkfifo(self.root/'pipe');r=self.scan();self.assertEqual((r['bytes'],r['skipped']),(0,1))
    def test_entry_limit_marks_partial(self):
        for i in range(5):self.write(str(i),b'x')
        r=self.scan(max_entries=2);self.assertEqual((r['bytes'],r['status']),(2,'partial'))
    def test_cancellation_returns_partial_progress(self):
        for i in range(10):self.write(str(i),b'x')
        class Provider(LocalSizeProvider):
            def children(p,uri,cancel):
                for i,e in enumerate(super().children(uri,cancel)):
                    yield e
                    if i==2:cancel.cancel()
        r=self.scan(provider=Provider());self.assertEqual(r['status'],'cancelled');self.assertLess(r['bytes'],10)
    def test_progress_and_completion_published(self):
        events=[];self.write('a',b'abc');r=self.scan(progress=events.append)
        self.assertEqual(events[0]['status'],'scanning');self.assertEqual(events[-1]['status'],'complete');self.assertEqual(events[-1]['bytes'],3)
    def test_max_seconds(self):
        n=[0]
        def clock():n[0]+=1;return n[0]
        self.write('a',b'abc');r=self.scan(max_seconds=1,clock=clock);self.assertEqual(r['status'],'partial')
    def test_server_root_refused(self):
        with self.assertRaises(ValueError):scan_folder('smb://nas/',self.cancel)
    def test_nested_mount_excluded(self):
        self.write('other/a',b'abc');provider=LocalSizeProvider();provider.mounts.add(str(self.root/'other'))
        r=self.scan(provider=provider);self.assertEqual((r['bytes'],r['status']),(0,'partial'))
    def test_unreadable_entries_reported(self):
        class Provider(LocalSizeProvider):
            def children(p,uri,cancel):yield {'name':'unreadable','unreadable':True}
        r=self.scan(provider=Provider());self.assertEqual((r['errors'],r['status']),(1,'partial'))
    def test_failed_subtree_reported(self):
        self.write('sub/a',b'abc')
        class Provider(LocalSizeProvider):
            def children(p,uri,cancel):
                if uri.endswith('/sub'):raise PermissionError('test refusal')
                yield from super().children(uri,cancel)
        r=self.scan(provider=Provider());self.assertEqual((r['errors'],r['status']),(1,'partial'))
    def test_root_enumeration_error_propagates_for_native_retry(self):
        class Provider(LocalSizeProvider):
            def children(p,uri,cancel):raise PermissionError('mount or authentication required')
        with self.assertRaises(PermissionError):self.scan(provider=Provider())
    def test_root_inspect_error_is_not_an_empty_directory(self):
        class Provider(LocalSizeProvider):
            def inspect(p,uri,cancel):raise FileNotFoundError('offline or missing')
        with self.assertRaises(FileNotFoundError):self.scan(provider=Provider())
    def test_unknown_size_is_not_fabricated_zero(self):
        class Provider(LocalSizeProvider):
            def children(p,uri,cancel):yield {'name':'unknown','regular':True,'size':None}
        r=self.scan(provider=Provider());self.assertEqual(r['status'],'partial')
    def test_spaces_in_path(self):
        p=self.write('spaces and #/file.txt',b'abc');r=scan_folder(p.parent.as_uri(),self.cancel);self.assertEqual(r['bytes'],3)

if __name__=='__main__':unittest.main()
