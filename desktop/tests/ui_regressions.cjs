// SPDX-License-Identifier: AGPL-3.0-only
// Actual shared UI in isolated Chromium; simulated storage and update transport.
// Run: CHROMIUM=/path/to/chromium node desktop/tests/ui_regressions.cjs
'use strict';
const assert = require('node:assert/strict');
const {spawn} = require('node:child_process');
const {createHash} = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {pathToFileURL} = require('node:url');

const executable = process.env.CHROMIUM || ['/usr/bin/chromium', '/usr/bin/chromium-browser', '/opt/brave.com/brave/brave'].find(p => fs.existsSync(p));
assert.ok(executable, 'Set CHROMIUM to an installed Chromium browser.');
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'openxplorer-ui-regressions-'));
const ui = path.resolve(__dirname, '../ui');
let html = fs.readFileSync(path.join(ui, 'index.html'), 'utf8');
const hashes = [];
for (const filename of ['bootstrap.js', 'text-size.js', 'type-select.js', 'snapshot-meta.js', 'app.js']) {
  const source = fs.readFileSync(path.join(ui, filename), 'utf8');
  hashes.push("'sha256-" + createHash('sha256').update(source).digest('base64') + "'");
  html = html.replace('<script src="' + filename + '"></script>', '<script>' + source + '</script>');
}
html = html.replace("script-src 'self'", 'script-src ' + hashes.join(' '));
html = html.replace('<link rel="stylesheet" href="style.css">', '<style>' + fs.readFileSync(path.join(ui, 'style.css'), 'utf8') + '</style>');
fs.writeFileSync(path.join(temporary, 'index.html'), html);
const browser = spawn(executable, ['--headless', '--no-sandbox', '--disable-gpu', '--no-first-run', '--remote-debugging-pipe', '--user-data-dir=' + path.join(temporary, 'profile')], {stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe']});
const exited = new Promise(resolve => browser.once('exit', resolve));
const pending = new Map();
let nextId = 0, buffer = '', session, count = 0;
const pageErrors = [];
browser.stdio[4].on('data', chunk => {
  buffer += chunk;
  for (let end; (end = buffer.indexOf('\0')) >= 0;) {
    const raw = buffer.slice(0, end); buffer = buffer.slice(end + 1);
    if (!raw) continue;
    const data = JSON.parse(raw);
    if (data.method === 'Runtime.exceptionThrown') pageErrors.push(data.params.exceptionDetails);
    if (!data.id) continue;
    const promise = pending.get(data.id); if (!promise) continue;
    pending.delete(data.id); clearTimeout(promise.timer);
    if (data.error) promise.reject(Error(JSON.stringify(data.error))); else promise.resolve(data.result);
  }
});
function command(method, params = {}, sessionId = session) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    const timer = setTimeout(() => {pending.delete(id); reject(Error('Timed out: ' + method));}, 15000);
    pending.set(id, {resolve, reject, timer});
    browser.stdio[3].write(JSON.stringify({id, method, params, ...(sessionId ? {sessionId} : {})}) + '\0');
  });
}
async function evaluate(expression) {
  const response = await command('Runtime.evaluate', {expression, awaitPromise: true, returnByValue: true});
  if (response.exceptionDetails) throw Error(JSON.stringify(response.exceptionDetails));
  return response.result.value;
}
async function until(expression) {
  for (let i = 0; i < 150; i++) {
    if (await evaluate(expression)) return;
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  throw Error('Condition not reached: ' + expression);
}
async function press(key, code = key, virtual = key === 'Enter' ? 13 : key === 'Escape' ? 27 : 0) {
  for (const type of ['keyDown', 'keyUp']) await command('Input.dispatchKeyEvent', {type, key, code, windowsVirtualKeyCode: virtual, ...(type==='keyDown'&&key==='Enter'?{text:'\r',unmodifiedText:'\r'}:{})});
}
function check(label, value) {assert.ok(value, label); console.log('PASS', label); count++;}

(async () => {
  try {
    const target = await command('Target.createTarget', {url: pathToFileURL(path.join(temporary, 'index.html')).href});
    session = (await command('Target.attachToTarget', {targetId: target.targetId, flatten: true})).sessionId;
    await command('Runtime.enable');
    await command('Emulation.setDeviceMetricsOverride', {width:1280,height:800,deviceScaleFactor:1,mobile:false});
    await until('Boolean(window.OpenXplorer?.state?.tabs[0]?.loaded)');
    await evaluate(`window.reviewCalls=[];window.reviewTransport=OpenXplorer.previewTransport.call;OpenXplorer.previewTransport.call=function(method,args){reviewCalls.push({method,args});return reviewTransport.call(this,method,args)};`);
    await evaluate('document.getElementById("newtab").focus()'); await press('Enter');
    await until('OpenXplorer.state.tabs.length===2&&OpenXplorer.state.tabs[1].loaded');
    check('Enter activates the New tab button', true);
    await evaluate(`document.querySelector('.tab.active .tab-close').focus()`); await press('Enter');
    await until('OpenXplorer.state.tabs.length===1');
    check('Enter activates the nested Close tab button', true);
    await evaluate(`document.querySelector('.file-row[data-uri="file:///home/demo/Downloads"]').click();document.getElementById('view').focus()`); await press('Enter');
    check('Enter on toolbar opens its menu without opening the selection', await evaluate(`!document.getElementById('menu').hidden&&OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri==='file:///home/demo'`));
    await evaluate(`[...document.querySelectorAll('#menu button')].find(b=>b.textContent==='Large icons').focus()`); await press('Enter');
    check('Enter activates the focused menu item', await evaluate(`OpenXplorer.state.view==='grid'&&document.getElementById('menu').hidden`));
    await evaluate(`document.querySelector('.side-entry[data-uri="file:///home/demo/Documents"]').focus()`); await press('Enter');
    await until(`OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri==='file:///home/demo/Documents'&&!OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).busy`);
    check('Enter activates a sidebar folder', true);
    await evaluate(`document.querySelector('.file-tile[data-uri="file:///home/demo/Documents/Client%20projects"]').click()`); await press('Enter');
    await until(`OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).uri.endsWith('/Client%20projects')&&!OpenXplorer.state.tabs.find(t=>t.id===OpenXplorer.state.activeId).busy`);
    check('Enter still opens the selected folder from the file pane', true);

    check('Delayed activation stays with its original tab', await evaluate(`(async()=>{const x=OpenXplorer;await x.navigate('file:///home/demo');const original=x.state.activeId,real=x.previewTransport.call;let resolve;x.previewTransport.call=function(m,a){return m==='activateItem'?new Promise(r=>resolve=r):real.call(this,m,a)};const operation=x.openEntry({uri:'file:///home/demo/Documents'});x.addTab('file:///home/demo/Downloads');const other=x.state.activeId;resolve({action:'directory',uri:'file:///home/demo/Documents'});await operation;x.previewTransport.call=real;return x.state.activeId===other&&x.state.tabs.find(t=>t.id===original).uri==='file:///home/demo/Documents'&&x.state.tabs.find(t=>t.id===other).uri==='file:///home/demo/Downloads'})()`));
    check('Navigation supersedes a delayed activation', await evaluate(`(async()=>{const x=OpenXplorer,real=x.previewTransport.call;let resolve;x.previewTransport.call=function(m,a){return m==='activateItem'?new Promise(r=>resolve=r):real.call(this,m,a)};const operation=x.openEntry({uri:'file:///home/demo/Documents'});await x.navigate('file:///home/demo/Pictures');resolve({action:'directory',uri:'file:///home/demo/Documents'});await operation;x.previewTransport.call=real;return x.state.tabs.find(t=>t.id===x.state.activeId).uri==='file:///home/demo/Pictures'})()`));
    check('Closing the original tab discards its pending activation', await evaluate(`(async()=>{const x=OpenXplorer,real=x.previewTransport.call,original=x.state.activeId;let resolve;x.previewTransport.call=function(m,a){return m==='activateItem'?new Promise(r=>resolve=r):real.call(this,m,a)};const operation=x.openEntry({uri:'file:///home/demo/Documents'});x.closeTab(original);const survivor=x.state.tabs.find(t=>t.id===x.state.activeId),uri=survivor.uri;resolve({action:'directory',uri:'file:///home/demo/Documents'});await operation;x.previewTransport.call=real;return !x.state.tabs.some(t=>t.id===original)&&survivor.uri===uri})()`));

    await evaluate(`OpenXplorer.navigate('file:///home/demo')`);
    await evaluate(`OpenXplorer.state.query='Downloads';document.getElementById('search').value='Downloads';OpenXplorer.runSearch()`);
    check('Parent search keeps visible matches outside cached descendants', await evaluate(`OpenXplorer.state.searchResults.some(e=>e.uri==='file:///home/demo/Downloads')`));
    check('Partial cache coverage is stated and offers parent indexing', await evaluate(`document.getElementById('search-info').textContent.includes('Other subfolders are not indexed.')&&document.getElementById('search-info').textContent.includes('Cache this folder')`));
    await evaluate(`OpenXplorer.state.query='Brand guidelines';OpenXplorer.runSearch()`);
    check('Parent search also keeps cached descendant matches', await evaluate(`OpenXplorer.state.searchResults.some(e=>e.name==='Brand guidelines.pdf')`));
    await evaluate(`OpenXplorer.navigate('file:///home/demo/Documents')`);
    await evaluate(`OpenXplorer.state.query='Brand guidelines';OpenXplorer.runSearch()`);
    check('Live and cached matches are deduplicated', await evaluate(`OpenXplorer.state.searchResults.filter(e=>e.name==='Brand guidelines.pdf').length===1`));
    await evaluate(`OpenXplorer.state.searchScope='all';OpenXplorer.state.query='Downloads';OpenXplorer.runSearch()`);
    check('All cached folders remains limited to the chosen cache', await evaluate(`!OpenXplorer.state.searchResults.some(e=>e.uri==='file:///home/demo/Downloads')`));

    await evaluate(`OpenXplorer.navigate('file:///home/demo')`);
    check('Folder browsing and search do not initiate update checks', await evaluate('!reviewCalls.some(c=>c.method.startsWith("update"))'));
    await evaluate(`document.getElementById('check-updates').focus()`); await press('Enter');
    await until(`document.getElementById('update-status')?.textContent.includes('Preview only')`);
    check('Manual updater check works in the offline preview', await evaluate(`reviewCalls.filter(c=>c.method==='updateCheck').length===1&&!reviewCalls.some(c=>c.method==='updateInstall')`));
    await press('Escape');
    await evaluate(`window.updateReplies={available:true,version:'9.9.9',currentVersion:'1.0.0',notes:'<img src=x onerror=alert(1)>\\nRelease notes',canInstall:true};OpenXplorer.previewTransport.call=function(method,args){reviewCalls.push({method,args});if(method==='updateCheck')return Promise.resolve({...updateReplies});if(method==='updateInstall')return new Promise((resolve,reject)=>{window.finishInstall=resolve;window.failInstall=reject});if(method==='updateRestart')return Promise.resolve(true);return reviewTransport.call(this,method,args)};document.getElementById('check-updates').click()`);
    await until(`!document.querySelector('.update-install').disabled`);
    check('Release notes render as plain text', await evaluate(`document.getElementById('update-notes').textContent.includes('<img')&&!document.getElementById('update-notes').querySelector('img')`));
    await evaluate(`document.querySelector('.update-install').click()`);
    await until(`Boolean(OpenXplorer.state.updateInstalling)`);
    await evaluate(`window.__nativeEvent('updateProgress',{message:'Installing verified package…'})`);
    check('Installation progress is announced', await evaluate(`document.getElementById('update-status').textContent==='Installing verified package…'`));
    await press('Escape');
    check('Installing locks dismissal and application controls', await evaluate(`!document.getElementById('modal-layer').hidden&&document.querySelector('.update-close').disabled&&document.getElementById('app').inert`));
    check('Install request includes explicit version and confirmation', await evaluate(`reviewCalls.some(c=>c.method==='updateInstall'&&c.args.version==='9.9.9'&&c.args.confirmed===true)`));
    await evaluate(`finishInstall({installed:true,version:'9.9.9'})`);
    await until(`!OpenXplorer.state.updateInstalling&&!document.querySelector('.update-restart').hidden`);
    check('Successful installation offers an explicit restart', await evaluate(`!document.getElementById('app').inert&&document.getElementById('update-status').textContent.includes('Restart')`));
    await evaluate(`document.querySelector('.update-restart').click()`);
    check('Restart uses the separate native action', await evaluate(`reviewCalls.some(c=>c.method==='updateRestart')`));
    await press('Escape');

    await evaluate(`updateReplies.restartRequired=true;document.getElementById('check-updates').click()`);
    await until(`!document.querySelector('.update-restart').hidden`);
    check('Reopened updater preserves a required restart', await evaluate(`document.querySelector('.update-install').hidden`));
    await press('Escape');
    await evaluate(`updateReplies.restartRequired=false;document.getElementById('check-updates').click()`);
    await until(`!document.querySelector('.update-install').disabled`);
    await evaluate(`document.querySelector('.update-install').click()`);
    await until(`Boolean(OpenXplorer.state.updateInstalling)`);
    await evaluate(`failInstall(Error('Administrator approval was declined.'))`);
    await until(`!OpenXplorer.state.updateInstalling`);
    check('Failed installation preserves the error and allows rechecking', await evaluate(`document.getElementById('update-status').textContent.includes('Administrator approval was declined.')&&!document.querySelector('.update-check').disabled&&document.querySelector('.update-install').hidden`));
    await evaluate(`document.querySelector('.update-check').click()`);
    await until(`!document.querySelector('.update-install').disabled`);
    await evaluate(`document.querySelector('.update-install').click()`);
    await until(`Boolean(OpenXplorer.state.updateInstalling)`);
    await evaluate(`updateReplies.restartRequired=true;failInstall(Error('Package configuration failed.'))`);
    await until(`!OpenXplorer.state.updateInstalling`);
    check('Partial package failure offers restart while preserving the error', await evaluate(`document.getElementById('update-status').textContent.includes('Package configuration failed.')&&!document.querySelector('.update-restart').hidden`));
    await press('Escape');
    await evaluate(`updateReplies.restartRequired=false;updateReplies.notes='Keyboard navigation, search and safer file transfers.\\nInstall this release, then restart OpenXplorer.';document.getElementById('check-updates').click()`);
    await until(`!document.querySelector('.update-install').disabled`);
    if(process.env.SCREENSHOT){const capture=await command('Page.captureScreenshot',{format:'png'});fs.writeFileSync(process.env.SCREENSHOT,Buffer.from(capture.data,'base64'));}
    await command('Emulation.setDeviceMetricsOverride', {width:800,height:600,deviceScaleFactor:1,mobile:false});
    await evaluate('OpenXplorer.applyTextSize(200)');
    check('Updater fits a narrow desktop window at 200% text size', await evaluate(`(()=>{const m=document.getElementById('modal'),r=m.getBoundingClientRect();return m.scrollWidth<=m.clientWidth+1&&r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=innerHeight})()`));
    check('No uncaught JavaScript exceptions', pageErrors.length === 0);
    console.log(`${count} checks passed (Chromium shared UI; simulated filesystem/updater; no native installation).`);
  } finally {
    browser.kill(); await exited;
    for (const promise of pending.values()) clearTimeout(promise.timer);
    fs.rmSync(temporary, {recursive: true, force: true, maxRetries: 5, retryDelay: 100});
  }
})().catch(error => {console.error(error); process.exitCode = 1;});
