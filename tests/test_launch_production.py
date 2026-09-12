#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Exercise the actual Next.js static export over local HTTP after pnpm build.

Checks React hydration, metadata, keyboard controls, preview isolation and
mobile iframe disposal. Uses fictional preview data; does not deploy or access
native files, real network shares, credentials or external applications.
"""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread
from xml.etree import ElementTree
import json
import os

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'apps/web/out'
checks, errors = [], []


def check(name, yes=True):
    assert yes, name
    checks.append(name)
    print('PASS', name, flush=True)


class Handler(SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


def main():
    if not (OUT/'index.html').is_file():
        raise SystemExit('Run pnpm build before the production website checks.')
    server = ThreadingHTTPServer(('127.0.0.1',0), partial(Handler,directory=str(OUT)))
    Thread(target=server.serve_forever,daemon=True).start()
    base = f'http://127.0.0.1:{server.server_port}'
    try:
        with sync_playwright() as pw:
            launch = {'args':['--no-sandbox']}
            if os.environ.get('CHROMIUM'):
                launch['executable_path'] = os.environ['CHROMIUM']
            browser = pw.chromium.launch(**launch)
            page = browser.new_page(viewport={'width':1440,'height':1000},reduced_motion='reduce')
            page.set_default_timeout(8000)
            page.on('pageerror',lambda error:errors.append(str(error)))
            for path, canonical in [('/','/'),('/source/','/source/'),
                                    ('/docs/interface/','/docs/interface/'),('/docs/','/docs/introduction/')]:
                response = page.goto(base+path)
                page.wait_for_timeout(150)
                check(path+' production page loads',response.status==200)
                check(path+' canonical points to confirmed domain',page.locator('link[rel=canonical]').get_attribute('href')=='https://openxplorer.app'+canonical)
                check(path+' social sharing metadata is complete',
                      page.locator('meta[property="og:image"]').get_attribute('content')=='https://openxplorer.app/assets/screenshots/explorer-light.png' and
                      page.locator('meta[name="twitter:card"]').get_attribute('content')=='summary_large_image')
                check(path+' has one primary heading',page.locator('h1').count()==1)
            page.goto(base+'/')
            page.wait_for_timeout(200)
            check('Hero screenshot loads eagerly at high priority',
                  page.locator('.product-hero-image img').get_attribute('loading')=='eager' and
                  page.locator('.product-hero-image img').get_attribute('fetchpriority')=='high')
            check('Header links to the public repository',page.locator('.header-download').get_attribute('href')=='https://github.com/AKolenda/openxplorer-public')
            check('Native file dragging links to its compatibility guide',page.locator('.bento-pins a[href="/docs/interface/#file-drag-drop"]').count()==1)
            page.locator('[data-search-open]').click()
            page.locator('#docs-search-input').fill('drag')
            check('Search works after production hydration',page.locator('#docs-search-results a').count()>0)
            page.keyboard.press('Escape')
            check('Search Escape restores focus',page.locator('[data-search-open]').evaluate('(e)=>document.activeElement===e'))
            page.locator('#demo').scroll_into_view_if_needed()
            page.wait_for_function('document.querySelector("[data-product-demo]").dataset.phase==="ready"')
            check('Production hydration preserves the inert iframe template',page.locator('[data-demo-template]').evaluate('(e)=>e.content.querySelectorAll("iframe").length')==1)
            check('Desktop production preview loads in a script-only sandbox',
                  page.locator('iframe').count()==1 and page.locator('iframe').get_attribute('sandbox')=='allow-scripts')
            for width in [320,390,768,959]:
                page.set_viewport_size({'width':width,'height':844})
                page.wait_for_timeout(120)
                check(f'{width}px removes the running iframe',page.locator('iframe').count()==0 and len(page.frames)==1)
                check(f'{width}px has no page overflow',page.evaluate('document.documentElement.scrollWidth<=innerWidth+1'))
                check(f'{width}px GitHub call to action fits',page.locator('#download .button').evaluate('(e)=>{const r=e.getBoundingClientRect();return r.left>=0&&r.right<=innerWidth}'))
            page.goto(base+'/docs/interface/')
            page.locator('[data-menu-toggle]').click()
            check('Production mobile menu shows every guide',page.locator('.mobile-doc-group a').count()==10)
            page.keyboard.press('Escape')
            check('Mobile menu Escape restores focus',page.locator('[data-menu-toggle]').evaluate('(e)=>document.activeElement===e'))
            page.set_viewport_size({'width':1440,'height':1000})
            page.wait_for_timeout(150)
            check('Documentation displays the current release',page.locator('.docs-label code').inner_text()=='1.0.0-rc.4')
            check('Documentation release label fits its sidebar',page.locator('.docs-label').evaluate('(e)=>e.scrollWidth<=e.clientWidth'))
            page.goto(base+'/source/')
            check('Source page omits maintainer-only configuration instructions','apps/web/lib/site.ts' not in page.locator('main').inner_text())
            check('Source page links to the public GitHub repository',page.locator('main a[href="https://github.com/AKolenda/openxplorer-public"]').count()==1)
            page.goto(base+'/concepts/')
            check('Design alternatives are excluded from indexing','noindex' in page.locator('meta[name=robots]').get_attribute('content'))
            check('No production JavaScript or React hydration errors',not errors)
            browser.close()
    finally:
        server.shutdown()
        server.server_close()
    check('robots.txt publishes the sitemap','Sitemap: https://openxplorer.app/sitemap.xml' in (OUT/'robots.txt').read_text())
    check('Website export contains no release downloads',not (OUT/'downloads').exists())
    ns = '{http://www.sitemaps.org/schemas/sitemap/0.9}'
    urls = ElementTree.fromstring((OUT/'sitemap.xml').read_text()).findall(ns+'url/'+ns+'loc')
    check('Sitemap includes ten guides plus home and source',len(urls)==12)
    check('Sitemap contains only confirmed first-party pages',all(url.text.startswith('https://openxplorer.app/') and '/concepts/' not in url.text for url in urls))
    result = ROOT/'test-results/launch-production.json'
    result.parent.mkdir(exist_ok=True)
    result.write_text(json.dumps({'passed':True,'checks':len(checks),'details':checks,
        'scope':'Next.js production static export served on local HTTP; Chromium hydration, metadata, desktop preview, keyboard controls and mobile iframe disposal. No deployment or native application validation.'},indent=2)+'\n')
    print('Passed',len(checks),'production launch checks')


if __name__=='__main__':
    main()
