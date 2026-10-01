# SPDX-License-Identifier: AGPL-3.0-only
"""The retired Python app's sources, for the tests that prove compatibility with it.

OpenXplorer 1.x was a Python/WebKit app in desktop/; its last release is tag
v1.1.4, and tag v2.0.0 holds its final sources (1.1.4 with the fixes that were
never released). The directory left the tree after 2.0.0, but users upgrade
from it, so some tests still run its code: the settings, session and keyring
interoperability tests and the location fixtures run its modules, and
updater_compatibility.py runs the 1.1.x updater against a built package.

Those tests read the sources from the directory named by $OX_PYTHON_APP.
check.py extracts desktop/ of tag v2.0.0 into a temporary directory for its
run and sets the variable; the tag is fetched from origin when the checkout
lacks it (a shallow CI clone). Run a single test the same way:

    OX_PYTHON_APP=$(python3 native/tools/python_app.py) cargo test ...

which prints the path of a copy under the system's temporary directory.
"""
from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

REPOSITORY = Path(__file__).resolve().parents[2]
# The tag that holds the Python app's final sources.
TAG = 'v2.0.0'
# The variable naming the extracted desktop/ directory.
VARIABLE = 'OX_PYTHON_APP'


class PythonAppMissing(Exception):
    """The Python app's sources could not be found or fetched; the message says how to fix it."""


def git(*arguments: str) -> subprocess.CompletedProcess[bytes]:
    """Run git in the repository and return its result, without raising."""
    return subprocess.run(['git', '-C', str(REPOSITORY), *arguments], capture_output=True,
                          check=False)


def ensure_tag() -> None:
    """Fetch the tag from origin unless the repository already has it."""
    if git('rev-parse', '--verify', '--quiet', f'{TAG}^{{commit}}').returncode == 0:
        return
    fetched = git('fetch', '--no-tags', '--depth=1', 'origin', f'+refs/tags/{TAG}:refs/tags/{TAG}')
    if fetched.returncode != 0:
        reason = fetched.stderr.decode(errors='replace').strip()
        raise PythonAppMissing(
            f'The compatibility tests need tag {TAG}, which holds the retired Python app. '
            f'Fetch it with "git fetch origin tag {TAG}": {reason}')


def extract(destination: Path) -> Path:
    """Write desktop/ of the tag into destination and return the desktop directory."""
    ensure_tag()
    archive = git('archive', '--format=tar', TAG, 'desktop')
    if archive.returncode != 0:
        raise PythonAppMissing(f'git archive {TAG} desktop failed: '
                               + archive.stderr.decode(errors='replace').strip())
    with tarfile.open(fileobj=io.BytesIO(archive.stdout)) as tar:
        tar.extractall(destination, filter='data')
    return destination / 'desktop'


@contextmanager
def sources() -> Iterator[Path]:
    """Yield the Python app's desktop/ directory: $OX_PYTHON_APP, or a temporary copy."""
    named = os.environ.get(VARIABLE)
    if named:
        yield Path(named)
        return
    with tempfile.TemporaryDirectory(prefix='openxplorer-python-app-') as temporary:
        yield extract(Path(temporary))


def main() -> int:
    """Extract the sources under the temporary directory and print where they are."""
    try:
        directory = extract(Path(tempfile.mkdtemp(prefix='openxplorer-python-app-')))
    except PythonAppMissing as error:
        print(error, file=sys.stderr)
        return 1
    print(directory)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
