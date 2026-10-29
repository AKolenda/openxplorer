#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Generate Markdown guides, or check that their committed copies are current."""
from __future__ import annotations

import argparse
from collections.abc import Sequence
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = Path('apps/web/public')
# Older generated versions still belong to the generator. A manual document
# with an ordinary AGPL notice does not have this complete footer.
GENERATED_FOOTER = re.compile(
    rb'\n---\n\nOpenXplorer \S+\. Project-authored documentation: AGPL-3\.0-only\.\n?\Z')


def markdown(doc: dict[str, Any], asset_prefix: str, demo_path: str) -> str:
    """Render one guide without changing the existing downloadable format."""
    text = f"# {doc['title']}\n\n{doc['description']}\n"
    for section in doc['sections']:
        text += f"\n## {section['title']}\n\n" + '\n\n'.join(section['paragraphs']) + '\n'
        if section.get('items'):
            text += '\n' + '\n'.join('- ' + item for item in section['items']) + '\n'
        if section.get('code'):
            text += ('\n```' + section.get('codeLanguage', 'sh') + '\n'
                     + section['code'] + '\n```\n')
        if section.get('image'):
            alt = section.get('imageAlt', section['title'])
            text += (f"\n![{alt}]({asset_prefix}{section['image']}.png)\n\n"
                     '*The native app, captured with fictional sample files. '
                     'No live NAS connection.*\n')
        if section.get('demo'):
            text += (f'\n[Open the click-through tour of the real app]({demo_path}): '
                     'pictures of the native app with sample files, '
                     'no access to your computer.\n')
        if section.get('callout'):
            text += '\n> ' + section['callout'].replace('\n', '\n> ') + '\n'
    text += '\n---\n\nOpenXplorer 2.0.9. Project-authored documentation: AGPL-3.0-only.\n'
    return text


def expected_outputs(root: Path) -> dict[Path, bytes]:
    """Render all canonical slugs, rejecting ambiguous or traversing output names."""
    docs = json.loads((root / 'apps/web/lib/docs.json').read_text(encoding='utf-8'))
    outputs = {}
    exports = {}
    for doc in docs:
        slug = doc['slug']
        if not isinstance(slug, str) or not re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', slug):
            raise ValueError(f'Invalid documentation slug: {slug!r}')
        if slug in exports:
            raise ValueError(f'Duplicate documentation slug: {slug}')
        outputs[Path('docs') / f'{slug}.md'] = markdown(
            doc, '../apps/web/public/assets/screenshots/',
            '../apps/web/public/tour/index.html').encode('utf-8')
        outputs[PUBLIC / 'docs-markdown' / f'{slug}.md'] = markdown(
            doc, '../assets/screenshots/', '../tour/index.html').encode('utf-8')
        exports[slug] = markdown(doc, '/assets/screenshots/', '/tour/index.html')
    outputs[PUBLIC / 'assets/markdown.json'] = json.dumps(
        exports, ensure_ascii=False).encode('utf-8')
    return outputs


def obsolete_outputs(root: Path, outputs: dict[Path, bytes]) -> list[Path]:
    """Find obsolete guides without treating manual repository docs as generated."""
    obsolete = []
    for folder in (Path('docs'), PUBLIC / 'docs-markdown'):
        for path in sorted((root / folder).glob('*.md')):
            relative = path.relative_to(root)
            if relative in outputs:
                continue
            if folder != Path('docs') or GENERATED_FOOTER.search(path.read_bytes()):
                obsolete.append(relative)
    return obsolete


def check_outputs(root: Path, outputs: dict[Path, bytes]) -> list[str]:
    """Compare guide content, guide membership and the public license copy."""
    problems = [f'Obsolete generated guide; review and remove: {path}'
                for path in obsolete_outputs(root, outputs)]
    for relative, expected in outputs.items():
        path = root / relative
        if not path.is_file():
            problems.append(f'Missing generated output: {relative}')
        elif path.read_bytes() != expected:
            problems.append(f'Out-of-date generated output: {relative}')
    return problems


def uncommitted_outputs(root: Path, paths: Sequence[Path]) -> list[str]:
    """Catch new, ignored, staged and modified outputs that git diff alone misses."""
    names = [path.as_posix() for path in paths]
    tracked = set(subprocess.run(
        ['git', 'ls-files', '--cached', '-z', '--', *names], cwd=root,
        check=True, capture_output=True, text=True).stdout.split('\0'))
    problems = [f'Generated output is not tracked: {name}'
                for name in names if name not in tracked]
    changed = subprocess.run(
        ['git', 'diff', '--name-only', '-z', 'HEAD', '--', *names], cwd=root,
        check=True, capture_output=True, text=True).stdout.split('\0')
    problems.extend(f'Generated output has uncommitted changes: {name}'
                    for name in changed if name)
    return problems


def main(argv: Sequence[str] | None = None) -> int:
    """Write guides, or fail without mutations when committed outputs have drifted."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true',
                        help='verify generated guides and license without writing any files')
    arguments = parser.parse_args(argv)
    try:
        outputs = expected_outputs(ROOT)
        if arguments.check:
            outputs[PUBLIC / 'LICENSE.txt'] = (ROOT / 'LICENSE').read_bytes()
            problems = check_outputs(ROOT, outputs) + uncommitted_outputs(ROOT, list(outputs))
        else:
            problems = [f'Obsolete generated guide; review and remove: {path}'
                        for path in obsolete_outputs(ROOT, outputs)]
            if not problems:
                for relative, content in outputs.items():
                    path = ROOT / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(content)
        if problems:
            print('\n'.join(problems), file=sys.stderr)
            return 1
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'Generated documentation check failed: {error}', file=sys.stderr)
        return 1
    if arguments.check:
        print('Committed Markdown guides and public license are current.')
    else:
        print(f'Generated {(len(outputs) - 1) // 2} Markdown guides, downloadable copies, '
              'and the clipboard content index.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
