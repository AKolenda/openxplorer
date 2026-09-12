# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Explicit, offline Brave download-preference synchronization.
Only detected native Brave profiles are writable. The browser must be fully
closed. Each write is backed up, private and atomic; unrelated prefs stay intact.
No browser policies, credentials, extensions, process termination or sudo.
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import time

PROFILE = re.compile(r'^(Default|Profile [0-9]+)$')
FLAVORS = ('Brave-Browser','Brave-Browser-Beta','Brave-Browser-Nightly')


def browser_running(proc=Path('/proc')):
    """Fail closed on relevant process entries we cannot inspect."""
    for p in proc.iterdir():
        if not p.name.isdecimal():continue
        try:
            if p.stat().st_uid!=os.getuid():continue
            cmd=(p/'cmdline').read_bytes().split(b'\0')
            exe=os.path.basename(os.fsdecode(cmd[0])) if cmd else ''
            if exe in ('brave','brave-browser','brave-browser-stable','brave-browser-beta','brave-browser-nightly','brave_crashpad_handler'):
                return True
        except FileNotFoundError:continue
        except PermissionError:
            try:
                if 'brave' in (p/'comm').read_text().casefold():return True
            except OSError:return True
    return False


def read_object(path):
    if path.is_symlink():raise ValueError('Refusing a symlinked browser preference file.')
    st=path.stat()
    if not stat.S_ISREG(st.st_mode) or st.st_uid!=os.getuid() or st.st_size>32_000_000:
        raise ValueError('Browser preferences are not a supported private regular file.')
    raw=path.read_bytes();data=json.loads(raw)
    if not isinstance(data,dict):raise ValueError('Browser preferences are not an object.')
    return raw,data


def atomic_bytes(path,raw):
    fd,tmp=tempfile.mkstemp(dir=path.parent,prefix='.winspace-')
    try:
        with os.fdopen(fd,'wb') as out:
            os.fchmod(out.fileno(),0o600);out.write(raw);out.flush();os.fsync(out.fileno())
        os.replace(tmp,path)
    finally:
        if os.path.exists(tmp):os.unlink(tmp)


class BraveIntegration:
    def __init__(self,directory,home=None,config_home=None,is_running=None):
        self.home=Path(home or Path.home())
        self.config=Path(config_home or os.environ.get('XDG_CONFIG_HOME',self.home/'.config'))
        self.directory=Path(directory)/'brave-backups'
        self.is_running=is_running or browser_running
    def profiles(self):
        result=[]
        for flavor in FLAVORS:
            root=self.config/'BraveSoftware'/flavor
            if not root.is_dir() or root.is_symlink():continue
            for path in sorted(root.iterdir()):
                if not PROFILE.fullmatch(path.name) or path.is_symlink() or not path.is_dir():continue
                try:
                    _,data=read_object(path/'Preferences')
                    name=data.get('profile',{}).get('name',path.name)
                    download=data.get('download',{}).get('default_directory','')
                    result.append({'id':flavor+':'+path.name,'name':str(name)[:160],
                                   'flavor':flavor,'directory':str(path), 'downloadPath':str(download)[:4096]})
                except (ValueError,OSError,TypeError,AttributeError):continue
        return result
    def status(self):
        sandbox=[]
        if (self.home/'.var/app/com.brave.Browser').exists():sandbox.append('Flatpak')
        if (self.home/'snap/brave').exists():sandbox.append('Snap')
        return {'profiles':self.profiles(),'running':self.is_running(),'sandboxed':sandbox,
                'manualUrl':'brave://settings/downloads','backups':str(self.directory)}
    def _profile(self,profile_id):
        found=next((p for p in self.profiles() if p['id']==profile_id),None)
        if not found:raise ValueError('Choose a detected native Brave profile. Custom, Snap and Flatpak profiles require manual browser settings.')
        return Path(found['directory'])/'Preferences'
    def _record(self,profile_id):
        return self.directory/(hashlib.sha256(profile_id.encode()).hexdigest()[:24]+'.json')
    def sync(self,profile_ids,destination,confirmed=False):
        if confirmed is not True:raise ValueError('Confirm updating the selected Brave profiles.')
        if not isinstance(profile_ids,list) or not 1<=len(profile_ids)<=40 or len(set(profile_ids))!=len(profile_ids):
            raise ValueError('Select 1–40 distinct Brave profiles.')
        if not isinstance(destination,str) or '\x00' in destination:raise ValueError('Invalid download directory.')
        if not Path(destination).is_absolute():raise ValueError('Use an absolute download directory.')
        target=Path(destination).resolve()
        if not target.is_absolute() or not target.is_dir() or not os.access(target,os.W_OK|os.X_OK):raise ValueError('Downloads must be an existing writable local or persistent-mount path.')
        if str(target).startswith(('/run/','/proc/','/sys/','/dev/')) or target==self.home or str(target)=='/':raise ValueError('Use a persistent, dedicated download directory.')
        if self.is_running():raise ValueError('Fully quit Brave, including background processes, then retry. OpenXplorer will not force it to close.')
        # Validate all before writing any. Individual completion is reported if a late race occurs.
        plans=[]
        for profile_id in profile_ids:
            path=self._profile(profile_id);raw,data=read_object(path)
            for key in ('download','savefile'):
                if key in data and not isinstance(data[key],dict):raise ValueError('Unsupported browser preference structure.')
            plans.append((profile_id,path,raw,data))
        if self.directory.is_symlink():raise ValueError('Refusing a symlinked backup directory.')
        self.directory.mkdir(parents=True,exist_ok=True,mode=0o700)
        os.chmod(self.directory,0o700)
        done=[];errors=[]
        for profile_id,path,raw,data in plans:
            try:
                old={key: {'present':'default_directory' in data.get(key,{}),'value':data.get(key,{}).get('default_directory')} for key in ('download','savefile')}
                backup=self.directory/(hashlib.sha256(profile_id.encode()).hexdigest()[:24]+'-'+str(time.time_ns())+'.preferences.bak')
                atomic_bytes(backup,raw)
                for key in ('download','savefile'):data.setdefault(key,{})['default_directory']=str(target)
                record={'profile':profile_id,'previous':old,'applied':str(target),'backup':str(backup)}
                if self.is_running() or path.is_symlink() or path.read_bytes()!=raw:
                    raise ValueError('Brave started or its preferences changed. Close Brave and retry.')
                # Record the undo information first. If the subsequent atomic replacement
                # fails, restore detects that the applied value is not present.
                atomic_bytes(self._record(profile_id),json.dumps(record).encode())
                atomic_bytes(path,(json.dumps(data,ensure_ascii=False,separators=(',',':'))+'\n').encode())
                done.append(profile_id)
            except (ValueError,OSError) as exc:errors.append({'profile':profile_id,'message':str(exc)})
        return {'updated':done,'errors':errors,'path':str(target),'backups':str(self.directory)}
    def restore(self,profile_id,confirmed=False):
        if confirmed is not True:raise ValueError('Confirm restoring the previous Brave download directory.')
        if self.is_running():raise ValueError('Fully quit Brave before restoring.')
        record_path=self._record(profile_id)
        if not record_path.exists():raise ValueError('No previous download setting was recorded for this profile.')
        _,record=read_object(record_path)
        path=self._profile(profile_id);raw,data=read_object(path)
        restored=[]
        for key,old in record['previous'].items():
            if key not in ('download','savefile'):continue
            if data.get(key,{}).get('default_directory')!=record['applied']:continue
            if old['present']:data[key]['default_directory']=old['value']
            else:data[key].pop('default_directory',None)
            restored.append(key)
        if not restored:raise ValueError('Brave settings changed since OpenXplorer last updated them. Nothing was overwritten.')
        if self.is_running() or path.is_symlink() or path.read_bytes()!=raw:raise ValueError('Browser preferences changed; retry after closing Brave.')
        atomic_bytes(path,json.dumps(data,ensure_ascii=False,separators=(',',':')).encode())
        record_path.unlink()
        return {'restored':restored}
