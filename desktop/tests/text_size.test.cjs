// SPDX-License-Identifier: AGPL-3.0-only
const test=require('node:test'),assert=require('node:assert/strict');const size=require('../ui/text-size.js');
for(const value of size.levels)test('supported '+value,()=>assert.equal(size.normalize(value),value));
for(const value of [undefined,null,0,'125',Infinity,NaN,99,1000,true])test('fallback '+String(value),()=>assert.equal(size.normalize(value),100));
for(const key of ['+','=','Add'])test('Ctrl '+key,()=>assert.equal(size.action({ctrlKey:true,key}),'increase'));
for(const key of ['-','_','Subtract'])test('Ctrl '+key,()=>assert.equal(size.action({ctrlKey:true,key}),'decrease'));
test('Ctrl 0',()=>assert.equal(size.action({ctrlKey:true,key:'0'}),'reset'));
test('keypad add',()=>assert.equal(size.action({ctrlKey:true,code:'NumpadAdd'}),'increase'));
test('keypad subtract',()=>assert.equal(size.action({ctrlKey:true,code:'NumpadSubtract'}),'decrease'));
test('keypad zero',()=>assert.equal(size.action({ctrlKey:true,code:'Numpad0'}),'reset'));
test('unmodified typing ignored',()=>assert.equal(size.action({key:'+'}),null));
test('AltGraph ignored',()=>assert.equal(size.action({ctrlKey:true,key:'+',getModifierState:n=>n==='AltGraph'}),null));
test('Alt ignored',()=>assert.equal(size.action({ctrlKey:true,altKey:true,key:'+'}),null));
test('composing ignored',()=>assert.equal(size.action({ctrlKey:true,isComposing:true,key:'+'}),null));
test('provisional IME ignored',()=>assert.equal(size.action({ctrlKey:true,keyCode:229,key:'+'}),null));
test('Ctrl+C not captured',()=>assert.equal(size.action({ctrlKey:true,key:'c'}),null));
test('bounded stepping',()=>{assert.equal(size.step(80,-1),80);assert.equal(size.step(200,1),200);assert.equal(size.step(100,1),110);assert.equal(size.step(150,-1),125)});
test('default metrics unchanged',()=>assert.deepEqual(size.metrics(100),{scale:1,detailRow:38,gridRow:130,gridWidth:135}));
test('large text row clearance',()=>{for(const value of size.levels){const m=size.metrics(value);assert.ok(m.detailRow>=12*m.scale*1.45);assert.ok(m.gridRow>=130);assert.ok(m.gridWidth>=135)}});
