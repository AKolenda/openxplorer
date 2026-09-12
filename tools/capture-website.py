#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Render shipped standalone pages and actual embedded app for documentation.

Uses the same presentational components, inline assets and script-only sandbox
as designs/. Not a Next.js production rendering or React hydration test.
"""
import json,os,io,hashlib
from PIL import Image
from pathlib import Path
from playwright.sync_api import sync_playwright
ROOT=Path(__file__).resolve().parents[1];D=ROOT/'designs';OUT=ROOT/'test-results';OUT.mkdir(exist_ok=True)
with sync_playwright() as p:
 b=p.chromium.launch(executable_path=os.environ.get('CHROMIUM','/usr/bin/chromium'),args=['--no-sandbox'])
 def open_page(name,width=1440,height=1000):
  page=b.new_page(viewport={'width':width,'height':height});page.set_default_timeout(10000)
  page.set_content((D/name).read_text(),wait_until='load')
  # Load below-the-fold PNGs for deterministic captures, without changing layout.
  page.evaluate("()=>document.querySelectorAll('img').forEach(i=>i.loading='eager')")
  page.wait_for_function('()=>Array.from(document.images).every(i=>i.complete&&i.naturalWidth>0)')
  return page
 page=open_page('index.html');page.screenshot(path=str(OUT/'website-top.png'))
 # Crop a full-page capture taken at scroll zero so the sticky header does not cover the section.
 box=page.locator('#features').bounding_box();shot=Image.open(io.BytesIO(page.screenshot(full_page=True)))
 shot.crop((round(box['x']),round(box['y']),round(box['x']+box['width']),round(box['y']+box['height']))).save(OUT/'website-bento.png')
 page.locator('#demo').scroll_into_view_if_needed();page.wait_for_function('()=>document.querySelector("[data-product-demo]").dataset.phase==="ready"')
 page.locator('[data-demo-command="play"]').click();page.wait_for_function('()=>["complete","error"].includes(document.querySelector("[data-product-demo]").dataset.phase)',timeout=20000)
 assert page.locator('[data-product-demo]').get_attribute('data-phase')=='complete'
 page.locator('#demo').screenshot(path=str(OUT/'website-tour.png'))
 page.close();page=open_page('docs-interface.html');page.screenshot(path=str(OUT/'website-docs.png'));page.close()
 page=open_page('index.html',390,844);page.screenshot(path=str(OUT/'website-mobile.png'));assert page.evaluate('document.documentElement.scrollWidth<=innerWidth+1');page.close()
 b.close()
# A relative, checked-in website screenshot for the website README.
assets=ROOT/'docs/assets';assets.mkdir(exist_ok=True)
(assets/'website-home.png').write_bytes((OUT/'website-top.png').read_bytes())
(assets/'website-bento.png').write_bytes((OUT/'website-bento.png').read_bytes())
(assets/'manifest.json').write_text(json.dumps({'source':'tools/capture-website.py; generated designs only','fictional':True,'sha256':{f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in assets.glob('*.png')}},indent=2)+'\n')
(OUT/'website-screenshot-manifest.json').write_text(json.dumps({'renderer':'Chromium, standalone HTML','nextBuild':False,'demoStorage':'fictional fixtures','screenshots':['website-top.png','website-bento.png','website-tour.png','website-docs.png','website-mobile.png']},indent=2)+'\n')
print('Captured homepage, bento, completed real-UI tour, docs and mobile.')
