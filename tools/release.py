#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Build and verify the local release artifacts, without installing anything.

The release is the Debian package, the corresponding-source archive and their
SHA256SUMS in dist/. The source archive holds every editable input, including
the scripts that regenerate generated files, and leaves out itself, binaries
and generated HTML. Website downloads are removed on purpose: releases are
published on GitHub, not on the website host. Nothing here changes desktop
settings.
"""
from __future__ import annotations

import argparse
from collections.abc import Iterable
from fnmatch import fnmatchcase
import hashlib
import importlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import NoReturn
import zipfile

ROOT = Path(__file__).resolve().parents[1]
DESKTOP = ROOT / 'desktop'
DIST = ROOT / 'dist'
DESKTOP_DIST = DESKTOP / 'dist'
TEST_RESULTS = ROOT / 'test-results'
DESIGNS = ROOT / 'designs'

# The source-archive policy below must agree with .gitignore;
# tests/test_release_source.py checks that it does.

# Directory names that are never release inputs, at any depth: dependencies,
# version-control data, caches, and build and test output.
OMITTED_DIRECTORIES = frozenset({
    'node_modules', '.next', 'out', '.git', '.hg', '.svn', '__pycache__',
    'test-results', 'dist', 'designs', '.pnpm-store', '.wrangler', '.vercel',
    '.venv', 'venv', '.pytest_cache', '.mypy_cache', '.ruff_cache', '.cache',
    '.tox', '.nox', '.hypothesis', '.nyc_output', 'htmlcov', 'coverage',
    'playwright-report', 'blob-report', 'tmp', '.tmp', 'temp', '.temp',
})
# File-name patterns, matched case-insensitively: secrets, local databases and
# logs, private-review identifiers, built packages and editor backups.
OMITTED_NAMES = (
    '.env*', '.dev.vars*', '.private-demo-terms*', 'private-terms.json',
    '.ds_store', '.coverage*', '*.py[cod]', '*.tsbuildinfo', '*.deb', '*.zip',
    '*.log', '*.log.*', '*.sqlite', '*.sqlite-*', '*.sqlite3', '*.sqlite3-*',
    '*.db', '*.db-*', '*.pem', '*.key', '*.p12', '*.pfx', '*.credentials',
    'credentials.json', 'credentials.toml', 'credentials.yaml', 'credentials.yml',
    'client_secret*.json', 'service-account*.json', '*.swp', '*.swo', '*.tmp',
    '*.temp', '*~',
)
# Example files with public placeholders. They match '.env*' or '.dev.vars*'
# but are published; only these exact, case-sensitive names are exempt.
SAFE_EXAMPLE_NAMES = frozenset({'.env.example', '.dev.vars.example'})
# Files the release regenerates from sources that the archive does include.
GENERATED_SOURCE_PATHS = frozenset({
    'desktop/preview.html', 'apps/web/public/app-preview.html',
    'apps/web/public/assets/site.js',
})

# Download folders in the website trees, removed by every release.
WEBSITE_DOWNLOADS = (
    ROOT / 'apps/web/public/downloads',
    ROOT / 'designs/downloads',
    ROOT / 'apps/web/out/downloads',
)
# Where the offline preview is copied: the website and the design review.
PREVIEW_COPIES = (
    ROOT / 'apps/web/public/app-preview.html',
    DESIGNS / 'app-preview.html',
)

# Every archive entry gets the same timestamp and mode, so the same tree always
# produces the same archive.
ARCHIVE_PREFIX = 'openxplorer/'
ARCHIVE_TIMESTAMP = (2026, 9, 6, 0, 0, 0)
ARCHIVE_FILE_MODE = 0o100644  # A regular file, rw-r--r--.


def source_path_excluded(relative: Path, *, directory: bool = False) -> bool:
    """Return whether a repository-relative path stays out of the source archive.

    The policy does not depend on Git being installed or initialised.
    source_files() additionally skips links and special files instead of
    following or reading them. The safe example names are intentional
    exceptions, not a content audit.
    """
    parts = tuple(part.casefold() for part in relative.parts)
    if parts[:2] == ('native', 'target'):  # Rust build output.
        return True
    if any(part in OMITTED_DIRECTORIES for part in parts):
        return True
    if parts[:4] == ('apps', 'web', 'public', 'downloads'):
        return True
    if '/'.join(parts) in GENERATED_SOURCE_PATHS:
        return True
    if not directory and relative.name in SAFE_EXAMPLE_NAMES:
        return False
    name = relative.name.casefold()
    return any(fnmatchcase(name, pattern) for pattern in OMITTED_NAMES)


def sha256_hex(path: Path) -> str:
    """Return the SHA-256 digest of a file as hexadecimal text."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def raise_walk_error(error: OSError) -> NoReturn:
    """Fail on a directory that cannot be read.

    Skipping it would silently produce incomplete corresponding source.
    """
    raise error


def is_source_directory(path: Path, root: Path) -> bool:
    """Return whether source_files() descends into this directory; links are never followed."""
    excluded = source_path_excluded(path.relative_to(root), directory=True)
    return not path.is_symlink() and not excluded


def is_source_file(path: Path, root: Path) -> bool:
    """Return whether this file is an editable input; links and special files are not."""
    excluded = source_path_excluded(path.relative_to(root))
    return not excluded and not path.is_symlink() and path.is_file()


def source_files(root: Path = ROOT) -> list[tuple[Path, Path]]:
    """Return every editable input under root as (path, relative path), in archive order.

    Excluded directories are pruned before they are read, and an unreadable
    directory raises its error instead of being skipped.
    """
    root = Path(root).resolve()
    files = []
    walk = os.walk(root, topdown=True, onerror=raise_walk_error, followlinks=False)
    for directory, subdirectories, names in walk:
        current = Path(directory)
        # Replacing the list in place is how os.walk() is told what to enter.
        subdirectories[:] = sorted(name for name in subdirectories
                                   if is_source_directory(current / name, root))
        for name in names:
            path = current / name
            if is_source_file(path, root):
                files.append((path, path.relative_to(root)))
    return sorted(files, key=lambda item: item[1].as_posix())


def release_versions() -> tuple[str, str]:
    """Return the app version and the Debian package version from desktop/core.py.

    core.py imports modules next to it, so desktop/ goes on the search path.
    """
    sys.path.insert(0, str(DESKTOP))
    core = importlib.import_module('core')
    version: str = core.VERSION
    debian_version: str = core.DEBIAN_VERSION
    return version, debian_version


def run_python(script: Path, *arguments: str) -> None:
    """Run a repository script with this interpreter and raise if it fails."""
    subprocess.run([sys.executable, str(script), *arguments], check=True)


def remove_website_downloads() -> None:
    """Delete the website download folders; releases are published on GitHub."""
    for directory in WEBSITE_DOWNLOADS:
        if directory.is_dir():
            shutil.rmtree(directory)


def is_previous_artifact(path: Path) -> bool:
    """Return whether path is a package, archive or checksum list of an earlier build."""
    return path.is_file() and (path.suffix in ('.zip', '.deb') or path.name == 'SHA256SUMS')


def prepare_output_directories() -> None:
    """Create the output directories and empty them of earlier artifacts."""
    for directory in (DIST, DESKTOP_DIST):
        directory.mkdir(parents=True, exist_ok=True)
        for path in directory.iterdir():
            if is_previous_artifact(path):
                path.unlink()
    TEST_RESULTS.mkdir(exist_ok=True)  # For the package verification report.
    DESIGNS.mkdir(exist_ok=True)  # For a copy of the preview.


def build_and_verify_package(debian_version: str) -> Path:
    """Build the Debian package into dist/, verify it and return its path."""
    package = DIST / f'openxplorer_{debian_version}_all.deb'
    report = TEST_RESULTS / 'package-verification.json'
    run_python(DESKTOP / 'tools/build_deb.py', '--output', str(package))
    run_python(DESKTOP / 'tools/verify_deb.py', str(package), '--json', str(report))
    return package


def write_source_archive(archive: Path, root: Path = ROOT) -> None:
    """Write every editable input under root to a reproducible ZIP archive.

    Entries use zlib's default compression level: an entry written from a
    ZipInfo ignores the archive's compresslevel.
    """
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as bundle:
        for path, relative in source_files(root):
            entry = zipfile.ZipInfo(ARCHIVE_PREFIX + relative.as_posix(), ARCHIVE_TIMESTAMP)
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = ARCHIVE_FILE_MODE << 16
            bundle.writestr(entry, path.read_bytes())


def write_checksums(artifacts: Iterable[Path], destination: Path) -> None:
    """Write the SHA-256 of each artifact in the format `sha256sum --check` reads."""
    lines = [f'{sha256_hex(path)}  {path.name}\n' for path in artifacts]
    destination.write_text(''.join(lines))


def publish_preview() -> None:
    """Rebuild the offline preview and copy it to the website and the design review."""
    run_python(DESKTOP / 'tools/build_preview.py')
    for destination in PREVIEW_COPIES:
        shutil.copyfile(DESKTOP / 'preview.html', destination)


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    """Accept no options but --help, so a mistyped option cannot start a release build."""
    parser = argparse.ArgumentParser(
        description='Build and verify the Debian package, the corresponding-source archive '
                    'and SHA256SUMS in dist/, and refresh the offline preview copies.')
    return parser.parse_args(argv)


def main() -> None:
    """Build the installer, the corresponding source and their checksums."""
    parse_arguments()
    version, debian_version = release_versions()
    remove_website_downloads()
    prepare_output_directories()
    package = build_and_verify_package(debian_version)
    source = DIST / f'openxplorer-{version}-source.zip'
    write_source_archive(source)
    write_checksums((package, source), DIST / 'SHA256SUMS')
    shutil.copyfile(package, DESKTOP_DIST / package.name)
    publish_preview()
    print('Built local installer, corresponding source and checksums. '
          'Website links remain on GitHub.')


if __name__ == '__main__':
    main()
