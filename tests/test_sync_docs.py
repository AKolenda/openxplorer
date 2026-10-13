# SPDX-License-Identifier: AGPL-3.0-only
"""Generated guide and license consistency, using disposable synthetic checkouts."""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'tools/sync-docs.py'
PUBLIC = Path('apps/web/public')
GIT_ENVIRONMENT = {
    **os.environ,
    'GIT_CONFIG_NOSYSTEM': '1',
    'GIT_CONFIG_GLOBAL': os.devnull,
    'GIT_CONFIG_COUNT': '1',
    'GIT_CONFIG_KEY_0': 'core.excludesFile',
    'GIT_CONFIG_VALUE_0': os.devnull,
}


class SyncDocsTests(unittest.TestCase):
    """Never use the real guide output, Git index or license as a writable fixture."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix='openxplorer-sync-docs-test-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        spec = importlib.util.spec_from_file_location('sync_docs', SCRIPT)
        assert spec is not None and spec.loader is not None
        self.sync = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.sync)
        self.sync.ROOT = self.root
        self.docs = [{
            'slug': 'start', 'title': 'A fictional guide', 'description': 'Sample documentation.',
            'sections': [{'title': 'First step', 'paragraphs': ['Use a disposable folder.']}],
        }]
        self.write_docs()
        self.put(Path('LICENSE'), b'A fictional test license.\n')
        self.manual = b'# Manual guide\n\nProject-authored documentation: AGPL-3.0-only.\n'
        self.put(Path('docs/MANUAL.md'), self.manual)
        # Ordinary generation also has to work in source archives without .git.
        self.assertEqual(self.invoke()[0], 0)
        self.put(PUBLIC / 'LICENSE.txt', (self.root / 'LICENSE').read_bytes())

    def put(self, relative: Path, content: bytes) -> None:
        """Write a fixture underneath the temporary repository."""
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)

    def write_docs(self) -> None:
        """Publish the fixture's canonical guide source."""
        self.put(Path('apps/web/lib/docs.json'), json.dumps(self.docs).encode('utf-8'))

    def invoke(self, *arguments: str) -> tuple[int, str]:
        """Run the script entry point and retain its user-facing explanation."""
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            status = self.sync.main(arguments)
        return status, output.getvalue()

    def git(self, *arguments: str) -> str:
        """Run Git only inside the fixture, without user configuration or hooks."""
        return subprocess.run(
            ['git', '-c', 'core.hooksPath=/dev/null', *arguments], cwd=self.root,
            env=GIT_ENVIRONMENT, check=True, capture_output=True, text=True).stdout

    def commit(self, *paths: str) -> None:
        """Commit selected fixture paths without relying on the user's identity."""
        self.git('add', '--', *(paths or ('.',)))
        self.git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                 'commit', '--quiet', '--no-gpg-sign', '-m', 'Synthetic generated outputs')

    def initialize_git(self) -> None:
        """Start with a clean, entirely tracked fixture checkout."""
        self.git('init', '--quiet')
        self.commit()

    def test_current_outputs_pass_and_manual_docs_remain_untouched(self) -> None:
        self.initialize_git()
        self.assertEqual(self.invoke('--check')[0], 0)
        self.assertEqual((self.root / 'docs/MANUAL.md').read_bytes(), self.manual)
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_new_untracked_guides_fail_even_when_git_diff_is_empty(self) -> None:
        self.initialize_git()
        self.docs.append({**self.docs[0], 'slug': 'another-guide'})
        self.write_docs()
        self.assertEqual(self.invoke()[0], 0)
        # A contributor committed the source and clipboard index but missed
        # both new Markdown pages. A plain diff after generation stays empty.
        self.commit('apps/web/lib/docs.json', 'apps/web/public/assets/markdown.json')
        self.assertEqual(self.git('diff', '--name-only'), '')

        status, output = self.invoke('--check')

        self.assertEqual(status, 1)
        self.assertIn('not tracked: docs/another-guide.md', output)
        self.assertIn('not tracked: apps/web/public/docs-markdown/another-guide.md', output)
        self.commit()
        self.assertEqual(self.invoke('--check')[0], 0)

    def test_ignored_generated_output_still_has_to_be_tracked(self) -> None:
        self.initialize_git()
        self.git('rm', '--cached', 'docs/start.md')
        self.put(Path('.gitignore'), b'docs/start.md\n')
        self.commit()
        self.assertEqual(self.git('status', '--porcelain'), '')

        status, output = self.invoke('--check')

        self.assertEqual(status, 1)
        self.assertIn('not tracked: docs/start.md', output)

    def test_renamed_slug_rejects_stale_repository_and_downloadable_pages(self) -> None:
        self.docs[0]['slug'] = 'renamed-guide'
        self.write_docs()
        outputs = self.sync.expected_outputs(self.root)
        # Reproduce what the old generator did on a slug rename.
        for path, contents in outputs.items():
            self.put(path, contents)
        old_public = PUBLIC / 'docs-markdown/start.md'
        for remove_public in (False, True):
            with self.subTest(old_download_removed=remove_public):
                if remove_public:
                    (self.root / old_public).unlink()
                obsolete = self.sync.obsolete_outputs(self.root, outputs)
                self.assertIn(Path('docs/start.md'), obsolete)
                self.assertEqual(old_public in obsolete, not remove_public)
                before = {path: path.read_bytes() for path in self.root.rglob('*.md')}

                status, output = self.invoke()

                self.assertEqual(status, 1)
                self.assertIn('Obsolete generated guide; review and remove: docs/start.md', output)
                self.assertEqual(before, {path: path.read_bytes()
                                          for path in self.root.rglob('*.md')})
                self.assertEqual((self.root / 'docs/MANUAL.md').read_bytes(), self.manual)

    def test_missing_and_modified_outputs_are_reported_without_rewriting(self) -> None:
        self.initialize_git()
        (self.root / 'docs/start.md').unlink()
        changed = PUBLIC / 'docs-markdown/start.md'
        self.put(changed, b'A stale copy.\n')

        status, output = self.invoke('--check')

        self.assertEqual(status, 1)
        self.assertIn('Missing generated output: docs/start.md', output)
        self.assertIn('Out-of-date generated output: '
                      'apps/web/public/docs-markdown/start.md', output)
        self.assertFalse((self.root / 'docs/start.md').exists())
        self.assertEqual((self.root / changed).read_bytes(), b'A stale copy.\n')

    def test_license_copy_must_match_and_its_regeneration_must_be_committed(self) -> None:
        self.initialize_git()
        self.put(Path('LICENSE'), b'A revised fictional license.\n')
        self.commit('LICENSE')
        status, output = self.invoke('--check')
        self.assertEqual(status, 1)
        self.assertIn('Out-of-date generated output: apps/web/public/LICENSE.txt', output)

        self.put(PUBLIC / 'LICENSE.txt', (self.root / 'LICENSE').read_bytes())
        # Also reject a staged fix until it is part of the reviewed commit.
        self.git('add', 'apps/web/public/LICENSE.txt')
        status, output = self.invoke('--check')
        self.assertEqual(status, 1)
        self.assertIn('uncommitted changes: apps/web/public/LICENSE.txt', output)
        self.commit()
        self.assertEqual(self.invoke('--check')[0], 0)

    def test_unsafe_and_duplicate_slugs_are_refused_before_writing(self) -> None:
        for slug in ('../MANUAL', '/outside', 'start'):
            with self.subTest(slug=slug):
                self.docs.append({**self.docs[0], 'slug': slug})
                self.write_docs()
                self.assertEqual(self.invoke()[0], 1)
                self.assertEqual((self.root / 'docs/MANUAL.md').read_bytes(), self.manual)
                self.docs.pop()


if __name__ == '__main__':
    unittest.main()
