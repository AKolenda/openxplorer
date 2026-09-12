# SPDX-License-Identifier: AGPL-3.0-only
"""Runtime upgrade guard, actual host action dispatch and short GIO read regressions.

The dispatcher is compiled from production source and given local filesystem
adapters. This exercises its real branch/response contracts without importing
unavailable PyGObject/WebKit. Graphical/native integration is tested separately.
"""
import ast
from contextlib import contextmanager
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock
import zipfile

from archives import Archives
from core import VERSION, normalise_location, is_smb_server
from runtime_guard import identity, same_build, require_current, Session
from tests.local_provider import LocalNode, Cancellation
from zip_extraction import ZipExtractor

ROOT=Path(__file__).resolve().parents[1]

class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.build={'version':VERSION,'protocol':1,'build':'abc'}
        self.session=Mock()
    def state(self, owner=':1.55', running=None):
        return {'owner':owner,'running':running,'installed':self.build,'matches':same_build(self.build,running)}
    def test_identity_changes_with_python_and_ui(self):
        with tempfile.TemporaryDirectory() as tmp:
            d=Path(tmp);(d/'ui').mkdir();(d/'x.py').write_text('a=1');(d/'ui/app.js').write_text('x=1;')
            one=identity(d,VERSION);(d/'x.py').write_text('a=2');two=identity(d,VERSION)
            self.assertNotEqual(one['build'],two['build']);(d/'ui/app.js').write_text('x=2;')
            self.assertNotEqual(two['build'],identity(d,VERSION)['build'])
    def test_new_process(self):
        self.session.status.return_value=self.state(None)
        require_current(self.session,self.build);self.session.stop.assert_not_called()
    def test_existing_current_process(self):
        self.session.status.return_value=self.state(running=self.build)
        require_current(self.session,self.build);self.session.stop.assert_not_called()
    def test_old_process_needs_consent(self):
        self.session.status.return_value=self.state()
        with self.assertRaisesRegex(RuntimeError,'restart'):require_current(self.session,self.build)
        self.session.stop.assert_not_called()
    def test_decline_never_stops(self):
        self.session.status.return_value=self.state()
        with self.assertRaises(RuntimeError):require_current(self.session,self.build,confirm=lambda s:False)
        self.session.stop.assert_not_called()
    def test_confirm_stops_only_exact_owner(self):
        self.session.status.return_value=self.state()
        require_current(self.session,self.build,confirm=lambda s:True)
        self.session.stop.assert_called_once_with(':1.55')
    def test_explicit_restart_of_current_process(self):
        self.session.status.return_value=self.state(running=self.build)
        require_current(self.session,self.build,restart=True)
        self.session.stop.assert_called_once_with(':1.55')
    def test_refused_quit_is_not_ignored(self):
        self.session.status.return_value=self.state();self.session.stop.side_effect=RuntimeError('active file operations')
        with self.assertRaisesRegex(RuntimeError,'active file'):require_current(self.session,self.build,restart=True)
    def test_session_stop_waits_for_release(self):
        class Error(Exception):pass
        s=Session(NS(),NS(Error=Error),connection=Mock());s.call=Mock();s.owner=Mock(side_effect=[':1.55',None])
        s.stop(':1.55',sleep=lambda _:None)
        self.assertEqual(s.call.call_args.args[0],':1.55')
        self.assertEqual(s.call.call_args.args[-1],('quit',[],{}))
    def test_busy_process_is_never_forced(self):
        class Error(Exception):pass
        s=Session(NS(),NS(Error=Error),connection=Mock());s.call=Mock();s.owner=Mock(return_value=':1.55')
        with self.assertRaisesRegex(RuntimeError,'No process was killed'):
            s.stop(':1.55',timeout=0,sleep=lambda _:None)
        self.assertEqual(s.call.call_count,1)
    def test_owner_changed_during_restart(self):
        class Error(Exception):pass
        s=Session(NS(),NS(Error=Error),connection=Mock());s.call=Mock();s.owner=Mock(return_value=':1.80')
        with self.assertRaisesRegex(RuntimeError,'Another OpenXplorer'):s.stop(':1.55')
    def test_runtime_response_parsed(self):
        class Error(Exception):pass
        s=Session(NS(),NS(Error=Error),connection=Mock());s.call=Mock(return_value=((True,'',[json.dumps(self.build)]),))
        self.assertEqual(s.running(':1.55'),self.build)
    def test_legacy_missing_action(self):
        class Error(Exception):pass
        s=Session(NS(),NS(Error=Error),connection=Mock());s.call=Mock(side_effect=Error('unknown action'))
        self.assertIsNone(s.running(':1.55'))

# Import only the real class, with a deliberately short-reading GIO test stream.
tree=ast.parse((ROOT/'native_opening.py').read_text())
klass=next(n for n in tree.body if isinstance(n,ast.ClassDef) and n.name=='GioReader')
ns={'io':io,'raw':lambda c:None,'Gio':NS(FileQueryInfoFlags=NS(NONE=0)),'GLib':NS(SeekType=NS(SET=0))}
exec(compile(ast.fix_missing_locations(ast.Module(body=[klass],type_ignores=[])),str(ROOT/'native_opening.py'),'exec'),ns)
Reader=ns['GioReader']
class Stream:
    def __init__(self,data,chunk=3):self.input=io.BytesIO(data);self.chunk=chunk;self.closed=False;self.calls=0
    def read_bytes(self,n,c):
        self.calls+=1;data=self.input.read(min(n,self.chunk));return NS(get_data=lambda:data)
    def can_seek(self):return True
    def seek(self,p,w,c):return self.input.seek(p,w)
    def tell(self):return self.input.tell()
    def close(self,c):self.closed=True
class File:
    def __init__(self,data):self.data=data;self.stream=Stream(data)
    def read(self,c):return self.stream
    def query_info(self,*a):return NS(get_size=lambda:len(self.data))
class ArchiveReaderTests(unittest.TestCase):
    def test_read_accumulates_partial_reads(self):
        f=File(b'abcdefghij');r=Reader(f,None)
        self.assertEqual(r.read(8),b'abcdefgh');self.assertEqual(r.read(8),b'ij');r.close()
    def test_zero_read_and_eof(self):
        r=Reader(File(b'abc'),None);self.assertEqual(r.read(0),b'');self.assertEqual(r.read(),b'abc');self.assertEqual(r.read(),b'');r.close()
    def test_real_zip_through_short_reads(self):
        data=io.BytesIO()
        with zipfile.ZipFile(data,'w',zipfile.ZIP_DEFLATED) as z:z.writestr('Project/notes.txt','hypothetical sample')
        with Reader(File(data.getvalue()),None) as r:
            with zipfile.ZipFile(r) as z:self.assertEqual(z.read('Project/notes.txt'),b'hypothetical sample')
    def test_seek_validation(self):
        with Reader(File(b'abc'),None) as r:
            with self.assertRaises(ValueError):r.seek(0,20)
            self.assertEqual(r.seek(-1,2),2);self.assertEqual(r.read(),b'c')
    def test_cancellation_between_reads(self):
        c=Cancellation();r=Reader(File(b'abc'),c);c.cancel()
        with self.assertRaises(Exception):r.read()
        r.close()
    def test_constructor_closes_stream_on_failure(self):
        f=File(b'abc');f.query_info=Mock(side_effect=OSError('query failed'))
        with self.assertRaises(OSError):Reader(f,None)
        self.assertTrue(f.stream.closed)
    def test_nonseekable_closes_stream(self):
        f=File(b'abc');f.stream.can_seek=lambda:False
        with self.assertRaises(ValueError):Reader(f,None)
        self.assertTrue(f.stream.closed)

# Real host dispatcher + real extractor against the local provider.
host=ast.parse((ROOT/'winspace.py').read_text())
cls=next(n for n in host.body if isinstance(n,ast.ClassDef) and n.name=='OpenXplorerWindow')
dispatch=next(n for n in cls.body if isinstance(n,ast.FunctionDef) and n.name=='dispatch')
import time
@contextmanager
def output(node,cancel):
    with Path(node.path).open('xb') as f:yield f
@contextmanager
def opener(uri,cancel):
    with Path(LocalNode(uri).path).open('rb') as f:yield f
class ContractHost:
    def __init__(self):
        self.archives=Archives(opener);self.writes=0;self.previous_versions=NS(assert_writable=lambda _:None)
        self.file_clipboard=None;self.responses=[];self.events=[]
    def respond(self,request,value=None,error=None):
        self.responses.append((request['id'],value,error));self.result=value
    def emit(self,name,data):self.events.append((name,data))
    def start_worker(self,request,fn,**kwargs):
        result=fn(Cancellation())
        if kwargs.get('complete'):kwargs['complete'](result)
        else:self.respond(request,result)

space={'normalise_location':normalise_location,'is_smb_server':is_smb_server,'ZipExtractor':ZipExtractor,
       'GioNode':LocalNode,'exclusive_output':output,'time':time,
       'inspect':lambda u,c:{'kind':'directory','isDir':True},'local_path':lambda u:str(LocalNode(u).path)}
exec(compile(ast.fix_missing_locations(ast.Module(body=[dispatch],type_ignores=[])),str(ROOT/'winspace.py'),'exec'),space)
ContractHost.dispatch=space['dispatch']
class DispatchTests(unittest.TestCase):
    def test_inspect_and_extract_actual_actions(self):
        with tempfile.TemporaryDirectory() as tmp:
            d=Path(tmp);source=d/'Assets.zip'
            with zipfile.ZipFile(source,'w') as z:z.writestr('Artwork/notes.txt','demo');z.writestr('readme.md','# example')
            h=ContractHost()
            h.dispatch({'id':1,'method':'archiveInspect','args':{'uri':source.as_uri(),'token':'check'}})
            self.assertEqual(h.result['files'],2);self.assertEqual(h.result['folders'],1);self.assertEqual(h.result['entries'],2)
            self.assertIsInstance(h.result['bytes'],int)
            h.dispatch({'id':2,'method':'archiveExtract','args':{'uri':source.as_uri(),'target':d.as_uri(),'name':'Unpacked','token':'extract'}})
            self.assertEqual((d/'Unpacked/Artwork/notes.txt').read_text(),'demo');self.assertEqual(h.result['files'],2)
            self.assertEqual(h.result['uri'],(d/'Unpacked').as_uri());self.assertTrue(source.exists())
    def test_extract_collision_never_merges(self):
        with tempfile.TemporaryDirectory() as tmp:
            d=Path(tmp);(d/'out').mkdir();(d/'out/sentinel').write_text('keep')
            source=d/'Assets.zip'
            with zipfile.ZipFile(source,'w') as z:z.writestr('data','x')
            h=ContractHost()
            with self.assertRaises(FileExistsError):h.dispatch({'id':3,'method':'archiveExtract','args':{'uri':source.as_uri(),'target':d.as_uri(),'name':'out'}})
            self.assertEqual((d/'out/sentinel').read_text(),'keep')
    def test_busy_writer_blocks_before_extract(self):
        h=ContractHost();h.writes=1
        with self.assertRaisesRegex(ValueError,'Finish'):h.dispatch({'id':1,'method':'archiveExtract','args':{}})
    def test_terminal_branch_connected(self):
        from terminal_integration import prepare_directory
        space['prepare_directory']=prepare_directory
        space['launch_terminal']=Mock(return_value={'opened':True,'terminal':'test recorder'})
        with tempfile.TemporaryDirectory() as tmp:
            h=ContractHost();h.dispatch({'id':1,'method':'openTerminal','args':{'uri':Path(tmp).as_uri()}})
            self.assertTrue(h.result['opened']);space['launch_terminal'].assert_called_once()
            self.assertEqual(space['launch_terminal'].call_args.args[0]['path'],tmp)

if __name__=='__main__':unittest.main()
