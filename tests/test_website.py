#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Standalone HTML + the script-only sandboxed tour iframe, actual mouse interactions.

Pages are served from designs/ over local HTTP, with PNG bytes identical to
the release assets; the tour iframe loads designs/tour/. Relative links are audited on disk.
This does not execute Next.js or React hydration.
"""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from html.parser import HTMLParser
from threading import Thread
from urllib.parse import urlsplit,unquote
import json,os,re
from playwright.sync_api import sync_playwright,expect
ROOT=Path(__file__).resolve().parents[1];D=ROOT/'designs';OUT=ROOT/'test-results';OUT.mkdir(exist_ok=True)
checks=[];errors=[];requests=[]
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
class Links(HTMLParser):
 def __init__(self,s):super().__init__();self.links=[];self.ids=set();self.images=[];self.feed(s)
 def handle_starttag(self,tag,attrs):
  a=dict(attrs)
  if a.get('id'):self.ids.add(a['id'])
  if tag=='a' and a.get('href'):self.links.append(a['href'])
  if tag=='img':self.images.append(a)
files=sorted(D.glob('*.html'))
parsed={p.name:Links(p.read_text()) for p in files};count=0
check('All 12 standalone site and guide pages exist',len(files)==12)
for p in files:
 for href in parsed[p.name].links:
  u=urlsplit(href)
  if u.scheme or u.netloc:continue
  target=p.parent/unquote(u.path) if u.path else p
  assert target.exists(),f'{p.name}: missing {href}'
  if u.fragment and target.name in parsed:assert unquote(u.fragment) in parsed[target.name].ids,f'{p.name}: missing anchor {href}'
  count+=1
check('All relative release / documentation / fragment links resolve: '+str(count))
check('Every product screenshot has alt text',all(a.get('alt') for v in parsed.values() for a in v.images))
check('Markdown download is generated for each guide',len(list((D/'docs-markdown').glob('*.md')))==10)
for path in (ROOT/'docs').glob('*.md'):
 for target in re.findall(r'!\[[^\]]*\]\(([^)]+)\)',path.read_text()):assert (path.parent/target).resolve().exists(),(path,target)
check('Repository guide image paths resolve')
with sync_playwright() as p:
 b=p.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
 def open_page(name,width=1440,height=1000,reduced=False):
  page=b.new_page(viewport={'width':width,'height':height},reduced_motion='reduce' if reduced else 'no-preference')
  page.set_default_timeout(8000);page.on('pageerror',lambda e:errors.append(str(e)));page.on('request',lambda r:requests.append(r.url))
  page.goto(BASE+name,wait_until='load');return page
 for name in ['index.html','source.html']:
  page=open_page(name)
  check(name+': single primary heading',page.locator('h1').count()==1)
  check(name+': no Command-K badge',page.locator('kbd').filter(has_text=re.compile(r'[⌘K]')).count()==0)
  check(name+': no imitation explorer markup',page.locator('.explorer-mock').count()==0)
  if name=='index.html':
   check(name+': guided tour runs over the real screenshot',page.locator('.tour-frame img').get_attribute('src').startswith('data:image/png'))
   check(name+': guided tour has three labelled steps',page.locator('.tour-cursor b').count()==3)
   check(name+': three feature details',page.locator('.details>article').count()==3)
   check(name+': homepage embeds no running explorer',page.locator('iframe').count()==0)
  page.set_viewport_size({'width':390,'height':844});page.wait_for_timeout(100)
  check(name+': no page-wide mobile overflow',page.evaluate('document.documentElement.scrollWidth<=innerWidth+1'))
  page.close()
 # The tour of the native app lives in the introduction guide.
 page=open_page('docs-introduction.html')
 host=page.locator('[data-product-demo]');host.scroll_into_view_if_needed()
 page.wait_for_function('()=>document.querySelector("[data-product-demo]").dataset.phase==="ready"')
 frame=page.frame_locator('iframe')
 tour=json.loads((ROOT/'apps/web/public/tour/scenes.json').read_text())
 check('Tour starts on the first picture of the real app',frame.locator('#picture').get_attribute('src')==tour['scenes'][0]['images']['light'])
 check('Tour pictures have their controls as buttons',frame.locator('.hotspot').count()>0)
 # Recovery handshake also covers parent script initialization after iframe load.
 host.evaluate("e=>{e.dataset.phase='loading';e.querySelector('iframe').contentWindow.postMessage({channel:'openxplorer-demo-command',command:'status'},'*')}")
 page.wait_for_function('()=>document.querySelector("[data-product-demo]").dataset.phase==="ready"')
 check('Readiness handshake recovers a late parent listener')
 check('Tour does not start automatically',page.locator('[data-demo-command="stop"]').is_hidden())
 page.wait_for_timeout(250)
 check('Parent cannot read script-only sandbox DOM',page.locator('iframe').evaluate('e=>e.contentDocument===null'))
 page.locator('[data-demo-command="play"]').click()
 page.wait_for_function('()=>document.querySelector("[data-product-demo]").dataset.phase==="playing"')
 check('Play shows Stop and running status',page.locator('[data-demo-command="stop"]').is_visible())
 check('Running tour disables its duplicate Play button',page.locator('[data-demo-command="play"]').is_disabled())
 page.wait_for_function('()=>["complete","error"].includes(document.querySelector("[data-product-demo]").dataset.phase)',timeout=20000)
 check('Played tour completed',host.get_attribute('data-phase')=='complete')
 page.locator('[data-demo-command="theme"]').click()
 expect(frame.locator('html')).to_have_attribute('data-theme','dark')
 check('Website theme button switches the tour to the dark pictures',frame.locator('#picture').get_attribute('src').endswith('-dark.png'))
 page.locator('[data-demo-command="scene"][data-demo-value="context-menu"]').click()
 expect(frame.locator('#title')).to_have_text('Context menu')
 check('Website scene buttons open a picture of the real app')
 frame.locator('.hotspot').first.click()
 expect(frame.locator('#back')).to_be_enabled()
 check('Clicking a control in the picture follows it, and Back is offered')
 page.locator('[data-demo-command="reset"]').click()
 page.wait_for_function('()=>document.querySelector("[data-product-demo]").dataset.phase==="ready"')
 check('Reset returns to the first picture',frame.locator('#title').inner_text()==tour['scenes'][0]['title'])
 # forged same-window message must not be accepted as frame status.
 page.evaluate("()=>window.postMessage({channel:'openxplorer-demo',phase:'error',text:'forged'},'*')")
 page.wait_for_timeout(60)
 check('Parent rejects forged status from a non-frame source','forged' not in page.locator('[data-demo-status]').inner_text())
 page.close()
 page=open_page('docs-introduction.html',reduced=True)
 page.locator('[data-product-demo]').scroll_into_view_if_needed();page.wait_for_function('()=>document.querySelector("[data-product-demo]").dataset.phase==="ready"')
 check('Reduced motion still requires user initiation',page.locator('[data-demo-command="stop"]').is_hidden())
 page.locator('[data-demo-command="play"]').click();page.wait_for_function('()=>document.querySelector("[data-product-demo]").dataset.phase==="complete"',timeout=20000)
 check('Tour works with reduced motion')
 page.close()
 for f in files:
  if not f.name.startswith('docs-'):continue
  page=open_page(f.name)
  check(f.name+': Markdown button identifies this guide',page.locator('[data-copy-markdown]').get_attribute('data-copy-markdown')==f.stem[5:])
  check(f.name+': no direct download links',page.locator('a[download]').count()==0)
  check(f.name+': left sidebar and right contents',page.locator('.docs-sidebar').is_visible() and page.locator('.docs-toc').is_visible())
  page.close()
 page=open_page('docs-network-shares.html')
 page.locator('[data-search-open]').first.focus();page.keyboard.press('Control+k')
 check('Control-K is not intercepted',not page.locator('#docs-search').evaluate('el=>el.open'))
 page.keyboard.press('Meta+k')
 check('Command-K is not intercepted',not page.locator('#docs-search').evaluate('el=>el.open'))
 page.locator('[data-search-open]').first.click();expect(page.locator('#docs-search')).to_be_visible()
 page.locator('#docs-search-input').fill('snapshot')
 check('Button-based search returns matching guides',page.locator('#docs-search-results a').count()>0)
 check('Search highlights matched words',page.locator('#docs-search-results mark').count()>0)
 page.locator('#docs-search-input').fill('<img src=x onerror=alert(1)>')
 check('Search text is not inserted as HTML',page.locator('#docs-search-results img').count()==0)
 page.keyboard.press('Escape')
 check('Escape closes search and restores button focus',page.evaluate('document.activeElement.hasAttribute("data-search-open")'))
 # Local HTTP is a secure context, so the page uses the asynchronous clipboard.
 page.evaluate("()=>{window.__copied='';Object.defineProperty(navigator,'clipboard',{value:{writeText:t=>{window.__copied=t;return Promise.resolve();}}});document.execCommand=cmd=>{window.__copied=document.querySelector('textarea')?.value||'';return cmd==='copy';}}")
 page.locator('[data-copy-markdown]').click()
 copied=page.evaluate('window.__copied')
 check('Copies complete Markdown, not page URL',copied.startswith('# SMB & network shares\n') and '## ' in copied and len(copied)>2000)
 check('Copied Markdown includes screenshot references','![The Network page' in copied)
 check('Copied Markdown includes the literal UNC example',r'\\studio-nas\Projects' in copied)
 check('Copied Markdown does not contain rendered HTML','<iframe' not in copied and '<h1' not in copied)
 check('Copy feedback is visible','Copied' in page.locator('#site-toast').inner_text())
 page.set_viewport_size({'width':390,'height':844});page.wait_for_timeout(100)
 check('Documentation has no narrow-screen page overflow',page.evaluate('document.documentElement.scrollWidth<=innerWidth+1'))
 page.close()
 check('No JavaScript exceptions',not errors)
 check('No external HTTP requests',not [u for u in requests if u.startswith(('http:','https:')) and not u.startswith(BASE)])
 b.close()
report={'passed':True,'checks':len(checks),'details':checks,'pages':len(files),'relativeLinks':count,'errors':errors,'externalRequests':[u for u in requests if u.startswith(('http:','https:')) and not u.startswith(BASE)],'scope':'Chromium on standalone pages served over local HTTP, the script-only sandboxed tour of native app pictures and real pointer handlers. Clipboard adapter mocked; links audited on disk. NOT Next/React hydration or HTTP delivery.'}
(OUT/'website-tests.json').write_text(json.dumps(report,indent=2)+'\n');print('Passed',len(checks),'website checks')
