#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Small, repeatable project-specific security checks, NOT a full SAST/audit.

Checks editable runtime code and declared configuration. Does not query an
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

def main():
    inventory=[];danger=[]
    for source in sorted((ROOT/'desktop').glob('*.py')):
        data=source.read_bytes();inventory.append({'path':source.relative_to(ROOT).as_posix(),'sha256':hashlib.sha256(data).hexdigest()})
        tree=ast.parse(data,filename=str(source))
        for n in ast.walk(tree):
            if not isinstance(n,ast.Call):continue
            name=ast.unparse(n.func)
            if name in ('eval','exec','os.system','os.popen','pickle.load','pickle.loads'):
                danger.append(f'{source.name}:{n.lineno}: {name}')
            if name.startswith('subprocess.') and any(k.arg=='shell' and not (isinstance(k.value,ast.Constant) and k.value.value is False) for k in n.keywords):
                danger.append(f'{source.name}:{n.lineno}: shell invocation')
    check('Desktop Python contains no eval/exec, pickle load or shell=True calls',not danger)
    ui=(ROOT/'desktop/ui/app.js').read_text()
    check('Desktop UI does not use HTML-string insertion',not any(x in ui for x in ('.innerHTML','.outerHTML','insertAdjacentHTML','document.write(')))
    html=(ROOT/'desktop/ui/index.html').read_text()
    check('Native UI denies network/frame/object access in CSP',all(x in html for x in ("connect-src 'none'","frame-src 'none'","object-src 'none'","default-src 'none'")))
    host=(ROOT/'desktop/winspace.py').read_text()
    check('WebKit sandbox enabled before WebView creation',host.index('context.set_sandbox_enabled(True)') < host.index('WebKit2.WebView(web_context='))
    check('Only packaged UI path added to sandbox read-only',"context.add_path_to_sandbox(str(ROOT / 'ui'), True)" in host)
    check('Desktop refuses running as root',"if os.geteuid()==0:" in host)
    check('Terminal bridge accepts only URI and native metadata/resolver',"elif method == 'openTerminal':" in host and 'prepare_directory(uri, inspect, local_path,' in host)
    terminal=(ROOT/'desktop/terminal_integration.py').read_text()
    check('Terminal uses argv with shell=False and explicit cwd','subprocess.Popen(argv, cwd=directory' in terminal and 'shell=False' in terminal)
    check('Terminal checks snapshots and mounted local directory','conventional_snapshot(local_uri)' in terminal and 'checked_directory(local)' in terminal)
    product=(ROOT/'apps/web/components/product.tsx').read_text()
    check('Public iframe retains script-only sandbox', 'sandbox="allow-scripts"' in product and 'allow-same-origin' not in product)
    check('Public preview uses viewport gating', 'data-demo' in product)
    package=json.loads((ROOT/'package.json').read_text());web=json.loads((ROOT/'apps/web/package.json').read_text())
    check('Patched pnpm 10.x pinned','pnpm@10.34.5'==package['packageManager'])
    check('Website direct versions are exact',all(re.fullmatch(r'\d+\.\d+\.\d+',v) for v in web['dependencies'].values()))
    check('Website is configured as static export',"output: 'export'" in (ROOT/'apps/web/next.config.ts').read_text())
    config=json.loads((ROOT/'vercel.json').read_text())
    check('Deployment requires frozen lockfile',config['installCommand']=='pnpm install --frozen-lockfile')
    headers={v['key']:v['value'] for v in config['headers'][0]['headers']}
    check('Deployment sends nosniff and restricted feature policy',headers.get('X-Content-Type-Options')=='nosniff' and 'camera=()' in headers.get('Permissions-Policy',''))
    ci=(ROOT/'.github/workflows/checks.yml').read_text()
    check('CI read-only token, no persisted checkout credentials','contents: read' in ci and 'persist-credentials: false' in ci)
    check('CI gates on lockfile, audit, typecheck and real build',all(x in ci for x in ('test -s pnpm-lock.yaml','--frozen-lockfile','pnpm audit --audit-level=moderate','pnpm check','pnpm build')))
    check('New security and terminal regression tests included',(ROOT/'desktop/tests/test_terminal_security.py').is_file() and (ROOT/'desktop/tests/ui_terminal.py').is_file())
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
      'No native Zorin/WebKit, real terminal GUI or live SMB/keyring test.'],
      'releaseGate':'1.0.0 stable; see docs/RELEASE-CHECKLIST.md.'}
    out=ROOT/'test-results';out.mkdir(exist_ok=True);(out/'security-sweep.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'passed':result['passed'],'checks':len(checks),'failed':[c['check'] for c in checks if not c['passed']]},indent=2))
    return 0 if result['passed'] else 1
if __name__=='__main__':sys.exit(main())
