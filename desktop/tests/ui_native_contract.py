#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Actual UI bridge -> actual production dispatcher -> disposable local ZIP I/O.
No GI/GTK, real terminal or SMB here. Complements native-target tests.
"""
import json
import os
from pathlib import Path
import tempfile
import sys
import zipfile
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from core import VERSION,normalise_location
from tests.test_rc2 import ContractHost,space
from terminal_integration import prepare_directory
from playwright.sync_api import sync_playwright,expect
D=Path(__file__).resolve().parents[1];OUT=D/'test-results'
checks=[]
def check(name,value=True):
    assert value,name
    checks.append(name)
with tempfile.TemporaryDirectory(prefix='openxplorer-contract-') as tmp,sync_playwright() as pw:
    directory=Path(tmp);src=directory/'Sample assets.zip'
    with zipfile.ZipFile(src,'w',zipfile.ZIP_DEFLATED) as z:
        z.writestr('Artwork/notes.txt','A fictional example.');z.writestr('readme.md','# Sample project')
    browser=pw.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
    base=browser.new_page();base.set_content((D/'preview.html').read_text())
    base.wait_for_function('()=>window.OpenXplorer?.state.ready')
    env=base.evaluate('()=>OpenXplorer.state.env');base.close()
    env.update(home=directory.as_uri(),startUri=directory.as_uri(),quick=[],shares=[],mounts=[],networkLocations=[],version=VERSION)
    host=ContractHost();seen=[];bad={'summary':False,'version':False}
    space['prepare_directory']=prepare_directory;space['launch_terminal']=lambda data:{**data,'opened':True,'terminal':'test recorder'}
    def bridge(request):
        seen.append(request);a=request['args'];m=request['method'];events=[]
        try:
            if m=='environment':value={**env,'version':'0.7.0' if bad['version'] else VERSION}
            elif m=='list':
                from urllib.parse import unquote,urlsplit
                folder=Path(unquote(urlsplit(a['uri']).path));rows=[]
                for f in folder.iterdir():
                    rows.append({'uri':f.as_uri(),'name':f.name,'isDir':f.is_dir(),'kind':'directory' if f.is_dir() else 'file','size':None if f.is_dir() else f.stat().st_size,'contentType':'application/zip' if f.suffix=='.zip' else 'text/plain','type':'File folder' if f.is_dir() else 'File','modified':f.stat().st_mtime})
                value={'uri':a['uri'],'count':len(rows)};events=[['entries',{'token':a['token'],'entries':rows}]]
            elif m=='cacheStatus':value={'roots':[]}
            elif m=='clipboardGet':value=None
            elif m=='normalise':value={'uri':normalise_location(a['value'],a.get('base'))}
            elif m=='archiveInspect' and bad['summary']:value={'uri':a['uri'],'files':None}
            elif m in ('archiveInspect','archiveExtract','openTerminal'):
                host.events.clear();host.dispatch(request);value=host.result;events=host.events[:]
            elif m in ('open','quit','uiReady','chrome','windowMetadata','cancel'):value=True
            else:raise ValueError('Unexpected test request: '+m)
            return {'value':value,'events':events}
        except Exception as e:return {'error':{'code':'test-error','message':str(e)},'events':events}
    def new_page():
        p=browser.new_page(viewport={'width':1320,'height':900});p.set_default_timeout(6000)
        p.expose_function('testBridge',bridge)
        p.evaluate('''()=>{Object.defineProperty(window,'__OPENXPLORER_NATIVE__',{value:true});window.webkit={messageHandlers:{host:{postMessage:text=>window.testBridge(JSON.parse(text)).then(out=>{for(const [n,d]of out.events||[])window.__nativeEvent(n,d);window.__nativeResolve(JSON.parse(text).id,out.value,out.error||null);})}}};}''')
        p.set_content((D/'preview.html').read_text());return p
    page=new_page();page.wait_for_function('()=>OpenXplorer.state.ready&&OpenXplorer.state.tabs[0].loaded')
    check('Actual native-transport UI starts against current host response')
    page.locator('.file-row').filter(has_text='Sample assets.zip').click(button='right')
    page.get_by_role('menuitem',name='Extract all…',exact=True).click()
    expect(page.locator('.extract-summary')).to_contain_text('2 files')
    check('Actual Python archiveInspect summary renders in UI')
    page.get_by_label('New folder name').fill('Extracted sample')
    page.get_by_role('button',name='Extract',exact=True).click()
    page.wait_for_function('()=>OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri.endsWith("Extracted%20sample")')
    check('Actual archiveExtract response navigates to published directory',(directory/'Extracted sample/Artwork/notes.txt').read_text()=='A fictional example.')
    check('Original ZIP unchanged and readable',zipfile.is_zipfile(src))
    page.locator('#file-canvas').click(position={'x':450,'y':400},button='right')
    page.get_by_role('menuitem',name='Open in Terminal',exact=True).click()
    page.wait_for_timeout(100)
    check('Actual dispatcher handles openTerminal request',any(x['method']=='openTerminal' for x in seen))
    page.evaluate('(entry)=>{void OpenXplorer.extractDialog(entry)}',{'uri':src.as_uri(),'name':src.name,'isDir':False})
    expect(page.locator('.extract-summary')).to_contain_text('2 files')
    page.get_by_role('button',name='Open in archive manager',exact=True).click();expect(page.locator('#modal-layer')).to_be_hidden()
    check('Archive manager button delegates through native open action',seen[-1]['method']=='open' or any(x['method']=='open' for x in seen))
    bad['summary']=True
    page.evaluate('(entry)=>{void OpenXplorer.extractDialog(entry)}',{'uri':src.as_uri(),'name':src.name,'isDir':False})
    expect(page.locator('.extract-summary')).to_contain_text('incomplete response')
    check('Malformed native summary gives actionable error, not a JavaScript TypeError')
    before=sum(x['method']=='archiveExtract' for x in seen)
    page.get_by_role('button',name='Extract',exact=True).click();expect(page.locator('#modal')).to_contain_text('Wait for the ZIP check')
    check('Malformed summary cannot start extraction',sum(x['method']=='archiveExtract' for x in seen)==before)
    page.close();bad['version']=True
    page=new_page();expect(page.locator('#main')).to_contain_text('running application is 0.7.0')
    check('Old backend and new UI mismatch is visible before browsing')
    rejected=page.evaluate('()=>OpenXplorer.call("archiveExtract",{}).then(()=>false,e=>e.message.includes("restart"))')
    check('Mismatch blocks modifying requests',rejected)
    check('Requests carry explicit release identity',all(x.get('release')==VERSION for x in seen))
    browser.close()
(OUT/'ui-native-contract.json').write_text(json.dumps({'passed':True,'checks':len(checks),'details':checks,'scope':'Actual JS UI and production Python dispatcher/extractor through a test transport with disposable local filesystem provider. Not GI/WebKit/SMB.'},indent=2)+'\n')
print('Passed',len(checks),'UI/native-contract checks')
