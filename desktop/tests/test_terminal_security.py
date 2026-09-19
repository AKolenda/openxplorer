# SPDX-License-Identifier: AGPL-3.0-only
"""Release-candidate regression checks, not a penetration-test certificate.

Real temporary files and processes; metadata/keyring and terminal executable
selection are test doubles. No real NAS, graphical terminal or GTK is started.
"""
from contextlib import contextmanager
import io
import json
import os
from pathlib import Path
import stat
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch
import zipfile

from archives import Archives, safe_member
from core import Settings, DEBIAN_VERSION, VERSION
from operations import TransferEngine
from private_storage import private_directory, private_file, private_text
from search_index import SearchIndex
from session_credentials import SessionCredentials
from terminal_integration import (Terminal, SYSTEM_PATH, checked_directory,
                                 find_terminal, prepare_directory, terminal_argv,
                                 launch_terminal)
from tests.local_provider import Cancellation
from tests.test_v05 import FakeSecret


class Temporary(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='openxplorer-security-')
        self.root = Path(self.temp.name)
    def tearDown(self): self.temp.cleanup()


class TerminalTests(Temporary):
    def prepare(self, uri=None, entry=None, local=None, guard=None, cancel=None):
        return prepare_directory(uri or self.root.as_uri(),
            lambda u,c: entry or {'kind':'directory','isDir':True},
            lambda u: str(self.root) if local is None else local,
            guard or (lambda u: None), cancel)

    def test_local_directory(self):
        self.assertEqual(self.prepare()['path'], str(self.root))
    def test_fresh_file_uses_parent(self):
        result=self.prepare(self.root.as_uri()+'/movie.mp4',{'kind':'file','isDir':False})
        self.assertEqual(result['uri'], self.root.as_uri())
    def test_folder_with_file_extension(self):
        folder=self.root/'media.mp4';folder.mkdir()
        self.assertEqual(self.prepare(folder.as_uri(),local=str(folder))['uri'],folder.as_uri())
    def test_smb_export_is_local_shell(self):
        result=self.prepare('smb://studio-nas/Projects',local=str(self.root))
        self.assertTrue(result['network']);self.assertEqual(result['path'],str(self.root))
    def test_bare_server_rejected_before_inspection(self):
        inspect=Mock()
        with self.assertRaises(ValueError): prepare_directory('smb://studio-nas',inspect,Mock(),Mock())
        inspect.assert_not_called()
    def test_smb_missing_fuse_mount_explains(self):
        with self.assertRaisesRegex(ValueError,'gvfs-fuse'):
            self.prepare('smb://studio-nas/Projects',local='')
    def test_symbolic_link_rejected(self):
        with self.assertRaises(ValueError):self.prepare(entry={'kind':'symlink','isDir':False,'symlink':True})
    def test_special_file_rejected(self):
        with self.assertRaises(ValueError):self.prepare(entry={'kind':'special','isDir':False})
    def test_unknown_metadata_rejected(self):
        with self.assertRaises(ValueError):self.prepare(entry={'kind':'unknown','isDir':False})
    def test_snapshot_rejected(self):
        with self.assertRaisesRegex(ValueError,'Previous-version'):
            self.prepare('smb://studio-nas/Projects/.zfs/snapshot/auto-2026-01-01')
    def test_snapshot_alias_rejected_after_resolving(self):
        snap=self.root/'.zfs/snapshot/day';snap.mkdir(parents=True)
        alias=self.root/'alias';alias.symlink_to(snap,target_is_directory=True)
        with self.assertRaisesRegex(ValueError,'Previous-version'):
            self.prepare(alias.as_uri(),local=str(alias))
    def test_custom_snapshot_guard_called_for_local_alias(self):
        seen=[]
        self.prepare('smb://studio-nas/Projects',guard=seen.append)
        self.assertIn(self.root.as_uri(),seen)
    def test_absolute_cwd_required(self):
        with self.assertRaises(ValueError):checked_directory('relative')
    def test_directory_must_exist(self):
        with self.assertRaises(FileNotFoundError):checked_directory(str(self.root/'missing'))
    def test_cwd_file_rejected(self):
        file=self.root/'a';file.write_text('x')
        with self.assertRaises(ValueError):checked_directory(str(file))
    def test_cwd_controls_rejected(self):
        with self.assertRaises(ValueError):checked_directory(str(self.root/'a\nb'))
    def test_uri_schemes_rejected(self):
        for uri in ('https://example.org','javascript:alert(1)','smb://user:password@studio-nas/Projects'):
            with self.subTest(uri=uri),self.assertRaises(ValueError):self.prepare(uri)
    def test_cancellation_checked(self):
        cancel=Cancellation();cancel.cancel()
        with self.assertRaises(Exception):self.prepare(cancel=cancel)
    def test_no_terminal_actionable(self):
        with patch('terminal_integration.shutil.which',return_value=None):
            with self.assertRaisesRegex(ValueError,'gnome-terminal'):find_terminal()
    def test_only_trusted_path_searched(self):
        with patch.dict(os.environ,{'PATH':str(self.root),'TERMINAL':'malicious -c command'}),patch('terminal_integration.shutil.which',return_value=None) as which:
            with self.assertRaises(ValueError):find_terminal()
            self.assertTrue(all(c.kwargs.get('path')==SYSTEM_PATH for c in which.call_args_list))
    def test_debian_gnome_alternative(self):
        alternative=self.root/'x-terminal-emulator';target=self.root/'gnome-terminal.wrapper';target.touch();alternative.symlink_to(target)
        def find(name,**kwargs):return str(alternative) if name=='x-terminal-emulator' else '/usr/bin/gnome-terminal' if name=='gnome-terminal' else None
        with patch('terminal_integration.shutil.which',side_effect=find):
            self.assertEqual(find_terminal(),Terminal('/usr/bin/gnome-terminal','gnome-terminal'))
    def test_all_cli_styles(self):
        for name,arg in [('kgx','--working-directory='),('gnome-terminal','--working-directory='),('xfce4-terminal','--working-directory='),('konsole','--workdir=')]:
            self.assertEqual(terminal_argv(Terminal('/usr/bin/'+name,name),str(self.root)),['/usr/bin/'+name,arg+str(self.root)])
        self.assertEqual(terminal_argv(Terminal('/usr/bin/xterm','xterm'),str(self.root)),['/usr/bin/xterm'])
    def test_unknown_executable_kind_rejected(self):
        with self.assertRaises(ValueError):terminal_argv(Terminal('/bin/sh','sh'),str(self.root))
    def test_real_process_preserves_literal_shell_metacharacters(self):
        # Benign recorder stands in for the graphical emulator. Nothing from cwd
        # is appended to a shell command, even though its name LOOKS like code.
        folder=self.root/'$(touch PWNED); apostrophe\' & spaces';folder.mkdir()
        recorder=self.root/'recorder';recorder.write_text('#!/usr/bin/python3\nimport json,os,sys\nopen("record.json","w").write(json.dumps({"cwd":os.getcwd(),"argv":sys.argv[1:]}))\n')
        recorder.chmod(0o700)
        result=launch_terminal({'path':str(folder),'uri':folder.as_uri()},Terminal(str(recorder),'gnome-terminal'))
        data=json.loads((folder/'record.json').read_text())
        self.assertEqual(data['argv'],['--working-directory='+str(folder)])
        self.assertEqual(data['cwd'],str(folder));self.assertTrue(result['opened'])
        self.assertFalse((folder/'PWNED').exists());self.assertFalse((self.root/'PWNED').exists())
    def test_immediate_failure_reported(self):
        recorder=self.root/'fail';recorder.write_text('#!/bin/sh\nexit 7\n');recorder.chmod(0o700)
        with self.assertRaisesRegex(RuntimeError,'exit 7'):
            launch_terminal({'path':str(self.root),'uri':self.root.as_uri()},Terminal(str(recorder),'xterm'))


class PrivateStorageTests(Temporary):
    def test_permissions(self):
        directory=self.root/'private';private_directory(directory)
        fd=private_file(directory/'state',create=True,writable=True);os.close(fd)
        self.assertEqual(directory.stat().st_mode&0o777,0o700)
        self.assertEqual((directory/'state').stat().st_mode&0o777,0o600)
    def test_directory_symlink_not_chmodded(self):
        real=self.root/'real';real.mkdir(mode=0o755);link=self.root/'link';link.symlink_to(real)
        with self.assertRaises(OSError):private_directory(link)
        self.assertEqual(real.stat().st_mode&0o777,0o755)
    def test_file_symlink_leaves_target_unchanged(self):
        real=self.root/'real';real.write_text('private');real.chmod(0o644)
        link=self.root/'link';link.symlink_to(real)
        with self.assertRaises(OSError):private_file(link,writable=True)
        self.assertEqual(real.read_text(),'private');self.assertEqual(real.stat().st_mode&0o777,0o644)
    def test_hardlink_rejected(self):
        real=self.root/'real';real.write_text('x');link=self.root/'link';os.link(real,link)
        with self.assertRaises(ValueError):private_file(link)
    def test_fifo_does_not_block(self):
        fifo=self.root/'pipe';os.mkfifo(fifo)
        with self.assertRaises(ValueError):private_file(fifo)
    def test_settings_read_bound(self):
        file=self.root/'large';file.write_bytes(b'x'*33)
        with self.assertRaises(ValueError):private_text(file,32)
    def test_settings_lock_symlink_refused(self):
        directory=self.root/'config';store=Settings(directory);directory.mkdir()
        target=self.root/'target';target.write_text('unchanged');(directory/'settings.lock').symlink_to(target)
        with self.assertRaises(OSError):store.update_preferences({'theme':'dark'})
        self.assertEqual(target.read_text(),'unchanged')
    def test_settings_json_symlink_not_overwritten(self):
        directory=self.root/'config';directory.mkdir();target=self.root/'target';target.write_text('{}')
        (directory/'settings.json').symlink_to(target);store=Settings(directory)
        self.assertTrue(store.warning)
        with self.assertRaises(OSError):store.save()
        self.assertTrue(store.path.is_symlink());self.assertEqual(target.read_text(),'{}')
    def test_database_symlink_refused(self):
        directory=self.root/'cache';directory.mkdir();target=self.root/'target';target.write_bytes(b'unchanged')
        (directory/'search.sqlite3').symlink_to(target)
        with self.assertRaises(OSError):SearchIndex(directory)
        self.assertEqual(target.read_bytes(),b'unchanged')
    def test_database_sidecar_symlink_refused(self):
        directory=self.root/'cache';directory.mkdir();target=self.root/'target';target.write_bytes(b'unchanged')
        (directory/'search.sqlite3-wal').symlink_to(target)
        with self.assertRaises(OSError):SearchIndex(directory)
        self.assertEqual(target.read_bytes(),b'unchanged')
    def test_sqlite_sidecar_unlinked_during_check_is_allowed(self):
        from types import SimpleNamespace
        file=self.root/'sidecar';file.touch()
        with patch('private_storage.os.fstat',return_value=SimpleNamespace(st_mode=stat.S_IFREG|0o600,st_uid=os.geteuid(),st_nlink=0)),patch('private_storage.os.fchmod') as chmod:
            fd=private_file(file,writable=True,allow_unlinked=True);os.close(fd)
            chmod.assert_not_called()
            with self.assertRaises(ValueError):private_file(file,writable=True)
    def test_settings_still_persist(self):
        directory=self.root/'config';store=Settings(directory);store.update_preferences({'textSize':125})
        self.assertEqual(Settings(directory).snapshot()['preferences']['textSize'],125)


class AdditionalSecurityTests(Temporary):
    def test_zip_rejects_ambiguous_and_control_names(self):
        for name in ('a//b.txt','a/./b.txt','a/../b.txt','/root.txt','a\nb.txt','a\x7fb.txt','C:/file','../file'):
            self.assertFalse(safe_member(name),repr(name))
        self.assertTrue(safe_member('Design files/draft.txt'))
    def test_zip_preview_rejects_special_members(self):
        data=io.BytesIO()
        with zipfile.ZipFile(data,'w') as z:
            for mode,name in [(stat.S_IFIFO,'pipe.txt'),(stat.S_IFCHR,'device.txt'),(stat.S_IFLNK,'link.txt')]:
                i=zipfile.ZipInfo(name);i.external_attr=(mode|0o600)<<16;z.writestr(i,'contents')
            z.writestr('ordinary.txt','ordinary')
        @contextmanager
        def opener(*args):yield io.BytesIO(data.getvalue())
        archives=Archives(opener,self.root)
        listing=archives.list('file:///sample.zip')
        self.assertEqual([e['name'] for e in listing['entries']],['ordinary.txt'])
        self.assertEqual(listing['skippedUnsafe'],3)
        with self.assertRaises(ValueError):archives.preview_member('file:///sample.zip','pipe.txt')
    def test_stale_credential_write_after_signout_discarded(self):
        secret=FakeSecret();cache=SessionCredentials(secret)
        uri='smb://security-test-nas/Projects';value={'username':'demo','domain':'','password':'fictional-test','remember':False}
        generation=cache.generation(uri);cache.accept_memory(uri,value)
        cache.forget_memory(uri);cache.forget(uri)
        cache.persist(uri,value,generation)
        self.assertFalse(secret.rows);self.assertIsNone(cache.peek(uri))
    def test_forget_waits_for_inflight_keyring_save(self):
        secret=FakeSecret();cache=SessionCredentials(secret);start=threading.Event();release=threading.Event()
        original=secret.password_store_sync
        def delayed(*args):start.set();release.wait(2);return original(*args)
        secret.password_store_sync=delayed
        uri='smb://security-test-nas/Projects';value={'username':'demo','domain':'','password':'fictional-test','remember':False}
        generation=cache.generation(uri)
        writer=threading.Thread(target=cache.persist,args=(uri,value,generation));writer.start();self.assertTrue(start.wait(1))
        clearer=threading.Thread(target=cache.forget,args=(uri,));clearer.start();release.set();writer.join(2);clearer.join(2)
        self.assertFalse(writer.is_alive() or clearer.is_alive());self.assertFalse(secret.rows)
    def test_admin_helper_checks_existing_parent_chain(self):
        from mount_share import secure_dir
        # /tmp is world-writable even when all child directories look private.
        directory=self.root/'owned';directory.mkdir(mode=0o700)
        with self.assertRaises(ValueError):secure_dir(directory,0o700)
    def test_stable_version_upgrades_candidates(self):
        import subprocess
        self.assertEqual(VERSION,'1.1.1')
        self.assertEqual(DEBIAN_VERSION,'1.1.1')
        # apt must treat the stable package as an upgrade over every candidate.
        subprocess.run(['dpkg','--compare-versions',DEBIAN_VERSION,'gt','1.0.0~rc4'],check=True)
        subprocess.run(['dpkg','--compare-versions',DEBIAN_VERSION,'gt','0.9.3'],check=True)


if __name__=='__main__':unittest.main()
