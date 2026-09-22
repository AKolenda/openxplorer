#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""File drag UI contract in Chromium with fictional data and a native bridge stub.

Real selection and pointer events exercise UI behavior. GTK gesture negotiation,
external app acceptance, native WebKit and SMB are deliberately outside scope.
Run tools/build_preview.py first. Does not capture screenshots or mutate files.
"""
import json
import os
import sys
from pathlib import Path
from types import SimpleNamespace as NS
from unittest.mock import Mock

from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from native_file_drag import NativeFileDrag, file_uri
HOME = 'file:///home/demo/Documents'
OUT = ROOT / 'test-results'
checks = []
errors = []
seen = []


def check(name, condition=True):
    assert condition, name
    checks.append(name)
    print('PASS', name, flush=True)


def entry(name, **extra):
    return {'uri': HOME + '/' + name, 'name': name, 'isDir': False,
            'kind': 'file', 'type': 'Text document', 'size': 10, 'modified': 0, **extra}


entries = [entry('Alpha.txt'), entry('Bravo.txt'),
           entry('Folder', isDir=True, kind='directory', type='File folder'),
           entry('Read-only', isDir=True, readOnly=True),
           entry('Socket', kind='special'),
           entry('Virtual.txt', isVirtual=True),
           entry('Zip-member.txt', archiveMember='Zip-member.txt', archiveUri=HOME+'/Sample.zip')]
punctuated = [entry('Meeting (1).mp4', uri=HOME+'/Meeting%20(1).mp4'),
              entry('Meeting (2026-09-01) - Transcript.docx',
                    uri=HOME+'/Meeting%20(2026-09-01)%20-%20Transcript.docx')]
entries.extend(punctuated)
native_drag = None
native_events = []

with sync_playwright() as pw:
    launch = {'args': ['--no-sandbox']}
    if os.environ.get('CHROMIUM'):
        launch['executable_path'] = os.environ['CHROMIUM']
    browser = pw.chromium.launch(**launch)
    sample = browser.new_page()
    sample.set_content((ROOT/'preview.html').read_text())
    sample.wait_for_function('()=>OpenXplorer.state.ready')
    env = sample.evaluate('OpenXplorer.state.env')
    sample.close()
    env.update(home=HOME, startUri=HOME, version='1.1.3', nativeFileDrag=True,
               quick=[{'uri':'file:///home/demo/Pictures','label':'Pictures'}],
               mounts=[], shares=[], networkLocations=[])

    def bridge(request):
        seen.append(request)
        method, args = request['method'], request['args']
        events, result = [], True
        if method == 'environment':
            result = env
        elif method == 'list':
            result = {'uri':args['uri'], 'count':len(entries)}
            events = [['entries', {'token':args['token'], 'entries':entries}]]
        elif method == 'transferConflicts':
            result = {'conflicts':[uri for uri in args['uris'] if uri.endswith('/Report.txt')]}
        elif method == 'cacheStatus':
            result = {'roots':[]}
        elif method == 'clipboardGet':
            result = None
        elif method == 'trashSupport':
            result = {'canTrash':True}
        elif method == 'beginFileDrag':
            if native_drag is not None:
                result = native_drag.begin(args['uri'], args['uris'])
                events = list(native_events)
                native_events.clear()
            else:
                result = {'started':True, 'count':len(args['uris']),
                          'remoteOnly':sum(uri.startswith('smb:') for uri in args['uris'])}
                events = [['fileDragStarted', {'uris':args['uris']}]]
        elif method == 'operate':
            result = {'done':args['uris'], 'errors':[], 'skipped':[]}
        return {'value':result, 'events':events}

    page = browser.new_page(viewport={'width':1320, 'height':850})
    page.set_default_timeout(6000)
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.expose_function('testBridge', bridge)
    page.evaluate('''()=>{Object.defineProperty(window,'__OPENXPLORER_NATIVE__',{value:true});
        window.webkit={messageHandlers:{host:{postMessage:text=>{
          const request=JSON.parse(text);window.testBridge(request).then(out=>{
            for(const [n,d] of out.events||[])window.__nativeEvent(n,d);
            window.__nativeResolve(request.id,out.value,null);
          });
        }}}};}''')
    page.set_content((ROOT/'preview.html').read_text())
    page.wait_for_function('()=>OpenXplorer.state.ready&&OpenXplorer.state.tabs[0].loaded')

    def row(name):
        return page.get_by_role('row', name=name, exact=True)

    def calls(method):
        return [request['args'] for request in seen if request['method'] == method]

    def layout():
        page.wait_for_timeout(60)
        return calls('fileDragLayout')[-1]

    def emit(name, data):
        page.evaluate('([name,data])=>window.__nativeEvent(name,data)', [name,data])
        page.wait_for_timeout(40)

    def finish_drag():
        emit('fileDragFinished', {'cancelled':True})
        page.wait_for_timeout(450)

    source_uris = [item['uri'] for item in layout()['items']]
    check('Real files and folders publish native source geometry',
          HOME+'/Alpha.txt' in source_uris and HOME+'/Folder' in source_uris)
    check('Virtual and ZIP member entries cannot export bogus paths',
          HOME+'/Virtual.txt' not in source_uris and HOME+'/Zip-member.txt' not in source_uris)
    check('Special filesystem objects cannot start a native file drag', HOME+'/Socket' not in source_uris)
    check('Folder copy targets precede the background target',
          next(i for i,t in enumerate(layout()['targets']) if t.get('uri')==HOME+'/Folder') <
          len(layout()['targets'])-1 and layout()['targets'][-1].get('uri')==HOME)
    check('Read-only folders have no copy destination',
          not any(t.get('uri')==HOME+'/Read-only' for t in layout()['targets']))
    quick = [t for t in layout()['targets'] if t['kind']=='pin']
    check('Quick access publishes insertion and end pin targets',
          len(quick)==2 and quick[0]['before']=='file:///home/demo/Pictures' and quick[1]['before'] is None)
    check('Native source disables HTML dragging', row('Alpha.txt').get_attribute('draggable')=='false')

    # Exercise production layout -> request -> real UI lookup -> production
    # begin/feedback. Only GTK's device/transport calls are simulated here.
    view = Mock()
    view.get_allocated_width.return_value = 1320
    view.drag_check_threshold.return_value = True
    controller = NS(webview=view, writes=0, ui_ready=True, tab_drag=None,
                    emit=lambda name, data: native_events.append([name, data]))
    native_drag = NativeFileDrag(controller, NS(TargetList=NS(new=lambda _: Mock()),
        drag_set_icon_name=Mock(), drag_cancel=Mock()),
        NS(DragAction=NS(COPY=2), ModifierType=NS(BUTTON1_MASK=256),
           EventMask=NS(BUTTON_PRESS_MASK=1, BUTTON_RELEASE_MASK=2, POINTER_MOTION_MASK=4)), None)
    for item in punctuated:
        row(item['name']).click()
        row('Alpha.txt').click(modifiers=['Control'])
        native_drag.update(layout())
        region = next(r for r in native_drag.layout['items'] if r['uri'] == item['uri'])
        event = NS(button=1, state=256, x=region['left']+20, y=(region['top']+region['bottom'])/2)
        event.copy = lambda: event
        native_drag.button_press(view, event)
        native_drag.pointer_motion(view, event)
        request = native_events.pop()
        before = len(calls('beginFileDrag'))
        emit(*request)
        check('Punctuated filename starts native drag with the complete selection: '+item['name'],
              len(calls('beginFileDrag')) == before+1 and
              set(calls('beginFileDrag')[-1]['uris']) == {item['uri'],HOME+'/Alpha.txt'})
        check('Native feedback retains both selected row identities: '+item['name'],
              page.locator('.file-dragging .drag-source').count() == 2 and
              set(native_drag.uris) == {file_uri(item['uri']),HOME+'/Alpha.txt'})
        native_drag.drag_end(view, native_drag.context)
        native_events.clear()
        finish_drag()
    native_drag.close()
    native_drag = None

    row('Alpha.txt').click()
    row('Bravo.txt').click(modifiers=['Control'])
    emit('fileDragRequest', {'uri':HOME+'/Alpha.txt'})
    check('Dragging a selected file exports the whole selection',
          calls('beginFileDrag')[-1]['uris']==[HOME+'/Alpha.txt', HOME+'/Bravo.txt'])
    check('Selection source feedback is visible', page.locator('.file-dragging .drag-source').count()==2)
    before_open = len(calls('activateItem'))
    row('Alpha.txt').dispatch_event('dblclick')
    check('Drag completion cannot accidentally double-open an item', len(calls('activateItem'))==before_open)
    emit('fileDragFinished', {'cancelled':True})
    row('Bravo.txt').dispatch_event('click')
    check('Release click after native drag end preserves the multi-selection',
          page.evaluate('[...OpenXplorer.state.selection]')==[HOME+'/Alpha.txt',HOME+'/Bravo.txt'])
    finish_drag()
    check('Cancelled native drag clears source feedback', page.locator('.drag-source').count()==0)

    emit('fileDragRequest', {'uri':HOME+'/Folder'})
    check('Dragging an unselected item replaces prior multi-selection',
          calls('beginFileDrag')[-1]['uris']==[HOME+'/Folder'] and
          page.evaluate('[...OpenXplorer.state.selection]')==[HOME+'/Folder'])
    finish_drag()

    row('Alpha.txt').click()
    row('Zip-member.txt').click(modifiers=['Control'])
    before = len(calls('beginFileDrag'))
    emit('fileDragRequest', {'uri':HOME+'/Alpha.txt'})
    check('Mixed selection containing a ZIP member rejects the entire drag', len(calls('beginFileDrag'))==before)
    row('Alpha.txt').click()
    box = row('Alpha.txt').bounding_box()
    page.mouse.move(box['x']+20,box['y']+15)
    page.mouse.down()
    page.mouse.move(box['x']+70,box['y']+15,steps=4)
    check('Native rows do not start the competing pin pointer drag',
          page.evaluate('!OpenXplorer.state.drag&&!OpenXplorer.state.pointerPending'))
    page.mouse.up()

    emit('fileDrop', {'kind':'copy','target':HOME+'/Folder','uris':['file:///home/demo/Downloads/Report.txt']})
    expect(page.get_by_role('heading',name='Items already exist')).to_be_visible()
    check('Drop waits for conflict policy before copying', not calls('operate'))
    check('Dialog clears native source and target geometry', not layout()['items'] and not layout()['targets'])
    before = len(calls('beginFileDrag'))
    emit('fileDragRequest', {'uri':HOME+'/Alpha.txt'})
    check('Native drag requests are rejected while a modal is open', len(calls('beginFileDrag'))==before)
    check('Drop conflict dialog offers Windows-style replace and skip choices',
          page.get_by_role('button',name='Replace existing',exact=True).count()==1 and
          page.get_by_role('button',name='Skip duplicates',exact=True).count()==1)
    page.get_by_role('button',name='Replace existing',exact=True).click()
    page.wait_for_function('()=>!OpenXplorer.state.operation&&document.getElementById("modal-layer").hidden')
    check('Copy drop reuses the copy operation and captured folder destination',
          calls('operate')[-1]['mode']=='copy' and calls('operate')[-1]['target']==HOME+'/Folder' and
          calls('operate')[-1]['policy']=='replace')
    check('Drop operation never uses clipboard cut/move state', not calls('clipboardConsume'))
    check('Native geometry returns when the operation finishes', bool(layout()['items']))

    before = len(calls('operate'))
    emit('fileDrop', {'kind':'copy','target':HOME+'/Folder','uris':['file:///home/demo/Downloads/New.txt']})
    page.wait_for_function('()=>!OpenXplorer.state.transferPlanning&&!OpenXplorer.state.operation')
    check('Drop without conflicts copies immediately without a dialog',
          len(calls('operate'))==before+1 and page.locator('#modal-layer').is_hidden())
    check('No-conflict copy protects names that appear after the check', calls('operate')[-1]['policy']=='skip')

    emit('fileDrop', {'kind':'pin','before':'file:///home/demo/Pictures','uris':[HOME+'/Folder']})
    check('Quick access drop reuses validated pin API and insertion position',
          calls('pin')[-1]['items'][0]['uri']==HOME+'/Folder' and calls('pin')[-1]['before']=='file:///home/demo/Pictures')
    before = len(calls('operate'))
    emit('fileDrop', {'kind':'copy','target':HOME+'/.snapshot/old','uris':[HOME+'/Alpha.txt']})
    check('Snapshot destinations reject dropped copies before confirmation',
          page.locator('#modal-layer').is_hidden() and len(calls('operate'))==before)
    emit('fileDrop', {'kind':'copy','target':HOME,'uris':['https://example.invalid/file.txt']})
    check('Web links are rejected as file payloads', page.locator('#modal-layer').is_hidden())

    page.evaluate('''()=>{const s=OpenXplorer.state;s.query='Alpha';s.searchResults=s.tabs[0].entries;
        s.filterCache=null;window.dispatchEvent(new Event('resize'));}''')
    check('Search results have no ambiguous background drop target',
          not any(t.get('uri')==HOME and t['left']>=page.locator('#file-scroll').bounding_box()['x'] for t in layout()['targets']))
    page.evaluate('''()=>{const s=OpenXplorer.state;s.query='';s.searchResults=null;s.filterCache=null;
        window.dispatchEvent(new Event('resize'));}''')
    page.locator('#status-grid').click()
    check('Large-icon view publishes file source rectangles', HOME+'/Alpha.txt' in [i['uri'] for i in layout()['items']])
    check('Source geometry stays bounded to the viewport', all(
          0<=i['left']<i['right']<=1320 and 0<=i['top']<i['bottom']<=850 for i in layout()['items']))
    page.evaluate('()=>{OpenXplorer.state.env.quick=[];OpenXplorer.renderSidebar();}')
    check('Empty Quick access remains a visible native pin destination',
          any(t['kind']=='pin' for t in layout()['targets']) and page.locator('.quick-drop-tail').is_visible())
    page.evaluate('''()=>{OpenXplorer.state.env.quick=[{uri:'smb://studio-nas/Projects',label:'Projects'}];
        OpenXplorer.renderSidebar();}''')
    emit('fileDragRequest', {'uri':'smb://studio-nas/Projects'})
    check('Unmapped SMB sources explain receiving-app compatibility',
          'supports SMB addresses' in page.locator('#toast').inner_text())
    finish_drag()
    check('No uncaught JavaScript errors', not errors)
    browser.close()

OUT.mkdir(exist_ok=True)
(OUT/'ui-file-drag.json').write_text(json.dumps({
    'passed':True, 'checks':len(checks), 'details':checks,
    'scope':'Chromium shared UI with fictional data, real selection/pointer events and a simulated native bridge. Not GTK, native WebKit, SMB or receiving-application validation.'
}, indent=2)+'\n')
print('Passed',len(checks),'file-drag UI contract checks')
