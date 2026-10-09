#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Generate repository and downloadable Markdown from the rendered guide source."""
import json
from pathlib import Path
root=Path(__file__).resolve().parents[1]
(root/'docs').mkdir(exist_ok=True)
public=root/'apps/web/public';(public/'docs-markdown').mkdir(parents=True,exist_ok=True)
exports={}
def markdown(doc,asset_prefix,demo_path):
    text=f"# {doc['title']}\n\n{doc['description']}\n"
    for s in doc['sections']:
        text+=f"\n## {s['title']}\n\n"+'\n\n'.join(s['paragraphs'])+'\n'
        if s.get('items'):text+='\n'+'\n'.join('- '+x for x in s['items'])+'\n'
        if s.get('code'):text+='\n```'+s.get('codeLanguage','sh')+'\n'+s['code']+'\n```\n'
        if s.get('image'):text+=f"\n![{s.get('imageAlt',s['title'])}]({asset_prefix}{s['image']}.png)\n\n*The native app, captured with fictional sample files. No live NAS connection.*\n"
        if s.get('demo'):text+=f"\n[Open the click-through tour of the real app]({demo_path}): pictures of the native app with sample files, no access to your computer.\n"
        if s.get('callout'):text+='\n> '+s['callout'].replace('\n','\n> ')+'\n'
    text+='\n---\n\nOpenXplorer 2.0.4. Project-authored documentation: AGPL-3.0-only.\n'
    return text
for doc in json.loads((root/'apps/web/lib/docs.json').read_text()):
    (root/'docs'/f"{doc['slug']}.md").write_text(markdown(doc,'../apps/web/public/assets/screenshots/','../apps/web/public/tour/index.html'))
    page=markdown(doc,'../assets/screenshots/','../tour/index.html')
    (public/'docs-markdown'/f"{doc['slug']}.md").write_text(page)
    exports[doc['slug']]=markdown(doc,'/assets/screenshots/','/tour/index.html')
(public/'assets/markdown.json').write_text(json.dumps(exports,ensure_ascii=False))
print('Generated 10 Markdown guides, downloadable copies, and the clipboard content index.')
