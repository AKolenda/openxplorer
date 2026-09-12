# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""v0.5 regression tests. Real local index/inotify/ZIP I/O; fake keyring/apps.
These tests do NOT validate a live NAS, GTK WebKit rendering or desktop keyring.
"""
from contextlib import contextmanager
import io,json,os,stat,tempfile,time,unittest,zipfile,threading
from pathlib import Path
from types import SimpleNamespace as NS
from unittest.mock import Mock,patch
from urllib.parse import unquote,urlsplit
from activation import activation_kind,choose_application
from archives import Archives,safe_member,MAX_DIRECTORY,BoundedReader
from auth_bridge import MountPrompts,split_identity
from core import Settings
from file_clipboard import validate_clipboard,encode_clipboard,decode_clipboard,CUSTOM,GNOME,URI_LIST
from session_credentials import SessionCredentials,server_key
from search_index import SearchIndex
from local_watch import LocalWatch
from index_service import IndexService
from previous_versions import PreviousVersions,conventional_snapshot
from mount_support import mount_plan,parse_mounts,resolve_smb_path
from tests.local_provider import Cancellation

class TempCase(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory(prefix='winspace-v05-test-');self.base=Path(self.tmp.name)
 def tearDown(self):self.tmp.cleanup()

def entry(root,name,directory=False,**kw):
 uri=root.rstrip('/')+'/'+name
 return dict(uri=uri,name=name,isDir=directory,kind='directory' if directory else 'file',size=None if directory else 12,hidden=False,modified=1,type='Folder' if directory else 'File',**kw)

class OpeningTests(unittest.TestCase):
 def test_regular_pdf(self):self.assertEqual(activation_kind({'kind':'file','name':'a.pdf'}),'file')
 def test_regular_mp4(self):self.assertEqual(activation_kind({'kind':'file','name':'a.mp4'}),'file')
 def test_directory_with_extension(self):self.assertEqual(activation_kind({'kind':'directory','name':'a.mp4'}),'directory')
 def test_directory_named_zip(self):self.assertEqual(activation_kind({'kind':'directory','name':'a.zip'}),'directory')
 def test_zip_by_mime(self):self.assertEqual(activation_kind({'kind':'file','name':'a','contentType':'application/zip'}),'archive')
 def test_zip_by_extension(self):self.assertEqual(activation_kind({'kind':'file','name':'a.ZIP'}),'archive')
 def test_unknown_not_directory(self):
  with self.assertRaises(ValueError):activation_kind({'kind':'unknown','name':'a'})
 def test_mountable(self):self.assertEqual(activation_kind({'kind':'mountable','isDir':True}),'directory')
 def test_regular_overrides_stale_bool(self):self.assertEqual(activation_kind({'kind':'file','isDir':True,'name':'a.pdf'}),'file')
 def test_excludes_smb_self_handler(self):
  own=Mock();own.get_id.return_value='io.winspace.Development.desktop';external=Mock();external.get_id.return_value='evince.desktop'
  self.assertIs(choose_application([own,external],own),external)
 def test_no_recursion_without_external_app(self):
  own=Mock();own.get_id.return_value='io.winspace.Development.desktop'
  with self.assertRaises(ValueError):choose_application([own],own)
 def test_respects_external_default(self):
  app=Mock();app.get_id.return_value='vlc.desktop';self.assertIs(choose_application([],app),app)

class ClipboardTests(unittest.TestCase):
 def value(self,mode='copy'):return {'uris':['file:///home/demo/a.pdf','smb://nas/share/a.mp4'],'mode':mode,'token':'token-A'}
 def test_custom_copy_roundtrip(self):
  v=self.value();self.assertEqual(decode_clipboard(CUSTOM,encode_clipboard(v)[CUSTOM]),v)
 def test_custom_cut_roundtrip(self):
  v=self.value('move');self.assertEqual(decode_clipboard(CUSTOM,encode_clipboard(v)[CUSTOM]),v)
 def test_gnome_cut_flag(self):self.assertEqual(decode_clipboard(GNOME,encode_clipboard(self.value('move'))[GNOME])['mode'],'move')
 def test_uri_list_is_copy_not_inferred_cut(self):self.assertEqual(decode_clipboard(URI_LIST,encode_clipboard(self.value('move'))[URI_LIST])['mode'],'copy')
 def test_plain_text_is_not_file_clipboard(self):self.assertIsNone(decode_clipboard('text/plain',b'/home/demo/a'))
 def test_unsafe_scheme(self):self.assertIsNone(decode_clipboard(URI_LIST,b'https://example.org/executable'))
 def test_no_password_in_clipboard_uri(self):self.assertIsNone(decode_clipboard(URI_LIST,b'smb://user:pass@nas/share/a'))
 def test_share_itself_not_transferable(self):self.assertIsNone(decode_clipboard(GNOME,b'cut\nsmb://nas/share'))
 def test_deduplication(self):
  v=self.value();v['uris']*=2;self.assertEqual(len(validate_clipboard(v)['uris']),2)
 def test_malformed_json(self):self.assertIsNone(decode_clipboard(CUSTOM,b'{'))
 def test_large_rejected(self):self.assertIsNone(decode_clipboard(CUSTOM,b'x'*1048577))
 def test_kde_cut_flag(self):self.assertEqual(encode_clipboard(self.value('move'))['x-kde-cutselection'],b'1')

class FakeSecret:
 Schema=NS(new=lambda *a:object());SchemaFlags=NS(NONE=0);SchemaAttributeType=NS(STRING=0)
 COLLECTION_SESSION='session';COLLECTION_DEFAULT='default'
 def __init__(self):self.rows={};self.saved=[]
 def password_lookup_sync(self,schema,attrs,c):return self.rows.get(tuple(sorted(attrs.items())))
 def password_store_sync(self,schema,attrs,collection,label,value,c):
  self.rows[tuple(sorted(attrs.items()))]=value;self.saved.append((collection,dict(attrs)));return True
 def password_clear_sync(self,schema,attrs,c):
  for key in list(self.rows):
   if all(dict(key).get(k)==v for k,v in attrs.items()):self.rows.pop(key)
  return True
class CredentialsTests(unittest.TestCase):
 def setUp(self):self.secret=FakeSecret();self.cache=SessionCredentials(self.secret);self.v={'username':'sam','domain':'WORKGROUP','password':'not-a-real-password','remember':False}
 def test_host_key_not_share_key(self):self.assertEqual(server_key('smb://NAS/a'),server_key('smb://nas/b'))
 def test_distinct_ports(self):self.assertNotEqual(server_key('smb://nas:1445/a'),server_key('smb://nas/a'))
 def test_no_alias_sharing(self):self.assertNotEqual(server_key('smb://nas.local/a'),server_key('smb://10.0.0.1/a'))
 def test_session_collection_unchecked(self):self.cache.persist('smb://nas/a',self.v);self.assertEqual(self.secret.saved[-1][0],'session')
 def test_cross_share_load(self):self.cache.persist('smb://nas/a',self.v);self.assertEqual(self.cache.load('smb://nas/b'),self.v)
 def test_other_window_load(self):self.cache.persist('smb://nas/a',self.v);self.assertEqual(SessionCredentials(self.secret).load('smb://nas/b'),self.v)
 def test_other_host_no_load(self):self.cache.persist('smb://nas/a',self.v);self.assertIsNone(self.cache.load('smb://other/a'))
 def test_permanent_checked(self):
  self.v['remember']=True;self.cache.persist('smb://nas/a',self.v);self.assertEqual(self.secret.saved[-1][0],'default')
 def test_session_overrides_older_account(self):
  self.cache.persist('smb://nas/a',{**self.v,'username':'old','remember':True});self.cache.persist('smb://nas/b',self.v)
  self.assertEqual(self.cache.load('smb://nas/a')['username'],'sam')
 def test_forget_clears_only_matching_host(self):
  self.cache.persist('smb://nas/a',self.v);self.cache.persist('smb://other/a',self.v);self.cache.forget('smb://nas/b')
  self.assertIsNone(self.cache.load('smb://nas/a'));self.assertIsNotNone(self.cache.load('smb://other/a'))
 def test_no_plaintext_fallback(self):
  c=SessionCredentials();c.accept_memory('smb://nas/a',self.v)
  with self.assertRaises(ValueError):c.persist('smb://nas/a',self.v)
  self.assertEqual(c.peek('smb://nas/b'),self.v)
 def test_domain_username_supported(self):self.assertEqual(split_identity('OFFICE\\sam'),('sam','OFFICE'))

class FakeOperation:
 def __init__(self):self.props={};self.replies=[]
 def connect(self,*a):pass
 def stop_emission_by_name(self,*a):pass
 def reply(self,value):self.replies.append(value)
 def __getattr__(self,k):
  if k.startswith('set_'):return lambda v:self.props.__setitem__(k[4:],v)
  raise AttributeError(k)
class AuthTests(unittest.TestCase):
 def setUp(self):
  self.events=[];self.s=FakeSecret();self.store=SessionCredentials(self.s)
  gio=NS(MountOperation=FakeOperation,MountOperationResult=NS(ABORTED=0,HANDLED=1),PasswordSave=NS(NEVER=0,FOR_SESSION=1,PERMANENTLY=2),AskPasswordFlags=NS(NEED_USERNAME=1,NEED_PASSWORD=2,SAVING_SUPPORTED=4,ANONYMOUS_SUPPORTED=8))
  glib=NS(idle_add=lambda fn:fn(),timeout_add_seconds=lambda *a:1,source_remove=lambda *a:None)
  self.p=MountPrompts(gio,glib,lambda *a:self.events.append(a),self.store)
 def tearDown(self):self.p.close()
 def prompt(self):
  op=self.p.create('smb://nas/a');r=self.p.operations[id(op)];r['attempts']=1;r['challenge']=('', 'user','WORKGROUP',7);self.p._show_password(op);return op,next(iter(self.p.pending))
 def test_unchecked_session_not_never(self):
  op,t=self.prompt();self.p.answer({'id':t,'username':'sam','password':'test','remember':False});self.assertEqual(op.props['password_save'],1)
 def test_checked_permanent(self):
  op,t=self.prompt();self.p.answer({'id':t,'username':'sam','password':'test','remember':True});self.assertEqual(op.props['password_save'],2)
 def test_failed_mount_not_saved(self):
  op,t=self.prompt();self.p.answer({'id':t,'username':'sam','password':'test','remember':False});self.p.finish(op,False);self.assertFalse(self.s.saved)
 def test_success_saved_after_finish(self):
  op,t=self.prompt();self.p.answer({'id':t,'username':'sam','password':'test','remember':False});self.assertFalse(self.s.saved)
  self.p.finish(op,True);self.p.workers.submit(lambda:None).result(2);self.assertEqual(self.s.saved[-1][0],'session')
 def test_reuse_without_dialog(self):
  self.store.accept_memory('smb://nas/a',{'username':'sam','domain':'','password':'test','remember':False})
  op=self.p.create('smb://nas/other');self.p._ask_password(op,'','user','',7)
  self.assertEqual(op.props['password'],'test');self.assertFalse(self.p.pending)
 def test_rejected_reuse_shows_prompt_not_loop(self):
  self.store.accept_memory('smb://nas/a',{'username':'sam','domain':'','password':'test','remember':False})
  op=self.p.create('smb://nas/a');self.p._ask_password(op,'','u','',7);self.p._ask_password(op,'','u','',7)
  self.assertTrue(self.p.pending);self.assertEqual(len(op.replies),1)
 def test_password_not_sent_back_to_ui(self):
  op,t=self.prompt();self.p.answer({'id':t,'username':'sam','password':'unique-secret-value','remember':False})
  self.assertNotIn('unique-secret-value',json.dumps(self.events))
 def test_cancelled_challenge_dismisses(self):
  op,t=self.prompt();self.p.answer({'id':t,'cancel':True});self.assertEqual(op.replies[-1],0);self.assertFalse(self.p.pending)

class IndexTests(TempCase):
 def setUp(self):super().setUp();self.db=SearchIndex(self.base/'db');self.uri='smb://nas/share';self.db.configure(self.uri,True);self.gen=self.db.begin(self.uri)
 def put(self,*es):self.db.put_batch(self.uri,self.gen,list(es));self.db.finish(self.uri,self.gen,complete=True)
 def test_cached_regular_not_directory(self):
  e=entry(self.uri,'bank.pdf');e['isDir']=True;self.put(e);self.assertFalse(self.db.search('bank')['entries'][0]['isDir'])
 def test_search_full_parent_path(self):
  self.put(entry(self.uri+'/Nested','bank.pdf'));self.assertEqual(self.db.search('bank')['entries'][0]['parentUri'],self.uri+'/Nested')
 def test_delta_add(self):self.db.replace_directory(self.uri,self.uri,[entry(self.uri,'bank.pdf')]);self.assertEqual(len(self.db.search('bank')['entries']),1)
 def test_delta_remove(self):
  self.put(entry(self.uri,'bank.pdf'));self.db.replace_directory(self.uri,self.uri,[]);self.assertFalse(self.db.search('bank')['entries'])
 def test_delete_folder_prunes_descendants_only(self):
  self.put(entry(self.uri,'old',True),entry(self.uri+'/old','bank.pdf'),entry(self.uri,'keep.pdf'))
  self.db.replace_directory(self.uri,self.uri,[entry(self.uri,'keep.pdf')]);self.assertFalse(self.db.search('bank')['entries']);self.assertEqual(len(self.db.search('keep')['entries']),1)
 def test_new_directory_traversal_hint(self):self.assertEqual(self.db.replace_directory(self.uri,self.uri,[entry(self.uri,'new',True)]),[self.uri+'/new'])
 def test_failed_scan_preserves_last_data(self):
  self.put(entry(self.uri,'bank.pdf'));g=self.db.begin(self.uri);self.db.finish(self.uri,g,complete=False,error='offline');self.assertEqual(len(self.db.search('bank')['entries']),1)
 def test_disable_clears_metadata(self):self.put(entry(self.uri,'bank.pdf'));self.db.configure(self.uri,False);self.assertFalse(self.db.search('bank')['entries'])
 def test_directory_to_file_prunes_old_children(self):
  self.put(entry(self.uri,'archive',True),entry(self.uri+'/archive','bank.pdf'));self.db.replace_directory(self.uri,self.uri,[entry(self.uri,'archive')]);self.assertFalse(self.db.search('bank')['entries'])
 def test_wrong_parent_ignored(self):self.db.replace_directory(self.uri,self.uri,[entry('smb://other/share','bank.pdf')]);self.assertFalse(self.db.search('bank')['entries'])
 def test_separate_process_connection_reads_updates(self):
  other=SearchIndex(self.base/'db',recover=False);self.put(entry(self.uri,'bank.pdf'));self.assertEqual(len(other.search('bank')['entries']),1)
 def test_command_queue_deduplicates(self):
  self.db.enqueue('refresh',self.uri);self.db.enqueue('refresh',self.uri);self.assertEqual(len(self.db.drain_requests()),1);self.assertEqual(self.db.drain_requests(),[])
 def test_private_database_modes(self):self.assertEqual(stat.S_IMODE(self.db.path.stat().st_mode),0o600);self.assertEqual(stat.S_IMODE(self.db.directory.stat().st_mode),0o700)
 def test_fts_special_characters_bound(self):
  self.put(entry(self.uri,'name.pdf'));self.assertIsInstance(self.db.search('" OR *; DROP TABLE entries;')['entries'],list)

class LiveTests(TempCase):
 def setUp(self):
  super().setUp();self.tree=self.base/'files';self.tree.mkdir();self.uri=self.tree.as_uri();self.db=SearchIndex(self.base/'index');self.db.configure(self.uri,True);self.list_calls=[]
  def listing(uri,hidden,c):
   self.list_calls.append(uri);path=Path(unquote(urlsplit(uri).path));rows=[]
   for p in path.iterdir():
    c.check()
    if p.name.startswith('.') and not hidden:continue
    rows.append(dict(uri=p.as_uri(),name=p.name,isDir=p.is_dir(),kind='directory' if p.is_dir() else 'file',size=p.stat().st_size,modified=p.stat().st_mtime,hidden=p.name.startswith('.'),symlink=p.is_symlink()))
   yield rows
  self.service=IndexService(self.db,listing,Cancellation);self.service.refresh(self.uri)
  self.until(lambda:self.db.roots()[0]['status']=='Ready')
 def until(self,predicate,seconds=5):
  end=time.monotonic()+seconds
  while time.monotonic()<end:
   self.service.refresh_due()
   if predicate():return
   time.sleep(.08)
  self.fail('Timed out waiting for real local index event')
 def tearDown(self):
  self.service.close();self.service.executor.shutdown(wait=True);super().tearDown()
 def test_live_create_without_full_scan(self):
  generation=self.db.roots()[0]['generation'];(self.tree/'live-bank.pdf').write_text('test')
  self.until(lambda:len(self.db.search('live-bank')['entries'])==1)
  self.assertEqual(self.db.roots()[0]['generation'],generation)
 def test_new_nested_folder_watched(self):
  folder=self.tree/'new';folder.mkdir();(folder/'first.txt').write_text('one');self.until(lambda:len(self.db.search('first.txt')['entries'])==1)
  (folder/'second.txt').write_text('two');self.until(lambda:len(self.db.search('second.txt')['entries'])==1)
  self.assertGreaterEqual(self.service.watcher.count(self.uri),2)
 def test_live_rename_and_delete(self):
  a=self.tree/'before.pdf';a.write_text('test');self.until(lambda:len(self.db.search('before.pdf')['entries'])==1)
  a.rename(self.tree/'after.pdf');self.until(lambda:len(self.db.search('after.pdf')['entries'])==1 and not self.db.search('before.pdf')['entries'])
  (self.tree/'after.pdf').unlink();self.until(lambda:not self.db.search('after.pdf')['entries'])
 def test_hidden_and_symlinks_not_traversed(self):
  (self.tree/'.hidden').write_text('secret');(self.tree/'link').symlink_to(self.base,target_is_directory=True)
  time.sleep(.4);self.service.refresh_due();time.sleep(.2);self.assertFalse(self.db.search('hidden')['entries']);self.assertFalse(self.db.search('link')['entries'])
 def test_two_window_index_leader(self):
  other=IndexService(SearchIndex(self.base/'index',recover=False),self.service.list_directory,Cancellation)
  try:
   self.assertFalse(other.leader);other.refresh(self.uri);self.service.refresh_due();self.until(lambda:not self.service.jobs)
  finally:other.close()
 def test_watch_limit_fallback_is_reported(self):
  self.service.watcher.max_watches=1;(self.tree/'extra').mkdir();self.until(lambda:self.uri in self.service.failed_watches)
  self.assertIn('fallback',self.db.roots()[0]['update_mode'])
 def test_root_exclusions(self):
  exclusions=self.service.policy('file:///');self.assertFalse(self.service.allowed('file:///proc/1','file:///',exclusions));self.assertFalse(self.service.allowed('file:///.snapshots/1','file:///',[]));self.assertFalse(self.service.allowed('file:///.zfs/snapshot','file:///',[]))
 def test_snapshot_folders_not_indexed(self):
  (self.tree/'.snapshot').mkdir();(self.tree/'.snapshot'/'secret.pdf').write_text('old');time.sleep(.5);self.service.refresh_due();time.sleep(.1)
  self.assertFalse(self.db.search('secret.pdf')['entries'])

class ZipTests(TempCase):
 def setUp(self):
  super().setUp();self.zip=self.base/'bank.zip';self.out=self.base/'previews'
  with zipfile.ZipFile(self.zip,'w') as z:z.writestr('Documents/bank.txt','old statement');z.writestr('readme.txt','hello');z.writestr('../escape.txt','bad')
  @contextmanager
  def open_zip(uri,c):
   with self.zip.open('rb') as f:yield f
  self.service=Archives(open_zip,self.out)
 def test_listing_extracts_nothing(self):
  r=self.service.list(self.zip.as_uri());self.assertFalse(self.out.exists());self.assertFalse(r['contentsExtracted'])
 def test_nested_virtual_directory(self):
  r=self.service.list(self.zip.as_uri(),'Documents/');self.assertEqual(r['entries'][0]['name'],'bank.txt')
 def test_dangerous_members_hidden(self):self.assertEqual(self.service.list(self.zip.as_uri())['skippedUnsafe'],1)
 def test_selected_member_only_temp_copy(self):
  r=self.service.preview_member(self.zip.as_uri(),'Documents/bank.txt');p=Path(unquote(urlsplit(r['uri']).path))
  self.assertEqual(p.read_text(),'old statement');self.assertEqual(len(list(self.out.rglob('*.*'))),1);self.assertEqual(stat.S_IMODE(p.stat().st_mode),0o400)
 def test_traversal_rejected(self):
  with self.assertRaises(ValueError):self.service.preview_member(self.zip.as_uri(),'../escape.txt')
 def test_absolute_rejected(self):self.assertFalse(safe_member('/tmp/x'))
 def test_windows_path_rejected(self):self.assertFalse(safe_member('C:\\tmp\\x'))
 def test_backslash_rejected(self):self.assertFalse(safe_member('dir\\x'))
 def test_script_preview_rejected(self):
  with zipfile.ZipFile(self.zip,'a') as z:z.writestr('run.sh','bad')
  with self.assertRaises(ValueError):self.service.preview_member(self.zip.as_uri(),'run.sh')
 def test_symlink_hidden_and_rejected(self):
  with zipfile.ZipFile(self.zip,'a') as z:
   zi=zipfile.ZipInfo('link.txt');zi.create_system=3;zi.external_attr=(stat.S_IFLNK|0o777)<<16;z.writestr(zi,'/etc/passwd')
  self.assertEqual(self.service.list(self.zip.as_uri())['skippedUnsafe'],2)
  with self.assertRaises(ValueError):self.service.preview_member(self.zip.as_uri(),'link.txt')
 def test_duplicate_member_preview_rejected(self):
  import warnings
  with warnings.catch_warnings():
   warnings.simplefilter('ignore')
   with zipfile.ZipFile(self.zip,'a') as z:z.writestr('readme.txt','different')
  with self.assertRaises(ValueError):self.service.preview_member(self.zip.as_uri(),'readme.txt')
 def test_invalid_zip_reports_error(self):
  self.zip.write_text('not a zip')
  with self.assertRaises(zipfile.BadZipFile):self.service.list(self.zip.as_uri())
 def test_central_directory_allocation_bounded(self):
  with self.assertRaises(ValueError):BoundedReader(io.BytesIO()).read(MAX_DIRECTORY+1)
 def test_empty_zip(self):
  with zipfile.ZipFile(self.zip,'w'):pass
  self.assertEqual(self.service.list(self.zip.as_uri())['entries'],[])

class SettingsWindowsTests(TempCase):
 def test_two_windows_preserve_each_others_preferences(self):
  a,b=Settings(self.base),Settings(self.base);a.update_preferences({'theme':'dark'});b.update_preferences({'details':False})
  self.assertEqual(a.snapshot()['preferences']['theme'],'dark');self.assertFalse(a.snapshot()['preferences']['details'])
 def test_two_windows_pins_not_lost(self):
  a,b=Settings(self.base),Settings(self.base);a.bookmark('add','pin','file:///home/demo/A');b.bookmark('add','pin','file:///home/demo/B');self.assertEqual(len(a.snapshot()['pins']),2)
 def test_default_context_menu_classic(self):self.assertEqual(Settings(self.base).snapshot()['preferences']['contextMenu'],'win10')
 def test_network_interval_whitelist(self):
  s=Settings(self.base);s.update_preferences({'networkInterval':30});s.update_preferences({'networkInterval':1});self.assertEqual(s.snapshot()['preferences']['networkInterval'],30)
 def test_snapshot_guard(self):
  v=PreviousVersions(self.base)
  with self.assertRaises(ValueError):v.assert_writable('smb://nas/share/.snapshot/old/file')
 def test_mapped_path_plan_requires_admin_not_automatic(self):
  result=mount_plan('smb://nas/Downloads',1000,1000);self.assertIn('sudo',result['command']);self.assertNotIn('password=',result['command'])
 def test_mount_resolution(self):
  mounts=[{'path':'/mnt/nas','fstype':'cifs','source':'//nas/share','root':'/'}]
  self.assertEqual(resolve_smb_path('smb://nas/share/folder/file.pdf',mounts),'/mnt/nas/folder/file.pdf')

if __name__=='__main__':unittest.main(verbosity=2)
