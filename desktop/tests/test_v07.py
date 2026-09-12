# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""0.7 regressions: real disposable configuration files, native seams with doubles.
These tests do not execute PyGObject/WebKit or launch Brave/NAS connections.
"""
import ast
import argparse
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch
from urllib.parse import urlsplit

ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT))
from network_locations import network_key, merge_network_locations
from window_state import tab_snapshot, filemanager_request, location
from reveal_integration import RevealRegistration, SERVICE, AUTOSTART
from brave_integration import BraveIntegration, read_object, browser_running
from filemanager_bus import FileManagerBus, XML, NAME, PATH
from core import normalise_location

class NetworkTests(unittest.TestCase):
    def test_default_port_and_case(self):self.assertEqual(network_key('smb://NAS:445/Share/'),network_key('smb://nas/share'))
    def test_custom_port_distinct(self):self.assertNotEqual(network_key('smb://nas:1445/share'),network_key('smb://nas/share'))
    def test_host_aliases_not_merged(self):self.assertNotEqual(network_key('smb://nas/share'),network_key('smb://10.0.0.1/share'))
    def test_connected_unsaved_share(self):
        x=merge_network_locations([], [{'uri':'smb://nas/work','label':'Work','mounted':True}]);self.assertTrue(x[0]['connected']);self.assertFalse(x[0]['saved'])
    def test_saved_label_preserved_and_mount_deduplicated(self):
        x=merge_network_locations([{'uri':'smb://nas/Work','label':'My Work'}],[{'uri':'smb://NAS:445/work/','label':'work','mounted':True}]);self.assertEqual(len(x),1);self.assertEqual(x[0]['label'],'My Work');self.assertTrue(x[0]['connected'])
    def test_unsaved_host_session_entry(self):
        x=merge_network_locations([],[],visited=[{'uri':'smb://nas/'}]);self.assertEqual(x[0]['kind'],'server');self.assertFalse(x[0]['connected'])
    def test_stable_cifs_mount(self):
        x=merge_network_locations([],[],stable=[{'fstype':'cifs','path':'/mnt/Work'}]);self.assertEqual(x[0]['uri'],'file:///mnt/Work');self.assertTrue(x[0]['connected'])
    def test_ignore_local_and_unmounted(self):
        self.assertEqual(merge_network_locations([], [{'uri':'smb://nas/work','mounted':False},{'uri':'file:///mnt/disk','mounted':True}],stable=[{'fstype':'ext4','path':'/mnt/disk'}]),[])
    def test_invalid_saved_ignored(self):self.assertEqual(merge_network_locations([{'uri':'https://bad'},{}],[]),[])
    def test_uri_not_label_is_unique_key(self):self.assertEqual(len(merge_network_locations([{'uri':'smb://a/work','label':'Work'},{'uri':'smb://b/work','label':'Work'}],[])),2)

class HandoffTests(unittest.TestCase):
    def test_tab_roundtrip(self):
        x=tab_snapshot({'uri':'smb://nas/work','history':['file:///home/demo','smb://nas/work'],'index':1,'selection':['smb://nas/work/report.pdf'],'view':'grid','scroll':1600,'sort':'size','descending':True});self.assertEqual(x['index'],1);self.assertEqual(x['selection'],['smb://nas/work/report.pdf']);self.assertEqual(x['view'],'grid')
    def test_no_password_fields_forwarded(self):self.assertNotIn('password',tab_snapshot({'uri':'home:','password':'not a real password'}))
    def test_unknown_scheme_rejected(self):
        with self.assertRaises(ValueError):tab_snapshot({'uri':'javascript:alert(1)'})
    def test_bad_history_position_rejected(self):
        with self.assertRaises(ValueError):tab_snapshot({'uri':'home:','index':True})
    def test_mismatching_history_resets_safely(self):self.assertEqual(tab_snapshot({'uri':'home:','history':['network:'],'index':0})['history'],['home:'])
    def test_history_size_limit(self):
        with self.assertRaises(ValueError):tab_snapshot({'history':['home:']*201})
    def test_infinite_scroll_rejected(self):
        with self.assertRaises(ValueError):tab_snapshot({'scroll':float('inf')})
    def test_selection_size_limit(self):
        with self.assertRaises(ValueError):tab_snapshot({'selection':['/tmp/a']*10001})
    def test_settings_supported(self):self.assertEqual(tab_snapshot({'uri':'settings:','settingsSection':'brave'})['settingsSection'],'brave')
    def test_showitems_keeps_file_path(self):self.assertEqual(filemanager_request('ShowItems',['file:///tmp/movie.mp4'])['uris'],['file:///tmp/movie.mp4'])
    def test_showfolders_and_properties(self):
        for method in ('ShowFolders','ShowItemProperties'):self.assertEqual(filemanager_request(method,['smb://nas/work'])['method'],method)
    def test_unsupported_method_rejected(self):
        with self.assertRaises(ValueError):filemanager_request('Execute',['/tmp/script'])
    def test_empty_request_rejected(self):
        with self.assertRaises(ValueError):filemanager_request('ShowItems',[])
    def test_reveal_limit(self):
        with self.assertRaises(ValueError):filemanager_request('ShowItems',['/tmp/x']*101)
    def test_no_virtual_locations_in_external_requests(self):
        with self.assertRaises(ValueError):filemanager_request('ShowFolders',['settings:'])

class RevealTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name);self.addCleanup(self.tmp.cleanup)
        self.r=RevealRegistration(self.root/'winspace',self.root/'config',self.root/'data')
    def test_disabled_by_default(self):self.assertFalse(self.r.enabled());self.assertEqual(list(self.root.iterdir()),[])
    def test_enable_exact_per_user_files(self):
        self.r.enable();self.assertTrue(self.r.enabled());self.assertEqual(len(self.r.files),2)
        for p in self.r.files:self.assertEqual(stat.S_IMODE(p.stat().st_mode),0o600)
    def test_enable_idempotent(self):self.r.enable();self.r.enable();self.assertTrue(self.r.enabled())
    def test_disable_exact_files(self):self.r.enable();self.r.disable();self.assertFalse(self.r.enabled());self.assertTrue(all(not p.exists() for p in self.r.files))
    def test_refuse_foreign_override(self):
        p=next(iter(self.r.files));p.parent.mkdir(parents=True);p.write_text('other manager')
        with self.assertRaises(ValueError):self.r.enable()
        self.assertEqual(p.read_text(),'other manager')
    def test_preserve_modified_override(self):
        self.r.enable();p=next(iter(self.r.files));p.write_text('my modified override');x=self.r.disable();self.assertEqual(p.read_text(),'my modified override');self.assertIn(str(p),x['preservedModifiedFiles'])
    def test_refuse_symlink(self):
        p=next(iter(self.r.files));p.parent.mkdir(parents=True);p.symlink_to(self.root/'elsewhere')
        with self.assertRaises(ValueError):self.r.enable()
    def test_activation_does_not_open_ui(self):self.assertIn('--filemanager-service',SERVICE);self.assertIn('--filemanager-service',AUTOSTART)
    def test_no_kill_or_system_files(self):self.assertNotIn('kill',SERVICE);self.assertTrue(all(p.is_relative_to(self.root) for p in self.r.files))

class BusTests(unittest.TestCase):
    def setUp(self):
        self.calls=[];self.events=[];self.registers=[];self.unregistered=[]
        conn=NS(register_object=lambda *args:self.registers.append(args) or 99,unregister_object=lambda i:self.unregistered.append(i))
        gio=NS(DBusNodeInfo=NS(new_for_xml=lambda x:NS(interfaces=['interface'])),BusNameOwnerFlags=NS(ALLOW_REPLACEMENT=1,REPLACE=2),bus_own_name_on_connection=lambda *args:self.calls.append(args) or 5,bus_unown_name=lambda i:self.events.append(i))
        self.requests=[];self.bus=FileManagerBus(gio,NS(Variant=lambda typ,v:(typ,v)),conn,lambda *args:self.requests.append(args))
    def invoke(self,method,values):
        out=[];inv=NS(return_value=lambda x:out.append(('ok',x)),return_dbus_error=lambda *x:out.append(('error',x)))
        self.bus.call(None,'peer',PATH,NAME,method,NS(unpack=lambda:values),inv);return out
    def test_registration_paths_and_flags(self):
        self.bus.enable();self.assertEqual(self.registers[0][0],PATH);self.assertEqual(self.calls[0][1:3],(NAME,2))
    def test_enable_idempotent(self):self.bus.enable();self.bus.enable();self.assertEqual(len(self.registers),1)
    def test_owner_status(self):self.bus.acquired();self.assertTrue(self.bus.owned);self.bus.lost();self.assertFalse(self.bus.owned)
    def test_disable_releases(self):self.bus.enable();self.bus.disable();self.assertEqual(self.events,[5]);self.assertEqual(self.unregistered,[99])
    def test_showitems_dispatch(self):
        out=self.invoke('ShowItems',(['file:///tmp/report.pdf'],'startup'));self.assertEqual(out[0][0],'ok');self.assertEqual(self.requests[0][0]['method'],'ShowItems');self.assertEqual(self.requests[0][1],'startup')
    def test_bad_scheme_error_not_launch(self):self.assertEqual(self.invoke('ShowFolders',(['https://example.test'],'x'))[0][0],'error');self.assertEqual(self.requests,[])
    def test_bad_method_error(self):self.assertEqual(self.invoke('Run',(['/tmp/x'],''))[0][0],'error')
    def test_no_request_executes_shell(self):
        self.invoke('ShowItems',(['/tmp/a; touch test'],''));self.assertIn('%3B',self.requests[0][0]['uris'][0])

class BraveTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup);self.root=Path(self.tmp.name);self.home=self.root/'home';self.home.mkdir();self.config=self.home/'.config'
        self.path=self.config/'BraveSoftware/Brave-Browser/Default/Preferences';self.path.parent.mkdir(parents=True)
        self.original={'profile':{'name':'Test person'},'download':{'default_directory':'/old/downloads','prompt_for_download':True},'savefile':{'default_directory':'/old/save'},'unrelated':{'keep':[1,2,3]}}
        self.path.write_text(json.dumps(self.original));self.destination=self.home/'Downloads';self.destination.mkdir();self.running=False
        self.b=BraveIntegration(self.root/'winspace',self.home,self.config,is_running=lambda:self.running);self.id='Brave-Browser:Default'
    def sync(self,**kw):return self.b.sync([self.id],str(self.destination),kw.get('confirmed',True))
    def test_detect_profiles(self):self.assertEqual(self.b.status()['profiles'][0]['name'],'Test person')
    def test_explicit_consent(self):
        with self.assertRaises(ValueError):self.sync(confirmed=False)
        self.assertEqual(json.loads(self.path.read_text()),self.original)
    def test_requires_quit(self):
        self.running=True
        with self.assertRaises(ValueError):self.sync()
        self.assertEqual(json.loads(self.path.read_text()),self.original)
    def test_changes_only_directories(self):
        self.assertEqual(self.sync()['updated'],[self.id]);data=json.loads(self.path.read_text());self.assertEqual(data['unrelated'],self.original['unrelated']);self.assertTrue(data['download']['prompt_for_download']);self.assertEqual(data['savefile']['default_directory'],str(self.destination));self.assertEqual(data['download']['default_directory'],str(self.destination))
    def test_private_backups_and_prefs(self):
        self.sync();backup=next(self.b.directory.glob('*.preferences.bak'));self.assertEqual(json.loads(backup.read_text()),self.original)
        for p in (backup,self.path,self.b._record(self.id)):self.assertEqual(stat.S_IMODE(p.stat().st_mode),0o600)
        self.assertEqual(stat.S_IMODE(self.b.directory.stat().st_mode),0o700)
    def test_restore_only_changed_keys(self):
        self.sync();d=json.loads(self.path.read_text());d['unrelated']={'new':True};self.path.write_text(json.dumps(d));self.b.restore(self.id,True);d=json.loads(self.path.read_text());self.assertEqual(d['download'],self.original['download']);self.assertEqual(d['savefile'],self.original['savefile']);self.assertEqual(d['unrelated'],{'new':True})
    def test_restore_does_not_overwrite_later_preference(self):
        self.sync();d=json.loads(self.path.read_text());d['download']['default_directory']='/manual';self.path.write_text(json.dumps(d));x=self.b.restore(self.id,True);self.assertEqual(x['restored'],['savefile']);self.assertEqual(json.loads(self.path.read_text())['download']['default_directory'],'/manual')
    def test_restore_consent_required(self):
        self.sync()
        with self.assertRaises(ValueError):self.b.restore(self.id,False)
    def test_unknown_profile(self):
        with self.assertRaises(ValueError):self.b.sync(['Brave-Browser:../outside'],str(self.destination),True)
    def test_missing_directory(self):
        with self.assertRaises(ValueError):self.b.sync([self.id],str(self.home/'missing'),True)
    def test_dedicated_directory_required(self):
        with self.assertRaises(ValueError):self.b.sync([self.id],str(self.home),True)
    def test_no_smb_uri_preference(self):
        with self.assertRaises(ValueError):self.b.sync([self.id],'smb://nas/downloads',True)
    def test_nonregular_pref_file_rejected(self):
        self.path.unlink();self.path.mkdir()
        self.assertEqual(self.b.profiles(),[])
    def test_symlink_pref_refused(self):
        target=self.root/'actual';self.path.replace(target);self.path.symlink_to(target)
        with self.assertRaises(ValueError):self.sync()
        self.assertEqual(json.loads(target.read_text()),self.original)
    def test_profile_symlink_ignored(self):
        (self.path.parent.parent/'Profile 1').symlink_to(self.path.parent,target_is_directory=True);self.assertEqual(len(self.b.profiles()),1)
    def test_late_running_race_no_pref_change(self):
        values=iter([False,True]);self.b.is_running=lambda:next(values);result=self.sync();self.assertEqual(result['updated'],[]);self.assertEqual(len(result['errors']),1);self.assertEqual(json.loads(self.path.read_text()),self.original)
    def test_preference_race_detected(self):
        calls=[]
        def running():
            calls.append(1)
            if len(calls)==2:self.path.write_text('{"external":true}')
            return False
        self.b.is_running=running;result=self.sync();self.assertEqual(result['updated'],[]);self.assertEqual(json.loads(self.path.read_text()),{'external':True})
    def test_invalid_preference_type_rejected(self):
        self.path.write_text('{"download":[]}')
        with self.assertRaises(ValueError):self.sync()
    def test_sandbox_detected_manual_only(self):
        (self.home/'.var/app/com.brave.Browser').mkdir(parents=True);(self.home/'snap/brave').mkdir(parents=True)
        self.assertEqual(self.b.status()['sandboxed'],['Flatpak','Snap']);self.assertEqual(len(self.b.profiles()),1)
    def test_multiple_profiles(self):
        second=self.path.parent.parent/'Profile 1/Preferences';second.parent.mkdir();second.write_text('{}');x=self.b.sync([self.id,'Brave-Browser:Profile 1'],str(self.destination),True);self.assertEqual(len(x['updated']),2)
    def test_relative_destination_refused(self):
        with self.assertRaises(ValueError):self.b.sync([self.id],'.',True)
    def test_proc_brave_detected(self):
        proc=self.root/'proc';p=proc/'123';p.mkdir(parents=True);(p/'cmdline').write_bytes(b'/opt/brave.com/brave/brave\0--background\0');self.assertTrue(browser_running(proc))
    def test_proc_unrelated_ignored(self):
        proc=self.root/'proc';p=proc/'123';p.mkdir(parents=True);(p/'cmdline').write_bytes(b'/usr/bin/python3\0script.py\0');self.assertFalse(browser_running(proc))

class AppManagerTests(unittest.TestCase):
    """Execute actual app coordinator methods against fake GTK/GIO objects."""
    def setUp(self):
        class Base:
            def __init__(self,**kw):self.identity=kw;self.active=None;self.options=[];self.held=0
            def connect(self,*a):pass
            def add_main_option(self,*a):self.options.append(a[0])
            def get_active_window(self):return self.active
            def hold(self):self.held+=1
            def release(self):self.held-=1
            def quit(self):self.did_quit=True
        class Controller:
            def __init__(self,app,initial=None,software_rendering=False,transfer=None):
                self.app=app;self.initial=initial;self.software=software_rendering;self.transfer=transfer;self.closed=False;self.ui_ready=True;self.tab_titles=[];self.pending_open=[];self.events=[];self.writes=0
                self.window=NS(get_id=lambda:id(self),get_title=lambda:initial or 'Home',present=lambda:setattr(app,'active',self.window),close=lambda:self.close(),set_startup_id=lambda v:None)
            def activate_window(self):self.window.present()
            def emit(self,*a):self.events.append(a)
            def close(self):self.closed=True;self.app.window_closed(self)
        tree=ast.parse((ROOT/'winspace.py').read_text());names={'OpenXplorer','argument_parser','CLI_OPTIONS'}
        nodes=[n for n in tree.body if getattr(n,'name',None) in names or isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id in names for t in n.targets)]
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        ctx={'Gtk':NS(Application=Base),'Gio':NS(ApplicationFlags=NS(HANDLES_COMMAND_LINE=1,HANDLES_OPEN=2)),
             'GLib':NS(OptionFlags=NS(NONE=0),OptionArg=NS(NONE=0)), 'Settings':lambda:NS(directory=Path(self.tmp.name)),
             'TabTransfers':__import__('tab_transfers').TabTransfers,'DesktopIntegration':lambda *a:None,'RevealRegistration':lambda *a:NS(enabled=lambda:False), 'BraveIntegration':lambda *a:None,
             'OpenXplorerWindow':Controller,'argparse':argparse,'Path':Path,'normalise_location':normalise_location,'os':os,'sys':sys,'urlsplit':urlsplit}
        exec(compile(ast.Module(body=nodes,type_ignores=[]),'app-manager-test','exec'),ctx)
        self.app=ctx['OpenXplorer']()
    def command(self,flags=(),arguments=()):
        return NS(get_arguments=lambda:['winspace',*arguments],get_options_dict=lambda:NS(lookup_value=lambda key,t:NS(unpack=lambda:True) if key in flags else None),get_cwd=lambda:self.tmp.name)
    def test_unique_application_id(self):self.assertEqual(self.app.identity['application_id'],'io.winspace.Development');self.assertEqual(self.app.identity['flags'],3)
    def test_cli_flags_registered(self):self.assertTrue({'new-window','settings','filemanager-service','windows','select','quit'}<=set(self.app.options))
    def test_new_window_not_new_application(self):self.app.create_window();self.app.create_window();self.assertEqual(len(self.app.controllers),2);self.assertTrue(all(c.app is self.app for c in self.app.controllers))
    def test_focus_existing_window(self):
        a=self.app.create_window('file:///a');self.app.create_window('file:///b');self.app.focus_window(a.window.get_id());self.assertIs(self.app.active_controller(),a)
    def test_closed_window_removed(self):c=self.app.create_window();c.close();self.assertEqual(self.app.window_list(),[])
    def test_open_settings_uses_window(self):c=self.app.create_window();self.app.open_settings();self.assertEqual(len(self.app.controllers),1);self.assertIn(('showSettings',{}),c.events)
    def test_glib_parsed_new_window_flag(self):self.app.create_window();self.assertEqual(self.app.command_line(None,self.command(['new-window'])),0);self.assertEqual(len(self.app.controllers),2)
    def test_glib_parsed_software_flag(self):self.app.command_line(None,self.command(['software-rendering']));self.assertTrue(self.app.controllers[0].software)
    def test_relative_path_uses_callers_working_directory(self):self.app.command_line(None,self.command(arguments=['child']));self.assertEqual(self.app.controllers[0].events[-1][1]['uris'],[(Path(self.tmp.name)/'child').as_uri()])
    def test_select_routes_to_reveal(self):self.app.command_line(None,self.command(['select'],['/tmp/report.pdf']));self.assertEqual(self.app.controllers[0].events[-1],('fileManagerRequest',{'method':'ShowItems','uris':['file:///tmp/report.pdf']}))
    def test_service_flag_does_not_create_window(self):self.app.command_line(None,self.command(['filemanager-service']));self.assertEqual(self.app.controllers,[])
    def test_quit_refuses_active_writes(self):c=self.app.create_window();c.writes=1;self.assertFalse(self.app.quit_safely());self.assertFalse(c.closed)
    def test_safe_quit_all_windows(self):self.app.create_window();self.app.create_window();self.assertTrue(self.app.quit_safely());self.assertTrue(self.app.did_quit)
    def test_session_network_root_not_every_child(self):self.app.remember_network('smb://nas/work/nested/a');self.app.remember_network('smb://nas/work/b');self.assertEqual(list(self.app.visited_network),['smb://nas/work'])

if __name__=='__main__':unittest.main()
