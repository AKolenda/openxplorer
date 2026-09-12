#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""0.9.1 mobile reading, conditional embeds and compact dates regression checks."""
from pathlib import Path
import json,os
from playwright.sync_api import sync_playwright,expect
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'test-results';OUT.mkdir(exist_ok=True)
checks=[];errors=[]
def check(name,condition=True):
 assert condition,name
 checks.append(name);print('PASS',name,flush=True)
with sync_playwright() as pw:
 b=pw.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
 def page_for(name,width=390):
  p=b.new_page(viewport={'width':width,'height':844});p.set_default_timeout(8000)
  p.on('pageerror',lambda e:errors.append(str(e)))
  p.set_content((ROOT/'designs'/name).read_text(),wait_until='load');return p
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
    check(f'{width}px: mobile CTA goes to features',p.locator('.hero-actions .mobile-feature-link').is_visible())
    check(f'{width}px: tour link hidden on mobile',p.locator('[data-play-tour]').is_hidden())
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
 check('Desktop restores the actual sandboxed preview',p.locator('iframe').count()==1)
 check('Script-only sandbox preserved',p.locator('iframe').get_attribute('sandbox')=='allow-scripts')
 check('Desktop three-column documentation unchanged',p.locator('.docs-sidebar').is_visible() and p.locator('.docs-toc').is_visible())
 p.set_viewport_size({'width':390,'height':844});p.wait_for_timeout(100)
 check('Shrinking viewport disposes the iframe',p.locator('iframe').count()==0 and len(p.frames)==1)
 p.set_viewport_size({'width':1440,'height':1000});p.locator('[data-product-demo]').scroll_into_view_if_needed()
 p.wait_for_function('document.querySelector("[data-product-demo]").dataset.phase==="ready"')
 check('Re-expanding creates a clean usable preview',p.locator('iframe').count()==1)
 p.close()
 # Every guide uses the same mobile topic menu, not a horizontal strip.
 for doc in json.loads((ROOT/'apps/web/lib/docs.json').read_text()):
  p=page_for('docs-'+doc['slug']+'.html');p.locator('[data-menu-toggle]').click()
  check(doc['slug']+': menu marks current guide',p.locator('.mobile-doc-group a[aria-current="page"]').inner_text()==doc['title'])
  check(doc['slug']+': no live explorer on phones',p.locator('iframe').count()==0)
  p.close()
 # Actual app date columns, not website imitation.
 p=b.new_page(viewport={'width':1280,'height':900});p.set_default_timeout(8000)
 p.on('pageerror',lambda e:errors.append(str(e)))
 p.evaluate('''()=>{const data={'winspace-preview-v3':JSON.stringify({pins:[{uri:'smb://legacy-demo/old-preview',label:'legacy-pin-must-not-import'}]})};Object.defineProperty(window,'localStorage',{value:{getItem:k=>data[k]??null,setItem:(k,v)=>data[k]=String(v),removeItem:k=>delete data[k]}})}''')
 p.set_content((ROOT/'desktop/preview.html').read_text(),wait_until='load')
 p.wait_for_function('()=>window.OpenXplorerTour && OpenXplorer.state.ready')
 p.evaluate("()=>{OpenXplorer.applyTheme('dark',false);void OpenXplorerTour.scene('snapshots')}")
 expect(p.locator('.version-row')).to_have_count(3)
 date=p.locator('.version-date').first;time=p.locator('.version-time').first;day=p.locator('.version-calendar-date').first
 check('Date and time occupy one readable line',abs(day.bounding_box()['y']-time.bounding_box()['y'])<4)
 check('Repeated per-row date-source labels removed',p.locator('.version-date-source').count()==0)
 check('One clear date-source note',p.locator('.version-date-note').count()==1)
 check('Date stays between the name and actions',date.bounding_box()['x']>p.locator('.version-text').first.bounding_box()['x'] and date.bounding_box()['x']+date.bounding_box()['width']<=p.locator('.version-actions').first.bounding_box()['x'])
 check('Semantic timestamp retains precision',p.locator('.version-stamp').first.get_attribute('datetime')=='2026-09-05T18:00:00')
 check('Source timezone caveat retained','not supplied' in date.get_attribute('title'))
 check('No stale personal fixture pins',p.locator('#quick-access').inner_text().find('legacy-pin-must-not-import')<0)
 p.locator('#modal').screenshot(path=str(OUT/'previous-versions-dark.png'))
 # Changing application window width does not collide columns.
 p.set_viewport_size({'width':740,'height':900});p.wait_for_timeout(100)
 check('Narrow application modal fits viewport',p.locator('#modal').bounding_box()['width']<=740)
 check('Narrow dialog actions are visible',p.locator('.version-actions').first.is_visible())
 p.close();b.close()
check('No JavaScript errors',not errors)
report={'passed':True,'checks':len(checks),'details':checks,'errors':errors,'scope':'Chromium standalone HTML and real application preview with fictional fixtures; mobile screen sizes 320–959px plus desktop re-entry; not native WebKit or physical Android.'}
(OUT/'mobile-review.json').write_text(json.dumps(report,indent=2)+'\n')
print('Passed',len(checks),'mobile/date checks')
