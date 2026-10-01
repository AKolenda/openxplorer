#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Small, repeatable project-specific security checks, NOT a full SAST/audit.

Checks the project's Python tools, the native workspace's safety settings, the
website's tour sandbox and declared configuration. Does not query an
advisory database, resolve transitive deps, prove dataflow, or test live GTK/SMB.
Run pnpm audit, a dependency-aware build and target-machine tests separately.
"""
from __future__ import annotations
import ast
import hashlib
import json
from pathlib import Path
import re
import sys
ROOT=Path(__file__).resolve().parents[1]
checks=[]
def check(name, condition):
    checks.append({'check':name,'passed':bool(condition)})
    return condition

# The Python the project runs: the repository, packaging and parity tools.
PYTHON_SOURCES=('tools','tests','native/tools','native/parity')


def main():
    inventory=[];danger=[]
    for source in sorted(path for directory in PYTHON_SOURCES for path in (ROOT/directory).glob('*.py')):
        data=source.read_bytes();inventory.append({'path':source.relative_to(ROOT).as_posix(),'sha256':hashlib.sha256(data).hexdigest()})
        tree=ast.parse(data,filename=str(source))
        for n in ast.walk(tree):
            if not isinstance(n,ast.Call):continue
            name=ast.unparse(n.func)
            if name in ('eval','exec','os.system','os.popen','pickle.load','pickle.loads'):
                danger.append(f'{source.relative_to(ROOT)}:{n.lineno}: {name}')
            if name.startswith('subprocess.') and any(k.arg=='shell' and not (isinstance(k.value,ast.Constant) and k.value.value is False) for k in n.keywords):
                danger.append(f'{source.relative_to(ROOT)}:{n.lineno}: shell invocation')
    check('Project Python contains no eval/exec, pickle load or shell=True calls',not danger)
    workspace=(ROOT/'native/Cargo.toml').read_text()
    check('Native workspace denies unsafe code','unsafe_code = "deny"' in workspace)
    allowed=[str(p.relative_to(ROOT)) for p in (ROOT/'native/crates').rglob('*.rs') if re.search(r'(allow|expect)\(unsafe_code',p.read_text())]
    check('No native source lifts the unsafe-code lint',not allowed)
    guard=(ROOT/'native/crates/ox-app/src/update/launch_guard.rs').read_text()
    check('Native app refuses running as root','fn refuse_root(' in guard and 'user_id != ROOT_USER_ID' in guard)
    product=(ROOT/'apps/web/components/product.tsx').read_text()
    check('Public iframe retains script-only sandbox', 'sandbox="allow-scripts"' in product and 'allow-same-origin' not in product)
    check('Public tour uses viewport gating', 'data-demo' in product)
    tour=(ROOT/'apps/web/public/tour/tour.js').read_text()
    check('Tour builds its controls without HTML-string insertion',not any(x in tour for x in ('.innerHTML','.outerHTML','insertAdjacentHTML','document.write(')))
    check('Tour answers only its parent window',"event.source !== window.parent" in tour)
    package=json.loads((ROOT/'package.json').read_text());web=json.loads((ROOT/'apps/web/package.json').read_text())
    check('Patched pnpm 10.x pinned','pnpm@10.34.5'==package['packageManager'])
    check('Website direct versions are exact',all(re.fullmatch(r'\d+\.\d+\.\d+',v) for v in web['dependencies'].values()))
    check('Website is configured as static export',"output: 'export'" in (ROOT/'apps/web/next.config.ts').read_text())
    config=json.loads((ROOT/'vercel.json').read_text())
    check('Deployment requires frozen lockfile',config['installCommand']=='pnpm install --frozen-lockfile')
    headers={v['key']:v['value'] for v in config['headers'][0]['headers']}
    check('Deployment sends nosniff and restricted feature policy',headers.get('X-Content-Type-Options')=='nosniff' and 'camera=()' in headers.get('Permissions-Policy',''))
    ci=(ROOT/'.github/workflows/checks.yml').read_text()
    check('CI default read permission, no persisted checkout credentials','contents: read' in ci and 'persist-credentials: false' in ci)
    check('CI gates on lockfile, audit, typecheck and real build',all(x in ci for x in ('test -s pnpm-lock.yaml','--frozen-lockfile','pnpm audit --audit-level=moderate','pnpm check','pnpm build')))
    forbidden=[]
    for p in ROOT.rglob('*'):
        rel=p.relative_to(ROOT)
        if not p.is_file() or any(t in rel.parts for t in ('node_modules','.git','__pycache__','dist','designs','test-results')):continue
        if p.suffix in ('.pem','.key','.p12','.sqlite3') or p.name.startswith('.env') and p.name!='.env.example':forbidden.append(str(rel))
    check('No common private key, environment or user database files in source',not forbidden)
    result={'passed':all(c['passed'] for c in checks),'checks':len(checks),'details':checks,
      'flaggedCalls':danger,'forbiddenFiles':forbidden,'runtimeSourceInventory':inventory,
      'limitations':['Project-specific syntactic checks; not complete SAST, malware detection or pentesting.',
      'No transitive dependency resolution/audit or Next production build in this offline environment.',
      'No native GTK run, real terminal GUI or live SMB/keyring test: native/tools/check.py covers the app.'],
      'releaseGate':'See docs/RELEASE-CHECKLIST.md.'}
    out=ROOT/'test-results';out.mkdir(exist_ok=True);(out/'security-sweep.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'passed':result['passed'],'checks':len(checks),'failed':[c['check'] for c in checks if not c['passed']]},indent=2))
    return 0 if result['passed'] else 1
if __name__=='__main__':sys.exit(main())
