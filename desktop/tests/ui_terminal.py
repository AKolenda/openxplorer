#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Real desktop HTML; browser actions are simulated and launch no programs."""
import json, os, re
from pathlib import Path
from playwright.sync_api import sync_playwright, expect
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'test-results';OUT.mkdir(exist_ok=True)
checks=[];errors=[]
def check(label,value=True):
 assert value,label
 checks.append(label);print('PASS',label,flush=True)
with sync_playwright() as pw:
 browser=pw.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
 page=browser.new_page(viewport={'width':1380,'height':900});page.set_default_timeout(8000)
 page.on('pageerror',lambda e:errors.append(str(e)))
 page.set_content((ROOT/'preview.html').read_text(),wait_until='load')
 page.wait_for_function('()=>OpenXplorer.state.ready&&OpenXplorer.state.tabs[0]?.loaded')
 def nav(uri):
  page.evaluate('(uri)=>OpenXplorer.navigate(uri)',uri)
  page.wait_for_function('()=>OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).loaded')
 nav('file:///home/demo/Documents')
 folder=page.locator('.file-row[data-kind="directory"]').first
 folder.click(button='right')
 action=page.get_by_role('menuitem',name='Open in Terminal',exact=True)
 check('Folder has Open in Terminal',action.is_enabled())
 check('Context action uses terminal icon',action.locator('[data-icon="terminal"]').count()==1)
 page.screenshot(path=str(OUT/'open-terminal-context.png'))
 action.click();expect(page.locator('#toast')).to_contain_text('Preview only: Terminal would open here')
 check('Preview explicitly does not launch a program')
 check('Terminal action did not change folder',page.evaluate('()=>OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri')=='file:///home/demo/Documents')
 file=page.locator('.file-row[data-kind="file"]').first;file.click(button='right')
 check('File action explicitly names containing folder',page.get_by_role('menuitem',name='Open containing folder in Terminal',exact=True).is_enabled())
 page.keyboard.press('Escape')
 page.evaluate('()=>document.getElementById("file-scroll").dispatchEvent(new MouseEvent("contextmenu",{bubbles:true,clientX:700,clientY:500}))')
 check('Background context includes current-folder Terminal',page.get_by_role('menuitem',name='Open in Terminal',exact=True).is_enabled())
 page.keyboard.press('Escape')
 page.evaluate('()=>OpenXplorer.entryMenu(400,200,{uri:"file:///home/demo/Documents",isDir:true},"win11")')
 check('Windows 11 menu also includes Terminal',page.get_by_role('menuitem',name='Open in Terminal',exact=True).is_enabled());page.keyboard.press('Escape')
 # Sidebar and Network entries exercise real contextmenu DOM handlers.
 page.locator('#quick-access button').filter(has_text='Documents').first.click(button='right')
 check('Sidebar pin includes Terminal',page.get_by_role('menuitem',name='Open in Terminal',exact=True).is_enabled());page.keyboard.press('Escape')
 nav('smb://studio-nas/Projects')
 page.evaluate('()=>document.getElementById("file-scroll").dispatchEvent(new MouseEvent("contextmenu",{bubbles:true,clientX:700,clientY:500}))')
 check('SMB share offers Terminal',page.get_by_role('menuitem',name='Open in Terminal',exact=True).is_enabled())
 page.get_by_role('menuitem',name='Open in Terminal',exact=True).click();expect(page.locator('#toast')).to_contain_text('Preview only')
 check('SMB preview stays simulated')
 page.locator('[data-network-saved]').filter(has_text='Studio NAS').first.click(button='right')
 check('Network sidebar share offers Terminal',page.get_by_role('menuitem',name='Open in Terminal',exact=True).is_enabled());page.keyboard.press('Escape')
 for uri,reason in [('smb://studio-nas','Bare server has no cwd'),('smb://studio-nas/Projects/.zfs/snapshot/auto-2026-01-01','Snapshot is protected'),('network:','Virtual location cannot be a terminal cwd')]:
  page.evaluate('(uri)=>OpenXplorer.entryMenu(400,200,{uri,isDir:true})',uri)
  check(reason,page.get_by_role('menuitem',name='Open in Terminal',exact=True).is_disabled());page.keyboard.press('Escape')
 page.evaluate('()=>{OpenXplorer.state.selection=new Set(["file:///one","file:///two"]);OpenXplorer.entryMenu(400,200,{uri:"file:///home/demo/Documents",isDir:true})}')
 check('Multiselection does not launch many terminals',page.get_by_role('menuitem',name='Open in Terminal',exact=True).is_disabled());page.keyboard.press('Escape')
 page.evaluate('()=>OpenXplorer.state.selection.clear()')
 result=page.evaluate('()=>OpenXplorer.call("openTerminal",{uri:"file:///home/demo/Documents/Brand%20guidelines.pdf"})')
 check('Cached-file transport resolves containing directory',result['uri']=='file:///home/demo/Documents')
 check('No actual process launch in public transport',result['opened'] is False and result['preview'] is True)
 check('Native capability not exposed by browser',page.evaluate('()=>!window.webkit?.messageHandlers?.host'))
 check('No JavaScript exceptions',not errors)
 browser.close()
(OUT/'ui-terminal.json').write_text(json.dumps({'passed':True,'checks':len(checks),'details':checks,'errors':errors,'scope':'Chromium, actual UI and fictional storage. No GTK, terminal emulator or NAS.'},indent=2)+'\n')
