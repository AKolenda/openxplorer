#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Build and verify local release artifacts. Does not install or alter desktop settings.

The corresponding-source archive is made from editable inputs, excludes
itself/binaries/generated HTML, and includes the scripts that regenerate them.
Website downloads are intentionally removed; public distribution belongs on
the GitHub repository rather than the website host.
"""
from __future__ import annotations
from fnmatch import fnmatchcase
import hashlib
import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'desktop'))
from core import VERSION, DEBIAN_VERSION

OMITTED_DIRECTORIES = frozenset({
    'node_modules', '.next', 'out', '.git', '.hg', '.svn', '__pycache__',
    'test-results', 'dist', 'designs', '.pnpm-store', '.wrangler', '.vercel',
    '.venv', 'venv', '.pytest_cache', '.mypy_cache', '.ruff_cache', '.cache',
    '.tox', '.nox', '.hypothesis', '.nyc_output', 'htmlcov', 'coverage',
    'playwright-report', 'blob-report', 'tmp', '.tmp', 'temp', '.temp',
})
OMITTED_NAMES = (
    '.env*', '.dev.vars*', '.private-demo-terms*', 'private-terms.json',
    '.ds_store', '.coverage*', '*.py[cod]', '*.tsbuildinfo', '*.deb', '*.zip',
    '*.log', '*.log.*', '*.sqlite', '*.sqlite-*', '*.sqlite3', '*.sqlite3-*',
    '*.db', '*.db-*', '*.pem', '*.key', '*.p12', '*.pfx', '*.credentials',
    'credentials.json', 'credentials.toml', 'credentials.yaml', 'credentials.yml',
    'client_secret*.json', 'service-account*.json', '*.swp', '*.swo', '*.tmp',
    '*.temp', '*~',
)
SAFE_EXAMPLE_NAMES = frozenset({'.env.example', '.dev.vars.example'})
GENERATED_SOURCE_PATHS = frozenset({
    'desktop/preview.html', 'apps/web/public/app-preview.html',
    'apps/web/public/assets/site.js',
})


def source_path_excluded(relative: Path, *, directory: bool = False) -> bool:
    """Archive policy, independent of Git being installed or initialized.

    Keep these exclusions aligned with .gitignore. The archive additionally
    rejects links and special files in source_files instead of following them.
    Safe example filenames are intentional exceptions, not a content audit.
    """
    parts = tuple(part.casefold() for part in relative.parts)
    if any(part in OMITTED_DIRECTORIES for part in parts):
        return True
    if parts[:4] == ('apps', 'web', 'public', 'downloads'):
        return True
    if '/'.join(parts) in GENERATED_SOURCE_PATHS:
        return True
    if not directory and relative.name in SAFE_EXAMPLE_NAMES:
        return False
    return any(fnmatchcase(relative.name.casefold(), pattern) for pattern in OMITTED_NAMES)


def sha(path:Path)->str:return hashlib.sha256(path.read_bytes()).hexdigest()
def source_files(root: Path = ROOT):
    """Yield deterministic editable inputs, pruning excluded trees before IO."""
    root = Path(root).resolve()
    files = []

    def failed(error):
        # An unreadable editable source directory must fail the archive rather
        # than silently produce incomplete corresponding source.
        raise error

    for directory, names, filenames in os.walk(root, topdown=True, onerror=failed, followlinks=False):
        current = Path(directory)
        names[:] = sorted(name for name in names
                          if not (current / name).is_symlink()
                          and not source_path_excluded((current / name).relative_to(root), directory=True))
        for name in filenames:
            path = current / name
            relative = path.relative_to(root)
            if source_path_excluded(relative) or path.is_symlink() or not path.is_file():
                continue
            files.append((path, relative))
    yield from sorted(files, key=lambda item: item[1].as_posix())

def main():
    out=ROOT/'dist';out.mkdir(exist_ok=True)
    website_downloads=[ROOT/'apps/web/public/downloads',ROOT/'designs/downloads',ROOT/'apps/web/out/downloads']
    for directory in website_downloads:
        if directory.is_dir():shutil.rmtree(directory)
    for directory in (out,ROOT/'desktop/dist'):
        directory.mkdir(parents=True,exist_ok=True)
        for old in directory.iterdir():
            if old.is_file() and (old.suffix in ('.zip','.deb') or old.name=='SHA256SUMS'):old.unlink()
    (ROOT/'test-results').mkdir(exist_ok=True)
    (ROOT/'desktop/dist').mkdir(exist_ok=True)
    (ROOT/'designs').mkdir(exist_ok=True)
    package=out/f'openxplorer_{DEBIAN_VERSION}_all.deb'
    subprocess.run([sys.executable,str(ROOT/'desktop/tools/build_deb.py'),'--output',str(package)],check=True)
    subprocess.run([sys.executable,str(ROOT/'desktop/tools/verify_deb.py'),str(package),'--json',str(ROOT/'test-results/package-verification.json')],check=True)
    source=out/f'openxplorer-{VERSION}-source.zip'
    with zipfile.ZipFile(source,'w',zipfile.ZIP_DEFLATED,compresslevel=9) as z:
        for p,rel in source_files():
            info=zipfile.ZipInfo('openxplorer/'+rel.as_posix(),(2026,9,6,0,0,0));info.compress_type=zipfile.ZIP_DEFLATED;info.external_attr=0o100644<<16;z.writestr(info,p.read_bytes())
    checksums=''.join(f'{sha(p)}  {p.name}\n' for p in (package,source))
    (out/'SHA256SUMS').write_text(checksums)
    shutil.copyfile(package,ROOT/'desktop/dist'/package.name)
    subprocess.run([sys.executable,str(ROOT/'desktop/tools/build_preview.py')],check=True)
    for dest in (ROOT/'apps/web/public/app-preview.html',ROOT/'designs/app-preview.html'):shutil.copyfile(ROOT/'desktop/preview.html',dest)
    print('Built local installer, corresponding source and checksums. Website links remain on GitHub.')
if __name__=='__main__':main()
