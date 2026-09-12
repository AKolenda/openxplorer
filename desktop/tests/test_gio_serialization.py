# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Production serializer with fake GFileInfo data (NOT native GIO execution).
Extract only these function bodies so tests can run without importing gi.
"""
import ast
from enum import IntEnum
from pathlib import Path
from types import SimpleNamespace
import unittest
from core import normalise_location
from entry_model import classify_entry

class FileType(IntEnum):
    UNKNOWN=0; REGULAR=1; DIRECTORY=2; SYMBOLIC_LINK=3; SPECIAL=4; SHORTCUT=5; MOUNTABLE=6

class File:
    def __init__(self,uri):self.uri=uri
    def get_uri(self):return self.uri
    def get_basename(self):return self.uri.rsplit('/',1)[-1]

class FileInfo:
    def __init__(self,kind,name='work',content='inode/directory',**attrs):
        self.kind=kind; self.name=name; self.content=content; self.attrs=attrs
    def get_file_type(self):return self.kind
    def get_display_name(self):return self.name
    def get_name(self):return self.name
    def get_content_type(self):return self.content
    def get_attribute_string(self,k):return self.attrs.get(k)
    def get_attribute_boolean(self,k):return bool(self.attrs.get(k))
    def get_attribute_uint64(self,k):return self.attrs.get(k,0)
    def has_attribute(self,k):return k in self.attrs
    def get_size(self):return self.attrs.get('standard::size',0)
    def get_is_hidden(self):return False
    def get_is_symlink(self):return False

scope={'Gio':SimpleNamespace(FileType=FileType,content_type_get_description=lambda t:'Text document' if t=='text/plain' else 'Folder'),
       'classify_entry':classify_entry,'normalise_location':normalise_location}
source=ast.parse((Path(__file__).resolve().parents[1]/'gio_backend.py').read_text())
functions=[node for node in source.body if isinstance(node,ast.FunctionDef) and node.name in ('entry_from_info','verify_pin')]
exec(compile(ast.Module(body=functions,type_ignores=[]),'gio_backend.py','exec'),scope)
serialize=scope['entry_from_info']

class GioSerializationTests(unittest.TestCase):
    def test_mountable_network_share_has_directory_icon_flag_and_unknown_size(self):
        e=serialize(File('smb://nas/work'),FileInfo(FileType.MOUNTABLE,**{'standard::target-uri':'smb://nas/work'}))
        self.assertTrue(e['isDir']);self.assertEqual(e['kind'],'mountable');self.assertEqual(e['type'],'Network share')
        self.assertIsNone(e['size']);self.assertEqual(e['modified'],0)
    def test_gvfs_mountable_browse_uri_and_target_remain_distinct(self):
        # GVfs smburi.c represents a server-browser child using ._share;
        # its standard::target-uri points to the actual share mount.
        e=serialize(File('smb://nas/._work'),FileInfo(FileType.MOUNTABLE,**{'standard::target-uri':'smb://nas/work'}))
        self.assertEqual(e['uri'],'smb://nas/._work')
        self.assertEqual(e['targetUri'],'smb://nas/work')
        self.assertTrue(e['isDir']);self.assertFalse(e['canOperate'])
    def test_pin_inspects_browse_item_but_saves_real_share_target(self):
        queried=[]
        def fake_inspect(uri,cancel):
            queried.append(uri)
            self.assertEqual(uri,'smb://nas/._work')
            return serialize(File(uri),FileInfo(FileType.MOUNTABLE,**{'standard::target-uri':'smb://nas/work'}))
        scope['inspect']=fake_inspect
        self.assertEqual(scope['verify_pin']('smb://nas/._work','work'),{'uri':'smb://nas/work','label':'work'})
        self.assertEqual(queried,['smb://nas/._work'])
    def test_real_directory_metadata(self):
        e=serialize(File('smb://nas/work/Design'),FileInfo(FileType.DIRECTORY,**{'standard::size':8192,'time::modified':12345}))
        self.assertTrue(e['isDir']);self.assertIsNone(e['size']);self.assertEqual(e['modified'],12345)
    def test_real_zero_byte_file_remains_zero_bytes(self):
        e=serialize(File('smb://nas/work/README'),FileInfo(FileType.REGULAR,'README','text/plain',**{'standard::size':0}))
        self.assertFalse(e['isDir']);self.assertEqual(e['size'],0)
    def test_missing_file_size_does_not_claim_zero(self):
        e=serialize(File('smb://nas/work/README'),FileInfo(FileType.REGULAR,'README','text/plain'))
        self.assertIsNone(e['size'])
    def test_server_shortcut_resolves_to_target_server(self):
        e=serialize(File('smb://group/alpha'),FileInfo(FileType.SHORTCUT,**{'standard::target-uri':'smb://ALPHA/'}))
        self.assertTrue(e['isDir']);self.assertEqual(e['targetUri'],'smb://alpha/');self.assertFalse(e['canOperate'])
    def test_verify_pin_uses_validated_target_not_browse_uri(self):
        scope['inspect']=lambda uri,cancel:serialize(File(uri),FileInfo(FileType.SHORTCUT,**{'standard::target-uri':'smb://ALPHA/'}))
        self.assertEqual(scope['verify_pin']('smb://group/alpha','Alpha')['uri'],'smb://alpha/')
    def test_verify_pin_rejects_regular_file(self):
        scope['inspect']=lambda uri,cancel:serialize(File(uri),FileInfo(FileType.REGULAR,'file','text/plain'))
        with self.assertRaises(ValueError):scope['verify_pin']('smb://nas/work/file')
    def test_serialization_requests_no_extra_filesystem_queries(self):
        # File exposes URI/name only; no stat/query_info/mount API exists here.
        e=serialize(File('smb://nas/work'),FileInfo(FileType.MOUNTABLE))
        self.assertTrue(e['isDir'])

if __name__=='__main__':unittest.main()
