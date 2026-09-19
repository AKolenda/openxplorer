#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Audit public text, nested release archives and generated image provenance.

No customer identifiers or their fingerprints are stored in this repository.
Supply a local JSON array via --private-terms, or use OX_PRIVATE_TERMS.
Image byte provenance is verified separately; this is not an OCR claim.
"""
from __future__ import annotations
import argparse,base64,hashlib,html,io,json,os,re,subprocess,tarfile,tempfile,zipfile
from pathlib import Path
from urllib.parse import unquote
ROOT=Path(__file__).resolve().parents[1]
DENIED={}
TEXT={'.py','.js','.cjs','.mjs','.ts','.tsx','.css','.html','.md','.txt','.json','.xml','.yml','.yaml','.sh','.svg','.desktop','.service'}
SKIP={'.git','node_modules','.next','.pnpm-store','__pycache__'}
def digest(data):return hashlib.sha256(data).hexdigest()
def deny_terms(terms):
    """Keep supplied identifiers in memory only, including full names/addresses."""
    for value in terms:
        term=' '.join(value.casefold().split())
        if not term:continue
        # Match complete identifiers, with whitespace allowed to wrap in text.
        # Underscores and punctuation remain word separators as in the previous
        # token checks; Unicode letters and digits are not separators.
        pattern=r'(?<![^\W_])'+r'\s+'.join(re.escape(part) for part in term.split())+r'(?![^\W_])'
        DENIED[digest(term.encode())]=re.compile(pattern)
def contains_private_term(value):
    return any(pattern.search(value.casefold()) for pattern in DENIED.values())
deny_terms(os.environ.get('OX_PRIVATE_TERMS','').split(','))
def audit(paths):
    seen=set();issues=[];texts=archives=images=0
    known_images=set()
    for manifest in (ROOT/'apps/web/public/assets/screenshots/manifest.json',ROOT/'docs/assets/manifest.json'):
        if manifest.exists():known_images.update(json.loads(manifest.read_text()).get('sha256',{}).values())
    def visit(name,data,depth=0):
        nonlocal texts,archives,images
        # Names are distinct publication data even when their payloads match.
        if contains_private_term(unquote(name)):issues.append(name+': rejected filename fingerprint')
        h=digest(data)
        if h in seen:return
        seen.add(h)
        if depth>8:issues.append(name+': nested archive depth exceeded');return
        suffix=Path(name).suffix.lower()
        if suffix=='.zip':
            archives+=1
            with zipfile.ZipFile(io.BytesIO(data)) as z:
                for m in z.infolist():
                    if not m.is_dir():visit(name+'!/'+m.filename,z.read(m),depth+1)
        elif suffix=='.deb':
            archives+=1
            with tempfile.NamedTemporaryFile(suffix='.deb') as f:
                f.write(data);f.flush()
                payload=subprocess.check_output(['dpkg-deb','--fsys-tarfile',f.name])
            with tarfile.open(fileobj=io.BytesIO(payload)) as t:
                for m in t.getmembers():
                    if m.isfile():visit(name+'!/'+m.name,t.extractfile(m).read(),depth+1)
        elif suffix in {'.png','.jpg','.jpeg','.webp'}:
            images+=1
            if any(part in name for part in ('assets/screenshots/','docs/assets/')) and h not in known_images:issues.append(name+': unregistered image inside publication/archive')
        elif suffix in TEXT or suffix=='':
            try:s=html.unescape(unquote(data.decode('utf-8'))).casefold()
            except UnicodeError:return
            texts+=1
            # Detect stale inline images in self-contained HTML as well as PNG files.
            for encoded in re.findall(rb'data:image/(?:png|jpeg|webp);base64,([A-Za-z0-9+/=]+)',data):
                try:image_hash=digest(base64.b64decode(encoded,validate=True))
                except ValueError:issues.append(name+': invalid inline image');continue
                if image_hash not in known_images:issues.append(name+': unregistered inline screenshot')
            if contains_private_term(s):issues.append(name+': rejected private-data fingerprint')
    for p in paths:
        if p.is_file():visit(str(p.relative_to(ROOT)) if p.is_relative_to(ROOT) else p.name,p.read_bytes());continue
        for f in sorted(p.rglob('*')):
            if f.is_file() and not any(part in SKIP for part in f.parts):visit(str(f.relative_to(p)),f.read_bytes())
    manifest_path=ROOT/'apps/web/public/assets/screenshots/manifest.json'
    provenance=[]
    if manifest_path.exists():
        manifest=json.loads(manifest_path.read_text())
        if not manifest.get('fixturePolicy'):issues.append('Screenshot manifest missing fixture policy')
        if manifest.get('fixtureSourceSha256')!=digest((ROOT/'desktop/ui/app.js').read_bytes()):issues.append('Screenshot fixture source has changed since capture')
        for name,expected in manifest.get('sha256',{}).items():
            path=manifest_path.parent/name
            if not path.exists() or digest(path.read_bytes())!=expected:issues.append('Screenshot hash mismatch: '+name)
            else:provenance.append(name)
        if len(provenance)!=7:issues.append('Expected seven regenerated product screenshots')
    else:issues.append('Missing screenshot manifest')
    # Every published PNG must come from the screenshot tools, not an attachment.
    web_manifest=ROOT/'docs/assets/manifest.json'
    trusted=set(json.loads(manifest_path.read_text()).get('sha256',{}).values()) if manifest_path.exists() else set()
    if web_manifest.exists():trusted.update(json.loads(web_manifest.read_text()).get('sha256',{}).values())
    for directory in ['apps/web/public','docs/assets','designs']:
        for p in (ROOT/directory).rglob('*.png'):
            if digest(p.read_bytes()) not in trusted:issues.append('Unregistered public screenshot: '+str(p.relative_to(ROOT)))
    return {'passed':not issues,'privateIdentifierRules':len(DENIED),'uniqueTextFiles':texts,'uniqueArchives':archives,'uniqueImages':images,'verifiedProductScreenshots':provenance,'issues':issues,'scope':'Known private-data fingerprints in UTF-8 text, URL/HTML-decoded text, and recursively inspected ZIP/deb payloads; public PNG hashes verified against regenerated capture manifests. No OCR; visual review also required.'}
if __name__=='__main__':
    ap=argparse.ArgumentParser();ap.add_argument('paths',nargs='*',type=Path);ap.add_argument('--json',type=Path);ap.add_argument('--private-terms',type=Path,help='Local JSON array, kept outside the repository');a=ap.parse_args()
    if a.private_terms:
        terms=json.loads(a.private_terms.read_text())
        if not isinstance(terms,list) or not all(isinstance(t,str) for t in terms):raise SystemExit('Private terms must be a JSON array of strings')
        deny_terms(terms)
    report=audit(a.paths or [ROOT]);print(json.dumps(report,indent=2))
    if a.json:a.json.parent.mkdir(parents=True,exist_ok=True);a.json.write_text(json.dumps(report,indent=2)+'\n')
    raise SystemExit(0 if report['passed'] else 1)
