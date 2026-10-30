#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Check openxplorer-admin-delete against a filesystem a user supplied.

The unit tests of the permanent-delete helper run as an ordinary user,
so they can neither create a root-owned symbolic link nor mount
anything. This check covers the one case they cannot: an ext4 image
whose folders and links all claim uid 0 and mode 0755, with a link
pointing at a bystander outside it, mounted the way a desktop mounts a
USB stick. The helper must refuse to follow that link, and must still
delete an ordinary folder on the image.

It needs root, a loop device and mkfs.ext4, so it is not part of
native/tools/check.py. Run it by hand, or in a disposable machine:

    OX_ADMIN_DELETE_MOUNT_CHECK=1 sudo -E python3 \\
        native/tools/check_admin_delete_mounts.py \\
        --helper native/target/debug/openxplorer-admin-delete

Everything it makes is inside one temporary folder and one loop mount,
and both are removed again however the check ends. It never touches a
path outside them.

The temporary folder is deliberately *not* under /tmp. The helper also
refuses to follow a link whose folders anyone can change, and /tmp is
world-writable, so a check run there would pass whether or not the rule
about mounts works at all. It is placed under a root-owned folder on
the root filesystem instead, so the only rule that can refuse the
image's link is the one this check is about.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

# Set this to say the machine is disposable. Without it the check does
# nothing, so a stray run on a workstation cannot mount anything.
CONSENT_VARIABLE = 'OX_ADMIN_DELETE_MOUNT_CHECK'
# The helper's exit status for a request it refused outright.
REFUSED = 2
IMAGE_MEGABYTES = 16
# What the helper must say, from walk.rs's other_mount_refusal, with
# {path} filled in. Matching the whole line is the point of this check:
# the helper has several refusals, and one of the others firing instead
# would mean the rule about mounts was never what stopped it. The
# helper is run with LANGUAGE and LC_ALL cleared to C so the message
# stays the English it was written in; a translated run would not match
# and is not what this check is for.
OTHER_MOUNT_REFUSAL = (
    '{path} is a shortcut (a symbolic link) on a drive, share or folder other than this '
    'computer\u2019s own system drive, so what it says about itself cannot be relied on. '
    'An administrator deletion does not follow one. Open the folder it leads to and delete '
    'the item there. Deleting items on that drive still works.'
)
# Where the workspace may go: a root-owned folder on the root
# filesystem that nobody else can write to, so no other rule of the
# helper's can be what refuses the image's link.
WORKSPACE_FOLDERS = ('/srv', '/opt', '/usr/local', '/var/lib', '/root')


class CheckError(Exception):
    """The check could not run, or the helper did the wrong thing."""


def run_helper(helper: Path, target: Path) -> subprocess.CompletedProcess[str]:
    """Run the helper on target with no standard input, and return it.

    The language is forced to C, so the refusal this check compares
    against is the English the message was written in.
    """
    environment = dict(os.environ, LANGUAGE='', LC_ALL='C', LC_MESSAGES='C', LANG='C')
    return subprocess.run([str(helper), '--', str(target)], capture_output=True, text=True,
                          stdin=subprocess.DEVNULL, check=False, env=environment)


def build_image(image: Path) -> None:
    """Write an empty ext4 image of IMAGE_MEGABYTES."""
    with image.open('wb') as file:
        file.truncate(IMAGE_MEGABYTES * 1024 * 1024)
    subprocess.run(['mkfs.ext4', '-q', '-F', str(image)], check=True)


def fill_image(mount_point: Path, bystander: Path) -> None:
    """Make the tree a user would author: everything claims root."""
    folder = mount_point / 'folder'
    folder.mkdir()
    os.chown(folder, 0, 0)
    folder.chmod(0o755)
    link = folder / 'link'
    link.symlink_to(bystander)
    os.chown(link, 0, 0, follow_symlinks=False)
    ordinary = folder / 'ordinary'
    ordinary.mkdir()
    (ordinary / 'item.txt').write_text('on the stick\n', encoding='utf-8')


def check_refuses_the_link(helper: Path, mount_point: Path, bystander: Path) -> str:
    """Check that the helper will not follow the image's link."""
    inside = bystander / 'important.txt'
    result = run_helper(helper, mount_point / 'folder/link/important.txt')
    if result.returncode != REFUSED:
        raise CheckError(
            f'the helper did not refuse the link: exit {result.returncode}\n{result.stdout}')
    if not result.stdout.startswith('refused\t'):
        raise CheckError(f'the helper refused without saying so:\n{result.stdout}')
    link = mount_point / 'folder/link'
    expected = 'refused\t' + OTHER_MOUNT_REFUSAL.format(path=link)
    if result.stdout.strip() != expected:
        raise CheckError(
            'the helper refused, but not with the refusal about mounts, so some other rule '
            f'is what stopped it:\n  said:     {result.stdout.strip()}\n'
            f'  expected: {expected}')
    if not inside.is_file():
        raise CheckError(f'{inside} was deleted through the link the image supplied')
    return result.stdout.strip()


def check_deletes_on_the_image(helper: Path, mount_point: Path) -> None:
    """Check that an ordinary folder on the image is still deleted."""
    ordinary = mount_point / 'folder/ordinary'
    result = run_helper(helper, ordinary)
    if result.returncode != 0 or ordinary.exists():
        raise CheckError(
            f'deleting an ordinary folder on the image failed: exit {result.returncode}\n'
            f'{result.stdout}{result.stderr}')


def check(helper: Path) -> str:
    """Run both checks and return what the refusal said."""
    with tempfile.TemporaryDirectory(prefix='openxplorer-mount-check-',
                                     dir=workspace_folder()) as temporary:
        workspace = Path(temporary)
        workspace.chmod(0o755)
        bystander = workspace / 'bystander'
        bystander.mkdir()
        bystander.chmod(0o755)
        (bystander / 'important.txt').write_text('do not delete\n', encoding='utf-8')
        image = workspace / 'stick.img'
        build_image(image)
        mount_point = workspace / 'run-media-demo-STICK'
        mount_point.mkdir()
        mount_point.chmod(0o755)
        subprocess.run(['mount', '-o', 'loop,nosuid,nodev', str(image), str(mount_point)],
                       check=True)
        try:
            fill_image(mount_point, bystander)
            refusal = check_refuses_the_link(helper, mount_point, bystander)
            check_deletes_on_the_image(helper, mount_point)
            return refusal
        finally:
            subprocess.run(['umount', str(mount_point)], check=False)


def workspace_folder() -> Path:
    """Return a folder to build the workspace in, or raise CheckError.

    It has to be on the same filesystem as / and writable by root
    alone, because the helper refuses a link under a folder others can
    change and that refusal would hide the one being checked.
    """
    root_device = Path('/').stat().st_dev
    for name in WORKSPACE_FOLDERS:
        folder = Path(name)
        try:
            status = folder.stat()
        except OSError:
            continue
        if (status.st_dev == root_device and status.st_uid == 0
                and not status.st_mode & 0o022):
            return folder
    raise CheckError(
        'no folder among ' + ', '.join(WORKSPACE_FOLDERS) + ' is on the root filesystem and '
        'writable by root alone, so this check cannot tell the rule about mounts apart from '
        'the rule about folders others can change')


def missing_tools() -> list[str]:
    """Return the programs this check needs that are not on PATH."""
    return [tool for tool in ('mkfs.ext4', 'mount', 'umount') if shutil.which(tool) is None]


def main(argv: list[str] | None = None) -> int:
    """Run the check and return the exit status."""
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--helper', type=Path, required=True,
                        help='the openxplorer-admin-delete executable to check')
    arguments = parser.parse_args(argv)
    if os.environ.get(CONSENT_VARIABLE) != '1':
        print(f'Set {CONSENT_VARIABLE}=1 to run this; it mounts a loop device and needs root.',
              file=sys.stderr)
        return 0
    if os.geteuid() != 0:
        print('This check needs root: it mounts a filesystem image.', file=sys.stderr)
        return 2
    missing = missing_tools()
    if missing:
        print(f'This check needs {", ".join(missing)}.', file=sys.stderr)
        return 2
    try:
        refusal = check(arguments.helper)
    except (CheckError, OSError, subprocess.CalledProcessError) as error:
        print(f'The supplied-filesystem check failed: {error}', file=sys.stderr)
        return 1
    print('The helper refuses to follow a link on a filesystem a user supplied:')
    print(f'  {refusal}')
    print('The helper still deletes an ordinary folder on that filesystem.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
