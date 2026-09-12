// SPDX-License-Identifier: AGPL-3.0-only
// Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {Controller, TIMEOUT_MS, MAX_PREFIX, findPrefix, isTypingKey} = require('../ui/type-select.js');
const rows = names => names.map(name => ({name}));
const list = rows(['Backups', 'Shared documents', 'Shipping', 'scripts', 'work']);

test('first character finds the first name when no item is selected', () => {
  assert.equal(new Controller().push('s', list, -1, 0).index, 1);
});
test('fresh typing starts after the current item', () => {
  assert.equal(new Controller().push('s', list, 1, 0).index, 2);
});
test('search wraps around the display list', () => {
  assert.equal(new Controller().push('s', list, 4, 0).index, 1);
});
test('SC refines to scripts without needing a search query', () => {
  const c = new Controller(); const first = c.push('S', list, 0, 0);
  const next = c.push('C', list, first.index, 100);
  assert.deepEqual(next, {text:'SC', index:3, cycling:false});
});
test('refining keeps an already matching selected row', () => {
  const c = new Controller(); c.push('s', list, 2, 0);
  assert.equal(c.push('c', list, 3, 100).index, 3);
});
test('repeated single letters cycle and wrap', () => {
  const c = new Controller(); let index = -1;
  const found = [0,100,200,300].map(t => (index = c.push('s',list,index,t).index));
  assert.deepEqual(found, [1,2,3,1]);
});
test('repeated letters are case-insensitive', () => {
  const c = new Controller(); c.push('s',list,0,0);
  assert.deepEqual(c.push('S',list,1,100), {text:'S',index:2,cycling:true});
});
test('prefix resets at the timeout boundary', () => {
  const c = new Controller(); c.push('s',list,0,0);
  assert.equal(c.push('w',list,1,TIMEOUT_MS).text,'w');
});
test('typing before timeout extends the prefix', () => {
  const c = new Controller(); c.push('s',list,0,0);
  assert.equal(c.push('c',list,1,TIMEOUT_MS-1).text,'sc');
});
test('unmatched text is retained, not silently replaced by the last letter', () => {
  const c = new Controller(); c.push('s',list,0,0);
  assert.deepEqual(c.push('z',list,1,100),{text:'sz',index:-1,cycling:false});
});
test('Backspace corrects an unmatched prefix', () => {
  const c = new Controller(); c.push('s',list,0,0); c.push('z',list,1,10);
  assert.equal(c.backspace(list,1,20).index,1);
  assert.equal(c.text,'s');
});
test('Backspace can empty a prefix without choosing a row', () => {
  const c = new Controller(); c.push('s',list,0,0);
  assert.deepEqual(c.backspace(list,1,10),{text:'',index:-1,cycling:false});
});
test('Backspace after expiry has no effect on selection', () => {
  const c = new Controller(); c.push('s',list,0,0);
  assert.equal(c.backspace(list,1,1100),null); assert.equal(c.text,'');
});
test('explicit reset clears accumulated text', () => {
  const c = new Controller(); c.push('s',list,0,0);c.reset();
  assert.equal(c.active(10),false);assert.equal(c.text,'');
});
test('matching is by prefix, not substring', () => {
  assert.equal(findPrefix(rows(['Old scripts','scripts']), 'sc'),1);
});
test('supplied sort order is preserved', () => {
  assert.equal(findPrefix(rows(['scripts-z','scripts-a']), 'sc'),0);
});
test('empty listings are safe', () => {
  assert.equal(new Controller().push('s',[],0,0).index,-1);
});
test('out-of-range anchors start at the beginning', () => {
  for (const at of [900,-90,NaN,1.5]) assert.equal(findPrefix(list,'s',at),1);
});
test('Unicode case-insensitive filename selection', () => {
  assert.equal(findPrefix(rows(['Документы','СКРИПТЫ']), 'ск'),1);
});
test('canonical Unicode accents match equivalent filenames', () => {
  assert.equal(findPrefix(rows(['e\u0301tudes']), 'É'),0);
});
test('supplementary Unicode characters can be typed', () => {
  assert.equal(isTypingKey({key:'📁'}),true);
  assert.equal(new Controller().push('📁',rows(['📁 Documents']),-1,0).index,0);
});
test('spaces within a filename are significant', () => {
  const c = new Controller(); c.push('s',list,0,0); c.push('h',list,1,10);
  for(const [i,key] of [...'ared '].entries()) c.push(key,list,1,20+i);
  assert.equal(c.text,'shared '); assert.equal(c.push('d',list,1,40).index,1);
});
test('punctuation and digits are supported', () => {
  assert.equal(findPrefix(rows(['0 notes','_scripts','.env']),'_'),1);
  assert.equal(isTypingKey({key:'.'}),true); assert.equal(isTypingKey({key:'1'}),true);
});
test('all entry types participate, without activation', () => {
  const list=[{name:'script.pdf',isDir:false},{name:'scripts',isDir:true}];
  assert.equal(findPrefix(list,'sc'),0);
});
test('no mutation of input records or their order', () => {
  const list=Object.freeze([Object.freeze({name:'z'}),Object.freeze({name:'scripts'})]);
  assert.equal(new Controller().push('s',list,-1,0).index,1);
});
test('prefix length is bounded', () => {
  const c = new Controller();
  for(let i=0;i<1000;i++)c.push(i%2?'b':'a',[], -1,i);
  assert.equal(Array.from(c.text).length,MAX_PREFIX);
});
test('a backwards clock resets the buffer', () => {
  const c = new Controller(); c.push('s',list,-1,100);
  assert.equal(c.push('w',list,-1,90).text,'w');
});
test('Shift and Caps Lock do not block uppercase letters', () => {
  assert.equal(isTypingKey({key:'S',shiftKey:true}),true);
});
for (const modifier of ['ctrlKey','altKey','metaKey','isComposing','defaultPrevented']) {
  test(modifier+' is never treated as ordinary file-list typing', () => {
    assert.equal(isTypingKey({key:'s',[modifier]:true}),false);
  });
}
test('IME keycode 229 is never interpreted', () => {
  assert.equal(isTypingKey({key:'s',keyCode:229}),false);
});
test('named keys and control characters are not printable prefixes', () => {
  for(const key of ['Enter','Dead','Process','Unidentified','Backspace','Shift','Tab','F2','\n','\x7f',''])
    assert.equal(isTypingKey({key}),false,key);
});
test('invalid key input does not alter the buffer', () => {
  const c = new Controller();c.push('s',list,-1,0);
  assert.equal(c.push('Enter',list,1,10),null);assert.equal(c.text,'s');
});
test('invalid timeout is rejected', () => {
  for(const value of [-1,0,NaN,Infinity])assert.throws(()=>new Controller(value),TypeError);
});
