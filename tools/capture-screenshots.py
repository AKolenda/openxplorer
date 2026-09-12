#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Capture the actual app HTML with Playwright, not generated imagery or a second UI.

The preview's storage adapter supplies fictional local/SMB fixtures. No native
GTK/WebKit, keyring, personal file, or real network connection is exercised.
"""
import json,os,hashlib
from pathlib import Path
from playwright.sync_api import sync_playwright
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'apps/web/public/assets/screenshots'
OUT.mkdir(parents=True,exist_ok=True)
with sync_playwright() as p:
    browser=p.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
    page=browser.new_page(viewport={'width':1440,'height':900},device_scale_factor=1)
    page.set_default_timeout(8000)
    errors=[];page.on('pageerror',lambda e:errors.append(str(e)))
    page.evaluate("""()=>{const d={};Object.defineProperty(window,'localStorage',{value:{getItem:k=>d[k]??null,setItem:(k,v)=>d[k]=String(v),removeItem:k=>delete d[k]}})}""")
    page.set_content((ROOT/'desktop/preview.html').read_text(),wait_until='load')
    page.wait_for_function('()=>window.OpenXplorerTour && OpenXplorer.state.ready')
    page.evaluate("()=>{document.body.style.padding='0';document.body.classList.add('website-embed');OpenXplorer.applyTheme('light',false);}")
    def ready(uri=None):
        page.wait_for_function("u=>{const a=OpenXplorer.state,t=a.tabs.find(t=>t.id===a.activeId);return t?.loaded&&!t.busy&&(!u||u===t.uri)}",arg=uri)
        page.wait_for_timeout(120)
    def nav(uri):page.evaluate('(u)=>OpenXplorer.navigate(u)',uri);ready(uri)
    # Use the real pin operation before taking the product stills.
    page.evaluate("async()=>{await OpenXplorer.call('pin',{items:[{uri:'smb://studio-nas/Projects/Design',label:'Design'}],before:'file:///home/demo/Documents'});await OpenXplorer.refreshEnvironment();}")
    nav('smb://studio-nas/Projects')
    page.locator('#app').screenshot(path=str(OUT/'explorer-light.png'))
    page.evaluate("OpenXplorer.applyTheme('dark',false)")
    page.locator('#app').screenshot(path=str(OUT/'explorer-dark.png'))
    page.evaluate("OpenXplorer.applyTheme('light',false)")
    nav('smb://studio-nas/Projects/Design')
    # Genuine image crops, not re-created controls.
    page.screenshot(path=str(OUT/'network-path.png'),clip={'x':208,'y':37,'width':990,'height':193})
    box=page.locator('.sidebar').bounding_box()
    page.screenshot(path=str(OUT/'pinned-sidebar.png'),clip={'x':box['x'],'y':box['y'],'width':box['width'],'height':410})
    nav('smb://archive-nas/Shared')
    page.locator('#search').fill('Launch')
    page.wait_for_function("()=>OpenXplorer.state.query==='Launch'&&!OpenXplorer.state.searchBusy")
    page.wait_for_timeout(160)
    page.locator('#app').screenshot(path=str(OUT/'cached-search.png'))
    page.evaluate("()=>{OpenXplorer.closeModal();void OpenXplorer.propertiesDialog({uri:'smb://archive-nas/Shared/Launch%20planning',name:'Launch planning',isDir:true},'versions');}")
    page.wait_for_selector('.version-date');page.wait_for_timeout(100)
    page.locator('#modal').screenshot(path=str(OUT/'previous-versions.png'))
    page.locator('.version-actions button').filter(has_text='Browse').first.click()
    ready()
    page.locator('#app').screenshot(path=str(OUT/'snapshot-tab.png'))
    assert page.locator('.snapshot-tab-badge').count()==1
    assert page.locator('#snapshot-banner').is_visible()
    assert not errors, errors
    browser.close()
manifest={'fixturePolicy':'Entirely fictional sample names, addresses and paths; fresh isolated browser context. No user screenshots.', 'source':'desktop/preview.html generated from desktop/ui/* + preview-only demo/showcase.js','storage':'simulated fixtures','renderer':'Chromium','nativeRuntime':False,'screenshots':[f.name for f in sorted(OUT.glob('*.png'))]}
manifest['sha256']={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(OUT.glob('*.png'))}
manifest['fixtureSourceSha256']=hashlib.sha256((ROOT/'desktop/ui/app.js').read_bytes()).hexdigest()
(OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps(manifest,indent=2))
