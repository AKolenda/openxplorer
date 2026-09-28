#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Write the source archives the RPM and Arch package builds start from.

openxplorer-<version>.tar.gz holds the committed files a package build needs
(native/, the Python mount helper in desktop/, and the licences), under the
folder openxplorer-<version>/. With --vendor, openxplorer-<version>-vendor.tar.gz
holds every crate of Cargo.lock under vendor/, which the RPM spec unpacks inside
the source folder, so an RPM build, which may not use the network, compiles
offline. Both archives are reproducible: sorted entries, root ownership and the
last commit's time.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import gzip
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

from build_deb import BuildError, cargo_version, source_date_epoch
from package_data import CARGO_MANIFEST, REPOSITORY

OUTPUT_DIRECTORY = REPOSITORY / 'dist' / 'native'
# The committed paths a package build reads.
SOURCE_PATHS = ('native', 'desktop', 'licenses', 'LICENSE', 'THIRD_PARTY_NOTICES.md')
# The top folder of the vendored-crate archive, which .cargo/config.toml in
# the RPM spec names as the replacement for crates.io.
VENDOR_FOLDER = 'vendor'


@dataclass(frozen=True)
class ArchiveRequest:
    """Which version to archive, at which time, and where to write it."""

    version: str
    epoch: int
    output_directory: Path

    @property
    def top_folder(self) -> str:
        """The folder every entry is in: openxplorer-<version>."""
        return f'openxplorer-{self.version}'


def write_source_archive(request: ArchiveRequest) -> Path:
    """Write openxplorer-<version>.tar.gz from the last commit and return its path."""
    output = request.output_directory / f'{request.top_folder}.tar.gz'
    command = ['git', '-C', str(REPOSITORY), 'archive', '--format=tar.gz',
               f'--prefix={request.top_folder}/', f'--output={output}',
               f'--mtime={request.epoch}', 'HEAD', *SOURCE_PATHS]
    subprocess.run(command, check=True)
    return output


def write_vendor_archive(request: ArchiveRequest) -> Path:
    """Download every locked crate with cargo vendor and archive them; return the path."""
    output = request.output_directory / f'{request.top_folder}-vendor.tar.gz'
    with tempfile.TemporaryDirectory(prefix='openxplorer-vendor-') as temporary:
        vendor = Path(temporary) / 'vendor'
        command = ['cargo', 'vendor', '--locked', '--versioned-dirs',
                   '--manifest-path', str(CARGO_MANIFEST), str(vendor)]
        subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
        write_reproducible_tar(vendor, VENDOR_FOLDER, output, request.epoch)
    return output


def write_reproducible_tar(folder: Path, prefix: str, output: Path, epoch: int) -> None:
    """Archive folder under prefix with sorted, root-owned entries dated epoch."""
    def normalise(member: tarfile.TarInfo) -> tarfile.TarInfo:
        member.uid = member.gid = 0
        member.uname = member.gname = 'root'
        member.mtime = epoch
        return member

    paths = sorted(folder.rglob('*'))
    # An empty file name keeps the output's name out of the gzip header.
    with output.open('wb') as file, \
            gzip.GzipFile(filename='', mode='wb', fileobj=file, mtime=epoch) as compressed, \
            tarfile.open(fileobj=compressed, mode='w', format=tarfile.PAX_FORMAT) as archive:
        for path in paths:
            name = f'{prefix}/{path.relative_to(folder).as_posix()}'
            archive.add(path, arcname=name, recursive=False, filter=normalise)


def parse_arguments(argv: list[str] | None) -> argparse.Namespace:
    """Read the command-line options."""
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--vendor', action='store_true',
                        help='also write the archive of vendored crates (needs the network)')
    parser.add_argument('--output-directory', type=Path, default=OUTPUT_DIRECTORY,
                        help='where to write the archives (default: dist/native)')
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    """Write the archives, print their paths and return the exit status."""
    arguments = parse_arguments(argv)
    arguments.output_directory.mkdir(parents=True, exist_ok=True)
    try:
        request = ArchiveRequest(cargo_version(), source_date_epoch(),
                                 arguments.output_directory.resolve())
        print(write_source_archive(request))
        if arguments.vendor:
            print(write_vendor_archive(request))
    except (BuildError, OSError, subprocess.CalledProcessError) as error:
        print(f'Writing the source archives failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
