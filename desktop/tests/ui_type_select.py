#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Type-to-select integration: real browser keyboard/pointer events, simulated data.

This is not native WebKitGTK, an OS keyboard accessibility test, or live SMB.
Run after tools/build_preview.py. No external requests or file mutations needed.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import re
import sys
import time
from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'test-results'
OUT.mkdir(exist_ok=True)


def main() -> int:
    checks, errors, requests = [], [], []
    report = {'scope': 'Actual UI in Chromium; real keyboard/pointer events; simulated files and SMB shares.'}
    started = time.monotonic()
    status = 1
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=os.environ.get('CHROMIUM', '/usr/bin/chromium'), args=['--no-sandbox'])
        page = browser.new_page(viewport={'width': 1360, 'height': 850})
        page.set_default_timeout(6000)
        page.on('pageerror', lambda e: errors.append(str(e)))
        page.on('request', lambda r: requests.append(r.url))
        page.evaluate("""()=>{const data={};Object.defineProperty(window,'localStorage',{value:{getItem:k=>data[k]??null,setItem:(k,v)=>{data[k]=String(v);},removeItem:k=>delete data[k]}});} """)

        def check(name, condition=True):
            assert condition, name
            checks.append(name)
            print('PASS', name, flush=True)

        def ready(location=None):
            page.wait_for_function("""uri=>{const s=window.OpenXplorer?.state,t=s?.tabs.find(t=>t.id===s.activeId);return s?.ready&&t?.loaded&&!t.busy&&(!uri||t.uri===uri)}""", arg=location)

        def uri():
            return page.evaluate('OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri')

        def navigate(location):
            page.evaluate('(uri)=>{void OpenXplorer.navigate(uri)}', location)
            ready(location)

        def row(name):
            return page.locator('.file-row,.file-tile').filter(has=page.locator('.name-text,.tile-name', has_text=re.compile('^'+re.escape(name)+'$')))

        def selected():
            return page.evaluate("()=>{const s=OpenXplorer.state,t=s.tabs.find(t=>t.id===s.activeId);return (s.searchResults||t.entries).filter(e=>s.selection.has(e.uri)).map(e=>e.name)}")

        def select(name):
            row(name).click()

        def type_text(value):
            page.keyboard.type(value, delay=25)

        def selection_is(name):
            page.wait_for_function("""name=>{const s=OpenXplorer.state,t=s.tabs.find(t=>t.id===s.activeId);const rows=(s.searchResults||t.entries).filter(e=>s.selection.has(e.uri));return rows.length===1&&rows[0].name===name}""", arg=name)

        def hints_hidden():
            return page.locator('#type-select-status').is_hidden()

        def fixture(names, hidden=None):
            # Only modifies the preview's in-memory list, not its transport or source.
            navigate('file:///home/demo/Documents')
            page.evaluate("""({names,hidden})=>{
                const s=OpenXplorer.state,t=s.tabs.find(t=>t.id===s.activeId);
                t.entries=names.map((name,i)=>({name,uri:t.uri+'/'+encodeURIComponent(name),isDir:!name.endsWith('.pdf'),type:name.endsWith('.pdf')?'PDF document':'File folder',size:null,hidden:name===hidden,modified:i}));
                t.dirty=true;s.filterCache=null;s.selection.clear();s.anchor=-1;s.sort='name';s.descending=false;s.showHidden=false;
                document.getElementById('file-scroll').scrollTop=0;window.dispatchEvent(new Event('resize'));
                document.getElementById('main').focus({preventScroll:true});
            }""", {'names': names, 'hidden': hidden})
            page.wait_for_timeout(80)

        def visible_in_pane(name):
            box = row(name).bounding_box(); pane = page.locator('#file-scroll').bounding_box()
            return box and pane and box['y'] >= pane['y']-1 and box['y']+box['height'] <= pane['y']+pane['height']+1

        try:
            page.set_content((ROOT/'preview.html').read_text(), wait_until='load')
            ready('file:///home/demo')
            check('UI starts with 1.1.1 label', '1.1.1' in page.locator('#status-mode').inner_text())
            check('No jump indicator is displayed until typing', hints_hidden())
            navigate('smb://studio-nas/')
            select('Backups')
            before = page.evaluate("()=>{const s=OpenXplorer.state,t=s.tabs.find(t=>t.id===s.activeId);return {token:t.loadToken,history:t.history.length,search:s.searchGeneration,count:t.entries.length}}")
            type_text('SC'); selection_is('scripts')
            check('Uppercase SC selects scripts in an SMB share list')
            check('Matching row is marked selected', row('scripts').get_attribute('aria-selected') == 'true')
            check('Main file pane retains keyboard focus', page.locator('#main').evaluate('(e)=>document.activeElement===e'))
            check('Type-to-select does not navigate', uri() == 'smb://studio-nas/')
            check('Search box is untouched', page.locator('#search').input_value() == '')
            after = page.evaluate("()=>{const s=OpenXplorer.state,t=s.tabs.find(t=>t.id===s.activeId);return {token:t.loadToken,history:t.history.length,search:s.searchGeneration,count:t.entries.length}}")
            check('No listing reload, history entry, search generation or filtering', before == after)
            expect(page.locator('#type-select-status')).to_contain_text('Jump to: SC — scripts')
            check('Brief status-bar prefix feedback appears')
            page.screenshot(path=str(OUT/'type-to-select-smb.png'))
            page.keyboard.press('Enter'); ready('smb://studio-nas/scripts')
            check('Enter opens the selected share, rather than typing doing so')
            check('Navigation clears the prefix', hints_hidden())
            page.locator('#back').click(); ready('smb://studio-nas/')
            select('Backups'); type_text('sc'); selection_is('scripts')
            check('Lowercase sc produces the same result')
            page.wait_for_timeout(1100)
            check('Prefix hint clears after the one-second timeout', hints_hidden())
            check('Timeout leaves the chosen item selected', selected() == ['scripts'])
            type_text('w'); selection_is('work')
            check('After a pause, a new letter begins a fresh prefix')

            fixture(['Backups','Scripts','Shipping','Songs','work'])
            type_text('s'); selection_is('Scripts')
            type_text('s'); selection_is('Shipping')
            type_text('s'); selection_is('Songs')
            type_text('s'); selection_is('Scripts')
            check('Repeated letters cycle through matching names and wrap')
            page.keyboard.press('Escape')
            check('Escape clears an active prefix without clearing selection', hints_hidden() and selected() == ['Scripts'])
            page.keyboard.press('Escape')
            check('A second Escape clears the selection', selected() == [])
            select('work'); type_text('s'); selection_is('Scripts')
            check('Fresh selection wraps to the beginning when needed')
            page.keyboard.press('c'); selection_is('Scripts')
            check('A second different letter refines, without skipping the current match')
            type_text('z')
            check('An unmatched prefix preserves the selection', selected() == ['Scripts'])
            expect(page.locator('#type-select-status')).to_contain_text('No name starts with')
            check('No-match feedback does not turn into a substring search')
            page.keyboard.press('Backspace'); selection_is('Scripts')
            expect(page.locator('#type-select-status')).to_contain_text('Jump to: sc')
            check('Backspace corrects the active prefix')
            page.keyboard.press('Backspace'); page.keyboard.press('Backspace')
            check('Backspace can clear the prefix without losing the selected row', hints_hidden() and selected()==['Scripts'])
            type_text('w'); selection_is('work')
            select('Backups'); type_text('s'); selection_is('Scripts')
            check('A mouse selection starts a new typing sequence')
            page.keyboard.press('ArrowDown'); selection_is('Shipping')
            check('Arrow navigation still works and clears the prefix', hints_hidden())
            page.keyboard.press('End'); selection_is('work')
            page.keyboard.press('Home'); selection_is('Backups')
            check('Home and End navigation are unchanged')
            type_text('s'); page.keyboard.press('Control+f')
            check('Ctrl+F still focuses Search and clears the prefix', page.locator('#search').evaluate('(e)=>document.activeElement===e') and hints_hidden())
            page.locator('#search').fill('shi')
            page.wait_for_function("()=>OpenXplorer.state.query==='shi'&&!OpenXplorer.state.searchBusy")
            check('Text entered into Search remains ordinary search input', hints_hidden())
            page.locator('#search').fill('')
            page.wait_for_function("()=>OpenXplorer.state.query===''&&!OpenXplorer.state.searchBusy")
            page.keyboard.press('Control+l'); page.locator('#address-input').fill('sc')
            check('Address-bar text is not intercepted', page.locator('#address-input').input_value()=='sc' and hints_hidden())
            page.keyboard.press('Escape'); page.locator('#main').focus()
            type_text('w'); selection_is('work')
            check('Returning from an input starts a fresh prefix')

            navigate('file:///home/demo/Documents'); select('Brand guidelines.pdf')
            page.keyboard.press('F2')
            expect(page.locator('#modal input')).to_be_visible()
            page.locator('#modal input').fill('sc')
            check('Rename text is not intercepted', page.locator('#modal input').input_value()=='sc' and hints_hidden())
            page.keyboard.press('Escape')
            select('Brand guidelines.pdf'); row('Brand guidelines.pdf').click(button='right')
            type_text('sc')
            check('Typing while a context menu is open does not change the file selection', selected()==['Brand guidelines.pdf'] and hints_hidden())
            page.keyboard.press('Escape')
            page.evaluate('OpenXplorer.authPreview()')
            expect(page.locator('#auth-username')).to_be_visible()
            page.locator('#auth-username').fill('sc-user'); page.locator('#auth-password').fill('sc-password')
            check('SMB username and password inputs are not intercepted', page.locator('#auth-username').input_value()=='sc-user' and page.locator('#auth-password').input_value()=='sc-password' and hints_hidden())
            page.keyboard.press('Escape')
            expect(page.locator('#auth-layer')).to_be_hidden()
            select('Brand guidelines.pdf'); type_text('r'); selection_is('Read me.txt')
            page.keyboard.press('Control+c')
            page.wait_for_function("()=>OpenXplorer.state.clipboard?.mode==='copy'")
            check('Ctrl+C remains a file-clipboard shortcut, not a prefix', hints_hidden() and selected()==['Read me.txt'])
            type_text('project b'); selection_is('Project budget.xlsx')
            page.keyboard.press('Control+t'); ready('file:///home/demo')
            check('Opening a tab clears the prefix', hints_hidden())
            page.keyboard.press('Control+w'); ready('file:///home/demo/Documents')
            check('Returning to a tab does not resurrect its prefix', hints_hidden())
            # Clear the visible Search box then explicitly move focus into blank space.
            navigate('file:///home/demo')
            page.locator('#search').focus()
            pane=page.locator('#file-scroll').bounding_box()
            page.mouse.click(pane['x']+pane['width']-40,pane['y']+pane['height']-25)
            check('Clicking blank file-pane space focuses the list', page.locator('#main').evaluate('(e)=>document.activeElement===e'))
            type_text('do'); selection_is('Documents')
            check('Typing works after clicking empty space, not only after clicking a row')

            fixture(['Archive','Scripts','ZZ-hidden'], hidden='ZZ-hidden')
            type_text('zz')
            check('Hidden items excluded from the current listing are not selected', selected()==[])
            fixture(['Archive','Reports.pdf','Scripts'])
            type_text('re'); selection_is('Reports.pdf')
            check('Type-to-select also matches files, without opening them', uri()=='file:///home/demo/Documents' and not page.evaluate('!!OpenXplorer.state.opening'))
            fixture(['Old scripts','Scripts'])
            type_text('sc'); selection_is('Scripts')
            check('Matching is filename prefix, not substring')

            # Virtualized results: the destination has no DOM row before typing.
            fixture([f'Folder {i:05d}' for i in range(10000)]+['Zeta destination'])
            check('Off-screen destination starts outside the rendered viewport', row('Zeta destination').count()==0)
            begin=time.monotonic(); type_text('ze'); selection_is('Zeta destination')
            report['tenThousandRowJumpMs']=round((time.monotonic()-begin)*1000,2)
            check('Prefix selects an off-screen item in a 10,001-entry list')
            check('Details view scrolls the selected item completely into view', visible_in_pane('Zeta destination'))
            check('DOM remains virtualized instead of rendering all 10,001 rows', page.locator('.file-row').count()<100)
            page.locator('#status-grid').click()
            page.locator('#main').focus(); type_text('ze'); selection_is('Zeta destination')
            check('Large-icon view also selects the off-screen item', row('Zeta destination').count()==1)
            check('Large-icon view scrolls by its multi-column layout', visible_in_pane('Zeta destination'))
            page.locator('#status-list').click()
            fixture(['Alpha','Scans','Scripts','Shipping','Zulu'])
            # User-selected sort order is respected. Do not sort again inside type-select.
            page.evaluate("()=>{OpenXplorer.state.descending=true;OpenXplorer.state.filterCache=null;window.dispatchEvent(new Event('resize'));document.getElementById('main').focus()}")
            type_text('s'); selection_is('Shipping')
            type_text('s'); selection_is('Scripts')
            check('Cycling follows the current descending display order')
            fixture(['Alpha','Scripts','Shipping'])
            select('Alpha'); page.keyboard.down('Control'); row('Shipping').click(); page.keyboard.up('Control')
            check('Precondition: multiple selection exists', len(selected())==2)
            type_text('sc'); selection_is('Scripts')
            check('Typing replaces an old multi-selection with one match')
            # Fake composing keyboard events must not invoke navigation or file actions.
            page.keyboard.press('Escape')
            page.locator('#main').dispatch_event('keydown',{'key':'a','isComposing':True,'bubbles':True})
            check('IME composition events do not move selection', selected()==['Scripts'] and hints_hidden())
            page.locator('#main').dispatch_event('keydown',{'key':'a','keyCode':229,'bubbles':True})
            check('Legacy IME keycode 229 is ignored', selected()==['Scripts'] and hints_hidden())
            page.locator('#main').dispatch_event('keydown',{'key':'Dead','bubbles':True})
            check('Dead-key names are not used as filename prefixes', selected()==['Scripts'] and hints_hidden())
            type_text('s')
            page.evaluate("window.dispatchEvent(new Event('blur'))")
            check('Window focus loss clears the buffered prefix', hints_hidden())
            page.locator('#main').focus(); type_text('a'); selection_is('Alpha')
            check('Focus return begins a fresh prefix')
            # Existing cached results can be traversed without changing their query.
            share='smb://archive-nas/Shared'
            navigate(share)
            page.locator('#search').fill('Design')
            page.wait_for_function("()=>OpenXplorer.state.query==='Design'&&!OpenXplorer.state.searchBusy")
            select('Design brief.pdf')
            search_gen=page.evaluate('OpenXplorer.state.searchGeneration')
            type_text('De')
            check('Type-to-select works within already displayed search results', len(selected())==1 and selected()[0].startswith('Design'))
            check('Existing cached search query is not modified or rerun', page.locator('#search').input_value()=='Design' and page.evaluate('OpenXplorer.state.searchGeneration')==search_gen)
            # Settings dropdown controls are not treated as file-list typing.
            page.locator('#settings-button').click()
            page.locator('#context-menu-style').focus(); type_text('s')
            check('Settings controls do not trigger file-list selection', hints_hidden())
            page.keyboard.press('Escape')
            check('No JavaScript exceptions', not errors)
            check('The offline preview made no external requests', not requests)
            status=0
        except Exception as exc:
            report['failure']=str(exc)
            import traceback
            traceback.print_exc()
            print('FAIL',str(exc),file=sys.stderr)
            page.screenshot(path=str(OUT/'type-select-failure.png'))
        finally:
            report.update(success=status==0, count=len(checks), checks=checks, errors=errors, requests=requests,
                          elapsedSeconds=round(time.monotonic()-started,2))
            (OUT/'type-select-browser.json').write_text(json.dumps(report,indent=2)+'\n')
            browser.close()
    return status


if __name__ == '__main__':
    raise SystemExit(main())
