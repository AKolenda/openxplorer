# SPDX-License-Identifier: AGPL-3.0-only
"""Source-archive privacy and completeness, checked on synthetic temporary trees.

tools/release.py chooses the files of the corresponding-source archive. These
tests build throwaway repository trees and check which files source_files()
selects, that it never follows links or reads special files, and that its
policy agrees with .gitignore. They also check which CI-built packages a
release stages and checksums, and that a failed release ends in a one-line
message. Run them from the repository root:

    python3 -m unittest discover -s tests -p 'test_release_source.py'
"""
from __future__ import annotations

from collections.abc import Iterator
import contextlib
import io
import os
from pathlib import Path
import subprocess
import tempfile
from typing import Any
import unittest
from unittest.mock import patch

from tools import release
from tools.release import source_files

REPOSITORY = Path(__file__).resolve().parents[1]

# Git reads only the fixture's .gitignore: no system or global configuration
# and no personal excludes file, which Git reads from ~/.config/git/ignore even
# when no configuration names it. A personal rule such as Cargo.lock would
# otherwise fail the agreement test.
GIT_ENVIRONMENT = {
    **os.environ,
    'GIT_CONFIG_NOSYSTEM': '1',
    'GIT_CONFIG_GLOBAL': os.devnull,
    'GIT_CONFIG_COUNT': '1',
    'GIT_CONFIG_KEY_0': 'core.excludesFile',
    'GIT_CONFIG_VALUE_0': os.devnull,
}

# Editable inputs the archive must keep, including build and packaging files.
EDITABLE_INPUTS = [
    '.github/workflows/checks.yml', '.gitignore', '.env.example', '.dev.vars.example',
    'AGENTS.md', 'LICENSE', 'NOTICE', 'THIRD_PARTY_NOTICES.md', 'pnpm-lock.yaml',
    'pnpm-workspace.yaml', 'package.json', 'requirements-dev.txt', 'wrangler.jsonc',
    'apps/web/public/tour/tour.js', 'apps/web/public/tour/scenes.json',
    'licenses/Winspace-MIT.txt', 'tools/release.py', 'tests/test_release_source.py',
    'apps/web/components/product.tsx', 'apps/web/public/assets/site.css',
    'apps/web/public/assets/screenshots/manifest.json', 'docs/PRIVACY.md',
    'apps/web/.env.example', 'apps/web/.dev.vars.example',
    # The native Rust workspace.
    'native/Cargo.toml', 'native/Cargo.lock', 'native/rustfmt.toml',
    'native/crates/ox-app/src/main.rs', 'native/crates/ox-app/resources/style.css',
    'native/crates/ox-core/tests/entry_enumeration.rs',
]

# Files inside dependency, cache, build, test-output and generated trees.
FILES_IN_EXCLUDED_TREES = [
    'node_modules/package/source.js', 'apps/web/node_modules/package/source.js',
    '.next/source.js', 'apps/web/out/index.html', '.git/config', '.hg/store/file',
    '.svn/entries', '__pycache__/module.pyc', 'test-results/result.json',
    'native/test-results/screenshot.png', 'dist/source.zip', 'native/dist/package.deb',
    'designs/index.html', '.pnpm-store/index.json', '.wrangler/state/v3/db.sqlite',
    'apps/web/.wrangler/config/default.toml', '.vercel/project.json', '.venv/bin/python',
    'tools/venv/bin/python', '.pytest_cache/state', '.mypy_cache/state',
    '.ruff_cache/state', '.cache/state', '.tox/state', '.nox/state', '.hypothesis/state',
    '.nyc_output/result.json', 'htmlcov/index.html', 'coverage/index.html',
    'playwright-report/index.html', 'blob-report/result.json', 'tmp/note.txt',
    '.tmp/note.txt', 'temp/note.txt', '.temp/note.txt',
    'apps/web/public/downloads/SHA256SUMS', 'apps/web/public/assets/site.js',
    'apps/web/public/tour/scenes.js',
    # Rust build output.
    'native/target/debug/openxplorer-native', 'native/target/debug/deps/object.o',
    'native/target/.rustc_info.json',
]

# Secrets, local databases and logs, private-review identifiers, built packages
# and editor backups, excluded wherever they are.
EXCLUDED_FILE_NAMES = [
    '.env', '.env.local', '.env.production', '.env.example.local',
    '.dev.vars', '.dev.vars.preview', '.dev.vars.example.secret',
    '.private-demo-terms', '.private-demo-terms.json', 'private-terms.json',
    '.DS_Store', '.coverage', '.coverage.worker', 'module.pyc', 'module.pyo',
    'module.pyd', 'web.tsbuildinfo', 'package.deb', 'source.zip',
    'server.log', 'server.log.1', 'local.sqlite', 'local.sqlite-wal',
    'local.sqlite3', 'local.sqlite3-shm', 'local.db', 'local.db-journal',
    'private.pem', 'private.key', 'private.p12', 'private.pfx', 'account.credentials',
    'credentials.json', 'credentials.toml', 'credentials.yaml', 'credentials.yml',
    'client_secret_sample.json', 'service-account-sample.json',
    'source.swp', 'source.swo', 'source.tmp', 'source.temp', 'source.py~',
]

# Whether each path must be ignored by Git and, equally, left out of the archive.
GIT_IGNORED = {
    '.wrangler/state/v3/local.sqlite': True, '.dev.vars': True, '.env.local': True,
    '.env.example': False, '.dev.vars.example': False, '.cache/private.txt': True,
    '.venv/pyvenv.cfg': True, 'nested/temp/private.txt': True, 'trace.log.1': True,
    'credentials.json': True, 'local.db-wal': True, 'private.pem': True,
    'native/test-results/native.json': True, 'apps/web/public/tour/scenes.js': True,
    'native/dist/SHA256SUMS': True, 'apps/web/out/index.html': True,
    'apps/web/public/downloads/SHA256SUMS': True, 'pnpm-lock.yaml': False,
    'wrangler.jsonc': False, 'apps/web/public/tour/tour.js': False,
    'licenses/Winspace-MIT.txt': False, '.github/workflows/checks.yml': False,
    # The native Rust workspace.
    'native/target/debug/openxplorer-native': True,
    'native/Cargo.toml': False, 'native/Cargo.lock': False,
}


class SourceArchiveTests(unittest.TestCase):
    """source_files() selects exactly the editable inputs of a repository tree."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-source-test-')
        self.addCleanup(temporary.cleanup)
        # source_files() resolves its root, so the fixture root must be resolved
        # too; otherwise a TMPDIR reached through a symlink breaks relative paths.
        self.root = Path(temporary.name).resolve()

    def put(self, relative: str, content: str = 'synthetic fixture\n') -> Path:
        """Create a file in the fixture tree, with its parent directories."""
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def selected(self) -> list[str]:
        """Return the fixture files source_files() selects, as relative POSIX paths."""
        return [relative.as_posix() for _, relative in source_files(self.root)]

    def git_ignores(self, relative: str) -> bool:
        """Return whether Git, with the fixture's .gitignore, ignores the path."""
        command = ['git', '-C', str(self.root), 'check-ignore', '--quiet', relative]
        return subprocess.run(command, check=False, env=GIT_ENVIRONMENT).returncode == 0

    def test_preserves_editable_build_inputs_not_just_runtime_code(self) -> None:
        """Build, packaging, licence and native files are kept, in sorted order."""
        for path in reversed(EDITABLE_INPUTS):
            self.put(path)
        self.assertEqual(self.selected(), sorted(EDITABLE_INPUTS))

    def test_excludes_local_state_and_generated_trees_at_any_depth(self) -> None:
        """Nothing inside an excluded tree is kept, however deep the tree is."""
        for path in FILES_IN_EXCLUDED_TREES:
            self.put(path)
        self.put('native/crates/ox-app/src/main.rs')
        self.assertEqual(self.selected(), ['native/crates/ox-app/src/main.rs'])

    def test_excludes_environment_credentials_databases_logs_and_editor_backups(self) -> None:
        """Sensitive and local-only file names are excluded in any directory."""
        for name in EXCLUDED_FILE_NAMES:
            self.put('nested/' + name)
        self.assertEqual(self.selected(), [])

    def test_sensitive_filename_checks_are_case_insensitive_but_examples_are_exact(self) -> None:
        """Upper-case secrets are excluded; only exactly named examples are kept."""
        upper_case_names = ('PRIVATE.KEY', 'credentials.JSON', 'TRACE.LOG', '.ENV',
                            '.ENV.EXAMPLE', '.DEV.VARS.EXAMPLE')
        for name in upper_case_names:
            self.put(name)
        self.put('.env.example')
        self.put('.dev.vars.example')
        self.assertEqual(self.selected(), ['.dev.vars.example', '.env.example'])

    def test_excluded_directories_are_never_scanned(self) -> None:
        """Excluded trees are pruned before they are read, not filtered afterwards."""
        self.put('node_modules/package/deep/source.js')
        self.put('.wrangler/state/v3/source.py')
        self.put('apps/web/public/downloads/deep/source.py')
        self.put('native/crates/ox-app/src/main.rs')
        visited = []
        real_scandir = os.scandir

        def scandir(path: str) -> Iterator[os.DirEntry[str]]:
            relative = Path(path).relative_to(self.root)
            visited.append(relative.as_posix())
            self.assertNotIn('node_modules', relative.parts)
            self.assertNotIn('.wrangler', relative.parts)
            self.assertNotIn('downloads', relative.parts)
            return real_scandir(path)

        # os.walk() lists every directory it enters with os.scandir().
        with patch.object(os, 'scandir', side_effect=scandir):
            self.assertEqual(self.selected(), ['native/crates/ox-app/src/main.rs'])
        self.assertIn('native/crates/ox-app/src', visited)

    def test_links_cannot_import_external_private_files_or_cycle(self) -> None:
        """Links to files or folders, inside or outside the tree, are never followed."""
        with tempfile.TemporaryDirectory(prefix='openxplorer-private-test-') as private:
            external = Path(private)
            (external / 'private.txt').write_text('synthetic private content')
            (self.root / 'linked-file.txt').symlink_to(external / 'private.txt')
            (self.root / 'linked-directory').symlink_to(external, target_is_directory=True)
            (self.root / 'cycle').symlink_to(self.root, target_is_directory=True)
            self.put('source.py')
            (self.root / 'internal-link.py').symlink_to(self.root / 'source.py')
            self.assertEqual(self.selected(), ['source.py'])

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'FIFO check requires a POSIX filesystem')
    def test_special_files_are_not_read_as_source(self) -> None:
        """A FIFO would block the archive if it were read; it is skipped."""
        os.mkfifo(self.root / 'pipe.txt')
        self.put('source.py')
        self.assertEqual(self.selected(), ['source.py'])

    def test_unreadable_source_tree_fails_instead_of_silently_omitting_inputs(self) -> None:
        """A directory that cannot be read fails the archive instead of being left out."""
        def inaccessible(root: Path, **options: Any) -> Iterator[tuple[str, list[str], list[str]]]:
            options['onerror'](PermissionError('Synthetic inaccessible source directory'))
            return iter(())

        with (patch.object(os, 'walk', side_effect=inaccessible),
              self.assertRaises(PermissionError)):
            self.selected()

    def test_gitignore_and_source_policy_agree_for_publication_paths(self) -> None:
        """Git ignores a path exactly when the archive leaves it out."""
        (self.root / '.gitignore').write_bytes((REPOSITORY / '.gitignore').read_bytes())
        subprocess.run(['git', 'init', '--quiet', str(self.root)], check=True,
                       env=GIT_ENVIRONMENT)
        for name, expected in GIT_IGNORED.items():
            self.put(name)
            with self.subTest(path=name):
                self.assertEqual(self.git_ignores(name), expected)
                self.assertEqual(name not in self.selected(), expected)


class ReleasePackageTests(unittest.TestCase):
    """--packages stages every stable package of the release and checksums it.

    The CI build's folder is simulated; building and verifying the Debian
    package, the source archive and the preview are replaced, and dist/ is a
    temporary folder, so nothing in the repository changes.
    """

    VERSION = '2.0.0'
    STABLE_PACKAGES = [
        'openxplorer_2.0.0_all.deb',
        'io.winspace.Development.flatpak',
        'openxplorer-2.0.0-1-x86_64.pkg.tar.zst',
        'openxplorer-2.0.0-1.fc44.x86_64.rpm',
        'openxplorer-2.0.0-1.opensuse_tumbleweed.x86_64.rpm',
    ]
    OTHER_FILES = [
        # The preview's packages and bundle.
        'openxplorer-native_2.0.0_amd64.deb',
        'openxplorer-native-2.0.0-1.fc44.x86_64.rpm',
        'openxplorer-native-2.0.0-1-x86_64.pkg.tar.zst',
        'io.winspace.Development.Native.flatpak',
        # Packages of another version.
        'openxplorer-1.9.0-1.fc44.x86_64.rpm',
        'openxplorer-2.0.1-1-x86_64.pkg.tar.zst',
    ]

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-release-test-')
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.packages = root / 'packages'
        self.packages.mkdir()
        self.dist = root / 'dist'
        replacements: dict[str, Any] = {
            'DIST': self.dist,
            'TEST_RESULTS': root / 'test-results',
            'release_version': lambda: self.VERSION,
            'remove_website_downloads': lambda: None,
            'verify_debian_package': lambda package: None,
            'write_source_archive': lambda archive: archive.write_bytes(b'source'),
        }
        for name, value in replacements.items():
            patcher = patch.object(release, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def build(self, names: list[str]) -> dict[str, str]:
        """Build a release from a CI folder holding names; return SHA256SUMS by file name."""
        for name in names:
            (self.packages / name).write_bytes(name.encode())
        release.build_release(release.parse_arguments(['--packages', str(self.packages)]))
        checksums = {}
        for line in (self.dist / 'SHA256SUMS').read_text().splitlines():
            digest, name = line.split('  ')
            checksums[name] = digest
        return checksums

    def test_every_stable_package_is_staged_and_checksummed(self) -> None:
        """The .deb, both RPMs, the Arch package and the Flatpak go into dist/ and SHA256SUMS."""
        checksums = self.build(self.STABLE_PACKAGES + self.OTHER_FILES)

        source = f'openxplorer-{self.VERSION}-source.zip'
        self.assertEqual(sorted(checksums), sorted([*self.STABLE_PACKAGES, source]))
        for name, digest in checksums.items():
            with self.subTest(name=name):
                self.assertEqual(digest, release.sha256_hex(self.dist / name))

    def test_preview_packages_and_other_versions_stay_out(self) -> None:
        """Only the stable app of the release's version is published."""
        self.build(self.STABLE_PACKAGES + self.OTHER_FILES)

        for name in self.OTHER_FILES:
            with self.subTest(name=name):
                self.assertFalse((self.dist / name).exists())

    def test_the_debian_package_alone_is_a_release(self) -> None:
        """RPM and Arch builds that failed leave the release with the .deb and the source."""
        checksums = self.build(['openxplorer_2.0.0_all.deb'])

        self.assertEqual(sorted(checksums),
                         ['openxplorer-2.0.0-source.zip', 'openxplorer_2.0.0_all.deb'])

    def test_a_missing_debian_package_stops_the_release(self) -> None:
        """The 1.1.x updater needs the .deb, so a release without it is refused."""
        with self.assertRaises(FileNotFoundError):
            self.build(self.STABLE_PACKAGES[1:])


class ReleaseFailureTests(unittest.TestCase):
    """main() ends a failed release with one line on stderr and exit status 1."""

    def failure_message(self, error: Exception) -> str:
        """Run main() with build_release() raising error and return what it printed."""
        stderr = io.StringIO()
        with (patch.object(release, 'build_release', side_effect=error),
              contextlib.redirect_stderr(stderr)):
            self.assertEqual(release.main([]), 1)
        return stderr.getvalue()

    def test_a_failed_build_step_is_named_with_its_exit_status(self) -> None:
        """A script that fails is named by its command line, not by a traceback."""
        command = ['python3', 'native/tools/verify_deb.py', 'dist/open xplorer.deb']
        message = self.failure_message(subprocess.CalledProcessError(2, command))
        self.assertEqual(message, 'Release failed: python3 native/tools/verify_deb.py '
                                  "'dist/open xplorer.deb' exited with status 2; "
                                  'its output is above.\n')

    def test_a_build_step_killed_by_a_signal_names_the_signal(self) -> None:
        """A negative return code is reported as the signal that ended the step."""
        command = ['python3', 'native/tools/build_deb.py']
        message = self.failure_message(subprocess.CalledProcessError(-9, command))
        self.assertEqual(message, 'Release failed: python3 native/tools/build_deb.py '
                                  'was killed by signal 9; its output is above.\n')

    def test_an_unreadable_directory_is_reported_in_one_line(self) -> None:
        """A filesystem error names the path and asks for a rerun."""
        error = PermissionError(13, 'Permission denied', '/repository/private')
        self.assertEqual(self.failure_message(error),
                         "Release failed: [Errno 13] Permission denied: '/repository/private'. "
                         'Fix this and rerun tools/release.py.\n')


if __name__ == '__main__':
    unittest.main()
