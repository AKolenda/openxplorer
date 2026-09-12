# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Explicit, per-user file associations with a saved previous-handler record.
Never run with sudo; installation does not call this module or change defaults.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from private_storage import private_directory, private_text

APP_ID='io.winspace.Development.desktop'
TYPES=('inode/directory','x-scheme-handler/smb')
ZIP_TYPES=('application/zip','application/x-zip','application/x-zip-compressed')
ALL_TYPES=TYPES+ZIP_TYPES
VALID_DESKTOP=re.compile(r'^[A-Za-z0-9_.@+-]+\.desktop$')


class DesktopIntegration:
    def __init__(self, directory: Path, run=None):
        self.path=directory/'previous-defaults.json'
        self.run=run or self._run

    @staticmethod
    def _run(args):
        try:
            return subprocess.run(args,capture_output=True,text=True,timeout=8,check=True).stdout.strip()
        except FileNotFoundError as exc:
            raise ValueError('Install xdg-utils to manage the default file explorer.') from exc
        except subprocess.CalledProcessError as exc:
            raise ValueError('The desktop did not accept the file-association change.') from exc
        except subprocess.TimeoutExpired as exc:
            raise ValueError('The desktop took too long to update the default. Try again.') from exc

    def previous(self):
        try:
            data=json.loads(private_text(self.path, 65536))
            return {k:v for k,v in data.items() if k in ALL_TYPES and (v=='' or isinstance(v,str) and VALID_DESKTOP.fullmatch(v))}
        except (OSError,ValueError,AttributeError):
            return {}

    def status(self):
        current={kind:self.run(['xdg-mime','query','default',kind]) for kind in ALL_TYPES}
        previous=self.previous()
        return {'current':current,'isDefault':current['inode/directory']==APP_ID,
                'allDefault':all(current[k]==APP_ID for k in TYPES),
                'zipDefault':all(current[k]==APP_ID for k in ZIP_TYPES),
                'canRestoreZip':any(previous.get(k) for k in ZIP_TYPES),
                'canRestore':any(self.previous().values()),'appId':APP_ID}

    def _save(self,data):
        private_directory(self.path.parent)
        fd,tmp=tempfile.mkstemp(dir=self.path.parent,prefix='.defaults-')
        try:
            with os.fdopen(fd,'w') as out:
                os.fchmod(out.fileno(),0o600);json.dump(data,out);out.flush();os.fsync(out.fileno())
            os.replace(tmp,self.path)
        finally:
            if os.path.exists(tmp):os.unlink(tmp)

    def make_default(self, include_zip=False):
        return self._make(TYPES + ZIP_TYPES if include_zip else TYPES)

    def zip_default(self):
        return self._make(ZIP_TYPES)

    def _make(self, kinds):
        state=self.status()
        previous=self.previous()
        for kind in kinds:
            value=state['current'][kind]
            if value!=APP_ID:
                if value and not VALID_DESKTOP.fullmatch(value):
                    raise ValueError('The current desktop handler cannot be safely recorded.')
                previous[kind]=value
        self._save(previous)
        for kind in kinds:
            self.run(['xdg-mime','default',APP_ID,kind])
        result=self.status()
        if not all(result['current'][k]==APP_ID for k in kinds):
            raise ValueError('The desktop did not confirm all requested defaults. Check your system’s Default Applications settings.')
        return result

    def restore(self, zip_only=False):
        previous=self.previous()
        if zip_only: previous={k:v for k,v in previous.items() if k in ZIP_TYPES}
        if not any(previous.values()):
            raise ValueError('No previous handler was recorded. Choose one in your desktop settings.')
        for kind,value in previous.items():
            # Respect any changes made by the user or another app after ours.
            if value and self.run(['xdg-mime','query','default',kind])==APP_ID:
                self.run(['xdg-mime','default',value,kind])
        result=self.status()
        result['note']='Restored recorded handlers. Types with no previous handler must be changed in desktop settings.'
        return result
