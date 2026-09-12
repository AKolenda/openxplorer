# SPDX-License-Identifier: AGPL-3.0-only
"""Source archive privacy and completeness using synthetic temporary trees."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools.release import source_files


class SourceArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='openxplorer-source-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def put(self, relative, value='synthetic fixture\n'):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value)
        return path

    def selected(self):
        return [relative.as_posix() for _, relative in source_files(self.root)]

    def test_preserves_editable_build_inputs_not_just_runtime_code(self):
        expected = ['.github/workflows/checks.yml', '.gitignore', '.env.example', '.dev.vars.example',
                    'AGENTS.md', 'LICENSE', 'NOTICE', 'THIRD_PARTY_NOTICES.md', 'pnpm-lock.yaml',
                    'pnpm-workspace.yaml', 'package.json', 'requirements-dev.txt', 'wrangler.jsonc',
                    'desktop/core.py', 'desktop/ui/app.js', 'desktop/licenses/Winspace-MIT.txt',
                    'licenses/Winspace-MIT.txt', 'tools/release.py', 'tests/test_release_source.py',
                    'apps/web/components/product.tsx', 'apps/web/public/assets/site.css',
                    'apps/web/public/assets/screenshots/manifest.json', 'docs/PRIVACY.md',
                    'apps/web/.env.example', 'apps/web/.dev.vars.example']
        for path in reversed(expected):
            self.put(path)
        self.assertEqual(self.selected(), sorted(expected))

    def test_excludes_local_state_and_generated_trees_at_any_depth(self):
        excluded = ['node_modules/package/source.js', 'apps/web/node_modules/package/source.js',
                    '.next/source.js', 'apps/web/out/index.html', '.git/config', '.hg/store/file',
                    '.svn/entries', '__pycache__/module.pyc', 'test-results/result.json',
                    'desktop/test-results/screenshot.png', 'dist/source.zip', 'desktop/dist/package.deb',
                    'designs/index.html', '.pnpm-store/index.json', '.wrangler/state/v3/db.sqlite',
                    'apps/web/.wrangler/config/default.toml', '.vercel/project.json', '.venv/bin/python',
                    'tools/venv/bin/python', '.pytest_cache/state', '.mypy_cache/state',
                    '.ruff_cache/state', '.cache/state', '.tox/state', '.nox/state', '.hypothesis/state',
                    '.nyc_output/result.json', 'htmlcov/index.html', 'coverage/index.html',
                    'playwright-report/index.html', 'blob-report/result.json', 'tmp/note.txt',
                    '.tmp/note.txt', 'temp/note.txt', '.temp/note.txt',
                    'apps/web/public/downloads/SHA256SUMS', 'apps/web/public/app-preview.html',
                    'apps/web/public/assets/site.js', 'desktop/preview.html']
        for path in excluded:
            self.put(path)
        self.put('desktop/ui/index.html')
        self.assertEqual(self.selected(), ['desktop/ui/index.html'])

    def test_excludes_environment_credentials_databases_logs_and_editor_backups(self):
        excluded = ['.env', '.env.local', '.env.production', '.env.example.local',
                    '.dev.vars', '.dev.vars.preview', '.dev.vars.example.secret',
                    '.private-demo-terms', '.private-demo-terms.json', 'private-terms.json',
                    '.DS_Store', '.coverage', '.coverage.worker', 'module.pyc', 'module.pyo',
                    'module.pyd', 'web.tsbuildinfo', 'package.deb', 'source.zip',
                    'server.log', 'server.log.1', 'local.sqlite', 'local.sqlite-wal',
                    'local.sqlite3', 'local.sqlite3-shm', 'local.db', 'local.db-journal',
                    'private.pem', 'private.key', 'private.p12', 'private.pfx', 'account.credentials',
                    'credentials.json', 'credentials.toml', 'credentials.yaml', 'credentials.yml',
                    'client_secret_sample.json', 'service-account-sample.json',
                    'source.swp', 'source.swo', 'source.tmp', 'source.temp', 'source.py~']
        for path in excluded:
            self.put('nested/' + path)
        self.assertEqual(self.selected(), [])

    def test_sensitive_filename_checks_are_case_insensitive_but_examples_are_exact(self):
        for name in ('PRIVATE.KEY', 'credentials.JSON', 'TRACE.LOG', '.ENV', '.ENV.EXAMPLE', '.DEV.VARS.EXAMPLE'):
            self.put(name)
        self.put('.env.example')
        self.put('.dev.vars.example')
        self.assertEqual(self.selected(), ['.dev.vars.example', '.env.example'])

    def test_excluded_directories_are_never_scanned(self):
        self.put('node_modules/package/deep/source.js')
        self.put('.wrangler/state/v3/source.py')
        self.put('apps/web/public/downloads/deep/source.py')
        self.put('desktop/ui/app.js')
        visited = []
        real_scandir = os.scandir

        def scandir(path):
            relative = Path(path).relative_to(self.root)
            visited.append(relative.as_posix())
            self.assertNotIn('node_modules', relative.parts)
            self.assertNotIn('.wrangler', relative.parts)
            self.assertNotIn('downloads', relative.parts)
            return real_scandir(path)

        with patch('tools.release.os.scandir', side_effect=scandir):
            self.assertEqual(self.selected(), ['desktop/ui/app.js'])
        self.assertIn('desktop/ui', visited)

    def test_links_cannot_import_external_private_files_or_cycle(self):
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
    def test_special_files_are_not_read_as_source(self):
        os.mkfifo(self.root / 'pipe.txt')
        self.put('source.py')
        self.assertEqual(self.selected(), ['source.py'])

    def test_unreadable_source_tree_fails_instead_of_silently_omitting_inputs(self):
        def inaccessible(root, **options):
            options['onerror'](PermissionError('Synthetic inaccessible source directory'))
            return iter(())
        with patch('tools.release.os.walk', side_effect=inaccessible), self.assertRaises(PermissionError):
            self.selected()

    def test_gitignore_and_source_policy_agree_for_publication_paths(self):
        repository = Path(__file__).resolve().parents[1]
        (self.root / '.gitignore').write_bytes((repository / '.gitignore').read_bytes())
        subprocess.run(['git', 'init', '--quiet', str(self.root)], check=True)
        cases = {'.wrangler/state/v3/local.sqlite': True, '.dev.vars': True, '.env.local': True,
                 '.env.example': False, '.dev.vars.example': False, '.cache/private.txt': True,
                 '.venv/pyvenv.cfg': True, 'nested/temp/private.txt': True, 'trace.log.1': True,
                 'credentials.json': True, 'local.db-wal': True, 'private.pem': True,
                 'desktop/test-results/native.json': True, 'desktop/preview.html': True,
                 'apps/web/public/downloads/SHA256SUMS': True, 'pnpm-lock.yaml': False,
                 'wrangler.jsonc': False, 'desktop/ui/app.js': False,
                 'licenses/Winspace-MIT.txt': False, '.github/workflows/checks.yml': False}
        for name, expected in cases.items():
            self.put(name)
            with self.subTest(path=name):
                ignored = subprocess.run(['git', '-C', str(self.root), 'check-ignore', '--quiet', name]).returncode == 0
                self.assertEqual(ignored, expected)
                self.assertEqual(name not in self.selected(), expected)


if __name__ == '__main__':
    unittest.main()
