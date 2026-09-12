/* SPDX-License-Identifier: AGPL-3.0-only */
/* Small progressive enhancements. No analytics, external requests, or filesystem access. */
(function(){
 'use strict';
 function init(){
  if(document.documentElement.dataset.oxReady)return;
  document.documentElement.dataset.oxReady='true';
  const dialog=document.getElementById('docs-search'),input=document.getElementById('docs-search-input'),results=document.getElementById('docs-search-results');
  let origin=null,timer=null;
  const index=(window.__OX_DOCS__||[]).flatMap(d=>d.sections.map(s=>({slug:d.slug,doc:d.title,id:s.id,title:s.title,text:[...s.paragraphs,s.code||'',...(s.items||[])].join(' ')})));
  const toast=text=>{const el=document.getElementById('site-toast');if(!el)return;el.textContent=text;el.classList.add('visible');clearTimeout(timer);timer=setTimeout(()=>el.classList.remove('visible'),2200);};
  function highlight(el,text,term){
   if(!term){el.textContent=text;return;}
   const i=text.toLocaleLowerCase().indexOf(term.toLocaleLowerCase());
   if(i<0){el.textContent=text;return;}
   el.append(document.createTextNode(text.slice(0,i)));
   const mark=document.createElement('mark');mark.textContent=text.slice(i,i+term.length);el.append(mark,document.createTextNode(text.slice(i+term.length)));
  }
  function path(slug,id,q){
   const base=window.__OX_OFFLINE__?'docs-'+slug+'.html':'/docs/'+slug+'/';
   return base+(q?'?q='+encodeURIComponent(q):'')+'#'+encodeURIComponent(id);
  }
  function search(){
   if(!results||!input)return;
   const q=input.value.trim().toLowerCase(),tokens=q.split(/\s+/).filter(Boolean);results.replaceChildren();
   let matched=index.map(x=>({...x,score:tokens.reduce((n,t)=>n+(x.title.toLowerCase().includes(t)?8:0)+(x.doc.toLowerCase().includes(t)?4:0)+(x.text.toLowerCase().includes(t)?1:0),0)}));
   if(tokens.length)matched=matched.filter(x=>tokens.every(t=>(x.doc+' '+x.title+' '+x.text).toLowerCase().includes(t))).sort((a,b)=>b.score-a.score);
   matched=matched.slice(0,10);
   if(!matched.length){const p=document.createElement('p');p.className='search-empty';p.textContent='No matching topics. Try “SMB”, “Downloads” or “install”.';results.append(p);return;}
   for(const x of matched){const a=document.createElement('a');a.href=path(x.slug,x.id,q);const small=document.createElement('small');small.textContent=x.doc;const title=document.createElement('strong');highlight(title,x.title,q);const p=document.createElement('p');let start=q?Math.max(0,x.text.toLowerCase().indexOf(q)-35):0;const excerpt=(start?'…':'')+x.text.slice(start,start+155);highlight(p,excerpt,q);a.append(small,title,p);results.append(a);}
  }
  function openSearch(){if(!dialog||!input)return;origin=document.activeElement;search();dialog.showModal();requestAnimationFrame(()=>input.focus());}
  function closeSearch(){if(dialog?.open)dialog.close();origin?.focus?.();}
  async function copy(text){
   try{if(navigator.clipboard&&window.isSecureContext)await navigator.clipboard.writeText(text);else{const area=document.createElement('textarea');area.value=text;area.style.cssText='position:fixed;opacity:0';document.body.append(area);area.select();const ok=document.execCommand('copy');area.remove();if(!ok)throw Error('Clipboard unavailable');}toast('Copied to clipboard');}
   catch{toast('Clipboard unavailable. Select and copy the text.');}
  }
  document.addEventListener('click',e=>{
   const target=e.target instanceof Element?e.target:null;if(!target)return;
   if(target.closest('[data-search-open]'))openSearch();
   if(target.closest('[data-search-close]'))closeSearch();
   const menu=target.closest('[data-menu-toggle]');if(menu){const nav=document.querySelector('[data-mobile-nav]');nav.hidden=!nav.hidden;menu.setAttribute('aria-expanded',String(!nav.hidden));}
   if(target.closest('[data-mobile-nav] a'))closeMobileMenu(false);
   if(!target.closest('.site-header'))closeMobileMenu(false);
   const code=target.closest('[data-copy-code]');if(code)void copy(code.closest('.code-block').querySelector('code').textContent);
   const md=target.closest('[data-copy-markdown]');if(md){const text=window.__OX_MARKDOWN__?.[md.dataset.copyMarkdown];if(text)void copy(text);else toast('Markdown unavailable. Use the .md download below.');}
   const hash=target.closest('a[href^="#"]');if(hash){const dest=document.getElementById(hash.getAttribute('href').slice(1));if(dest){dest.classList.remove('search-highlight');void dest.offsetWidth;dest.classList.add('search-highlight');}}
  });
  function closeMobileMenu(restore){const nav=document.querySelector('[data-mobile-nav]'),button=document.querySelector('[data-menu-toggle]');if(!nav||nav.hidden)return;nav.hidden=true;button?.setAttribute('aria-expanded','false');if(restore)button?.focus();}
  document.addEventListener('keydown',e=>{
   if(e.key==='Escape'&&!dialog?.open)closeMobileMenu(true);
   // No global Command-K / Control-K interception; search opens from its labeled button.
   if(e.key==='Escape'&&dialog?.open){e.preventDefault();closeSearch();}
   if(dialog?.open){const links=Array.from(results.querySelectorAll('a'));if(e.key==='ArrowDown'){e.preventDefault();links[Math.min(links.length-1,links.indexOf(document.activeElement)+1)]?.focus();}if(e.key==='ArrowUp'){e.preventDefault();const n=links.indexOf(document.activeElement);n<=0?input.focus():links[n-1]?.focus();}if(e.key==='Enter'&&document.activeElement===input){e.preventDefault();links[0]?.click();}}
  });
  const demos=[...document.querySelectorAll('[data-product-demo]')];
  function sendDemo(host,command,value){if(!host)return;if(command==='play'&&(!host.dataset.phase||host.dataset.phase==='loading')){host.dataset.pendingPlay='true';return;}if(command==='stop')delete host.dataset.pendingPlay;if(command==='reset'||command==='scene'){host.dataset.phase='loading';host.querySelector('[data-demo-status]').textContent='Loading sample files…';}const frame=host?.querySelector('iframe');frame?.contentWindow?.postMessage({channel:'openxplorer-demo-command',command,value},'*');}
  const desktopDemo=window.matchMedia('(min-width: 960px)');
  function syncDemos(){
   if(desktopDemo.matches)closeMobileMenu(false);
   for(const host of demos){
    const frame=host.querySelector('iframe');
    if(!desktopDemo.matches){sendDemo(host,'stop');frame?.remove();host.dataset.phase='disabled';delete host.dataset.pendingPlay;continue;}
    if(frame)continue;
    const template=host.querySelector('[data-demo-template]');if(!template)continue;
    host.dataset.phase='loading';
    const clone=template.content.cloneNode(true),next=clone.querySelector('iframe');
    next?.addEventListener('load',()=>sendDemo(host,'status'));
    host.querySelector('.demo-viewport').append(clone);setTimeout(()=>sendDemo(host,'status'),0);
   }
  }
  if(desktopDemo.addEventListener)desktopDemo.addEventListener('change',syncDemos);else desktopDemo.addListener(syncDemos);
  syncDemos();
  for(const host of demos){
   host.addEventListener('click',e=>{const b=e.target.closest('[data-demo-command]');if(!b||!desktopDemo.matches)return;sendDemo(host,b.dataset.demoCommand,b.dataset.demoValue);
    if(b.dataset.demoCommand==='theme'){const dark=b.dataset.demoValue==='dark';b.dataset.demoValue=dark?'light':'dark';b.setAttribute('aria-label','Switch preview to '+(dark?'light':'dark')+' mode');b.querySelector('span').textContent=dark?'Light':'Dark';}
   });
  }
  document.querySelectorAll('[data-play-tour]').forEach(link=>link.addEventListener('click',()=>{const host=demos[0];if(!host)return;document.getElementById('demo')?.scrollIntoView({block:'start'});setTimeout(()=>sendDemo(host,'play'),350);}));
  window.addEventListener('message',event=>{
   const host=demos.find(h=>h.querySelector('iframe')?.contentWindow===event.source);
   if(!host||event.data?.channel!=='openxplorer-demo')return;
   const {phase,text}=event.data;
   if(!['ready','playing','complete','stopped','error'].includes(phase)||typeof text!=='string')return;
   host.dataset.phase=phase;host.querySelector('[data-demo-status]').textContent=text.slice(0,500);
   host.querySelector('[data-demo-command="play"]').disabled=phase==='playing';host.querySelector('[data-demo-command="stop"]').hidden=phase!=='playing';
   for(const b of host.querySelectorAll('.demo-scenes button,[data-demo-command="reset"]'))b.disabled=phase==='playing';
   if(phase==='ready'&&host.dataset.pendingPlay==='true'){delete host.dataset.pendingPlay;sendDemo(host,'play');}
  });
  input?.addEventListener('input',search);
  dialog?.addEventListener('click',e=>{if(e.target===dialog){const r=dialog.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)closeSearch();}});
  dialog?.addEventListener('cancel',e=>{e.preventDefault();closeSearch();});
  const sections=Array.from(document.querySelectorAll('[data-doc-section]')),toc=Array.from(document.querySelectorAll('.docs-toc nav a'));
  if(sections.length&&'IntersectionObserver'in window){const visible=new Set();const update=()=>{const candidate=sections.find(s=>visible.has(s.id));if(!candidate)return;for(const a of toc){const active=a.hash==='#'+candidate.id;a.classList.toggle('active',active);if(active)a.setAttribute('aria-current','location');else a.removeAttribute('aria-current');}};const observer=new IntersectionObserver(entries=>{for(const entry of entries)entry.isIntersecting?visible.add(entry.target.id):visible.delete(entry.target.id);update();},{rootMargin:'-95px 0px -52% 0px',threshold:0});sections.forEach(s=>observer.observe(s));}
  if(location.hash){const dest=document.getElementById(decodeURIComponent(location.hash.slice(1)));if(dest){setTimeout(()=>{dest.scrollIntoView({block:'start'});if(new URLSearchParams(location.search).has('q'))dest.classList.add('search-highlight');},100);}}
 }
 if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',init,{once:true});else init();
})();
