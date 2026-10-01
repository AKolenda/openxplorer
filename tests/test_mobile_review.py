#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Mobile reading, conditional embeds and the standalone tour regression checks."""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread
import json,os
from playwright.sync_api import sync_playwright,expect
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'test-results';OUT.mkdir(exist_ok=True)
checks=[];errors=[]
D=ROOT/'designs'
# The pages are served over local HTTP: a script-only sandboxed frame cannot
# load the tour's files from file:// URLs.
class QuietHandler(SimpleHTTPRequestHandler):
 def log_message(self,*_args):pass
SERVER=ThreadingHTTPServer(('127.0.0.1',0),partial(QuietHandler,directory=str(D)))
Thread(target=SERVER.serve_forever,daemon=True).start()
BASE=f'http://127.0.0.1:{SERVER.server_port}/'
def check(name,condition=True):
 assert condition,name
 checks.append(name);print('PASS',name,flush=True)
with sync_playwright() as pw:
 b=pw.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
 def page_for(name,width=390):
  p=b.new_page(viewport={'width':width,'height':844});p.set_default_timeout(8000)
  p.on('pageerror',lambda e:errors.append(str(e)))
  p.goto(BASE+name,wait_until='load');return p
 for width in (320,360,390,430,600,768,959):
  for filename in ('index.html','docs-introduction.html'):
   p=page_for(filename,width)
   check(f'{filename} {width}px: no running embedded explorer',p.locator('iframe').count()==0 and len(p.frames)==1)
   check(f'{filename} {width}px: no horizontal overflow',p.evaluate('document.documentElement.scrollWidth<=innerWidth+1'))
   check(f'{filename} {width}px: interactive controls are not visible',p.locator('[data-demo-command="play"]').is_hidden())
   check(f'{filename} {width}px: hypothetical screenshot remains',p.locator('.product-shot img').first.is_visible())
   if filename.startswith('docs'):
    check(f'{width}px: no docs navigation strip',p.locator('.docs-sidebar').is_hidden())
    check(f'{width}px: readable title',p.locator('.doc-article h1').is_visible())
    toggle=p.locator('[data-menu-toggle]');check(f'{width}px: menu tap target >=44px',toggle.bounding_box()['height']>=44)
    toggle.click();expect(p.locator('[data-mobile-nav]')).to_be_visible()
    check(f'{width}px: all ten topics are in the menu',p.locator('.mobile-doc-group a').count()==10)
    check(f'{width}px: active topic indicated',p.locator('.mobile-doc-group a[aria-current="page"]').inner_text()=='Introduction')
    check(f'{width}px: no horizontal scrolling menu',p.locator('[data-mobile-nav]').evaluate('e=>e.scrollWidth<=e.clientWidth+1'))
    p.keyboard.press('Escape');check(f'{width}px: Escape closes menu',p.locator('[data-mobile-nav]').is_hidden())
    check(f'{width}px: Escape returns focus',toggle.evaluate('e=>e===document.activeElement'))
   else:
    check(f'{width}px: download button fits',p.locator('.hero-actions .button').evaluate('e=>{const r=e.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth}'))
    check(f'{width}px: guided tour stays visible',p.locator('.product-tour').is_visible())
   p.close()
 p=page_for('docs-introduction.html')
 p.screenshot(path=str(OUT/'docs-mobile.png'))
 p.locator('[data-menu-toggle]').click();p.screenshot(path=str(OUT/'docs-mobile-menu.png'))
 p.mouse.click(8,600); # Click in expanded menu may be inside; test explicit outside header after closing.
 p.keyboard.press('Escape')
 p.locator('[data-copy-markdown]').click()
 check('Copy Markdown button still present on mobile',p.locator('[data-copy-markdown]').is_visible())
 # Breakpoint creates/destroys the iframe, not just a display:none wrapper.
 p.set_viewport_size({'width':1440,'height':1000});p.locator('[data-product-demo]').scroll_into_view_if_needed()
 p.wait_for_function('document.querySelector("[data-product-demo]").dataset.phase==="ready"')
 check('Desktop restores the sandboxed tour',p.locator('iframe').count()==1)
 check('Script-only sandbox preserved',p.locator('iframe').get_attribute('sandbox')=='allow-scripts')
 check('Desktop three-column documentation unchanged',p.locator('.docs-sidebar').is_visible() and p.locator('.docs-toc').is_visible())
 p.set_viewport_size({'width':390,'height':844});p.wait_for_timeout(100)
 check('Shrinking viewport disposes the iframe',p.locator('iframe').count()==0 and len(p.frames)==1)
 p.set_viewport_size({'width':1440,'height':1000});p.locator('[data-product-demo]').scroll_into_view_if_needed()
 p.wait_for_function('document.querySelector("[data-product-demo]").dataset.phase==="ready"')
 check('Re-expanding creates a clean usable tour',p.locator('iframe').count()==1)
 p.close()
 # Every guide uses the same mobile topic menu, not a horizontal strip.
 for doc in json.loads((ROOT/'apps/web/lib/docs.json').read_text()):
  p=page_for('docs-'+doc['slug']+'.html');p.locator('[data-menu-toggle]').click()
  check(doc['slug']+': menu marks current guide',p.locator('.mobile-doc-group a[aria-current="page"]').inner_text()==doc['title'])
  check(doc['slug']+': no live explorer on phones',p.locator('iframe').count()==0)
  p.close()
 # The tour of the native app: real pictures with keyboard-reachable controls.
 p=b.new_page(viewport={'width':1280,'height':900});p.set_default_timeout(8000)
 p.on('pageerror',lambda e:errors.append(str(e)))
 p.goto(BASE+'tour/index.html?theme=dark')
 tour=json.loads((ROOT/'apps/web/public/tour/scenes.json').read_text())
 first=tour['scenes'][0]
 check('Tour opens on its first scene in the requested theme',p.locator('#picture').get_attribute('src')==first['images']['dark'])
 check('Every hotspot is a labelled button',p.locator('.hotspot').count()==len(first['hotspots']) and all(p.locator('.hotspot').nth(i).get_attribute('aria-label') for i in range(len(first['hotspots']))))
 check('Back is disabled on the first scene',p.locator('#back').is_disabled())
 p.keyboard.press('Tab');p.keyboard.press('Tab');p.keyboard.press('Enter')
 check('Enter on a hotspot opens the scene it names',p.locator('#title').inner_text()!=first['title'])
 p.locator('#back').click()
 check('Back returns to the previous scene',p.locator('#title').inner_text()==first['title'])
 check('Captions say the pictures are the real app','Real app' in p.locator('.badge').inner_text())
 p.set_viewport_size({'width':740,'height':900});p.wait_for_timeout(100)
 check('Narrow tour keeps the picture inside the viewport',p.locator('#frame').bounding_box()['width']<=740)
 p.close();b.close()
check('No JavaScript errors',not errors)
report={'passed':True,'checks':len(checks),'details':checks,'errors':errors,'scope':'Chromium standalone HTML and the tour of native app pictures with fictional fixtures; mobile screen sizes 320–959px plus desktop re-entry; not physical Android.'}
(OUT/'mobile-review.json').write_text(json.dumps(report,indent=2)+'\n')
print('Passed',len(checks),'mobile and tour checks')
