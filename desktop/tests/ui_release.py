#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
# Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
"""Current-release browser checks. Real UI; simulated storage/auth/apps only.

Run with Python + Playwright; CHROMIUM selects the installed browser executable.
The localStorage fixture avoids file:// navigation prohibited in CI containers.
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
    passed, errors, requests = [], [], []
    started = time.monotonic()
    report = {'scope': 'Chromium + actual UI + simulated storage/auth/apps; NOT native WebKit or live SMB'}
    status = 1
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=os.environ.get('CHROMIUM', '/usr/bin/chromium'), args=['--no-sandbox'])
        page = browser.new_page(viewport={'width': 1440, 'height': 900}, device_scale_factor=1)
        page.set_default_timeout(6000)
        page.on('pageerror', lambda e: errors.append(str(e)))
        page.on('request', lambda r: requests.append(r.url))
        page.evaluate("""()=>{const data={};window.__testStorage=data;Object.defineProperty(window,'localStorage',{value:{getItem:k=>data[k]??null,setItem:(k,v)=>{data[k]=String(v);},removeItem:k=>delete data[k]}});} """)

        def check(name, value=True):
            assert value, name
            passed.append(name)
            print('PASS', name, flush=True)

        def ready(uri=None):
            page.wait_for_function("""u=>{const s=window.OpenXplorer?.state,t=s?.tabs.find(t=>t.id===s.activeId);return s?.ready&&t?.loaded&&!t.busy&&(!u||u===t.uri)}""", arg=uri)

        def uri():
            return page.evaluate('OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri')

        def navigate(value):
            page.evaluate('(u)=>{void OpenXplorer.navigate(u)}', value)
            ready(value)

        def row(name):
            return page.locator('.file-row').filter(has=page.locator('.name-text', has_text=re.compile('^' + re.escape(name) + '$')))

        def close_modal():
            page.keyboard.press('Escape')
            expect(page.locator('#modal-layer')).to_be_hidden()

        def search(value):
            page.locator('#search').fill(value)
            page.wait_for_function('v=>OpenXplorer.state.query===v&&!OpenXplorer.state.searchBusy', arg=value)
            page.wait_for_timeout(200)

        try:
            page.set_content((ROOT / 'preview.html').read_text(), wait_until='load')
            ready('file:///home/demo')
            check('Home starts at the real demo home URI', uri() == 'file:///home/demo')
            check('Home lists standard directories', row('Downloads').count() == 1 and row('Documents').count() == 1)
            check('Release label is 1.1.0', '1.1.0' in page.locator('#status-mode').inner_text())
            check('Dark appearance is available', page.locator('html').get_attribute('data-theme') == 'dark')
            page.locator('#theme-toggle').click()
            page.get_by_role('menuitem', name='Light appearance', exact=True).click()
            check('Light appearance switch', page.locator('html').get_attribute('data-theme') == 'light')
            page.locator('#theme-toggle').click()
            page.get_by_role('menuitem', name='Dark appearance', exact=True).click()
            check('Dark appearance switch', page.locator('html').get_attribute('data-theme') == 'dark')

            page.locator('#more').click()
            page.get_by_role('menuitem', name='License & source', exact=True).click()
            expect(page.locator('#modal-layer')).to_be_visible()
            legal = page.locator('#modal').inner_text()
            check('License dialog carries new brand and source location', 'OpenXplorer' in legal and '/opt/openxplorer' in legal)
            check('Full AGPL license is available in the application', '13. Remote Network Interaction' in legal and 'END OF TERMS AND CONDITIONS' in legal)
            check('Original MIT attribution remains accessible', 'Copyright (c) 2026 Winspace contributors' in legal)
            close_modal()

            row('Documents').dblclick(); ready('file:///home/demo/Documents')
            docs = uri()
            check('Double-click folder navigates', row('Brand guidelines.pdf').count() == 1)
            row('Brand guidelines.pdf').dblclick(); page.wait_for_timeout(150)
            check('PDF opens without navigating into it', uri() == docs)
            row('Brand guidelines.pdf').click(button='right')
            expect(page.locator('#menu')).to_be_visible()
            bounds = page.locator('#menu').bounding_box()
            check('Classic menu is the default', page.locator('#menu').get_attribute('class') == 'menu win10')
            check('Classic menu has a compact bounded width', bounds['width'] <= 280)
            check('Menu fits the window', bounds['x'] >= 0 and bounds['x'] + bounds['width'] <= 1440 and bounds['y'] + bounds['height'] <= 900)
            for label in ('Open with…', 'Previous versions', 'Properties'):
                check('Menu includes ' + label, page.get_by_role('menuitem', name=label, exact=True).count() == 1)
            page.get_by_role('menuitem', name='Open with…', exact=True).click()
            expect(page.locator('.app-choice').first).to_be_visible()
            check('Open with does not change defaults by default', not page.locator('#open-with-default').is_checked())
            page.get_by_text('Show all installed applications', exact=True).click()
            page.locator('#app-filter').fill('Visual Studio')
            expect(page.locator('.app-choice')).to_have_count(1)
            check('Open with can filter installed applications')
            page.locator('.app-choice').click()
            page.locator('#modal .primary').click()
            expect(page.locator('#modal-layer')).to_be_hidden()
            check('Open with completes simulated selection', uri() == docs)

            row('Brand guidelines.pdf').click(); page.keyboard.press('Alt+Enter')
            expect(page.locator('#prop-tab-general')).to_be_visible()
            expect(page.locator('#prop-general')).to_contain_text('PDF document')
            check('Alt+Enter opens current-item Properties')
            page.locator('#prop-tab-permissions').click()
            check('Permissions tab opens', page.locator('#prop-permissions').is_visible())
            close_modal()

            page.locator('#new').click()
            for label in ('Folder', 'File…', 'Text document', 'JSON file', 'From template…'):
                check('New includes ' + label, page.get_by_role('menuitem', name=label, exact=True).count() == 1)
            page.get_by_role('menuitem', name='File…', exact=True).click()
            page.locator('#new-file-name').fill('release-check.conf')
            page.locator('#modal .primary').click()
            expect(page.locator('#modal-layer')).to_be_hidden(); ready(docs)
            expect(row('release-check.conf')).to_have_count(1)
            check('New arbitrary filename appears in the listing')

            page.locator('#settings-button').click()
            expect(page.locator('#context-menu-style')).to_be_visible()
            settings = page.locator('#landing').inner_text()
            check('Whole local disk is a cache candidate', 'Local Disk' in settings)
            check('Mounted 2 TB volume is a cache candidate', '2 TB Volume' in settings)
            check('Local watcher control is exposed', 'Watch enabled local folders' in settings)
            check('Network fallback is explicitly distinguished from push', 'not push notifications' in settings)
            page.locator('#context-menu-style').select_option('win11'); page.get_by_role('button', name='Back to files', exact=True).click()
            row('Brand guidelines.pdf').click(button='right')
            check('Windows 11 menu has an action strip', page.locator('#menu .context-strip').count() == 1)
            page.get_by_role('menuitem', name='Show more options', exact=True).click()
            check('Show more options opens classic menu', page.locator('#menu').get_attribute('class') == 'menu win10')
            page.keyboard.press('Escape')

            page.keyboard.press('Control+l')
            page.locator('#address-input').fill(r'\\archive-nas\Shared')
            page.keyboard.press('Enter')
            share = 'smb://archive-nas/Shared'; ready(share)
            check('UNC navigation reaches the share', row('Design brief.pdf').count() == 1)
            row('Product walkthrough.mp4').dblclick(); page.wait_for_timeout(150)
            check('MP4 does not navigate into a directory', uri() == share)
            row('Archive.mp4').dblclick(); ready(share + '/Archive.mp4')
            check('A genuine directory ending in .mp4 still navigates')
            page.locator('#back').click(); ready(share)
            check('Back returns to the share')
            search('Design')
            expect(row('Design brief.pdf')).to_have_count(1)
            check('Cached search is used', page.evaluate("OpenXplorer.state.searchResultMeta?.source==='cache'"))
            check('Search results include a Folder path column', 'Folder path' in page.locator('#column-head').inner_text())
            row('Design brief.pdf').dblclick(); page.wait_for_timeout(150)
            check('Cached PDF activates without directory navigation', uri() == share)
            row('Design assets.zip').dblclick()
            expect(page.locator('.archive-row')).to_have_count(2)
            check('ZIP opens the read-only archive browser', 'Read-only ZIP' in page.locator('#modal').inner_text())
            page.locator('#modal').get_by_role('button', name='Documents', exact=True).dblclick()
            expect(page.locator('.archive-row')).to_have_count(1)
            check('Nested ZIP directory browsing', 'Statement.pdf' in page.locator('.archive-list').inner_text())
            page.locator('#modal').get_by_role('button', name='Statement.pdf', exact=True).dblclick()
            expect(page.locator('.archive-error')).to_contain_text('Preview only')
            check('ZIP member uses a separate open operation'); close_modal()
            search('Launch')
            expect(row('Launch planning')).to_have_count(1)
            row('Launch planning').dblclick(); ready(share + '/Launch%20planning')
            check('Cached folder result resolves to its actual URI')
            check('Opening a folder leaves search', page.locator('#search').input_value() == '')
            page.locator('#address-edit').click()
            check('Address displays the full UNC path', page.locator('#address-input').input_value() == r'\\archive-nas\Shared\Launch planning')
            page.keyboard.press('Escape')

            navigate(share)
            # Pointer drag, not a direct invocation of the pin API.
            source = row('Incoming').bounding_box()
            target = page.locator('#quick-access').bounding_box()
            page.mouse.move(source['x'] + 80, source['y'] + 18)
            page.mouse.down()
            page.mouse.move(target['x'] + 90, target['y'] + target['height'] - 6, steps=30)
            check('Pointer drag shows the pin badge', page.locator('#pin-drag-badge').is_visible())
            page.mouse.up()
            pin = page.locator('#quick-access .side-entry').filter(has_text=re.compile('^Incoming$'))
            expect(pin).to_have_count(1)
            check('Pointer drop creates an SMB pin')
            check('SMB pin has the green network marker', pin.locator('.shared-bar').count() == 1)
            check('Pinning leaves source directory intact', row('Incoming').count() == 1)
            page.wait_for_timeout(400)

            page.evaluate('()=>{OpenXplorer.authPreview()}')
            expect(page.locator('#auth-dialog')).to_be_visible()
            check('Credentials default to remembered', page.locator('#auth-remember').is_checked())
            check('No separate domain field', page.get_by_label('Domain', exact=True).count() == 0)
            page.locator('#auth-remember').uncheck()
            page.locator('#auth-username').fill('test-user')
            page.locator('#auth-password').fill('not-a-real-password')
            page.locator('#auth-connect').click()
            expect(page.locator('#auth-layer')).to_be_hidden()
            check('Session-only credential form submits and clears its DOM', page.locator('#auth-password').count() == 0)

            navigate(docs)
            row('release-check.conf').click(); page.keyboard.press('Control+c')
            page.wait_for_function("()=>OpenXplorer.state.clipboard?.mode==='copy'")
            check('Ctrl+C produces a file clipboard payload')
            row('Work').dblclick(); ready(docs + '/Work')
            page.keyboard.press('Control+v')
            expect(page.locator('#modal-layer')).to_be_hidden()
            expect(row('release-check.conf')).to_have_count(1)
            check('Ctrl+V copies into the target listing (simulated)')
            row('release-check.conf').click(); page.keyboard.press('Control+x')
            page.wait_for_function("()=>OpenXplorer.state.clipboard?.mode==='move'")
            check('Ctrl+X produces a cut payload')
            navigate(docs + '/Personal')
            page.keyboard.press('Control+v')
            expect(page.locator('#modal-layer')).to_be_hidden()
            expect(row('release-check.conf')).to_have_count(1)
            check('Cut/paste reaches destination (simulated)')
            navigate(docs + '/Work')
            check('Cut/paste removes simulated source', row('release-check.conf').count() == 0)

            page.keyboard.press('Control+t'); ready('file:///home/demo')
            check('New tab opens Home', page.locator('.tab').count() == 2)
            page.keyboard.press('Control+w'); ready(docs + '/Work')
            check('Close-tab shortcut returns to previous tab')
            navigate('file:///home/demo/Downloads')
            page.evaluate("()=>{void OpenXplorer.propertiesDialog({uri:'file:///home/demo/Downloads',name:'Downloads',isDir:true},'location')}")
            expect(page.locator('#folder-location')).to_be_visible()
            check('Downloads has a Location tab', page.locator('#prop-tab-location').is_visible())
            check('Location page rejects silent relocation by requiring confirmation', page.locator('#prop-location input[type=checkbox]').count() > 0)
            close_modal()

            navigate(share)
            row('Launch planning').click(button='right')
            page.get_by_role('menuitem', name='Previous versions', exact=True).click()
            expect(page.locator('.version-row')).to_have_count(3)
            check('Exposed snapshot fixtures populate Previous versions')
            check('Previous versions offers safe copy restoration', page.get_by_role('button', name='Restore a copy…', exact=True).count() == 3)
            close_modal()
            navigate('network:')
            page.wait_for_function('()=>OpenXplorer.state.discovery.started&&!OpenXplorer.state.discovery.busy')
            check('Network discovery populates sample servers', len(page.evaluate('OpenXplorer.state.discovery.servers')) >= 2)
            navigate(share)
            check('No JavaScript exceptions', not errors)
            check('The preview made no external requests', not requests)
            page.screenshot(path=str(OUT / 'browser-release.png'), full_page=False)
            status = 0
        except Exception as exc:
            report['failure'] = str(exc)
            print('FAIL', exc, file=sys.stderr)
            page.screenshot(path=str(OUT / 'browser-failure.png'), full_page=False)
        finally:
            report.update(passed=passed, count=len(passed), errors=errors, requests=requests,
                          success=status == 0, elapsedSeconds=round(time.monotonic()-started, 2))
            (OUT / 'browser-tests.json').write_text(json.dumps(report, indent=2) + '\n')
            browser.close()
    return status


if __name__ == '__main__':
    raise SystemExit(main())
