#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Write the Flatpak's offline Cargo sources from native/Cargo.lock.

flatpak-builder builds without network access, so the manifest lists every
crate of Cargo.lock as a download with its checksum, unpacked into a vendor
folder that a Cargo configuration puts in place of crates.io. This is the
registry part of flatpak-cargo-generator (github.com/flatpak/flatpak-builder-tools)
using only the standard library; the lock file has no Git dependencies.

Run it after every Cargo.lock change; test_flatpak_cargo_sources.py fails
while native/packaging/flatpak/cargo-sources.json is out of date.
"""
from __future__ import annotations

import argparse
from collections.abc import Iterator
from dataclasses import dataclass
import json
from pathlib import Path
import sys
import tomllib
from typing import Any

NATIVE = Path(__file__).resolve().parents[1]
CARGO_LOCK = NATIVE / 'Cargo.lock'
SOURCES_FILE = NATIVE / 'packaging' / 'flatpak' / 'cargo-sources.json'

CRATES_IO = 'registry+https://github.com/rust-lang/crates.io-index'
DOWNLOAD_URL = 'https://static.crates.io/crates/{name}/{name}-{version}.crate'

# Relative to the build folder of the Flatpak module, which the manifest makes
# CARGO_HOME's parent: CARGO_HOME is <build folder>/cargo.
VENDOR_FOLDER = 'cargo/vendor'
CARGO_HOME_FOLDER = 'cargo'

# Cargo resolves a relative directory in $CARGO_HOME/config.toml from the
# folder above CARGO_HOME, which is the module's build folder.
CARGO_CONFIG = (
    '[source.vendored-sources]\n'
    f'directory = "{VENDOR_FOLDER}"\n'
    '\n'
    '[source.crates-io]\n'
    'replace-with = "vendored-sources"\n'
)


class LockFileError(Exception):
    """Cargo.lock holds a package this tool cannot vendor; the message says which."""


@dataclass(frozen=True)
class LockedCrate:
    """A crates.io package pinned in Cargo.lock."""

    name: str
    version: str
    checksum: str

    @property
    def vendor_folder(self) -> str:
        """Where the crate is unpacked, relative to the module's build folder."""
        return f'{VENDOR_FOLDER}/{self.name}-{self.version}'


def locked_crates(lock_text: str) -> list[LockedCrate]:
    """Return the crates.io packages of a Cargo.lock, in the lock file's order.

    Workspace members have no source and are skipped. Raises LockFileError for
    a Git or other non-crates.io source, or a crate without a checksum.
    """
    crates = []
    for package in tomllib.loads(lock_text).get('package', []):
        source = package.get('source')
        if source is None:
            continue
        crates.append(locked_crate(package, source))
    return crates


def locked_crate(package: dict[str, Any], source: str) -> LockedCrate:
    """Return the crate a [[package]] table of Cargo.lock pins."""
    name = package['name']
    if source != CRATES_IO:
        raise LockFileError(f'{name} comes from {source}; only crates.io packages are vendored.')
    checksum = package.get('checksum')
    if checksum is None:
        raise LockFileError(f'{name} has no checksum in Cargo.lock.')
    return LockedCrate(name=name, version=package['version'], checksum=checksum)


def crate_sources(crate: LockedCrate) -> Iterator[dict[str, str]]:
    """Yield the two flatpak-builder sources of one crate.

    The download is checked against Cargo.lock's SHA-256. Cargo requires a
    .cargo-checksum.json in each vendored crate; an empty "files" map makes it
    trust the unpacked files, which the archive checksum already covers.
    """
    yield {
        'type': 'archive',
        'archive-type': 'tar-gzip',
        'url': DOWNLOAD_URL.format(name=crate.name, version=crate.version),
        'sha256': crate.checksum,
        'dest': crate.vendor_folder,
    }
    checksum_file = json.dumps({'package': crate.checksum, 'files': {}})
    yield {
        'type': 'inline',
        'contents': checksum_file,
        'dest': crate.vendor_folder,
        'dest-filename': '.cargo-checksum.json',
    }


def flatpak_sources(crates: list[LockedCrate]) -> list[dict[str, str]]:
    """Return every source the manifest needs: each crate, then Cargo's configuration."""
    sources = [source for crate in crates for source in crate_sources(crate)]
    sources.append({
        'type': 'inline',
        'contents': CARGO_CONFIG,
        'dest': CARGO_HOME_FOLDER,
        'dest-filename': 'config.toml',
    })
    return sources


def sources_text(lock_text: str) -> str:
    """Return cargo-sources.json for a Cargo.lock, as it is committed."""
    sources = flatpak_sources(locked_crates(lock_text))
    return json.dumps(sources, indent=4) + '\n'


def main(argv: list[str] | None = None) -> int:
    """Write the sources file and return the exit status."""
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--output', type=Path, default=SOURCES_FILE,
                        help='where to write the sources (default: '
                             'native/packaging/flatpak/cargo-sources.json)')
    arguments = parser.parse_args(argv)
    try:
        text = sources_text(CARGO_LOCK.read_text(encoding='utf-8'))
        arguments.output.write_text(text, encoding='utf-8')
    except (LockFileError, OSError, tomllib.TOMLDecodeError) as error:
        print(f'Writing the Flatpak Cargo sources failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
