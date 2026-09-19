# SPDX-License-Identifier: AGPL-3.0-only
"""Explicit GitHub release updates through the system package manager.

Only fixed upstream HTTPS endpoints are used. No remote code is evaluated and
no command, URL or package path is accepted from the WebKit interface. GitHub's
asset digest verifies the download; it is not an independent publisher signature.
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import Request, build_opener, HTTPRedirectHandler

from core import VERSION
from private_storage import private_directory

REPOSITORY = 'https://github.com/AKolenda/openxplorer'
LATEST = 'https://api.github.com/repos/AKolenda/openxplorer/releases/latest'
MAX_PACKAGE = 100 * 1024 * 1024
HOSTS = {'api.github.com', 'github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com'}


def version_tuple(value):
    if not isinstance(value, str) or not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', value):
        raise ValueError('The release does not have a supported stable version.')
    return tuple(int(part) for part in value.split('.'))


def trusted_url(url):
    parsed = urlsplit(url)
    if (parsed.scheme != 'https' or parsed.hostname not in HOSTS or
            parsed.username or parsed.password or parsed.port not in (None, 443)):
        raise ValueError('The update server returned an untrusted download location.')
    return url


class TrustedRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return super().redirect_request(req, fp, code, msg, headers, trusted_url(newurl))


def open_url(url):
    request = Request(trusted_url(url), headers={'User-Agent': 'OpenXplorer/' + VERSION,
        'Accept': 'application/vnd.github+json' if url == LATEST else 'application/octet-stream'})
    return build_opener(TrustedRedirect()).open(request, timeout=30)


def release_metadata(data, current=VERSION):
    if not isinstance(data, dict) or data.get('draft') or data.get('prerelease'):
        raise ValueError('No stable release is available.')
    tag = data.get('tag_name', '')
    if not isinstance(tag, str) or not tag.startswith('v'):
        raise ValueError('The release tag is invalid.')
    version = tag[1:]
    newer = version_tuple(version) > version_tuple(current)
    name = f'openxplorer_{version}_all.deb'
    expected_url = f'{REPOSITORY}/releases/download/{tag}/{name}'
    assets = data.get('assets')
    if not isinstance(assets, list):
        raise ValueError('The release asset list is invalid.')
    asset = next((a for a in assets if isinstance(a, dict) and a.get('name') == name), None)
    if not asset or asset.get('browser_download_url') != expected_url:
        raise ValueError('The release is missing its expected Debian installer.')
    digest = asset.get('digest') or ''
    if not isinstance(digest, str) or not re.fullmatch(r'sha256:[a-f0-9]{64}', digest):
        raise ValueError('The release installer has no verified SHA-256 digest yet. Try again later.')
    size = asset.get('size')
    if type(size) is not int or not 0 < size <= MAX_PACKAGE:
        raise ValueError('The release installer size is invalid.')
    return {'currentVersion': current, 'version': version, 'available': newer,
        'notes': str(data.get('body') or '')[:20000], 'releaseUrl': f'{REPOSITORY}/releases/tag/{tag}',
        'url': expected_url, 'sha256': digest[7:], 'size': size, 'name': name}


class Updater:
    def __init__(self, *, current=VERSION, root=None, directory=None, opener=open_url, run=subprocess.run):
        self.current = current
        self.root = Path(root or Path(__file__).resolve().parent)
        self.directory = Path(directory or Path(os.environ.get('XDG_CACHE_HOME', Path.home()/'.cache'))/'winspace'/'updates')
        self.opener, self.run = opener, run
        self.lock = threading.Lock()
        self.release = None
        self.busy = False
        self.installed_version = None

    def can_install(self):
        return (self.root == Path('/opt/openxplorer') and
                all(Path(p).is_file() and os.access(p, os.X_OK) for p in
                    ('/usr/bin/pkexec', '/usr/bin/apt-get', '/usr/bin/dpkg-deb', '/usr/bin/openxplorer')))

    def check(self):
        if not self.lock.acquire(blocking=False):
            raise ValueError('An update task is already running.')
        try:
            self.release = None
            try:
                with self.opener(LATEST) as response:
                    raw = response.read(2 * 1024 * 1024 + 1)
                if len(raw) > 2 * 1024 * 1024:
                    raise ValueError('The update response is too large.')
                self.release = release_metadata(json.loads(raw), self.current)
            except HTTPError as exc:
                raise ValueError('GitHub could not check for updates (HTTP %s). Try again later.' % exc.code) from exc
            except (URLError, TimeoutError) as exc:
                raise ValueError('Could not reach GitHub. Check your connection and try again.') from exc
            return {k: v for k, v in self.release.items() if k not in ('url', 'sha256', 'size', 'name')} | {
                'canInstall': self.can_install(), 'restartRequired': bool(self.installed_version)}
        finally:
            self.lock.release()

    def install(self, version, confirmed, progress=lambda message: None):
        if confirmed is not True:
            raise ValueError('Confirm installation before updating.')
        if not self.lock.acquire(blocking=False):
            raise ValueError('An update task is already running.')
        self.busy = True
        try:
            release = self.release
            if not release or not release['available'] or version != release['version']:
                raise ValueError('Check for updates again before installing.')
            if not self.can_install():
                raise ValueError('In-app installation requires the installed Debian package and polkit. Use GitHub Releases for this build.')
            private_directory(self.directory)
            with tempfile.TemporaryDirectory(prefix='release-', dir=self.directory) as temporary:
                package = Path(temporary)/release['name']
                progress('Downloading OpenXplorer ' + version + '…')
                digest, received = hashlib.sha256(), 0
                with self.opener(release['url']) as response, package.open('xb') as output:
                    os.chmod(package, 0o600)
                    while True:
                        block = response.read(128 * 1024)
                        if not block:
                            break
                        received += len(block)
                        if received > release['size']:
                            raise ValueError('The downloaded installer is larger than expected.')
                        output.write(block)
                        digest.update(block)
                    output.flush()
                    os.fsync(output.fileno())
                if received != release['size'] or digest.hexdigest() != release['sha256']:
                    raise ValueError('The installer checksum or size did not match. Nothing was installed.')
                metadata = self.run(['/usr/bin/dpkg-deb', '-f', str(package), 'Package', 'Version', 'Architecture'],
                    capture_output=True, text=True, check=True, timeout=30).stdout
                fields = dict(line.split(': ', 1) for line in metadata.splitlines() if ': ' in line)
                if fields != {'Package': 'openxplorer', 'Version': version, 'Architecture': 'all'}:
                    raise ValueError('The installer metadata did not match this release. Nothing was installed.')
                progress('Approve the system administrator prompt to install. Do not close OpenXplorer…')
                # No timeout: killing apt/dpkg mid-install can damage package state.
                # --no-remove refuses dependency resolutions that remove packages.
                result = self.run(['/usr/bin/pkexec', '/usr/bin/apt-get', '-y', '--no-remove', 'install', str(package)],
                    capture_output=True, text=True)
                if result.returncode:
                    message = ((result.stderr or result.stdout or '').strip())[-2000:]
                    raise ValueError('Installation was cancelled or failed. ' + message)
                installed = self.run(['/usr/bin/dpkg-query', '-W', '-f=${Status}\n${Version}', 'openxplorer'],
                    capture_output=True, text=True, check=True, timeout=30).stdout.strip()
                if installed != 'install ok installed\n' + version:
                    raise ValueError('The package manager did not confirm the expected installed version.')
                self.installed_version = version
                progress('Update installed. Restart OpenXplorer to use it.')
                return {'installed': True, 'version': version}
        finally:
            self.busy = False
            self.lock.release()
