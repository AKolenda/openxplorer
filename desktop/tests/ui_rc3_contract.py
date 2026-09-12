#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Two real UI pages -> production dispatcher -> production transfer broker.
GTK drag transport is exercised separately by native_tab_drag.py. Storage and
xdg-mime are fixtures; nothing touches the user's desktop association files.
"""
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace as NS
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from core import VERSION
from window_state import tab_snapshot
from tab_transfers import TabTransfers
from native_tab_drag import layout_value
from desktop_integration import DesktopIntegration,TYPES,ZIP_TYPES,APP_ID
from tests.test_rc2 import ContractHost,space
from playwright.sync_api import sync_playwright,expect
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'test-results';OUT.mkdir(exist_ok=True)
checks=[];errors=[]
def check(name,value=True):
    assert value,name
    checks.append(name);print('PASS',name,flush=True)
space['tab_snapshot']=tab_snapshot
with tempfile.TemporaryDirectory() as tmp,sync_playwright() as pw:
    browser=pw.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
    base=browser.new_page();base.set_content((ROOT/'preview.html').read_text());base.wait_for_function('()=>OpenXplorer.state.ready')
    env=base.evaluate('()=>OpenXplorer.state.env');base.close()
    mapping={k:('org.kde.dolphin.desktop' if k in ZIP_TYPES else APP_ID) for k in TYPES+ZIP_TYPES}
    def run(args):
        if args[1]=='query':return mapping[args[3]]
        mapping[args[3]]=args[2];return ''
    defaults=DesktopIntegration(Path(tmp),run)
    events={1:[],2:[]};pages={};seen=[];closed=[]
    app=NS(controllers=[],focus_window=lambda i:True,transfer_available=lambda i:i in (1,2),reveal_status=lambda:{'revealEnabled':True,'revealOwned':False,'revealOwner':'dolphin'})
    app.tab_transfers=TabTransfers(lambda i,n,d:events[i].append([n,d]),app.transfer_available)
    app.window_list=lambda:[{'id':i,'title':('Source' if i==1 else 'Destination')+' — OpenXplorer','ready':True,'active':False} for i in (1,2)]
    hosts={}
    for ident in (1,2):
        h=ContractHost();h.app=app;h.desktop_integration=defaults;h.window=NS(get_id=lambda i=ident:i,close=lambda i=ident:closed.append(i));h.tab_drag=NS(update=layout_value,begin=lambda tab_id,snapshot,i=ident:{'started':True,'token':app.tab_transfers.offer(i,tab_id,snapshot)})
        hosts[ident]=h
    def bridge(ident,request):
        seen.append((ident,request));a=request['args'];m=request['method'];out=[]
        try:
            if m=='environment':value={**env,'version':VERSION,'windowId':ident,'nativeTabDrag':True,'startUri':env['home']}
            elif m=='list':
                value={'uri':a['uri'],'entries':[{'name':'Readme.md','uri':a['uri'].rstrip('/')+'/Readme.md','isDir':False,'kind':'file','size':500,'modified':1700000000,'type':'Markdown document','contentType':'text/markdown'}]}
            elif m=='cacheStatus':value={'roots':[]}
            elif m=='clipboardGet':value=None
            elif m in ('moveTabToWindow','tabTransferReady','beginTabDrag','tabDragLayout','desktopStatus','zipDefault','zipRestore','window','windows'):
                hosts[ident].dispatch(request);value=hosts[ident].result
            elif m in ('uiReady','chrome','windowMetadata','cancel','preferences'):value=True
            elif m=='folderLocations':value={'folders':[]}
            else:raise ValueError('Unexpected action: '+m)
            return {'value':value,'events':out}
        except Exception as exc:return {'error':{'code':'contract','message':str(exc)},'events':out}
    for ident in (1,2):
        page=browser.new_page(viewport={'width':1300,'height':900});page.set_default_timeout(7000);page.on('pageerror',lambda e:errors.append(str(e)))
        page.expose_function('testBridge',lambda req,i=ident:bridge(i,req))
        page.evaluate('''()=>{Object.defineProperty(window,'__OPENXPLORER_NATIVE__',{value:true});window.webkit={messageHandlers:{host:{postMessage:text=>window.testBridge(JSON.parse(text)).then(out=>{for(const [n,d]of out.events||[])window.__nativeEvent(n,d);window.__nativeResolve(JSON.parse(text).id,out.value,out.error||null);})}}};}''')
        page.set_content((ROOT/'preview.html').read_text());page.wait_for_function('()=>OpenXplorer.state.ready&&OpenXplorer.state.tabs[0].loaded');pages[ident]=page
    def drain():
        for _ in range(10):
            for i,p in pages.items():
                batch=events[i][:];events[i].clear()
                if batch:p.evaluate('(events)=>{for(const [name,data]of events)window.__nativeEvent(name,data)}',batch)
            pages[1].wait_for_timeout(30)
    one,two=pages[1],pages[2]
    # A second source tab so committing the moved one doesn't close the window.
    one.evaluate("()=>OpenXplorer.addTab('smb://studio-nas/Projects')")
    one.wait_for_function('()=>OpenXplorer.state.tabs.at(-1).loaded')
    source_id=one.evaluate('OpenXplorer.state.activeId')
    one.evaluate("()=>{const s=OpenXplorer.state,t=s.tabs.find(t=>t.id===s.activeId);t.history=['file:///home/demo','smb://studio-nas/Projects'];t.index=1;s.view='grid';s.selection=new Set(['smb://studio-nas/Projects/Readme.md']);}")
    one.locator('.tab.active').click(button='right');one.get_by_role('menuitem',name='Move tab to window…',exact=True).click()
    expect(one.get_by_role('menuitem',name='Destination — OpenXplorer',exact=True)).to_be_visible()
    check('Tab context menu lists the other window, not its own',one.get_by_role('menuitem',name='Source — OpenXplorer',exact=True).count()==0)
    one.screenshot(path=str(OUT/'move-tab-to-window.png'))
    one.get_by_role('menuitem',name='Destination — OpenXplorer',exact=True).click()
    check('Source tab is retained before destination acknowledgement',one.locator('[role=tab]').count()==2)
    check('Actual dispatcher accepts moveTabToWindow',any(r['method']=='moveTabToWindow' for _,r in seen))
    drain()
    check('Destination receives an additional tab',two.locator('[role=tab]').count()==2)
    check('Original source tab removed only after ACK',one.locator('[role=tab]').count()==1)
    check('Other source tab preserved',one.evaluate('OpenXplorer.state.tabs[0].uri')==env['home'])
    check('Snapshot URI preserved',two.evaluate('OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri')=='smb://studio-nas/Projects')
    check('Back history preserved',two.evaluate('OpenXplorer.state.tabs.at(-1).history')==['file:///home/demo','smb://studio-nas/Projects'])
    check('View preserved',two.evaluate('OpenXplorer.state.view')=='grid')
    check('Selection restored after listing',two.evaluate('[...OpenXplorer.state.selection]')==['smb://studio-nas/Projects/Readme.md'])
    check('No live transfer token after commit',not app.tab_transfers.pending)
    # Merge back in the opposite direction.
    ident=two.evaluate('OpenXplorer.state.activeId');two.evaluate('id=>{void OpenXplorer.moveTabToWindow(id,1)}',ident);drain()
    check('Round trip merges back into the original window',one.locator('[role=tab]').count()==2 and two.locator('[role=tab]').count()==1)
    check('Source unblocked after round trip',not one.evaluate('!!OpenXplorer.state.outgoingTab'))
    # Busy target refuses without consuming source.
    two.evaluate('()=>OpenXplorer.state.operation={test:true}')
    id1=one.evaluate('OpenXplorer.state.activeId');one.evaluate('id=>{void OpenXplorer.moveTabToWindow(id,2)}',id1);drain()
    check('Busy destination does not lose or duplicate the source tab',one.locator('[role=tab]').count()==2 and two.locator('[role=tab]').count()==1)
    check('Busy target clears the transfer lock',not one.evaluate('!!OpenXplorer.state.outgoingTab'))
    two.evaluate('()=>OpenXplorer.state.operation=null')
    one.evaluate('id=>{void OpenXplorer.moveTabToWindow(id,999)}',id1);drain()
    check('Closed/nonexistent window gives a safe failure',one.locator('[role=tab]').count()==2 and not app.tab_transfers.pending)
    # Exercise request names used by the real GTK adapter, plus explicit insertion.
    one.evaluate('(id)=>window.__nativeEvent("tabDragRequest",{id})',id1);one.wait_for_timeout(80)
    check('Native drag request reaches registered beginTabDrag action',bool(app.tab_transfers.pending))
    token=next(iter(app.tab_transfers.pending));before=two.evaluate('OpenXplorer.state.tabs[0].id');app.tab_transfers.claim(token,2,before);drain()
    check('Cross-window native-path drop inserts at requested position',two.evaluate('OpenXplorer.state.tabs[0].uri')=='smb://studio-nas/Projects')
    check('Native-path ACK retires source',one.locator('[role=tab]').count()==1)
    check('Geometry publication reaches registered tabDragLayout action',any(r['method']=='tabDragLayout' for _,r in seen))
    # Association UI uses real production default logic with isolated runner.
    one.keyboard.press('Control+,');one.wait_for_function('()=>OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri==="settings:"');one.locator('#settings-default').scroll_into_view_if_needed()
    expect(one.locator('#default-status')).to_contain_text('ZIP files')
    expect(one.locator('#zip-status')).to_contain_text('dolphin')
    check('Folder default does not falsely claim ZIP ownership',one.locator('#default-status [data-kind="inode/directory"]').inner_text().endswith('OpenXplorer') and 'dolphin' in one.locator('#default-status [data-kind="application/zip"]').inner_text())
    check('ZIP opt-in starts unchecked',not one.locator('#include-zip').is_checked())
    expect(one.locator('#reveal-status')).to_contain_text('waiting for dolphin')
    check('FileManager1 owner reported separately from MIME defaults')
    one.get_by_role('button',name='Use OpenXplorer for ZIPs',exact=True).click();expect(one.locator('#zip-status')).to_contain_text('ZIP opening: OpenXplorer')
    check('ZIP button changes actual backend associations for all ZIP types',all(mapping[k]==APP_ID for k in ZIP_TYPES))
    check('ZIP setting does not alter folder routes',all(mapping[k]==APP_ID for k in TYPES))
    one.get_by_role('button',name='Restore ZIP handler',exact=True).click();expect(one.locator('#zip-status')).to_contain_text('dolphin')
    check('Restore ZIP handler restores previous association',mapping['application/zip']=='org.kde.dolphin.desktop')
    one.screenshot(path=str(OUT/'default-routes.png'))
    check('No JavaScript errors in either window',not errors)
    check('All native requests include the current release',all(r['release']==VERSION for _,r in seen))
    browser.close()
(OUT/'ui-rc3-contract.json').write_text(json.dumps({'scope':'Two Chromium UI pages, actual Python dispatcher and transfer broker, fixture filesystem and xdg-mime; native GTK transport tested separately','passed':len(checks),'checks':checks},indent=2))
