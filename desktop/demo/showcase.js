// SPDX-License-Identifier: AGPL-3.0-only
/* Included ONLY by build_preview.py. No native filesystem bridge and no network.
   Guided tour drives the same real UI click / pointer handlers as the desktop.
   Incoming postMessages accept fixed demo commands, never arbitrary paths. */
(()=>{
 'use strict';
 if(window.__OPENXPLORER_NATIVE__)return;
 const params=new URLSearchParams(location.search),embedded=params.get('embed')==='1'||window.name==='openxplorer-website-demo';
 const sleep=ms=>new Promise(r=>setTimeout(r,ms));
 let generation=0,running=false,cursor=null,api=null,dragNode=null,lastStatus={phase:'ready',text:'Sample files only. Explore the real interface.'};
 const reduced=()=>matchMedia('(prefers-reduced-motion: reduce)').matches;
 const announce=(phase,text)=>{lastStatus={phase,text};document.documentElement.dataset.tourPhase=phase;window.parent!==window&&window.parent.postMessage({channel:'openxplorer-demo',phase,text},'*');};
 const current=()=>api.state.tabs.find(t=>t.id===api.state.activeId);
 async function wait(test,ms=6000){const end=performance.now()+ms;while(!test()){if(performance.now()>end)throw Error('Demo could not load. Reset it and try again.');await sleep(30);}}
 function pointer(type,node,x,y){node.dispatchEvent(new PointerEvent(type,{bubbles:true,cancelable:true,clientX:x,clientY:y,pointerId:91,pointerType:'mouse',isPrimary:true,button:0,buttons:type==='pointerup'||type==='pointercancel'?0:1}));}
 function stop(){generation++;running=false;if(dragNode){pointer('pointercancel',document,0,0);dragNode=null;}cursor?.remove();cursor=null;document.querySelectorAll('.demo-hover').forEach(el=>el.classList.remove('demo-hover'));announce('stopped','Tour stopped. You can use the sample files yourself.');}
 async function scene(name){
  if(document.getElementById('modal-layer')?.hidden===false)api.closeModal();
  const uri={home:'file:///home/demo/Documents',nas:'smb://studio-nas/',projects:'smb://studio-nas/Projects',snapshots:'smb://archive-nas/Shared',settings:'settings:'}[name]||'file:///home/demo/Documents';
  await api.navigate(uri);await wait(()=>current()?.loaded&&!current().busy);
  if(name==='snapshots'){
   const info={uri:'smb://archive-nas/Shared/Launch%20planning',name:'Launch planning',isDir:true};
   void api.propertiesDialog(info,'versions');await wait(()=>document.querySelector('.version-row'));
  }
 }
 async function reset(name='home'){
  stop();await api.call('bookmark',{kind:'pin',action:'remove',uri:'smb://studio-nas/Projects/Design'});await api.refreshEnvironment();
  await scene(name);announce('ready','Sample files only. Double-click folders or play the pinning tour.');
 }
 async function play(){
  if(!api||running)return;
  await reset('home');running=true;const ticket=++generation;
  const guard=()=>{if(ticket!==generation)throw Error('cancelled');};
  const delay=async ms=>{await sleep(reduced()?100:ms);guard();};
  function point(el){const r=el.getBoundingClientRect();return{x:r.left+Math.min(r.width/2,95),y:r.top+r.height/2};}
  async function move(el,drag=false){guard();el.scrollIntoView({block:'nearest'});const p=point(el);const from={x:parseFloat(cursor.style.left)||innerWidth/2,y:parseFloat(cursor.style.top)||100};
   el.classList.add('demo-hover');const steps=reduced()?1:28;
   for(let i=1;i<=steps;i++){guard();const u=i/steps,t=u*u*(3-2*u),x=from.x+(p.x-from.x)*t,y=from.y+(p.y-from.y)*t;cursor.style.left=x+'px';cursor.style.top=y+'px';if(drag)pointer('pointermove',document,x,y);await sleep(reduced()?0:22);}
   await delay(260);el.classList.remove('demo-hover');return p;
  }
  async function click(el,double=false){await move(el);cursor.classList.add('pressed');await delay(130);await wait(()=>performance.now()>=(api.state.suppressClickUntil||0));guard();el.click();if(double)el.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));cursor.classList.remove('pressed');await delay(460);}
  try{
   cursor=document.createElement('div');cursor.className='demo-cursor';cursor.setAttribute('aria-hidden','true');cursor.innerHTML='<svg width="28" height="34" viewBox="0 0 28 34"><path d="M3 2v26l7-7 5 11 5-3-6-10h10Z" fill="#fff" stroke="#123d63" stroke-width="2"/></svg>';document.body.append(cursor);
   announce('playing','Open the sample NAS with a Windows-style path.');
   const addr=document.getElementById('address-edit');await click(addr);const input=document.getElementById('address-input');input.value='';
   const address='\\\\studio-nas\\Projects';
   for(const ch of address){guard();input.value+=ch;await delay(40);}
   input.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));await wait(()=>current()?.uri==='smb://studio-nas/Projects'&&!current().busy);guard();await delay(550);
   announce('playing','Drag Design into the pinned folders. This creates a shortcut, not a copy.');
   const row=[...document.querySelectorAll('.file-row')].find(r=>r.querySelector('.name-text')?.textContent==='Design');if(!row)throw Error('Sample Design folder not found.');
   const p=await move(row);row.click();pointer('pointerdown',row,p.x,p.y);dragNode=row;cursor.classList.add('pressed');await delay(240);
   const target=document.querySelector('#quick-access .quick-drop-tail')||document.getElementById('quick-access');const end=await move(target,true);await delay(500);pointer('pointerup',document,end.x,end.y);dragNode=null;cursor.classList.remove('pressed');
   await wait(()=>api.state.env.quick.some(p=>p.uri==='smb://studio-nas/Projects/Design'));guard();await delay(420);
   const pin=[...document.querySelectorAll('#quick-access .side-entry')].find(r=>r.dataset.uri==='smb://studio-nas/Projects/Design');if(!pin)throw Error('Pin was not added.');await click(pin);
   await wait(()=>current()?.uri==='smb://studio-nas/Projects/Design'&&!current().busy);guard();
   announce('complete','Design is pinned. The green marker identifies a network folder. Try it yourself.');
  }catch(e){if(ticket===generation)announce('error',e.message);}
  finally{if(ticket===generation){running=false;cursor?.remove();cursor=null;}}
 }
 async function boot(){
  await wait(()=>window.OpenXplorer?.state.ready&&window.OpenXplorer.state.tabs.length>0);api=window.OpenXplorer;
  // Opaque sandbox origins have no storage. Clipboard preview is still memory-only.
  try{void localStorage.length;}catch{const memory=new Map();try{Object.defineProperty(window,'localStorage',{value:{getItem:k=>memory.get(k)||null,setItem:(k,v)=>memory.set(k,String(v)),removeItem:k=>memory.delete(k)}});}catch{}}
  if(embedded){document.body.style.padding='0';document.body.classList.add('website-embed');}
  if(embedded||['light','dark'].includes(params.get('theme')))api.applyTheme(params.get('theme')==='dark'?'dark':'light',false);
  window.OpenXplorerTour={play,stop,reset,scene};
  window.addEventListener('message',async e=>{
   if(e.source!==window.parent||e.source===window||e.data?.channel!=='openxplorer-demo-command')return;
   const {command,value}=e.data;
   try{if(command==='status')announce(lastStatus.phase,lastStatus.text);else if(command==='play')void play();else if(command==='stop')stop();else if(command==='reset')await reset();else if(command==='scene'&&['home','nas','projects','snapshots','settings'].includes(value)){stop();await scene(value);announce('ready','Sample files only. Explore the real interface.');}else if(command==='theme'&&['light','dark'].includes(value))api.applyTheme(value,false);}catch(e){announce('error',e.message);}
  });
  document.addEventListener('pointerdown',e=>{if(e.isTrusted&&running)stop();},true);
  document.addEventListener('keydown',e=>{if(e.isTrusted&&running)stop();},true);
  if(embedded||params.has('scene'))await reset(params.get('scene')||'home');
  announce('ready','Sample files only. Double-click folders or play the pinning tour.');
 }
 void boot().catch(e=>announce('error',e.message));
})();
