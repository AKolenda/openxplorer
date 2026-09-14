#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Actual UI/pointer/keyboard regressions; native handoffs and browser prefs simulated."""
import json,os,re,time,traceback
from pathlib import Path
from playwright.sync_api import sync_playwright,expect
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'test-results';OUT.mkdir(exist_ok=True)

def main():
    checks=[];errors=[];requests=[];report={};status=1;start=time.monotonic()
    with sync_playwright() as pw:
        browser=pw.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
        page=browser.new_page(viewport={'width':1440,'height':940});page.set_default_timeout(7000)
        page.on('pageerror',lambda e:errors.append(str(e)));page.on('request',lambda r:requests.append(r.url))
        page.evaluate("""()=>{const s={};Object.defineProperty(window,'localStorage',{value:{getItem:k=>s[k]??null,setItem:(k,v)=>s[k]=String(v),removeItem:k=>delete s[k]}})}""")
        def check(label,value=True):
            assert value,label;checks.append(label);print('PASS',label,flush=True)
        def ready(uri=None):page.wait_for_function("u=>{const s=window.OpenXplorer?.state,t=s?.tabs.find(t=>t.id===s.activeId);return s?.ready&&t?.loaded&&!t.busy&&(!u||u===t.uri)}",arg=uri)
        def nav(uri):page.evaluate('u=>{void OpenXplorer.navigate(u)}',uri);ready(uri)
        def active():return page.evaluate('OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri')
        def setting(q):page.locator('#settings-search-input').fill(q);page.wait_for_timeout(80)
        try:
            page.set_content((ROOT/'preview.html').read_text(),wait_until='load');ready()
            page.evaluate("()=>OpenXplorer.applyTheme('dark',false)")
            check('Version 1.0.0 is visible','1.0.0' in page.locator('#status-right').inner_text() if page.locator('#status-right').count() else '1.0.0' in page.locator('body').inner_text())
            share='smb://archive-nas/work'
            nav('smb://archive-nas/');nav(share)
            entry=page.locator('#sidebar [data-uri="'+share+'"][data-network-saved]')
            expect(entry).to_have_count(1)
            check('Browsing an unsaved share adds a Network row',entry.get_attribute('data-network-saved')=='false')
            check('Network entry has green share indicator',entry.locator('.shared-bar').count()==1)
            check('Browsing alone does not create a durable bookmark',page.evaluate('u=>!OpenXplorer.state.env.shares.some(s=>s.uri===u)',share))
            check('Visited server also appears under Network',page.locator('#sidebar [data-uri="smb://archive-nas/"][data-network-saved]').count()==1)
            entry.click(button='right');page.get_by_role('menuitem',name='Keep in Network',exact=True).click()
            page.wait_for_function('u=>OpenXplorer.state.env.shares.some(s=>s.uri===u)',arg=share)
            check('Keep in Network explicitly persists location',entry.get_attribute('data-network-saved')=='true')
            entry.click(button='right');check('Saved network row includes sign-out',page.get_by_role('menuitem',name='Sign out of server…',exact=True).count()==1);page.keyboard.press('Escape')
            nav('network:');expect(page.get_by_text('Connected & saved locations',exact=True)).to_be_visible()
            check('Network landing page uses connected and saved union')
            page.screenshot(path=str(OUT/'network-connected.png'))
            # Search and cache input use real keyboard entry, no replacing visible UI.
            page.keyboard.press('Control+,');ready('settings:')
            check('Settings is still a dedicated tab',page.locator('#modal-layer').is_hidden())
            check('Open windows button remains accessible in Settings',page.locator('#windows-button').is_visible())
            check('Settings DOM has unique identifiers',page.evaluate('()=>{const ids=[...document.querySelectorAll("[id]")].map(e=>e.id);return ids.length===new Set(ids).size}'))
            check('Settings has search and five navigation sections',page.locator('.settings-nav-link').count()==5 and page.locator('#settings-search-input').is_visible())
            setting('custom folder');expect(page.locator('.settings-search-result')).to_have_count(1)
            page.keyboard.press('Enter');page.wait_for_timeout(350)
            add=page.locator('.cache-add');expect(add).to_have_class(re.compile('settings-target'))
            check('Settings Enter highlights the matching custom-folder control')
            inp=add.locator('input');inp.scroll_into_view_if_needed();width=inp.bounding_box()['width']
            check('Custom path input has usable width at desktop size',width>350)
            check('Custom path input has normal input height',inp.bounding_box()['height']>=36)
            example='/media/demo/Archive/a very long folder name/Backups/2026';inp.fill(example)
            check('Custom input retains full untruncated value',inp.input_value()==example)
            check('Highlight uses accent border, without changing text',add.evaluate("e=>getComputedStyle(e).outlineStyle==='solid'"))
            page.screenshot(path=str(OUT/'settings-search-custom-folder.png'))
            setting('Brave');check('Search finds both browser defaults and Downloads controls',page.locator('.settings-search-result').count()>=2)
            page.get_by_role('button',name='Brave download location',exact=True).click();expect(page.locator('#settings-brave')).to_have_class(re.compile('settings-target'))
            check('Clicking a result scrolls and highlights its section')
            setting('this setting does not exist');expect(page.locator('#settings-search-results')).to_contain_text('No matching settings');check('Unknown query reports no matches')
            page.locator('#settings-search-input').press('Escape');expect(page.locator('#settings-search-results')).to_be_hidden();check('Escape clears search and highlights',page.locator('.settings-match').count()==0)
            setting('watch live');expect(page.locator('.settings-search-result')).to_have_count(1);check('Search matches descriptive keywords as well as labels')
            page.locator('#settings-search-input').press('Escape')
            # Compact window: no horizontal overflow; the Add button can wrap.
            page.set_viewport_size({'width':780,'height':800});add.scroll_into_view_if_needed()
            check('Input remains readable at narrow window width',inp.bounding_box()['width']>180)
            check('Custom row does not overflow section',add.evaluate('e=>e.scrollWidth<=e.clientWidth+1'))
            page.set_viewport_size({'width':1440,'height':940})
            page.locator('#windows-button').click();expect(page.get_by_role('menuitem',name='New window',exact=True)).to_be_visible()
            check('Window picker works while Settings is active',page.get_by_role('menuitem',name='Quit OpenXplorer',exact=True).count()==1)
            check('Window picker contains the current settings window',page.get_by_role('menuitem',name=re.compile('Settings.*OpenXplorer')).count()==1)
            page.keyboard.press('Escape')
            # Opt-in defaults, service status controls.
            setting('show in folder');page.keyboard.press('Enter');page.wait_for_timeout(250)
            consent=page.locator('#include-reveal');check('Default action offers Show in folder integration',consent.is_checked())
            page.get_by_role('button',name='Enable Show in folder',exact=True).click();expect(page.locator('#reveal-status')).to_contain_text('owns FileManager1')
            check('Reveal integration has a status distinct from MIME defaults')
            check('Reveal integration has explicit test and disable controls',page.get_by_role('button',name='Test Show in folder',exact=True).count()==1 and page.get_by_role('button',name='Disable Show in folder',exact=True).count()==1)
            # Explicit one-time Brave sync, simulated app/prefs transport here.
            page.locator('#settings-search-input').press('Escape');page.evaluate('()=>{void OpenXplorer.braveDialog("/home/demo/Downloads")}')
            expect(page.locator('.brave-profile')).to_have_count(1)
            check('Brave sync shows selected destination',page.locator('.brave-destination').inner_text()=='/home/demo/Downloads')
            check('Brave sync confirmation starts unchecked',not page.locator('#confirm-brave').is_checked())
            page.get_by_role('button',name='Apply to Brave',exact=True).click();expect(page.locator('#modal-layer')).to_be_visible();check('No profile change without consent')
            page.evaluate("""()=>{window.__calls=[];window.__originalCall=OpenXplorer.previewTransport.call;OpenXplorer.previewTransport.call=function(m,a){window.__calls.push({m,a});return window.__originalCall.call(this,m,a);}}""")
            page.locator('#confirm-brave').check();page.get_by_role('button',name='Apply to Brave',exact=True).click();expect(page.locator('#modal-layer')).to_be_hidden()
            payload=page.evaluate("__calls.find(x=>x.m==='braveSync').a")
            check('Sync sends only chosen profiles, destination and explicit consent',payload=={'profiles':['Brave-Browser:Default'],'path':'/home/demo/Downloads','confirmed':True})
            # Downloads relocation opens browser sync only after explicit opt-in.
            page.evaluate("()=>{void OpenXplorer.propertiesDialog({uri:'file:///home/demo/Downloads',name:'Downloads',isDir:true},'location')}")
            expect(page.locator('#sync-brave-location')).to_be_visible()
            check('Downloads-to-Brave follow-up starts opt-out',not page.locator('#sync-brave-location').is_checked())
            page.locator('#folder-location').fill('/home/demo/Downloads');page.locator('#check-location').click();page.locator('#confirm-location').check();page.locator('#sync-brave-location').check()
            expect(page.locator('#apply-location')).to_be_enabled();page.locator('#apply-location').click()
            expect(page.locator('.brave-profile')).to_have_count(1)
            check('Applying opted-in Downloads location opens profile sync',page.locator('.brave-destination').inner_text()=='/home/demo/Downloads')
            check('Browser consent still required after Linux-folder consent',not page.locator('#confirm-brave').is_checked())
            page.get_by_role('button',name='Cancel',exact=True).click();expect(page.locator('#modal-layer')).to_be_hidden()
            # ShowItems must reveal the parent of the file, not reinterpret PDF as directory.
            page.evaluate("()=>{void OpenXplorer.handleFileManagerRequest({method:'ShowItems',uris:['file:///home/demo/Documents/Brand%20guidelines.pdf']})}");ready('file:///home/demo/Documents')
            check('External file-reveal opens the parent directory',active()=='file:///home/demo/Documents')
            check('Existing revealed PDF is visibly selected',page.locator('.file-row.selected .name-text').inner_text()=='Brand guidelines.pdf')
            check('External file-reveal preserves exact selected URI',page.evaluate("OpenXplorer.state.selection.has('file:///home/demo/Documents/Brand%20guidelines.pdf')"))
            page.evaluate("()=>{void OpenXplorer.handleFileManagerRequest({method:'ShowFolders',uris:['file:///home/demo/Downloads']})}");ready('file:///home/demo/Downloads')
            check('ShowFolders opens a directory in a file tab, not Settings')
            # Detach via real pointer capture; default preview keeps original.
            original_id=page.evaluate('OpenXplorer.state.activeId');count=page.locator('.tab').count();box=page.locator('.tab.active').bounding_box()
            page.mouse.move(box['x']+45,box['y']+15);page.mouse.down();page.mouse.move(box['x']+65,box['y']+130,steps=10)
            expect(page.locator('#tab-drag-hint')).to_be_visible();check('Dragging tab below strip shows release affordance')
            page.mouse.up();page.wait_for_function('()=>!OpenXplorer.state.detaching');check('Preview detach keeps source tab because no native window exists',page.locator('.tab').count()==count and page.evaluate('OpenXplorer.state.activeId')==original_id)
            check('Tab transfer includes actual location and history',page.evaluate('OpenXplorer.state.lastDetachedTab.uri')=='file:///home/demo/Downloads')
            check('Drag indicator clears on release',page.locator('#tab-drag-hint').is_hidden())
            # Reject / then acknowledge handoff, explicitly simulated at the transport seam.
            page.evaluate("""()=>{OpenXplorer.previewTransport.call=function(m,a){if(m==='detachTab')throw Error('Simulated window could not start');return window.__originalCall.call(this,m,a);}}""")
            page.evaluate('id=>OpenXplorer.detachTab(id)',original_id)
            check('Failed native handoff does not lose source tab',page.locator('.tab').count()==count)
            page.locator('.tab.active').click(button='right');check('Tab menu offers explicit move fallback',page.get_by_role('menuitem',name='Move tab to new window',exact=True).count()==1);page.keyboard.press('Escape')
            page.evaluate("""()=>{OpenXplorer.previewTransport.call=function(m,a){if(m==='detachTab')return Promise.resolve({ready:true,windowId:7});return window.__originalCall.call(this,m,a);}}""")
            page.evaluate('id=>OpenXplorer.detachTab(id)',original_id)
            check('Source tab removed only after ready acknowledgment',page.locator('.tab').count()==count-1 and not page.evaluate('id=>OpenXplorer.state.tabs.some(t=>t.id===id)',original_id))
            page.evaluate('()=>{OpenXplorer.previewTransport.call=window.__originalCall;}')
            transferred={'uri':'file:///home/demo/Documents','history':['file:///home/demo','file:///home/demo/Documents'],'index':1,'scroll':0,'selection':['file:///home/demo/Documents/Brand%20guidelines.pdf'],'view':'details','sort':'size','descending':True,'settingsSection':None}
            page.evaluate('x=>OpenXplorer.restoreTransferredTab(x)',transferred);ready(transferred['uri'])
            check('Transferred tab restores its navigation history',page.evaluate('OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).history')==transferred['history'])
            check('Transferred tab restores selection and sort',page.evaluate("OpenXplorer.state.selection.has('file:///home/demo/Documents/Brand%20guidelines.pdf')&&OpenXplorer.state.sort==='size'&&OpenXplorer.state.descending"))
            # Snapshot modal ownership and consent disallow detach to protect work.
            nav('smb://archive-nas/Shared');page.evaluate("()=>{void OpenXplorer.propertiesDialog({uri:'smb://archive-nas/Shared',isDir:true,name:'Shared'})}")
            expect(page.locator('#modal-layer')).to_be_visible();count=page.locator('.tab').count();page.evaluate('()=>OpenXplorer.detachTab(OpenXplorer.state.activeId)')
            check('Open properties protects its tab from accidental detach',page.locator('.tab').count()==count and page.locator('#modal-layer').is_visible());page.keyboard.press('Escape')
            # Final settings screenshot and benign reveal after settings.
            page.keyboard.press('Control+,');ready('settings:');setting('Brave');page.get_by_role('button',name='Brave download location',exact=True).click();page.wait_for_timeout(500)
            page.screenshot(path=str(OUT/'settings-search-brave.png'))
            check('Opening Settings repeatedly reuses its tab',page.evaluate('OpenXplorer.state.tabs.filter(t=>t.uri==="settings:").length')==1)
            check('No JavaScript errors',not errors);check('No external network requests from preview',not requests)
            status=0
        except Exception as e:
            traceback.print_exc();report['failure']=str(e);page.screenshot(path=str(OUT/'v07-failure.png'))
        finally:
            report.update(success=status==0,count=len(checks),checks=checks,errors=errors,requests=requests,elapsedSeconds=round(time.monotonic()-start,2),scope='Chromium actual pointer and keyboard UI; simulated filesystem/Brave/desktop transport. Does not execute native GTK, D-Bus, live SMB or browser preferences.')
            (OUT/'ui-v07.json').write_text(json.dumps(report,indent=2)+'\n');browser.close()
    return status
if __name__=='__main__':raise SystemExit(main())
