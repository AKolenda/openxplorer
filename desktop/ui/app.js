// SPDX-License-Identifier: AGPL-3.0-only
// Modified 2026-09-06; original notices: licenses/Winspace-MIT.txt.
'use strict';
/* OpenXplorer 1.1.0 — one interface, two transports: a native GIO bridge and an
   explicitly simulated, offline preview. No libraries, CDNs or web services. */
(() => {
const $ = id => document.getElementById(id);
const native = window.__OPENXPLORER_NATIVE__ === true;
const UI_RELEASE = '1.1.0';
if (native) document.body.classList.add('native');
document.body.classList.toggle('dark', document.documentElement.dataset.theme === 'dark');
const NS = 'http://www.w3.org/2000/svg';
const paths = {
  terminal:'M3 5h18v14H3zM6 9l3 3-3 3M12 15h5',
  settings:'M10 2h4l1 3 3 1 3 2-2 3v2l2 3-3 2-3 1-1 3h-4l-1-3-3-1-3-2 2-3v-2L3 8l3-2 3-1zM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
  plus:'M12 5v14M5 12h14', close:'m6 6 12 12M18 6 6 18', minus:'M5 12h14', maximize:'M5 5h14v14H5z',
  back:'m12 5-7 7 7 7M5 12h14', forward:'m12 5 7 7-7 7M5 12h14', up:'m5 12 7-7 7 7M12 5v14',
  refresh:'M19 10a7 7 0 1 0-1 7M19 4v6h-6', down:'m7 10 5 5 5-5', chevron:'m9 6 6 6-6 6',
  search:'M16 16l5 5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0',
  home:'m3 11 9-8 9 8M5 10v10h5v-6h4v6h5V10',
  desktop:'M3 4h18v13H3zM9 21h6M12 17v4', downloads:'M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5',
  documents:'M6 3h9l4 4v14H6zM14 3v5h5M9 12h7M9 16h7', pictures:'M3 4h18v16H3zM3 17l6-6 4 4 3-3 5 5M16 8h.01',
  music:'M9 18V5l11-2v13M9 7l11-2M9 18c0 2-6 3-6 0s6-3 6 0Zm11-2c0 2-6 3-6 0s6-3 6 0Z',
  videos:'M4 4h16v16H4zM4 8h16M4 16h16M8 4v4M16 4v4M8 16v4M16 16v4',
  pin:'m8 3 9 9M15 4l5 5-5 2-3 5-4-4-5-1 5-3 2-5M9 15l-6 6',
  cut:'m9 9 10 12M9 15 19 3M9 7a3 3 0 1 1-6 0 3 3 0 0 1 6 0Zm0 10a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z',
  copy:'M8 8h12v13H8zM16 8V3H3v13h5', paste:'M8 5H5v16h14V5h-3M9 3h6v4H9z',
  rename:'M3 7h7v10H3zM16 3v18M13 3h6M13 21h6M20 7h2v10h-2',
  share:'M14 4h7v7M21 4 10 15M10 5H4v16h16v-6',
  trash:'M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7M14 10v7',
  sort:'M7 3v18m-4-4 4 4 4-4M14 5h7M14 10h5M14 15h3',
  grid:'M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z',
  list:'M3 5h2M9 5h12M3 12h2M9 12h12M3 19h2M9 19h12',
  details:'M3 4h18v16H3zM15 4v16M18 8h.01M18 12h.01M18 16h.01',
  more:'M4 12h.01M12 12h.01M20 12h.01',
  network:'M8 3h8v6H8zM3 16h6v5H3zM15 16h6v5h-6zM12 9v4M6 16v-3h12v3',
  server:'M5 3h14v7H5zM5 14h14v7H5zM8 6.5h.01M8 17.5h.01M12 6.5h4M12 17.5h4',
  drive:'m5 5-3 11v5h20v-5L19 5zM2 16h20M17 18.5h.01M20 18.5h.01',
  phone:'M8 2h8a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2zM10 5h4M11 19h2',
  check:'m5 12 4 4L19 6', shield:'m12 2 8 3v6c0 5-8 11-8 11S4 16 4 11V5zM8 11l3 3 5-6',
  info:'M12 11v6M12 7h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
  sun:'M12 2v2M12 20v2M2 12h2M20 12h2m-3-7 1.5-1.5M5 19l-1.5 1.5m0-17L5 5m14 14 1.5 1.5M17 12a5 5 0 1 1-10 0 5 5 0 0 1 10 0',
  moon:'M20 15A9 9 0 0 1 9 4a9 9 0 1 0 11 11Z',
  eject:'m5 14 7-10 7 10zM5 20h14', clock:'M12 7v6l4 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
  link:'m9 15 6-6M8 16l-1 1a4 4 0 0 1-6-6l5-5a4 4 0 0 1 6 0m0 2 1-1a4 4 0 0 1 6 6l-5 5a4 4 0 0 1-6 0',
  eye:'M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
  folderline:'M3 7V4h7l2 3h9v13H3z', cancel:'M5 5l14 14M5 19 19 5'
};
function icon(name, size=18) {
  if (name === 'folder') return folderIcon(size);
  if (name === 'zip') return zipFolderIcon(size);
  const s = document.createElementNS(NS,'svg');
  if(name==='terminal')s.dataset.icon='terminal';
  s.setAttribute('viewBox','0 0 24 24'); s.setAttribute('width',size);s.setAttribute('height',size);
  s.setAttribute('fill','none');s.setAttribute('stroke','currentColor');s.setAttribute('stroke-width',name==='more'?'3':'1.35');
  if(name==='phone')s.dataset.icon='phone';
  s.setAttribute('stroke-linecap','round');s.setAttribute('stroke-linejoin','round');s.classList.add('svg-icon');s.setAttribute('aria-hidden','true');
  const p=document.createElementNS(NS,'path');p.setAttribute('d',paths[name]||paths.documents);s.append(p);return s;
}
function svgEl(tag,attrs){const e=document.createElementNS(NS,tag);for(const[k,v]of Object.entries(attrs))e.setAttribute(k,v);return e;}
function folderIcon(size=24,mark=''){
  const s=svgEl('svg',{viewBox:'0 0 48 48',width:size,height:size,'aria-hidden':'true'});s.classList.add('svg-icon');
  s.append(svgEl('path',{d:'M4 12a3 3 0 0 1 3-3h12l5 5h17a3 3 0 0 1 3 3v20a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z',fill:'#d99a22'}));
  s.append(svgEl('path',{d:'M5 16h36v6H5z',fill:'#fff0bd'}));
  s.append(svgEl('path',{d:'M4 20h17l4-4h17a3 3 0 0 1 3 3l-2 19a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z',fill:'#ffce56'}));
  s.append(svgEl('path',{d:'M4 25h40l-1 13a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z',fill:'#f7bd40'}));
  s.append(svgEl('path',{d:'M6 21h15l4-4h16',fill:'none',stroke:'#fff0a9','stroke-width':'1'}));
  if(mark){const t=svgEl('text',{x:28,y:34,'font-size':13,'text-anchor':'middle',fill:'#936d11','font-family':'sans-serif','font-weight':600});t.textContent=mark;s.append(t);}return s;
}
function isZipEntry(entry){return !!entry&&!entry.isDir&&(/\.zip$/i.test(entry.name||'')||['application/zip','application/x-zip','application/x-zip-compressed'].includes(entry.contentType));}
function zipFolderIcon(size=24){
  const s=folderIcon(size);s.dataset.icon='zip-folder';
  s.append(svgEl('rect',{x:28,y:15,width:7,height:25,rx:1,fill:'#d59622'}));
  for(let i=0;i<6;i++)s.append(svgEl('rect',{x:i%2?31.5:28,y:16+i*3,width:3.5,height:2.5,rx:.4,fill:'#fff3c5'}));
  s.append(svgEl('rect',{x:27.5,y:32,width:8,height:8,rx:2,fill:'#647789',stroke:'#f8edce','stroke-width':.8}));
  s.append(svgEl('rect',{x:29.5,y:34,width:4,height:3.5,rx:.7,fill:'#ffdb70'}));
  return s;
}
function fileIcon(entry,size=24){
  if(entry.isDir){
    const s=folderIcon(size);s.dataset.icon=entry.isVirtual?'network-folder':'folder';
    if(entry.isVirtual){
      s.append(svgEl('rect',{x:27,y:29,width:20,height:17,rx:3,fill:'var(--bg)'}));
      s.append(svgEl('path',{d:'M34 31h6v4h-6zM37 35v4m-6 0h12m-12 0v4h4v-4m4 0v4h4v-4',fill:'none',stroke:'var(--accent)','stroke-width':'1.4','stroke-linejoin':'round'}));
    }
    return s;
  }
  const ext=(entry.name||'').split('.').pop().toLowerCase();
  if(isZipEntry(entry))return zipFolderIcon(size);
  const map={pdf:['#c84032','PDF'],docx:['#2868bd','W'],doc:['#2868bd','W'],xlsx:['#258150','X'],csv:['#258150','X'],pptx:['#ce6a35','P'],zip:['#a8833d','ZIP'],png:['#7d69bd',''],jpg:['#7d69bd',''],jpeg:['#7d69bd',''],webp:['#7d69bd',''],mp4:['#975abe','▶'],md:['#65747f',''],txt:['#748da8',''],py:['#3d849e',''],js:['#d0a522','']};
  const [color,letter]=map[ext]||['#8092a3',''];
  const s=svgEl('svg',{viewBox:'0 0 48 48',width:size,height:size,'aria-hidden':'true'});s.classList.add('svg-icon');s.dataset.icon='file';
  s.append(svgEl('path',{d:'M11 3h19l9 9v31a2 2 0 0 1-2 2H11a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z',fill:'var(--doc-paper)',stroke:'var(--doc-stroke)','stroke-width':'1.2'}));
  s.append(svgEl('path',{d:'M30 3v9h9',fill:'var(--doc-fold)',stroke:'var(--doc-stroke)','stroke-width':'1.2'}));
  if(letter){s.append(svgEl('rect',{x:4,y:18,width:30,height:19,rx:2,fill:color}));const t=svgEl('text',{x:19,y:31,'font-size':letter.length>1?10:14,'text-anchor':'middle',fill:'white','font-family':'sans-serif','font-weight':600});t.textContent=letter;s.append(t);}
  else if(['png','jpg','jpeg','webp'].includes(ext)){s.append(svgEl('path',{d:'M14 36V20h20v16Z',fill:'#e3dcf7'}));s.append(svgEl('path',{d:'m14 34 7-8 5 5 4-4 4 7Z',fill:color}));s.append(svgEl('circle',{cx:29,cy:23,r:2,fill:'#f0b253'}));}
  else{s.append(svgEl('path',{d:'M15 22h17M15 27h17M15 32h12',stroke:color,'stroke-width':2}));}return s;
}
function elem(tag,cls,text){const e=document.createElement(tag);if(cls)e.className=cls;if(text!==undefined)e.textContent=text;return e;}
function button(label,fn,cls='',ico=''){const e=elem('button',cls,label);if(ico)e.prepend(icon(ico));e.type='button';if(fn)e.addEventListener('click',event=>{try{Promise.resolve(fn(event)).catch(error=>toast(error.message));}catch(error){toast(error.message);}});return e;}
function setButton(id,ico,label='',arrow=false){const b=$(id);b.replaceChildren(icon(ico));if(label)b.append(document.createTextNode(label));if(arrow){const v=icon('down');v.classList.add('chevron');b.append(v);}}
function prettyBytes(n){if(n===null||n===undefined)return '—';if(n<1024)return `${n} bytes`;const units=['KB','MB','GB','TB'];let u=-1;do{n/=1024;u++;}while(n>=1024&&u<3);return `${n>=100?n.toFixed(0):n.toFixed(1)} ${units[u]}`;}
function dateText(n){return n?new Date(n*1000).toLocaleDateString(undefined,{year:'numeric',month:'2-digit',day:'2-digit'}):'—';}
function deviceParts(uri){const m=typeof uri==='string'&&uri.match(/^(mtp|gphoto2|afc):\/\/([^/?#]+)(\/[^?#]*)?$/i);return m?{protocol:m[1].toLowerCase()+':',host:m[2],pathname:m[3]||'/'}:null;}
function locationParts(uri){return deviceParts(uri)||new URL(uri);}
function deviceLocation(uri){return !!deviceParts(uri);}
function deviceRoot(uri){const u=deviceParts(uri);return u?u.protocol+'//'+u.host+'/':null;}
function deviceMountName(uri){const root=deviceRoot(uri);return(state.env?.mounts||[]).find(m=>m.mounted&&deviceRoot(m.uri)===root)?.label||'Connected device';}
function baseName(uri){try{const u=locationParts(uri),part=u.pathname.split('/').filter(Boolean).pop();return decodeURIComponent(part||'')||(deviceLocation(uri)?deviceMountName(uri):u.host)||'Local Disk';}catch{return uri;}}
function parentUri(uri){try{const d=deviceParts(uri);if(d){const path=d.pathname.replace(/\/$/,'');if(!path)return null;const parent=path.slice(0,path.lastIndexOf('/')+1)||'/';return d.protocol+'//'+d.host+parent;}const u=new URL(uri);let p=u.pathname.replace(/\/$/,'');if(!p)return null;u.pathname=p.slice(0,p.lastIndexOf('/')+1)||'/';return u.href.replace(/\/$/,(u.pathname==='/'?'/':''));}catch{return null;}}
function displayUri(uri){if(uri==='settings:')return 'Settings';if(uri==='home:')return 'Home';if(uri==='pc:')return 'This PC';if(uri==='network:')return 'Network';try{const u=locationParts(uri);if(u.protocol==='smb:')return '\\\\'+u.host+decodeURIComponent(u.pathname).replaceAll('/','\\');if(u.protocol==='file:')return decodeURIComponent(u.pathname);if(deviceLocation(uri)){const path=decodeURIComponent(u.pathname).replace(/^\/+|\/+$/g,'');return deviceMountName(uri)+(path?' / '+path:'');}}catch{}return uri;}
function titleFor(uri){if(uri==='settings:')return 'Settings';if(uri==='home:'||sameLocation(uri,state.env?.home))return'Home';if(uri==='pc:')return'This PC';if(uri==='network:')return'Network';return baseName(uri);}
function uriChild(uri,name){return uri.replace(/\/$/,'')+'/'+encodeURIComponent(name);}
function validateName(name){if(typeof name!=='string'||!name||name==='.'||name==='..'||/[\/\\\x00-\x1f]/.test(name))throw Error('Use a name without slashes or control characters.');return name;}
function normaliseAddress(text,base){
  text=text.trim();
  if(!text)throw Error('Enter a folder path or SMB address.');
  if(/[\x00-\x1f\x7f]/.test(text))throw Error('Control characters are not allowed.');
  if(text.startsWith('\\\\')||text.startsWith('//')){
    const bits=text.replaceAll('\\','/').replace(/^\/+/, '').split('/');
    const server=bits.shift();
    if(!server||/[@:\s%]/.test(server))throw Error('Use a server name without credentials.');
    text='smb://'+server+'/'+bits.map(encodeURIComponent).join('/');
  }
  if(/^[a-z]:[\\/]/i.test(text))throw Error('Windows drive letters are labels, not Linux paths.');
  if(text==='~')text='/home/demo';
  if(text.startsWith('~/'))text='/home/demo/'+text.slice(2);
  if(text.startsWith('/'))text='file://'+text.split('/').map(encodeURIComponent).join('/');
  if(!/^[a-z][a-z0-9+.-]*:/i.test(text))text=uriChild(base&&/^(file|smb):/.test(base)?base:'file:///home/demo',text);
  const u=new URL(text);
  if(!['file:','smb:'].includes(u.protocol))throw Error('Use a local path, smb://server/share, or \\\\server\\share.');
  if(u.username||u.password||u.host.includes('@'))throw Error('Do not put passwords or usernames in the address. Use the OpenXplorer sign-in dialog.');
  if(u.search||u.hash)throw Error('In URLs, encode ? as %3F and # as %23, or use a normal file/UNC path.');
  if(/[\x00-\x1f\x7f]/.test(decodeURIComponent(u.pathname)))throw Error('Encoded control characters are not allowed.');
  if(u.protocol==='file:'&&u.hostname&&u.hostname!=='localhost')throw Error('Use smb:// for a network location.');
  if(u.protocol==='smb:'&&(!u.hostname||/[\s%]/.test(u.hostname)))throw Error('Enter an unescaped SMB server name.');
  return u.href;
}

let seq=0;const pending=new Map();
window.__nativeResolve=(id,result,error)=>{const p=pending.get(id);if(!p)return;pending.delete(id);clearTimeout(p.timer);if(error){const e=Error(error.message||String(error));e.code=error.code;p.reject(e);}else p.resolve(result);};
window.__nativeEvent=(name,data)=>{
  if(name==='fileDragRequest')void beginNativeFileDrag(data.uri);
  if(name==='fileDragStarted')markNativeFileDrag(data);
  if(name==='fileDragFinished')finishNativeFileDrag();
  if(name==='fileDrop')void receiveFileDrop(data);
  if(name==='fileDropHint')showFileDropHint(data);
  if(name==='restoreTab')void restoreTransferredTab(data);
  if(name==='tabDragRequest')void beginNativeTabDrag(data.id);
  if(name==='tabDetachRequested')void detachTab(data.id);
  if(name==='tabReceive')void receiveTransferredTab(data);
  if(name==='tabTransferDone')finishTabTransfer(data);
  if(name==='tabTransferSettled')settleIncomingTab(data);
  if(name==='tabReorder')reorderTab(data.id,data.beforeId);
  if(name==='tabDropHint')showTabDropHint(data);
  if(name==='tabTearOutHint'){const h=$('tab-drag-hint');h.hidden=!data.show;h.textContent='Release to open this tab in a new window';h.style.left=Math.max(12,Math.min(data.x||40,innerWidth-360))+'px';h.style.top=Math.max(56,Math.min(data.y||90,innerHeight-65))+'px';}

  if(name==='fileManagerRequest')void handleFileManagerRequest(data);
  if(name==='showWindows')void windowsMenu();
  if(name==='showSettings')void settingsDialog();
  if(name==='integrationChanged')void updateDefaultStatus();
  if(name==='folderSizeProgress')receiveFolderSize(data);
  if(name==='auth')receiveAuth(data);
  if(name==='authDismiss')dismissAuth(data.id);
  if(name==='mouseNavigate'&&!activeAuth&&$('modal-layer').hidden)goHistory(data.delta);
  if(name==='openLocations'){const uris=data.uris||[];if(uris.length)void openIncoming(uris);}
  if(name==='clipboardChanged')void refreshClipboard();
  if(name==='cacheChanged'){clearTimeout(state.cacheTimer);state.cacheTimer=setTimeout(async()=>{await refreshCacheStatus();if(state.query)void runSearch();},200);}
  if(name==='entries'){const t=state.tabs.find(t=>t.loadToken===data.token);if(t){if(data.reset)t.entries=[];t.entries.push(...data.entries);t.dirty=true;if(t===active())scheduleListing();}}
  if(name==='textSizeChanged')applyTextSize(data.textSize);
  if(name==='theme'){if(state.env)state.env.systemDark=!!data.systemDark;applyTheme(state.theme,false);}
  if(name==='transfer')updateTransfer(data);
  if(name==='updateProgress'&&state.updateInstalling&&$('update-status'))$('update-status').textContent=data.message||'Installing update…';
  if(name==='serverSigningOut'){state.signedOutHosts.add(data.host);state.sessionNetwork=(state.sessionNetwork||[]).filter(s=>new URL(s.uri).hostname!==data.host);}
  if(name==='changed'){const t=active();if(t&&t.uri===data.uri&&!state.operation&&!state.query&&!state.signedOutHosts.has((()=>{try{return new URL(data.uri).hostname;}catch{return '';}})()))load(t,false);}
  if(name==='mounts'||name==='environmentChanged')refreshEnvironment();
  if(name==='close-request')askClose();
  if(name==='notice')toast(data.message);
};
async function call(method,args={}){
  if(!native)return demo.call(method,args);
  if(state.hostMismatch&&!['environment','quit','uiReady'].includes(method))throw Error('Finish file operations, then run openxplorer --restart to load the installed version.');
  const id=++seq;
  return new Promise((resolve,reject)=>{
    const timer=['environment','uiReady','chrome','preferences'].includes(method)?setTimeout(()=>{pending.delete(id);reject(Error('The desktop interface did not respond. Restart OpenXplorer or try software rendering.'));},15000):null;
    pending.set(id,{resolve,reject,timer});
    try{window.webkit.messageHandlers.host.postMessage(JSON.stringify({id,method,args,release:UI_RELEASE}));}
    catch(e){pending.delete(id);clearTimeout(timer);reject(e);}
  });
}
function fire(method,args={}){call(method,args).catch(e=>toast(e.message));}

const state={folderSizes:new Map(),trashSupport:new Map(),sizeRun:null,modalOwner:null,settingsOrigin:null,tabs:[],activeId:null,env:null,selection:new Set(),clipboard:null,view:'details',details:true,showHidden:false,sort:'name',descending:false,operation:null,anchor:-1,query:'',renderQueued:false,modalResolve:null,filterCache:null,filterVersion:0,theme:window.__OPENXPLORER_FIRST_THEME__||'system',drag:null,pinBusy:false,ready:false,searchGeneration:0,searchResults:null,searchScope:'folder',searchBusy:false,cache:{roots:[]},signedOutHosts:new Set(),pointerPending:null,suppressClickUntil:0,discovery:{servers:[],busy:false,started:false,generation:0}};

// 0.5.1: typing in the file pane selects a name; it never starts a search.
const typeSelect = new window.OpenXplorerTypeSelect.Controller();
let typeSelectTimer = null;
function resetTypeSelect(){
  typeSelect.reset();clearTimeout(typeSelectTimer);typeSelectTimer=null;
  const hint=$('type-select-status');if(hint){hint.hidden=true;hint.textContent='';hint.removeAttribute('data-miss');}
}
function typeSelectTarget(target){
  if(!target||target.nodeType!==1||target.closest('input,textarea,select,button,a,[contenteditable]'))return false;
  return target===$('main')||target===$('file-scroll')||target===$('file-canvas')||
    ($('file-canvas').contains(target)&&!!target.closest('.file-row,.file-tile'));
}
function revealEntry(index){
  const sc=$('file-scroll'),metrics=textMetrics(),rowH=state.view==='grid'?metrics.gridRow:metrics.detailRow;
  const cols=state.view==='grid'?Math.max(1,Math.floor(sc.clientWidth/metrics.gridWidth)):1;
  const top=Math.floor(index/cols)*rowH+5;
  if(top<sc.scrollTop)sc.scrollTop=Math.max(0,top-5);
  else if(top+rowH>sc.scrollTop+sc.clientHeight)sc.scrollTop=top+rowH-sc.clientHeight;
  // The matching row may not exist yet: render the new virtual viewport now.
  renderRows();
}
function handleTypeSelect(event){
  if(!typeSelectTarget(event.target)||!active()||$('folder-view').hidden||
      state.drag||state.pointerPending||state.outgoingFile||!$('menu').hidden||!$('modal-layer').hidden||activeAuth)return false;
  const now=performance.now();
  if(event.key==='Escape'&&typeSelect.active(now)){
    resetTypeSelect();event.preventDefault();return true;
  }
  const erase=event.key==='Backspace'&&!event.ctrlKey&&!event.metaKey&&!event.altKey;
  if(!erase&&!window.OpenXplorerTypeSelect.isTypingKey(event))return false;
  const entries=filtered();
  const at=state.anchor>=0&&state.anchor<entries.length&&state.selection.has(entries[state.anchor].uri)
    ?state.anchor:entries.findIndex(item=>state.selection.has(item.uri));
  const result=erase?typeSelect.backspace(entries,at,now):typeSelect.push(event.key,entries,at,now);
  if(!result)return false;
  event.preventDefault();
  if(result.index>=0){
    // No modifier keys here: typing deliberately replaces a prior multi-selection.
    revealEntry(result.index);selectEntry(entries[result.index],result.index);
  }
  clearTimeout(typeSelectTimer);
  if(!result.text){resetTypeSelect();return true;}
  const hint=$('type-select-status');hint.hidden=false;
  hint.dataset.miss=String(result.index<0);
  hint.textContent=result.index>=0?`Jump to: ${result.text} — ${entries[result.index].name}`:
    `No name starts with “${result.text}”`;
  typeSelectTimer=setTimeout(resetTypeSelect,typeSelect.timeoutMs);
  return true;
}

function active(){return state.tabs.find(t=>t.id===state.activeId);}
function selected(){return filtered().filter(e=>state.selection.has(e.uri));}
function addTab(uri='home:',options={}){uri=realLocation(uri);const t={id:'t'+(++seq),uri,entries:[],history:[uri],index:0,loadToken:'',busy:false,error:'',dirty:true,scroll:0};state.tabs.push(t);if(options.background){renderTabs();}else switchTab(t.id);return t;}
// Middle-click never invokes the external file launcher. Deferred background
// loading also avoids moving an SMB credential prompt over the current tab.
// Use mouseup as well as swallowing auxclick: older WebKitGTK releases do not
// expose auxclick. mousedown.preventDefault stops primary-selection paste and
// auto-scroll without cancelling compatibility mouse events at pointerdown.
function bindMiddleClick(node,fn){
  let pressed=false;
  node.addEventListener('mousedown',e=>{if(e.button===1){pressed=true;e.preventDefault();e.stopPropagation();}});
  node.addEventListener('mouseleave',()=>{pressed=false;});
  node.addEventListener('pointercancel',()=>{pressed=false;});
  node.addEventListener('mouseup',e=>{if(e.button!==1)return;const use=pressed;pressed=false;e.preventDefault();e.stopPropagation();if(use)fn(e);});
  node.addEventListener('auxclick',e=>{if(e.button===1){e.preventDefault();e.stopPropagation();}});
}
function bindMiddleOpen(node,getLocation){
  bindMiddleClick(node,e=>{
    if(activeAuth||!$('modal-layer').hidden||state.drag||state.detaching||state.outgoingTab)return;
    const uri=getLocation();if(!uri)return;
    closeMenu();resetTypeSelect();addTab(uri,{background:!e.shiftKey});
  });
}
function switchTab(id){if(id!==state.activeId){suspendTabDialog();closeMenu();}finishAddress();state.anchor=-1;resetSearch();const prev=active();if(prev)prev.scroll=$('file-scroll').scrollTop;state.activeId=id;state.selection.clear();state.query='';$('search').value='';state.filterCache=null;renderTabs();renderNavigation();renderSidebar();renderContent();$('file-scroll').scrollTop=active().scroll;if(!active().loaded)load(active());else updateStatus();restoreTabDialog();applyPendingTabRestore(active());}
function closeTab(id){if(state.outgoingTab===id){toast('Wait for this tab to finish moving.');return;}if(state.tabs.length===1){askClose();return;}const i=state.tabs.findIndex(t=>t.id===id);const t=state.tabs[i];discardTabDialog(t);if(t?.busy)fire('cancel',{token:t.loadToken});state.tabs.splice(i,1);if(state.activeId===id)switchTab(state.tabs[Math.min(i,state.tabs.length-1)].id);else renderTabs();}
function renderTabs(){
  const list=$('tabs');list.replaceChildren();
  for(const t of state.tabs){
    const tab=elem('div','tab'+(t.id===state.activeId?' active':''));tab.dataset.tabId=t.id;
    tab.setAttribute('role','tab');tab.setAttribute('aria-selected',String(t.id===state.activeId));tab.tabIndex=t.id===state.activeId?0:-1;
    const snapshot=snapshotFor(t);
    if(snapshot)tab.classList.add('snapshot-tab');
    const im=t.uri==='settings:'?icon('settings'):t.uri==='network:'?icon('network'):deviceLocation(t.uri)?icon('phone'):folderIcon(17);
    const wrap=elem('span','tab-icon-wrap');wrap.append(im);im.classList.add('tab-icon');
    if(networkLocation(t.uri)){wrap.classList.add('shared');wrap.append(elem('span','shared-bar'));wrap.title='Network location';}
    tab.title=displayUri(t.uri)+(networkLocation(t.uri)?' · Network location':'')+(t.savedDialog?' · Properties open':'');
    tab.append(wrap,elem('span','tab-title',titleFor(t.uri)));
    if(snapshot){const badge=elem('span','snapshot-tab-badge','Previous version');badge.prepend(icon('clock',12));badge.title='Previous version · '+snapshot.label;tab.append(badge);tab.title+=' · Previous version · '+snapshot.label;tab.setAttribute('aria-label',titleFor(t.uri)+' — Previous version — '+snapshot.label);}
    const close=button('',e=>{e.stopPropagation();closeTab(t.id);},'tab-close','close');close.title='Close tab';close.setAttribute('aria-label','Close '+titleFor(t.uri));tab.append(close);
    setupTabDrag(tab,t);tab.addEventListener('click',()=>{if(performance.now()>(state.tabClickSuppress||0))switchTab(t.id);});tab.addEventListener('keydown',e=>{if(e.key==='Enter'&&e.target===tab){e.preventDefault();switchTab(t.id);}});
    bindMiddleClick(tab,()=>closeTab(t.id));list.append(tab);
  }
  requestAnimationFrame(updateChrome);
}
function updateChrome(){if(!native)return;scheduleFileDragLayout();if(state.env?.nativeTabDrag)publishTabDragLayout();fire('windowMetadata',{title:titleFor(active()?.uri||state.env?.home||'home:'),titles:state.tabs.map(t=>titleFor(t.uri))});const r=$('title-drag').getBoundingClientRect();fire('chrome',{x:Math.round(r.x),y:Math.round(r.y),width:Math.max(1,Math.round(r.width)),height:Math.round(r.height)});}
async function navigate(uri,history=true,t=active()){
  if(!t||!state.tabs.includes(t))return;
  if(state.outgoingTab===t.id){toast('Wait for this tab to finish moving.');return;}
  uri=realLocation(uri);t.navigationGeneration=(t.navigationGeneration||0)+1;
  try{state.signedOutHosts.delete(new URL(uri).hostname);}catch{}
  if(t===active()){state.anchor=-1;resetSearch();state.searchScope='folder';state.query='';$('search').value='';state.selection.clear();state.filterCache=null;}
  t.scroll=0;if(history&&t.uri!==uri){t.history=t.history.slice(0,t.index+1);t.history.push(uri);t.index=t.history.length-1;}
  t.uri=uri;t.loaded=false;renderTabs();if(t===active()){renderNavigation();renderSidebar();}await load(t);
}
async function load(t,clear=true){
  if(t===active())resetTypeSelect();
  if(t.busy)fire('cancel',{token:t.loadToken});
  if(['home:','pc:','network:','settings:'].includes(t.uri)){t.busy=false;t.entries=[];t.loaded=true;t.error='';if(t===active())renderContent();if(t.uri==='network:'&&!state.discovery.started)void discoverNetwork();return;}
  const token='list-'+(++seq);t.loadToken=token;t.busy=true;t.error='';if(clear)t.entries=[];t.dirty=true;state.filterCache=null;
  if(t===active()){renderContent();renderNavigation();}
  const oldSelection=new Set(t===active()?state.selection:[]);let batches=[];
  if(!clear){t.entries=[];}
  try{const result=await call('list',{uri:t.uri,token,showHidden:state.showHidden});if(t.loadToken!==token)return;if(result.entries)t.entries=result.entries;t.uri=result.uri||t.uri;t.loaded=true;t.error='';rememberPreviewNetwork(t.uri);void refreshTrashSupport(t.uri);}
  catch(e){if(t.loadToken!==token)return;if(e.code==='not-directory'){const file=t.uri;t.uri=parentUri(file)||state.env.home;t.busy=false;await load(t);void openEntry({uri:file,name:baseName(file)});return;}if(e.code!=='cancelled')t.error=e.message;}
  finally{if(t.loadToken===token){t.busy=false;t.dirty=true;if(t===active()){state.selection=new Set(t.entries.filter(e=>oldSelection.has(e.uri)).map(e=>e.uri));state.filterCache=null;renderContent();renderNavigation();updateStatus();applyPendingTabRestore(t);}}}
}
function scheduleListing(){if(state.renderQueued)return;state.renderQueued=true;setTimeout(()=>{state.renderQueued=false;state.filterCache=null;renderRows();updateStatus();},90);}
function filtered(){const t=active();if(!t)return[];const key=[t.id,t.entries.length,state.searchGeneration,state.query,state.sort,state.descending,state.showHidden].join('|');if(state.filterCache?.key===key&&!t.dirty)return state.filterCache.items;let a=(state.searchResults||t.entries).filter(e=>(state.showHidden||!e.hidden)&&(state.searchResults||state.query.toLocaleLowerCase().trim().split(/\s+/).every(term=>e.name.toLocaleLowerCase().includes(term))));const collator=new Intl.Collator(undefined,{numeric:true,sensitivity:'base'});a.sort((x,y)=>{if(x.isDir!==y.isDir)return x.isDir?-1:1;let n;if(state.sort==='size'||state.sort==='modified')n=(state.sort==='size'?itemSize(x):x[state.sort]||0)-(state.sort==='size'?itemSize(y):y[state.sort]||0);else n=collator.compare(String(x[state.sort]||''),String(y[state.sort]||''));return (state.descending?-1:1)*n||collator.compare(x.name,y.name);});t.dirty=false;state.filterCache={key,items:a};return a;}
function renderNavigation(){const t=active();if(!t)return;renderSnapshotBanner(t);$('back').disabled=t.index===0;$('forward').disabled=t.index>=t.history.length-1;$('up').disabled=['home:','pc:','network:','settings:'].includes(t.uri)||!parentUri(t.uri);$('search').placeholder='Search '+titleFor(t.uri);$('search').title='Search cached filenames and paths. Enable folders in Settings → Search cache.';$('search').disabled=['home:','pc:','network:','settings:'].includes(t.uri)||deviceLocation(t.uri);$('address-icon').replaceChildren(icon(t.uri==='home:'?'home':t.uri==='pc:'?'desktop':t.uri==='network:'||t.uri.startsWith('smb:')?'network':deviceLocation(t.uri)?'phone':'folder'));
  const crumbs=$('breadcrumbs');crumbs.replaceChildren();
  const segments=breadcrumbSegments(t.uri);
  segments.forEach((segment,i)=>{
    if(i&&!(i===1&&segments[0].label==='/')){const divider=elem('span','crumb-divider',t.uri.startsWith('smb:')?'\\':'/');divider.setAttribute('aria-hidden','true');crumbs.append(divider);}
    const b=button(segment.label,e=>{e.stopPropagation();finishAddress();navigate(segment.uri);},'crumb');
    b.dataset.uri=segment.uri;b.title=displayUri(segment.uri);b.setAttribute('aria-label','Go to '+segment.label);
    if(i===segments.length-1)b.setAttribute('aria-current','location');
    bindMiddleOpen(b,()=>segment.uri);b.addEventListener('keydown',e=>{if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();e.stopPropagation();const buttons=[...crumbs.querySelectorAll('button')];buttons[Math.max(0,Math.min(buttons.length-1,buttons.indexOf(b)+(e.key==='ArrowLeft'?-1:1)))]?.focus();}});crumbs.append(b);
  });
  $('address-input').value=displayUri(t.uri);$('address').title=displayUri(t.uri)+' · Click blank space or press Ctrl+L to edit';
  requestAnimationFrame(()=>{crumbs.scrollLeft=crumbs.scrollWidth;});
  renderSearchInfo();updateToolbar();
}

function editAddress(){if(active()?.uri==='settings:')return;const input=$('address-input');input.value=displayUri(active().uri);$('breadcrumbs').hidden=true;input.hidden=false;input.focus();input.select();}
function finishAddress(){$('address-input').hidden=true;$('breadcrumbs').hidden=false;}
async function submitAddress(){try{const r=await call('normalise',{value:$('address-input').value,base:active().uri});finishAddress();await openEntry({uri:r.uri,name:baseName(r.uri)});}catch(e){toast(e.message);}}
function goHistory(delta){const t=active();const n=t.index+delta;if(n<0||n>=t.history.length)return;t.index=n;navigate(t.history[n],false);}
function refreshEnvironment(){return call('environment').then(env=>{state.env=env;applyLayout();renderTabs();if(!native)state.env.editors=[{id:'code.desktop',name:'Visual Studio Code'}];renderSidebar();if(['home:','pc:','network:','settings:'].includes(active()?.uri))renderContent();}).catch(e=>toast(e.message));}
function renderSidebar(){
  scheduleFileDragLayout();
  if(!state.env||state.drag||state.pointerPending||state.outgoingFile)return;
  const sb=$('sidebar');sb.replaceChildren();const current=active()?.uri;
  const add=(label,uri,ico,opts={})=>{
    const b=button('',()=>opts.volume?mountVolume(opts.volume):navigate(uri),'side-entry'+(sameLocation(current,uri)?' selected':'')+(opts.indent?' indent':''));
    b.dataset.uri=uri||'';bindMiddleOpen(b,()=>opts.volume?null:uri);
    if(opts.expand){const v=icon('down');v.classList.add('expand');b.append(v);}
    const im=opts.folder?folderIcon(19):icon(ico);if(opts.color)im.style.color=opts.color;
    const wrap=elem('span','side-icon');wrap.append(im);if(opts.shared||uri?.startsWith('smb:')){wrap.classList.add('shared');wrap.append(elem('span','shared-bar'));wrap.title='Network share';}b.append(wrap,elem('span','name',label));
    if(opts.pin){const p=elem('span','pin');p.append(icon('pin'));b.append(p);}
    if(!opts.volume&&fileDragEntry({uri}))makeFileDraggable(b,()=>[{uri,name:label,isDir:true,fromSidebar:true}]);
    b.title=uri?displayUri(uri):label;
    if(opts.network)b.addEventListener('contextmenu',e=>{e.preventDefault();networkLocationMenu(e.clientX,e.clientY,opts.network);});
    if(!opts.network&&(opts.pin||opts.share||opts.drive))b.addEventListener('contextmenu',e=>{
      e.preventDefault();if(opts.drive){driveMenu(e.clientX,e.clientY,uri,label,opts.volume);return;}sidebarMenu(e.clientX,e.clientY,uri,label,opts.share);
    });
    (opts.parent||sb).append(b);return b;
  };
  add('Home',state.env.home,'home',{color:'#0078d4'});sb.append(elem('div','side-separator'));
  const quick=elem('section','quick-access');quick.id='quick-access';quick.classList.toggle('quick-empty',!state.env.quick.length);quick.setAttribute('aria-label','Quick access — drop folders here to pin');
  // Quick access remains an accessible drop target, without a visible label or counter.
  for(const p of state.env.quick)add(p.label,p.uri,p.icon||'folder',{parent:quick,pin:true,color:p.color||null,folder:!p.icon,shared:p.isShared});
  const tail=elem('div','quick-drop-tail','Pin to Quick access');tail.prepend(icon('plus',13));quick.append(tail);sb.append(quick);
  setupPinDrop(quick);
  sb.append(elem('div','side-separator'));add('This PC','pc:','desktop',{expand:true,color:'#347ba7'});
  add('Local Disk','file:///','drive',{indent:true,drive:true});
  for(const m of state.env.mounts.filter(m=>!m.uri?.startsWith('smb:')))add(m.label,m.uri||'',m.kind==='device'?'phone':'drive',{indent:true,drive:true,volume:!m.mounted?m.id:null});
  sb.append(elem('div','side-separator'));add('Network','network:','network',{expand:true,color:'#318db9'});
  for(const s of networkLocations()){const row=add(s.label,s.uri,'server',{indent:true,shared:true,network:s});row.dataset.networkSaved=String(!!s.saved);row.title=displayUri(s.uri)+' · '+(s.connected?'Connected':s.saved?'Saved · connect on open':'Opened this session');}
}

async function removeBookmark(uri,share){try{await call('bookmark',{action:'remove',uri,kind:share?'share':'pin'});await refreshEnvironment();toast(share?'Saved location removed.':'Unpinned. The folder was not deleted.');}catch(e){toast(e.message);}}
function renderContent(){scheduleFileDragLayout();const t=active();if(!t)return;document.body.classList.toggle('settings-open',t.uri==='settings:');renderSearchInfo();const landing=['home:','pc:','network:','settings:'].includes(t.uri);$('landing').hidden=!landing;$('folder-view').hidden=landing;$('loading-line').hidden=!t.busy;$('empty-state').hidden=true;$('details').hidden=!state.details;$('details-toggle').classList.toggle('toggled',state.details);if(landing)renderLanding(t.uri);else{renderColumns();renderRows();}renderDetails();updateStatus();}
function renderRows(){
  scheduleFileDragLayout();
  applyColumnLayout();
  if(state.drag||state.pointerPending||state.outgoingFile)return;
  if(!active()||['home:','pc:','network:','settings:'].includes(active().uri))return;
  const entries=filtered(),scroll=$('file-scroll'),canvas=$('file-canvas');const grid=state.view==='grid',metrics=textMetrics();const cols=grid?Math.max(1,Math.floor(scroll.clientWidth/metrics.gridWidth)):1;const rowH=grid?metrics.gridRow:metrics.detailRow;const totalRows=Math.ceil(entries.length/cols);canvas.style.height=(totalRows*rowH+12)+'px';canvas.replaceChildren();canvas.setAttribute('aria-rowcount',String(totalRows));
  const firstRow=Math.max(0,Math.floor(scroll.scrollTop/rowH)-3),lastRow=Math.min(totalRows,Math.ceil((scroll.scrollTop+scroll.clientHeight)/rowH)+4);
  for(let i=firstRow*cols;i<Math.min(entries.length,lastRow*cols);i++){const e=entries[i],row=elem('div',grid?'file-tile':'file-row');row.dataset.uri=e.uri;row.dataset.index=i;row.setAttribute('role','row');row.setAttribute('aria-selected',String(state.selection.has(e.uri)));row.setAttribute('aria-label',e.name);row.tabIndex=-1;row.style.top=(Math.floor(i/cols)*rowH+5)+'px';row.style.height=(rowH-2)+'px';if(grid){const w=(scroll.clientWidth-20)/cols;row.style.left=(10+(i%cols)*w)+'px';row.style.width=(w-4)+'px';row.append(fileIcon(e,56),elem('span','tile-name',e.name));}else{const nc=elem('div','cell name-cell');nc.append(fileIcon(e),elem('span','name-text',e.name));row.append(nc,elem('div','cell date-cell',state.query?displayUri(e.parentUri||parentUri(e.uri)||e.uri):dateText(e.modified)),elem('div','cell type-cell',e.type),elem('div','cell size-cell',e.isDir?folderSizeText(e.uri):prettyBytes(e.size)));}
    if(!grid&&e.isDir){const sizeCell=row.querySelector('.size-cell');sizeCell.dataset.sizeValue=e.uri;const measured=state.folderSizes.get(sizeKey(e.uri));if(measured)sizeCell.title=`${measured.status} · ${measured.reason||'Logical file bytes'} · ${measured.updated?new Date(measured.updated*1000).toLocaleString():''}`;}row.classList.toggle('selected',state.selection.has(e.uri));row.classList.toggle('cut',state.clipboard?.mode==='move'&&state.clipboard.uris.includes(e.uri));row.title=state.query?displayUri(e.uri):e.name;row.dataset.kind=e.kind||(e.isDir?'directory':'file');bindMiddleOpen(row,()=>e.isDir?(e.targetUri||e.uri):null);if(fileDragEntry(e))makeFileDraggable(row,()=>state.selection.has(e.uri)?selected():[e]);row.addEventListener('click',ev=>{if(!state.drag&&!state.outgoingFile)selectEntry(e,i,ev);});row.addEventListener('dblclick',ev=>{ev.preventDefault();if(!state.outgoingFile&&performance.now()>=state.suppressClickUntil)openEntry(e);});row.addEventListener('contextmenu',ev=>{ev.preventDefault();if(!state.selection.has(e.uri)){state.selection=new Set([e.uri]);state.anchor=i;renderRows();renderDetails();updateToolbar();updateStatus();}entryMenu(ev.clientX,ev.clientY,e);});canvas.append(row);
  }
  $('empty-state').hidden=entries.length>0||active().busy;
  if(!entries.length&&!active().busy){const box=$('empty-state');box.replaceChildren(icon(active().error?'network':'folderline'),elem('h3','',active().error?'This location is unavailable':state.query?'No matching items':'This folder is empty'),elem('p','',active().error|| (state.query?(state.searchError||((state.searchScope==='all'||cacheRootsFor(active().uri).length)?'No cached matches. Refresh the cache if this folder changed, or try another search.':'Only this folder is being filtered. Enable its search cache to include subfolders.')):'Create a folder or paste files here.')));if(active().error)box.append(button('Try again',()=>load(active())));}
  $('loading-line').hidden=!active().busy;
}
function syncRowSelection(){for(const row of $('file-canvas').children){const yes=state.selection.has(row.dataset.uri);row.classList.toggle('selected',yes);row.setAttribute('aria-selected',String(yes));}}
function selectEntry(e,i,event={}){if(event.shiftKey&&state.anchor>=0){if(!event.ctrlKey)state.selection.clear();const list=filtered();for(let j=Math.min(state.anchor,i);j<=Math.max(state.anchor,i);j++)state.selection.add(list[j].uri);}else if(event.ctrlKey||event.metaKey){if(state.selection.has(e.uri))state.selection.delete(e.uri);else state.selection.add(e.uri);state.anchor=i;}else{state.selection=new Set([e.uri]);state.anchor=i;}syncRowSelection();renderDetails();updateStatus();updateToolbar();$('main').focus({preventScroll:true});}
function clearSelection(){resetTypeSelect();state.anchor=-1;state.selection.clear();renderRows();renderDetails();updateToolbar();updateStatus();}
async function openEntry(e){
  const tab=active(),generation=tab?.navigationGeneration||0;
  if(!tab||tab.opening)return;
  tab.opening=true;
  try{
    const result=await call('activateItem',{uri:e.targetUri||e.uri});
    if(!state.tabs.includes(tab)||(tab.navigationGeneration||0)!==generation)return;
    if(result.action==='directory')await navigate(result.uri,true,tab);
    else if(result.action==='archive'&&tab===active())void archiveDialog(result.entry||e);
    else if(!native)toast('Preview only — this file would open in its file-type application.');
  }catch(error){if(state.tabs.includes(tab)&&(tab.navigationGeneration||0)===generation){if(tab===active())showMessage('Could not open the item',error.message);else toast('Could not open '+(e.name||baseName(e.uri))+': '+error.message);}}
  finally{tab.opening=false;}
}
async function openIncoming(uris){for(let i=0;i<uris.length;i++){if(i)addTab();await openEntry({uri:uris[i],name:baseName(uris[i])});}}

function renderDetails(){const pane=$('details');pane.replaceChildren();if(!state.details)return;const header=elem('div','detail-header','Details');header.append(button('',()=>toggleDetails(),'','close'));pane.append(header);const entries=selected();const e=entries.length===1?entries[0]:null;const preview=elem('div','detail-preview');preview.append(e?fileIcon(e,84):entries.length>1?icon('copy',80):folderIcon(84));pane.append(preview,elem('div','detail-name',e?e.name:entries.length>1?entries.length+' items selected':titleFor(active().uri)),elem('div','detail-type',e?e.type:entries.length>1?'Multiple items':'Folder'));
  if(e){pane.append(button('Open',()=>openEntry(e),'detail-open','share'));if(e.isDir)pane.append(button('Pin to Quick access',()=>pinEntry(e),'detail-open','pin'));}
  else if(!['home:','pc:','network:','settings:'].includes(active().uri))pane.append(button('Pin to Quick access',()=>pinCurrent(),'detail-open','pin'));
  pane.append(elem('div','detail-section','Properties'));const props=elem('dl','detail-props');let values=e?[['Type',e.type],['Size',e.isDir?(folderSizeText(e.uri)||'Not scanned'):prettyBytes(e.size)],['Modified',dateText(e.modified)],['Location',displayUri(parentUri(e.uri)||e.uri)]]:[['Items',String(entries.length||active().entries.length)],['Location',displayUri(active().uri)],['Storage',active().uri.startsWith('smb:')?'Network share':'This computer']];for(const[k,v]of values)props.append(elem('dt','',k),elem('dd','',v));pane.append(props);const note=elem('div','detail-note');note.append(icon('info'),document.createTextNode(!native?'Interactive preview. Files and network locations shown here are sample data.':active().uri.startsWith('smb:')?'Files are accessed through GIO/GVfs. A saved location is not a system-wide drive letter.':'Select an item to see its properties. Double-click to open it.'));pane.append(note);
}
function renderLanding(uri){const l=$('landing');if(uri==='settings:'){renderSettingsPage(l);return;}if(uri==='network:'){l.classList.remove('settings-page');renderNetwork(l);return;}l.classList.remove('settings-page');l.replaceChildren();if(!state.env)return;const title=uri==='home:'?'Home':uri==='pc:'?'This PC':'Network';l.append(elem('h1','',title),elem('p','subtitle',uri==='home:'?'Your folders and network locations, in one place.':uri==='pc:'?'Folders, devices, and connected storage.':'Connect to your NAS, Windows PC, or shared folders.'));
  const section=(name,ico)=>{const h=elem('div','section-title');h.append(icon(ico),document.createTextNode(name));l.append(h);return h;};
  const quick=()=>{section('Quick access','pin');const g=elem('div','quick-grid');for(const p of state.env.quick){const c=button('',()=>navigate(p.uri),'quick-card');bindMiddleOpen(c,()=>p.uri);c.append(folderIcon(43));const d=elem('div');d.append(elem('div','card-name',p.label),elem('div','card-sub',p.uri.startsWith('smb:')?'Network folder':'Stored on this PC'));c.append(d);g.append(c);}l.append(g);};
  const shares=()=>{const h=section('Network locations','network');h.append(button('Map network location',()=>connectDialog()));const g=elem('div','drive-grid');for(const s of state.env.shares){const c=button('',()=>navigate(s.uri),'drive-card');bindMiddleOpen(c,()=>s.uri);const im=icon('server',46);im.style.color='#4b96c0';c.append(im);const d=elem('div','drive-info');d.append(elem('div','card-name',s.label),elem('div','card-sub',displayUri(s.uri)));const stat=elem('div','connected');stat.append(elem('span','status-dot'+(!s.connected?' offline':'')),document.createTextNode(!native?'Sample SMB location':s.connected?'Mounted in this session':'Connect on open'));d.append(stat);c.append(d);c.addEventListener('contextmenu',ev=>{ev.preventDefault();const items=[{label:'Open',icon:'folderline',fn:()=>navigate(s.uri)},{label:'Open in new tab',icon:'plus',fn:()=>addTab(s.uri)},{label:'Remove saved location',icon:'pin',fn:()=>removeBookmark(s.uri,true)}];items.push({label:'Sign out of server…',icon:'eject',fn:()=>signOut(s.uri)});openMenu(ev.clientX,ev.clientY,items);});g.append(c);}l.append(g);if(!state.env.shares.length)l.append(elem('div','notice','No network locations saved. Use “Map network location” to connect to a share and add it to the sidebar.'));};
  if(uri==='home:'){quick();shares();section('Recently opened','clock');const recent=state.env.recent||[];if(!recent.length)l.append(elem('p','quiet','Files you open in OpenXplorer will appear here.'));else{const table=elem('table','recent-table');const h=elem('tr');for(const n of ['Name','Location','Modified'])h.append(elem('th','',n));table.append(h);for(const e of recent.slice(0,8)){const r=elem('tr');r.tabIndex=0;const n=elem('td');const box=elem('div','recent-name');box.append(fileIcon(e),document.createTextNode(e.name));n.append(box);r.append(n,elem('td','',titleFor(parentUri(e.uri)||e.uri)),elem('td','',dateText(e.modified)));r.addEventListener('dblclick',()=>openEntry(e));r.addEventListener('keydown',ev=>{if(ev.key==='Enter')openEntry(e);});table.append(r);}l.append(table);}}
  if(uri==='pc:'){quick();section('Devices and drives','drive');const g=elem('div','drive-grid');const drives=[{label:'Local Disk',uri:'file:///',mounted:true},...state.env.mounts.filter(m=>!m.uri?.startsWith('smb:'))];for(const m of drives){const c=button('',()=>m.mounted?navigate(m.uri):mountVolume(m.id),'drive-card');bindMiddleOpen(c,()=>m.mounted?m.uri:null);c.append(icon(m.kind==='device'?'phone':'drive',46));const d=elem('div','drive-info');d.append(elem('div','card-name',m.label),elem('div','card-sub',m.mounted?(m.kind==='device'?'Connected device':displayUri(m.uri)):'Click to connect'));if(m.total){const bar=elem('div','capacity');const fill=elem('span');fill.style.width=((m.total-m.free)/m.total*100)+'%';bar.append(fill);d.append(bar,elem('div','card-sub',prettyBytes(m.free)+' free of '+prettyBytes(m.total)));}c.append(d);if(m.mounted&&m.uri!=='file:///'&&m.canUnmount!==false)c.addEventListener('contextmenu',ev=>{ev.preventDefault();openMenu(ev.clientX,ev.clientY,[{label:m.kind==='device'?'Disconnect device':'Disconnect mount',icon:'eject',fn:()=>unmount(m.uri)}]);});g.append(c);}l.append(g);shares();}
  if(uri==='network:'){const b=elem('div','network-banner');b.append(icon('network',38));const d=elem('div');d.append(elem('div','','Map a network location'),elem('p','','Use \\\\server\\share or smb://server/share.'));b.append(d,button('Connect',()=>connectDialog(),'primary'));l.append(b);shares();section('How connections work','shield');l.append(elem('div','notice',native?'Credentials are requested by the system mount dialog, not this interface. Saved locations reconnect when opened. This does not edit /etc/fstab, assign Windows drive letters, or change server permissions.':'This preview uses sample files. It never connects to your NAS, asks for a password, or accesses your computer. The desktop application uses native GIO/GVfs mounts.'));}
}
function updateStatus(){const t=active();if(!t)return;const n=filtered().length;$('status-count').textContent=t.busy?`${n} items · Loading…`:['home:','pc:','network:','settings:'].includes(t.uri)?'Ready':`${n} item${n!==1?'s':''}`;$('status-selected').textContent=state.selection.size?`${state.selection.size} selected`:'';if(state.query)$('status-count').textContent=state.searchBusy?'Searching…':`${n} result${n===1?'':'s'}${state.searchResultMeta?.truncated?' (first 500)':''}${state.searchResultMeta?.source==='cache'?' · Cached':''}`;$('status-mode').textContent=native?'OpenXplorer 1.1.0 · Stable release':'OpenXplorer 1.1.0 preview · Sample data';$('status-list').classList.toggle('active',state.view==='details');$('status-grid').classList.toggle('active',state.view==='grid');}
function updateToolbar(){scheduleFileDragLayout();const s=selected(),busy=!!state.operation||s.some(e=>!canOperate(e)),readOnly=s.some(e=>e.readOnly||readonlyLocation(e.uri));$('copy').disabled=!s.length||busy;for(const id of ['cut','trash'])$(id).disabled=!s.length||busy||readOnly;$('rename').disabled=s.length!==1||busy||readOnly;{const label=deleteLabel(s[0]?.uri||active()?.uri||'');$('trash').title=label+' (Delete)';$('trash').setAttribute('aria-label',label);}$('copy-path').disabled=s.length>1;$('paste').disabled=!!state.query||!state.clipboard||busy||!writableLocation(active()?.uri);$('new').disabled=!!state.query||!!state.operation||!writableLocation(active()?.uri);}
function toggleDetails(){state.details=!state.details;fire('preferences',{details:state.details});renderContent();}
function changeView(view){resetTypeSelect();state.view=view;fire('preferences',{view});$('file-scroll').scrollTop=0;renderContent();}
function toast(text){const box=$('toast');box.textContent=text;box.hidden=false;clearTimeout(toast.timer);toast.timer=setTimeout(()=>box.hidden=true,4000);}
function closeMenu(){$('menu').hidden=true;}
function menuBelow(id,items){const r=$(id).getBoundingClientRect();openMenu(r.left,r.bottom+5,items);}
function closeModal(result=null){if(state.updateInstalling)return;state.modalOwner=null;$('modal-layer').classList.remove('tab-scoped');if(result===null)state.modalCancel?.();state.modalCancel=null;$('modal-layer').hidden=true;const resolve=state.modalResolve;state.modalResolve=null;$('modal').replaceChildren();resolve?.(result);$('main').focus({preventScroll:true});}
function showModal(title,description,build,actions){if(state.updateInstalling)return Promise.resolve(null);resetTypeSelect();closeMenu();if(state.modalResolve)closeModal();const box=$('modal');box.setAttribute('aria-modal','true');box.hidden=false;box.className='modal';box.replaceChildren(elem('h2','',title));box.firstChild.id='modal-title';if(description)box.append(elem('p','',description));const content=elem('div');box.append(content);const fields=build?.(content)||{};state.modalCancel=fields.onCancel||null;const error=elem('div','modal-error');box.append(error);const bar=elem('div','modal-actions');box.append(bar);$('modal-layer').hidden=false;
  const promise=new Promise(resolve=>state.modalResolve=resolve);
  for(const a of actions){const b=button(a.label,async()=>{if(a.cancel){closeModal();return;}try{error.textContent='';const value=await a.fn(fields,b);if(value!==false)closeModal(value??true);}catch(e){error.textContent=e.message;}},a.className||'');bar.append(b);}
  setTimeout(()=>{if(!content.isConnected||box.id!=='modal'||$('modal-layer').hidden||box.contains(document.activeElement))return;const input=content.querySelector('input');(input||bar.querySelector('button'))?.focus();input?.select?.();},30);return promise;
}
function showMessage(title,text){return showModal(title,text,null,[{label:'OK',className:'primary',fn:()=>true}]);}
function textField(parent,label,value,placeholder=''){const l=elem('label','field-label',label);const input=elem('input');input.type='text';input.value=value;input.placeholder=placeholder;input.autocomplete='off';input.spellcheck=false;const id='field-'+(++seq);input.id=id;l.htmlFor=id;parent.append(l,input);return input;}
async function nameDialog(title,initial,action){return showModal(title,'Names must not contain slashes.',body=>({name:textField(body,'Name',initial)}),[{label:'Cancel',cancel:true},{label:'Save',className:'primary',fn:async f=>{const name=validateName(f.name.value);await action(name);return true;}}]);}
async function newItem(kind){if(state.query)return;const uri=active().uri;if(!writableLocation(uri))return;const result=await nameDialog(kind==='folder'?'New folder':'New text document',kind==='folder'?'New folder':'New document.txt',name=>call('create',{uri,name,kind}));if(result)load(active(),false);}
async function refreshClipboard(){
  try{state.clipboard=await call('clipboardGet');if(state.ready){renderRows();updateToolbar();}}catch{state.clipboard=null;}
  return state.clipboard;
}
async function copySelection(mode){
  const items=selected();if(!items.length)return;
  if(items.some(e=>!canOperate(e))){toast('Open the share first, then select its files or folders.');return;}
  if(mode==='move'&&items.some(e=>readonlyLocation(e.uri))){toast('Previous versions are read-only. Use Restore a copy.');return;}
  try{state.clipboard=await call('clipboardSet',{mode,uris:items.map(e=>e.uri)});toast(`${items.length} item(s) ${mode==='move'?'cut':'copied'} — ready to paste in another window.`);renderRows();updateToolbar();}
  catch(error){toast(error.message);}
}

async function copyPath(){const e=selected();const uri=e.length===1?e[0].uri:active().uri;if(['home:','pc:','network:','settings:'].includes(uri)){toast('Open a folder first.');return;}const text=displayUri(uri);try{await call('clipboardText',{text});toast('Path copied. Sharing permissions are unchanged.');}catch(e){showMessage('Location',text);}}
async function transferWithConflicts(mode,args){
  if(state.operation||state.transferPlanning)return;
  state.transferPlanning=true;
  try{
    const {conflicts}=await call('transferConflicts',{target:args.target,uris:args.uris});
    let policy='skip'; // A name appearing after this check must never be overwritten silently.
    if(conflicts.length){
      const answer=await showModal('Items already exist',`${conflicts.length} matching name(s) in ${displayUri(args.target)}\n\nReplace existing files or skip conflicts. Same-name folders are merged; destination-only files stay in place.`,null,[{label:'Cancel',cancel:true},{label:'Skip duplicates',fn:()=>({policy:'skip'})},{label:'Replace existing',className:'primary',fn:()=>({policy:'replace'})}]);
      if(!answer)return;
      policy=answer.policy;
    }
    if(!state.operation)await runOperation(mode,{...args,policy});
  }catch(e){await showMessage('Could not check destination',e.message);}
  finally{state.transferPlanning=false;}
}
async function paste(){await refreshClipboard();if(state.query){toast('Open the destination folder before pasting.');return;}if(!state.clipboard||state.operation)return;const target=active().uri;if(!writableLocation(target))return;const clip={...state.clipboard,uris:[...state.clipboard.uris]};await transferWithConflicts(clip.mode,{uris:clip.uris,target,clipboardToken:clip.token});}
async function rename(){const s=selected();if(s.length!==1||state.operation||!canOperate(s[0])||readonlyLocation(s[0].uri))return;const result=await nameDialog('Rename',s[0].name,name=>call('rename',{uri:s[0].uri,name}));if(result){state.selection.clear();load(active(),false);}}
// Locations without a Trash (SMB shares, most remote backends) get an explicit
// permanent delete instead of a Trash move that can only fail.
function trashScope(uri){return parentUri(uri)||active()?.uri||uri;}
function trashKnown(uri){return state.trashSupport.get(trashScope(uri));}
async function trashSupported(uri){const scope=trashScope(uri);if(state.trashSupport.has(scope))return state.trashSupport.get(scope);try{const r=await call('trashSupport',{uri:scope});const ok=!!r?.canTrash;state.trashSupport.set(scope,ok);return ok;}catch{return true;}}
async function refreshTrashSupport(uri){if(!uri||['home:','pc:','network:','settings:'].includes(uri)||state.trashSupport.has(uri))return;try{const r=await call('trashSupport',{uri});state.trashSupport.set(uri,!!r?.canTrash);if(active()?.uri===uri)updateToolbar();}catch{}}
function deleteLabel(uri){return trashKnown(uri)===false?'Delete permanently':'Move to Trash';}
async function trash(){const s=selected();if(!s.length||state.operation||s.some(e=>!canOperate(e)||readonlyLocation(e.uri)))return;
  // A search across all cached locations can select local and share items at
  // once. Each item is decided by its own folder, never by the first one.
  const toTrash=[],toDelete=[];
  for(const e of s)(await trashSupported(e.uri)?toTrash:toDelete).push(e.uri);
  const what=s.length===1?s[0].name:s.length+' selected items';
  const mixed=toTrash.length&&toDelete.length;
  const ok=await showModal(mixed?'Delete items?':toDelete.length?'Delete permanently?':'Move to Trash?',
    mixed?`${what}\n\n${toTrash.length} item(s) go to the Trash and can be restored from there.\n${toDelete.length} item(s) are on a location without Trash and are deleted permanently, without any way to recover them.`
    :toDelete.length?`${what}\n\nThis location has no Trash. The items are deleted permanently and cannot be recovered.`
    :`${what}\n\nItems go to the Trash and can be restored from there.`,
    null,[{label:'Cancel',cancel:true},{label:mixed?'Delete items':toDelete.length?'Delete permanently':'Move to Trash',className:'danger',fn:()=>true}]);
  if(!ok)return;
  if(toTrash.length)await runOperation('trash',{uris:toTrash});
  if(toDelete.length)await runOperation('delete',{uris:toDelete});}
async function runOperation(mode,args){const token='op-'+(++seq);state.operation=token;updateToolbar();$('transfer').hidden=false;updateTransfer({token,label:mode==='trash'?'Moving to Trash…':mode==='delete'?'Deleting items…':mode==='move'?'Moving items…':'Preparing copy…',fraction:0});
  try{const r=await call('operate',{...args,mode,token});if(mode==='move'&&r.done?.length){state.clipboard=await call('clipboardConsume',{token:args.clipboardToken,done:r.done});}
    if(r.errors?.length||r.cancelled||r.skipped?.length){const report=[`${r.done?.length||0} completed.`,r.skipped?.length?`${r.skipped.length} skipped (name already exists).`:'',r.cancelled?'Cancelled. Completed items remain in place.':'',...(r.errors||[])].filter(Boolean).join('\n');await showMessage('Operation result',report);}else toast(`${r.done?.length||0} item(s) ${mode==='copy'?'copied':mode==='move'?'moved':mode==='delete'?'permanently deleted':'sent to Trash'}.`);
  }catch(e){showMessage('Operation stopped',e.message);}finally{state.operation=null;$('transfer').hidden=true;state.selection.clear();await load(active(),false);updateToolbar();}}
function updateTransfer(data){if(data.token&&state.operation!==data.token)return;$('transfer-label').textContent=data.label||'Working…';$('transfer-progress').style.width=(Math.max(0,Math.min(1,data.fraction||0))*100)+'%';}
function sameLocation(a,b){return typeof a==='string'&&typeof b==='string'&&a.replace(/\/$/,'')===b.replace(/\/$/,'');}
function isSmbServer(uri){try{const u=new URL(uri);return u.protocol==='smb:'&&!u.pathname.replaceAll('/','');}catch{return false;}}
function writableLocation(uri){return !!uri&&!['home:','pc:','network:','settings:'].includes(uri)&&!isSmbServer(uri)&&!readonlyLocation(uri);}
function canOperate(e){return e.canOperate!==false&&!e.isVirtual&&!isSmbShareRoot(e.uri);}
function isSmbShareRoot(uri){try{const u=new URL(uri);return u.protocol==='smb:'&&u.pathname.split('/').filter(Boolean).length<=1;}catch{return false;}}
async function pinEntries(entries,before=null){
  if(state.pinBusy)return;
  if(!entries.length||entries.some(e=>!e.isDir)){toast('Only folders and network shares can be pinned. Select folders only.');return;}
  state.pinBusy=true;
  try{
    await call('pin',{items:entries.map(e=>({uri:e.uri,label:e.name||e.label||baseName(e.uri)})),before});
    await refreshEnvironment();
    toast(entries.length===1?'Pinned to Quick access. No files were moved.':`${entries.length} folders pinned. No files were moved.`);
  }catch(e){toast('Could not pin: '+e.message);}
  finally{state.pinBusy=false;}
}
async function pinEntry(e){
  if((state.env?.quick||[]).some(p=>sameLocation(p.uri,e.targetUri||e.uri))){toast('Already pinned to Quick access.');return;}
  return pinEntries([e]);
}
function pinCurrent(){if(!['home:','pc:','network:','settings:'].includes(active()?.uri))return pinEntry({uri:active().uri,name:titleFor(active().uri),isDir:true});}

// GTK owns native file gestures and MIME negotiation. DOM drags cannot export
// real files from WebKitGTK; the sample preview only simulates folder pinning.
function fileDragEntry(entry){
  return !!entry&&typeof entry.uri==='string'&&/^(file|smb):/.test(entry.uri)&&
    !entry.isVirtual&&!entry.archiveMember&&!entry.archiveUri&&!isSmbServer(entry.uri)&&
    (!entry.kind||['file','directory','symlink'].includes(entry.kind));
}
function makeFileDraggable(node,getEntries){
  node.draggable=false;node.style.webkitUserDrag='none';node.dataset.fileDraggable='true';
  node.addEventListener('dragstart',event=>event.preventDefault());
  node.addEventListener('pointerdown',event=>{
    if(native&&state.env?.nativeFileDrag)return;
    pointerPinDown(event,node,getEntries);
  });
}
function fileInteractionBlocked(){
  return !state.ready||!!activeAuth||!!state.operation||!!state.detaching||!!state.outgoingTab||
    !$('modal-layer').hidden||!$('menu').hidden;
}
let fileDragLayoutFrame=0,fileDragLayoutSignature='';
function scheduleFileDragLayout(){
  if(!native||!state.env?.nativeFileDrag||fileDragLayoutFrame)return;
  fileDragLayoutFrame=requestAnimationFrame(()=>{fileDragLayoutFrame=0;publishFileDragLayout();});
}
function clippedFileRect(node,clip){
  if(!node||node.hidden||!node.getClientRects().length)return null;
  const r=node.getBoundingClientRect(),c=clip?.getBoundingClientRect()||{left:0,top:0,right:innerWidth,bottom:innerHeight};
  const box={left:Math.max(0,r.left,c.left),right:Math.min(innerWidth,r.right,c.right),top:Math.max(0,r.top,c.top),bottom:Math.min(innerHeight,r.bottom,c.bottom)};
  return box.right>box.left&&box.bottom>box.top?box:null;
}
function publishFileDragLayout(){
  if(!native||!state.env?.nativeFileDrag)return;
  const layout={width:innerWidth,height:innerHeight,items:[],targets:[]};
  if(!fileInteractionBlocked()){
    const pane=$('file-scroll'),sidebar=$('sidebar'),rows=filtered(),byUri=new Map(rows.map(e=>[e.uri,e]));
    for(const node of document.querySelectorAll('[data-file-draggable]')){
      const clip=node.closest('#sidebar')?sidebar:pane,rect=clippedFileRect(node,clip);
      if(rect)layout.items.push({uri:node.dataset.uri,...rect});
    }
    // Quick access always means pin/reorder, including a drop over a pinned
    // folder. Native code validates every dropped item as an actual directory.
    const quick=$('quick-access'),quickRect=clippedFileRect(quick,sidebar);
    if(quickRect){
      let top=quickRect.top;
      for(const row of quick.querySelectorAll('.side-entry')){
        const rect=clippedFileRect(row,sidebar);if(!rect)continue;
        const bottom=Math.min(quickRect.bottom,(rect.top+rect.bottom)/2);
        if(bottom>top)layout.targets.push({kind:'pin',before:row.dataset.uri,left:quickRect.left,right:quickRect.right,top,bottom});
        top=bottom;
      }
      if(quickRect.bottom>top)layout.targets.push({kind:'pin',before:null,...quickRect,top});
    }
    for(const node of document.querySelectorAll('.file-row,.file-tile,.side-entry')){
      if(node.closest('#quick-access'))continue;
      const entry=node.closest('#sidebar')?{uri:node.dataset.uri,isDir:true}:byUri.get(node.dataset.uri);
      const uri=entry?.targetUri||entry?.uri;
      if(!entry?.isDir||!fileDragEntry(entry)||entry.readOnly||!writableLocation(uri))continue;
      // Unmounted devices have no source binding and must never receive drops.
      if(node.closest('#sidebar')&&!node.dataset.fileDraggable)continue;
      const rect=clippedFileRect(node,node.closest('#sidebar')?sidebar:pane);
      if(rect)layout.targets.push({kind:'copy',uri,...rect});
    }
    const rect=clippedFileRect(pane);
    if(rect&&!state.query&&!active()?.busy&&!active()?.error&&fileDragEntry({uri:active()?.uri})&&writableLocation(active()?.uri))layout.targets.push({kind:'copy',uri:active().uri,...rect});
  }
  const signature=JSON.stringify(layout);if(signature===fileDragLayoutSignature)return;
  fileDragLayoutSignature=signature;fire('fileDragLayout',layout);
}
async function beginNativeFileDrag(uri){
  if(!native||!state.env?.nativeFileDrag||fileInteractionBlocked()||state.outgoingFile)return;
  const rows=filtered(),entry=rows.find(e=>e.uri===uri);
  const side=[...$('sidebar').querySelectorAll('[data-file-draggable]')].find(node=>node.dataset.uri===uri);
  const items=entry?(state.selection.has(uri)?selected():[entry]):side?[{uri,name:side.querySelector('.name')?.textContent||baseName(uri),isDir:true}]:[];
  if(!items.length||items.length>200||items.some(e=>!fileDragEntry(e))){toast('Select up to 200 files or folders. Extract ZIP contents before dragging them.');return;}
  if(entry&&!state.selection.has(uri)){state.selection=new Set([uri]);state.anchor=rows.indexOf(entry);syncRowSelection();renderDetails();updateToolbar();updateStatus();}
  closeMenu();resetTypeSelect();state.outgoingFile=items.map(e=>e.uri);state.suppressClickUntil=performance.now()+700;
  try{const result=await call('beginFileDrag',{uri,uris:[...state.outgoingFile]});
    if(result?.remoteOnly>0)toast('This network item needs an app that supports SMB addresses. For a local-only editor, open it through an existing local mount.');
  }
  catch(error){finishNativeFileDrag();toast(error.message);}
}
function markNativeFileDrag(data={}){
  if(Array.isArray(data.uris))state.outgoingFile=data.uris;
  document.body.classList.add('file-dragging');
  for(const node of document.querySelectorAll('[data-file-draggable]'))node.classList.toggle('drag-source',state.outgoingFile?.includes(node.dataset.uri));
}
function finishNativeFileDrag(){
  state.outgoingFile=null;state.suppressClickUntil=performance.now()+400;
  document.body.classList.remove('file-dragging');document.querySelectorAll('.drag-source').forEach(node=>node.classList.remove('drag-source'));
  showFileDropHint({show:false});renderRows();scheduleFileDragLayout();
}
function showFileDropHint(data={}){
  document.querySelectorAll('.file-drop-active').forEach(node=>node.classList.remove('file-drop-active'));
  clearDropFeedback();
  if(!data.show)return;
  if(data.kind==='pin'){
    const quick=$('quick-access');quick?.classList.add('pin-drop-active');
    const before=[...(quick?.querySelectorAll('.side-entry')||[])].find(node=>node.dataset.uri===data.before);
    (before||quick?.querySelector('.quick-drop-tail'))?.classList.add(before?'drop-before':'drop-end');
  }else{
    const uri=data.target||data.uri;
    const target=[...document.querySelectorAll('.file-row,.file-tile,.side-entry')].find(node=>node.dataset.uri===uri&&!node.closest('#quick-access'));
    (target||((active()?.uri===uri)?$('file-scroll'):null))?.classList.add('file-drop-active');
  }
}
async function receiveFileDrop(data){
  showFileDropHint({show:false});
  if(fileInteractionBlocked()){toast('Close the dialog and finish the current operation before dropping files.');return;}
  const uris=Array.isArray(data.uris)?[...new Set(data.uris)]:[];
  if(!uris.length||uris.length>200||uris.some(uri=>!fileDragEntry({uri}))){toast('Drop up to 200 local files or connected network items.');return;}
  if(data.kind==='pin'){await pinEntries(uris.map(uri=>({uri,name:baseName(uri),isDir:true})),data.before||null);return;}
  const target=data.target;
  if(data.kind!=='copy'||!fileDragEntry({uri:target})||!writableLocation(target)){toast('Open a writable destination folder before dropping files.');return;}
  if(uris.some(uri=>sameLocation(uri,target))){toast('A folder cannot be copied into itself.');return;}
  await transferWithConflicts('copy',{uris,target});
}
function clearDropFeedback(){
  const q=$('quick-access');q?.classList.remove('pin-drop-active');
  q?.querySelectorAll('.drop-before,.drop-end').forEach(e=>e.classList.remove('drop-before','drop-end'));
  $('pin-drag-badge').hidden=true;
}
function finishPinDrag(){
  state.pointerPending=null;
  if(!state.drag)return;
  state.suppressClickUntil=performance.now()+350;
  document.querySelectorAll('.drag-source').forEach(n=>n.classList.remove('drag-source'));
  state.drag=null;document.body.classList.remove('pin-dragging');clearDropFeedback();
  renderSidebar();renderRows();
}
function setupPinDrop(zone){
  zone.addEventListener('dragover',e=>e.preventDefault());
  zone.addEventListener('drop',e=>{e.preventDefault();if(!native)toast('Drag a sample folder from inside this preview to pin it. Desktop file drops require the installed application.');});
}
function connectDialog(){return showModal('Map network location','Add a shared folder to your sidebar. Connect using a Windows-style address or an SMB URL.',body=>{const location=textField(body,'Folder','','\\\\nas\\Projects');const label=textField(body,'Display name (optional)','','Projects (Z:)');const line=elem('label','checkbox-row');const remember=elem('input');remember.type='checkbox';remember.checked=true;line.append(remember,document.createTextNode('Save in the sidebar · reconnect when opened'));body.append(line);body.append(elem('div','modal-note',native?'OpenXplorer will ask for your username and password if needed. Remember my credentials is selected by default. No passwords are saved in OpenXplorer settings. A label such as “Z:” is only a label, not a system-wide drive letter.':'Preview mode: this opens a simulated share with sample files. It will not connect to a server or ask for credentials.'));const fields={location,label,remember,token:'connect-'+(++seq),cancelled:false};fields.onCancel=()=>{fields.cancelled=true;fire('cancel',{token:fields.token});};return fields;},[{label:'Cancel',cancel:true},{label:native?'Connect':'Open sample share',className:'primary',fn:async(f,b)=>{b.disabled=true;b.textContent='Connecting…';try{const r=await call('connect',{address:f.location.value,label:f.label.value,remember:f.remember.checked,token:f.token});if(f.cancelled)return false;await refreshEnvironment();if(f.cancelled)return false;navigate(r.uri);return true;}finally{b.disabled=false;b.textContent=native?'Connect':'Open sample share';}}}]);}
async function mountVolume(id){try{const r=await call('mountVolume',{id});await refreshEnvironment();navigate(r.uri);}catch(e){showMessage('Could not mount device',e.message);}}
async function unmount(uri){if(state.operation){toast('Finish the current operation before disconnecting.');return;}const ok=await showModal('Disconnect this mount?',displayUri(uri)+'\n\nClose files using this mount first. This disconnects the session mount for other applications too.',null,[{label:'Cancel',cancel:true},{label:'Disconnect',className:'primary',fn:()=>true}]);if(!ok)return;try{await call('unmount',{uri});await refreshEnvironment();navigate('network:');}catch(e){showMessage('Could not disconnect',e.message);}}
async function askClose(){if(state.updateInstalling){toast('Wait for the update to finish before closing OpenXplorer.');return;}if(!native){toast('This is an offline preview. Close the browser tab to exit.');return;}if(state.operation){await showMessage('A file operation is running','Cancel the operation and wait for its result before closing OpenXplorer.');return;}fire('window',{action:'close'});}
function toggleHidden(){state.showHidden=!state.showHidden;fire('preferences',{showHidden:state.showHidden});for(const t of state.tabs)t.loaded=false;load(active());}
function updatesDialog(){
  if(state.updateInstalling)return;
  let release=null,checking=false,installed=false,alive=true,status,versions,notes;
  const sync=()=>{
    if(!alive)return;
    const busy=checking||!!state.updateInstalling;
    $('modal').querySelector('.update-close').disabled=!!state.updateInstalling;
    $('modal').querySelector('.update-check').disabled=busy||installed;
    const install=$('modal').querySelector('.update-install');install.hidden=!release?.available||installed;install.disabled=busy||!release?.canInstall;
    const restart=$('modal').querySelector('.update-restart');restart.hidden=!installed;restart.disabled=busy;
    $('modal').setAttribute('aria-busy',String(busy));
  };
  const check=async()=>{
    if(checking||state.updateInstalling||installed)return;
    checking=true;release=null;notes.textContent='';status.textContent='Checking for updates…';sync();
    try{
      const result=await call('updateCheck');if(!alive)return;release=result;installed=!!result.restartRequired;
      versions.textContent='Installed: '+result.currentVersion+(result.available?' · Available: '+result.version:'');
      notes.textContent=typeof result.notes==='string'?result.notes:'';
      status.textContent=installed?'Updated application files are installed. Restart OpenXplorer before continuing.':result.available?(result.canInstall?'An update is available. Install it when file operations have finished.':'An update is available. Automatic installation is unavailable for this installation.'):(native?'OpenXplorer is up to date.':'Preview only — no update available. No network request was made.');
    }catch(error){if(alive)status.textContent='Could not check for updates. '+error.message;}
    finally{checking=false;sync();}
  };
  const promise=showModal('Software updates','Updates are checked only when you ask. Installing requires administrator approval. Restart OpenXplorer after installation.',body=>{
    body.className='update-body';versions=elem('p','update-versions','Installed: '+(state.env?.version||UI_RELEASE));versions.id='update-versions';
    status=elem('p','update-status');status.id='update-status';status.setAttribute('role','status');status.setAttribute('aria-live','polite');
    notes=elem('pre','update-notes');notes.id='update-notes';notes.setAttribute('aria-label','Release notes');body.append(versions,status,notes);
    return{onCancel:()=>{alive=false;}};
  },[{label:'Close',cancel:true,className:'update-close'},
    {label:'Check again',className:'update-check secondary',fn:()=>{void check();return false;}},
    {label:'Install update…',className:'update-install primary',fn:async()=>{
      if(!release?.available||!release.canInstall||state.updateInstalling)return false;
      if(state.operation||state.transferPlanning){status.textContent='Finish file operations before installing the update.';return false;}
      state.updateInstalling=true;$('app').inert=true;status.textContent='Preparing update. Approve the administrator prompt to install.';sync();status.tabIndex=-1;status.focus();
      try{const result=await call('updateInstall',{version:release.version,confirmed:true});if(result?.installed!==true)throw Error('The installer did not confirm completion.');installed=true;status.textContent='OpenXplorer '+result.version+' is installed. Restart to use the update.';}
      catch(error){
        release=null;let recovery=' Check again to retry.';
        try{const result=await call('updateCheck');installed=!!result.restartRequired;if(installed)recovery=' Some application files changed. Restart OpenXplorer before continuing.';}catch{}
        status.textContent='Update installation did not complete. '+error.message+recovery;
      }
      finally{state.updateInstalling=false;$('app').inert=false;sync();$('modal').querySelector(installed?'.update-restart':'.update-check').focus();}
      return false;
    }},
    {label:'Restart now',className:'update-restart primary',fn:async(_fields,b)=>{if(!installed)return false;b.disabled=true;try{await call('updateRestart');}catch(error){status.textContent='Could not restart. '+error.message;b.disabled=false;}return false;}}
  ]);
  $('modal').classList.add('updates-modal');sync();void check();return promise;
}
function applyTheme(theme,save=true){
  if(!['light','dark','system'].includes(theme))theme='system';
  state.theme=theme;
  const systemDark=native?!!(state.env?.systemDark??window.__OPENXPLORER_BOOT__?.systemDark):!!window.matchMedia?.('(prefers-color-scheme: dark)').matches;
  const dark=theme==='dark'||(theme==='system'&&systemDark);
  document.documentElement.dataset.theme=dark?'dark':'light';document.body.classList.toggle('dark',dark);
  const b=$('theme-toggle');b.replaceChildren(icon(dark?'moon':'sun'),elem('span','appearance-label',dark?'Dark':'Light'));
  b.title=`Appearance: ${theme==='system'?'System ('+(dark?'dark':'light')+')':theme}. Click to change.`;
  if(save)fire('preferences',{theme});
}
function appearanceMenu(){menuBelow('theme-toggle',[
  {label:'Light appearance',icon:state.theme==='light'?'check':'sun',fn:()=>applyTheme('light')},
  {label:'Dark appearance',icon:state.theme==='dark'?'check':'moon',fn:()=>applyTheme('dark')},
  {label:'Use system appearance',icon:state.theme==='system'?'check':'desktop',fn:()=>applyTheme('system')}
]);}

// ---- 0.3: metadata-cache search, integrated authentication, and settings ----
function realLocation(uri){return uri==='home:'?(state.env?.home||'file:///home/demo'):uri;}
function resetSearch(){resetTypeSelect();if(state.searchToken)fire('cancel',{token:state.searchToken});state.searchToken=null;clearTimeout(state.searchTimer);state.searchGeneration=(state.searchGeneration||0)+1;state.searchResults=null;state.searchResultMeta=null;state.searchBusy=false;state.filterCache=null;}
function cacheRootsFor(uri){return (state.cache?.roots||[]).filter(r=>r.enabled&&(sameLocation(uri,r.uri)||uri?.startsWith(r.uri.replace(/\/$/,'')+'/')||r.uri.startsWith(uri?.replace(/\/$/,'')+'/')));}
function cacheCovers(uri){return cacheRootsFor(uri).some(r=>sameLocation(uri,r.uri)||uri.startsWith(r.uri.replace(/\/$/,'')+'/'));}
function currentFolderMatches(tab,text){const terms=text.toLocaleLowerCase().split(/\s+/);return tab.entries.filter(e=>(state.showHidden||!e.hidden)&&terms.every(term=>(e.name+' '+displayUri(tab.uri)).toLocaleLowerCase().includes(term)));}
async function refreshCacheStatus(){try{state.cache=await call('cacheStatus');renderSearchInfo();if($('settings-cache-list'))renderSettingsCache();return state.cache;}catch(e){state.cacheError=e.message;return null;}}
function queueSearch(){resetSearch();state.query=$('search').value;state.selection.clear();$('file-scroll').scrollTop=0;state.searchBusy=!!state.query;state.searchTimer=setTimeout(runSearch,120);renderSearchInfo();renderRows();updateStatus();updateToolbar();}
async function runSearch(){
  const text=state.query.trim(),t=active();if(!t)return;
  if(!text){resetSearch();renderSearchInfo();renderColumns();renderRows();renderDetails();updateStatus();updateToolbar();return;}
  const generation=++state.searchGeneration,tabId=t.id,uri=t.uri;
  const useCache=state.searchScope==='all'||cacheRootsFor(uri).length>0;
  state.searchBusy=useCache;state.searchError='';
  if(!useCache){state.searchResults=null;state.searchResultMeta={source:'folder'};state.filterCache=null;renderSearchInfo();renderColumns();renderRows();renderDetails();updateStatus();return;}
  try{
    state.searchToken='search-'+(++seq);const result=await call('search',{token:state.searchToken,query:text,scope:state.searchScope==='all'?null:uri,limit:500,showHidden:state.showHidden});
    if(generation!==state.searchGeneration||active()?.id!==tabId||active()?.uri!==uri)return;
    const merged=new Map();
    // A cached child does not cover its parent. Keep matches from the visible
    // listing, then add cached descendants without duplicating the same URI.
    if(state.searchScope!=='all')for(const entry of currentFolderMatches(t,text))merged.set(entry.uri,entry);
    for(const entry of result.entries)if(!merged.has(entry.uri))merged.set(entry.uri,entry);
    state.searchResults=[...merged.values()].slice(0,500);
    state.searchResultMeta={...result,truncated:result.truncated||merged.size>500,partialCache:state.searchScope!=='all'&&!cacheCovers(uri)};state.filterCache=null;t.dirty=true;
  }catch(e){if(generation!==state.searchGeneration)return;state.searchError=e.message;state.searchResults=[];state.searchResultMeta={source:'cache'};}
  finally{if(generation===state.searchGeneration){state.searchToken=null;state.searchBusy=false;renderSearchInfo();renderColumns();renderRows();renderDetails();updateStatus();updateToolbar();}}
}
function renderSearchInfo(){
  const strip=$('search-info');if(!strip||!active())return;strip.hidden=!state.query;
  document.body.classList.toggle('searching',!!state.query);
  if(!state.query)return;
  strip.replaceChildren(icon('search',15));
  const cached=state.searchScope==='all'||cacheRootsFor(active().uri).length>0;
  const partial=cached&&state.searchScope!=='all'&&!cacheCovers(active().uri);
  const label=state.searchError|| (state.searchBusy?'Searching…':partial?'Current folder + cached subfolders':cached?'Cached names & paths':'Current folder only');
  const desc=elem('span','search-caption',label);strip.append(desc);
  const scope=elem('select');scope.setAttribute('aria-label','Search scope');
  for(const [value,text]of [['folder','This folder + subfolders'],['all','All cached folders']]){const o=elem('option','',text);o.value=value;scope.append(o);}
  scope.value=state.searchScope;scope.onchange=()=>{state.searchScope=scope.value;resetSearch();void runSearch();};strip.append(scope);
  if(!cached||partial){strip.append(button('Cache this folder',()=>setCache(active().uri,true),'cache-link','plus'));}
  if(cached){const info=elem('span','cache-freshness',state.searchResultMeta?.truncated?'First 500 results · narrow your search':partial?'Other subfolders are not indexed.':'Cached metadata · see update coverage in Settings');info.title='Names and paths are stored locally. Refresh the cache to pick up changes on a disconnected or unmonitored share.';strip.append(info);}
  const clear=button('',()=>{$('search').value='';state.query='';void runSearch();},'','close');clear.title='Clear search';clear.setAttribute('aria-label','Clear search');strip.append(clear);
}
async function setCache(uri,enabled,label){
  try{state.cache=await call('cacheSet',{uri,enabled,label:label||titleFor(uri)});renderSearchInfo();if($('settings-cache-list'))renderSettingsCache();if(state.query)void runSearch();toast(enabled?'Caching filenames and paths in the background. No file contents are downloaded.':'Cache disabled; this root’s indexed names were removed.');}
  catch(e){toast(e.message);await refreshCacheStatus();}
}
function isCachedRoot(uri){return (state.cache?.roots||[]).some(r=>r.enabled&&sameLocation(r.uri,uri));}
async function openContainingFolder(e){await navigate(e.parentUri||parentUri(e.uri));const i=filtered().findIndex(v=>v.uri===e.uri);if(i>=0){state.selection=new Set([e.uri]);state.anchor=i;$('file-scroll').scrollTop=Math.max(0,i*38-80);renderRows();renderDetails();updateToolbar();updateStatus();}}
function cacheMenuItems(uri,label){if(!uri||deviceLocation(uri)||isSmbServer(uri)||['pc:','network:'].includes(uri))return[];return[{label:isCachedRoot(uri)?'Stop caching this folder':'Cache this folder for search',icon:isCachedRoot(uri)?'check':'search',fn:()=>setCache(uri,!isCachedRoot(uri),label)}];}

// A separate, higher modal layer leaves an in-progress Map Location dialog
// intact. Multiple GIO challenges are queued; Cancel replies only to its token.
const authQueue=[];let activeAuth=null;
function receiveAuth(data){if(authQueue.some(r=>r.id===data.id)||activeAuth?.id===data.id)return;authQueue.push(data);pumpAuth();}
function dismissAuth(id){const i=authQueue.findIndex(r=>r.id===id);if(i>=0)authQueue.splice(i,1);if(activeAuth?.id!==id)return;const pass=$('auth-password');if(pass)pass.value='';activeAuth=null;$('auth-layer').hidden=true;$('auth-dialog').replaceChildren();$('app').inert=false;$('modal-layer').inert=false;pumpAuth();if(!activeAuth){const target=$('modal-layer').hidden?$('main'):$('modal').querySelector('button');target?.focus({preventScroll:true});}}
function pumpAuth(){if(activeAuth||!authQueue.length)return;activeAuth=authQueue.shift();renderAuth(activeAuth);}
function authPreview(){receiveAuth({id:'preview-'+(++seq),kind:'password',host:'archive-nas',uri:'smb://archive-nas/Shared',username:'',needUsername:true,needPassword:true,canSave:true,canGuest:false,retry:false});}
function renderAuth(data){
  resetTypeSelect();closeMenu();finishPinDrag();const layer=$('auth-layer'),box=$('auth-dialog');box.replaceChildren();layer.hidden=false;$('app').inert=true;$('modal-layer').inert=true;
  const caption=elem('div','auth-caption');caption.append(icon('shield',17),elem('span','','OpenXplorer Security'));
  const cancel=()=>answerAuth({cancel:true});const close=button('',cancel,'auth-close','close');close.setAttribute('aria-label','Cancel sign-in');caption.append(close);box.append(caption);
  const body=elem('div','auth-body');box.append(body);
  if(data.kind==='question'){
    const h=elem('h2','','Network connection');h.id='auth-title';body.append(h,elem('p','auth-description',data.message));
    const actions=elem('div','auth-actions');for(const [i,label]of data.choices.entries())actions.append(button(label,()=>answerAuth({choice:i}),'secondary'));actions.append(button('Cancel',cancel,'secondary'));body.append(actions);
  }else{
    const heading=elem('div','auth-heading'),badge=elem('div','auth-device');badge.append(icon('desktop',29));heading.append(badge);
    const titles=elem('div');const h=elem('h2','','Enter network credentials');h.id='auth-title';titles.append(h,elem('p','','Connect to '+data.host));heading.append(titles);body.append(heading);
    const intro=elem('p','auth-description',data.retry?'The previous sign-in was not accepted. Check your username and password.':'Enter the credentials for this computer or network storage.');if(data.retry)intro.classList.add('auth-retry');body.append(intro);
    const target=elem('div','auth-target');target.append(icon('network',16),elem('span','',data.host));body.append(target);
    const userLabel=elem('label','auth-label','Username');userLabel.htmlFor='auth-username';const user=elem('input','auth-input');user.id='auth-username';user.value=data.username||'';user.placeholder='Your network username';user.autocomplete='off';user.spellcheck=false;user.maxLength=512;body.append(userLabel,user);
    const passLabel=elem('label','auth-label','Password');passLabel.htmlFor='auth-password';const wrap=elem('div','auth-password-wrap');const pass=elem('input','auth-input');pass.id='auth-password';pass.type='password';pass.autocomplete='off';pass.maxLength=16384;pass.placeholder='Password';
    const eye=button('',()=>{pass.type=pass.type==='password'?'text':'password';eye.setAttribute('aria-pressed',String(pass.type==='text'));eye.title=pass.type==='text'?'Hide password':'Show password';},'auth-eye','eye');eye.title='Show password';eye.setAttribute('aria-label','Show or hide password');eye.setAttribute('aria-pressed','false');wrap.append(pass,eye);body.append(passLabel,wrap);
    const check=elem('label','auth-remember');const remember=elem('input');remember.id='auth-remember';remember.type='checkbox';remember.checked=!!data.canSave;remember.disabled=!data.canSave;check.append(remember,document.createTextNode('Remember my credentials'));body.append(check);
    const note=elem('div','auth-note');note.append(icon('shield',14),elem('span','',native?(data.canSave?'Saved through your system keyring, not in OpenXplorer settings.':'This connection does not support saving credentials.'):'Preview only. Use sample credentials here. Nothing is sent or saved.'));body.append(note);
    if(native){const update=()=>{note.querySelector('span').textContent=remember.checked?'Saved in your system keyring for future sign-ins.':'Reused for this server during your Linux login session. Not saved permanently.';};remember.addEventListener('change',update);update();}
    if(data.canGuest){const guest=button('Connect as guest',()=>answerAuth({guest:true,username:'',password:'',remember:false}),'auth-guest');body.append(guest);}
    const err=elem('div','auth-error');err.id='auth-error';err.setAttribute('role','alert');body.append(err);
    const actions=elem('div','auth-actions');const connect=button('Connect',()=>submitAuth(),'primary');connect.id='auth-connect';actions.append(button('Cancel',cancel,'secondary'),connect);box.append(actions);
  }
  setTimeout(()=>{if(activeAuth?.id!==data.id||layer.hidden||box.contains(document.activeElement))return;$('auth-username')?.focus();$('auth-username')?.select();if(!$('auth-username'))box.querySelector('button')?.focus();},30);
}
async function submitAuth(){
  if(!activeAuth||activeAuth.kind!=='password')return;
  const username=$('auth-username').value,pass=$('auth-password'),remember=$('auth-remember').checked;
  if(activeAuth.needUsername&&!username.trim()){$('auth-error').textContent='Enter your username.';$('auth-username').focus();return;}
  const password=pass.value;pass.value='';await answerAuth({username,password,remember});
}
async function answerAuth(values){
  if(!activeAuth)return;const id=activeAuth.id;const connect=$('auth-connect');if(connect){connect.disabled=true;connect.textContent='Connecting…';}
  try{await call('authReply',{id,...values});dismissAuth(id);if(!native&&!values.cancel)toast('Preview only. No credentials were saved and no server was contacted.');}
  catch(e){if(activeAuth?.id===id){const error=$('auth-error');if(error)error.textContent=e.message;if(connect){connect.disabled=false;connect.textContent='Connect';}}}
}
function authKeys(e){
  if(!activeAuth)return;
  if(e.key==='Escape'){e.preventDefault();e.stopImmediatePropagation();void answerAuth({cancel:true});return;}
  if(e.key==='Enter'&&activeAuth.kind==='password'&&!e.target.closest('button')){e.preventDefault();e.stopImmediatePropagation();void submitAuth();return;}
  if(e.key==='Tab'){const all=[...$('auth-dialog').querySelectorAll('button:not(:disabled),input:not(:disabled)')].filter(n=>n.offsetParent!==null),i=all.indexOf(document.activeElement);if(!all.length)return;let next=i+(e.shiftKey?-1:1);if(next<0||next>=all.length||i<0){e.preventDefault();all[e.shiftKey?all.length-1:0].focus();}}
  e.stopPropagation();
}

async function settingsDialog(sectionName){
  if(!state.ready)return;
  if(active()?.uri!=='settings:')state.settingsOrigin=active()?.uri;
  state.settingsSection=typeof sectionName==='string'?sectionName:null;
  const existing=state.tabs.find(t=>t.uri==='settings:');
  if(existing){switchTab(existing.id);}else addTab('settings:');
  await refreshCacheStatus();void updateDefaultStatus();
}
function showLicense(){return showMessage('OpenXplorer · License & source',"Copyright (c) 2026 OpenXplorer contributors.\nAGPL-3.0-only. No warranty. You may redistribute and modify under the included terms.\n\nInstalled editable source: /opt/openxplorer\nComplete corresponding source and build tools: the source ZIP supplied alongside this release.\n\n                    GNU AFFERO GENERAL PUBLIC LICENSE\n                       Version 3, 19 November 2007\n\n Copyright (C) 2007 Free Software Foundation, Inc. <https://fsf.org/>\n Everyone is permitted to copy and distribute verbatim copies\n of this license document, but changing it is not allowed.\n\n                            Preamble\n\n  The GNU Affero General Public License is a free, copyleft license for\nsoftware and other kinds of works, specifically designed to ensure\ncooperation with the community in the case of network server software.\n\n  The licenses for most software and other practical works are designed\nto take away your freedom to share and change the works.  By contrast,\nour General Public Licenses are intended to guarantee your freedom to\nshare and change all versions of a program--to make sure it remains free\nsoftware for all its users.\n\n  When we speak of free software, we are referring to freedom, not\nprice.  Our General Public Licenses are designed to make sure that you\nhave the freedom to distribute copies of free software (and charge for\nthem if you wish), that you receive source code or can get it if you\nwant it, that you can change the software or use pieces of it in new\nfree programs, and that you know you can do these things.\n\n  Developers that use our General Public Licenses protect your rights\nwith two steps: (1) assert copyright on the software, and (2) offer\nyou this License which gives you legal permission to copy, distribute\nand/or modify the software.\n\n  A secondary benefit of defending all users' freedom is that\nimprovements made in alternate versions of the program, if they\nreceive widespread use, become available for other developers to\nincorporate.  Many developers of free software are heartened and\nencouraged by the resulting cooperation.  However, in the case of\nsoftware used on network servers, this result may fail to come about.\nThe GNU General Public License permits making a modified version and\nletting the public access it on a server without ever releasing its\nsource code to the public.\n\n  The GNU Affero General Public License is designed specifically to\nensure that, in such cases, the modified source code becomes available\nto the community.  It requires the operator of a network server to\nprovide the source code of the modified version running there to the\nusers of that server.  Therefore, public use of a modified version, on\na publicly accessible server, gives the public access to the source\ncode of the modified version.\n\n  An older license, called the Affero General Public License and\npublished by Affero, was designed to accomplish similar goals.  This is\na different license, not a version of the Affero GPL, but Affero has\nreleased a new version of the Affero GPL which permits relicensing under\nthis license.\n\n  The precise terms and conditions for copying, distribution and\nmodification follow.\n\n                       TERMS AND CONDITIONS\n\n  0. Definitions.\n\n  \"This License\" refers to version 3 of the GNU Affero General Public License.\n\n  \"Copyright\" also means copyright-like laws that apply to other kinds of\nworks, such as semiconductor masks.\n\n  \"The Program\" refers to any copyrightable work licensed under this\nLicense.  Each licensee is addressed as \"you\".  \"Licensees\" and\n\"recipients\" may be individuals or organizations.\n\n  To \"modify\" a work means to copy from or adapt all or part of the work\nin a fashion requiring copyright permission, other than the making of an\nexact copy.  The resulting work is called a \"modified version\" of the\nearlier work or a work \"based on\" the earlier work.\n\n  A \"covered work\" means either the unmodified Program or a work based\non the Program.\n\n  To \"propagate\" a work means to do anything with it that, without\npermission, would make you directly or secondarily liable for\ninfringement under applicable copyright law, except executing it on a\ncomputer or modifying a private copy.  Propagation includes copying,\ndistribution (with or without modification), making available to the\npublic, and in some countries other activities as well.\n\n  To \"convey\" a work means any kind of propagation that enables other\nparties to make or receive copies.  Mere interaction with a user through\na computer network, with no transfer of a copy, is not conveying.\n\n  An interactive user interface displays \"Appropriate Legal Notices\"\nto the extent that it includes a convenient and prominently visible\nfeature that (1) displays an appropriate copyright notice, and (2)\ntells the user that there is no warranty for the work (except to the\nextent that warranties are provided), that licensees may convey the\nwork under this License, and how to view a copy of this License.  If\nthe interface presents a list of user commands or options, such as a\nmenu, a prominent item in the list meets this criterion.\n\n  1. Source Code.\n\n  The \"source code\" for a work means the preferred form of the work\nfor making modifications to it.  \"Object code\" means any non-source\nform of a work.\n\n  A \"Standard Interface\" means an interface that either is an official\nstandard defined by a recognized standards body, or, in the case of\ninterfaces specified for a particular programming language, one that\nis widely used among developers working in that language.\n\n  The \"System Libraries\" of an executable work include anything, other\nthan the work as a whole, that (a) is included in the normal form of\npackaging a Major Component, but which is not part of that Major\nComponent, and (b) serves only to enable use of the work with that\nMajor Component, or to implement a Standard Interface for which an\nimplementation is available to the public in source code form.  A\n\"Major Component\", in this context, means a major essential component\n(kernel, window system, and so on) of the specific operating system\n(if any) on which the executable work runs, or a compiler used to\nproduce the work, or an object code interpreter used to run it.\n\n  The \"Corresponding Source\" for a work in object code form means all\nthe source code needed to generate, install, and (for an executable\nwork) run the object code and to modify the work, including scripts to\ncontrol those activities.  However, it does not include the work's\nSystem Libraries, or general-purpose tools or generally available free\nprograms which are used unmodified in performing those activities but\nwhich are not part of the work.  For example, Corresponding Source\nincludes interface definition files associated with source files for\nthe work, and the source code for shared libraries and dynamically\nlinked subprograms that the work is specifically designed to require,\nsuch as by intimate data communication or control flow between those\nsubprograms and other parts of the work.\n\n  The Corresponding Source need not include anything that users\ncan regenerate automatically from other parts of the Corresponding\nSource.\n\n  The Corresponding Source for a work in source code form is that\nsame work.\n\n  2. Basic Permissions.\n\n  All rights granted under this License are granted for the term of\ncopyright on the Program, and are irrevocable provided the stated\nconditions are met.  This License explicitly affirms your unlimited\npermission to run the unmodified Program.  The output from running a\ncovered work is covered by this License only if the output, given its\ncontent, constitutes a covered work.  This License acknowledges your\nrights of fair use or other equivalent, as provided by copyright law.\n\n  You may make, run and propagate covered works that you do not\nconvey, without conditions so long as your license otherwise remains\nin force.  You may convey covered works to others for the sole purpose\nof having them make modifications exclusively for you, or provide you\nwith facilities for running those works, provided that you comply with\nthe terms of this License in conveying all material for which you do\nnot control copyright.  Those thus making or running the covered works\nfor you must do so exclusively on your behalf, under your direction\nand control, on terms that prohibit them from making any copies of\nyour copyrighted material outside their relationship with you.\n\n  Conveying under any other circumstances is permitted solely under\nthe conditions stated below.  Sublicensing is not allowed; section 10\nmakes it unnecessary.\n\n  3. Protecting Users' Legal Rights From Anti-Circumvention Law.\n\n  No covered work shall be deemed part of an effective technological\nmeasure under any applicable law fulfilling obligations under article\n11 of the WIPO copyright treaty adopted on 20 December 1996, or\nsimilar laws prohibiting or restricting circumvention of such\nmeasures.\n\n  When you convey a covered work, you waive any legal power to forbid\ncircumvention of technological measures to the extent such circumvention\nis effected by exercising rights under this License with respect to\nthe covered work, and you disclaim any intention to limit operation or\nmodification of the work as a means of enforcing, against the work's\nusers, your or third parties' legal rights to forbid circumvention of\ntechnological measures.\n\n  4. Conveying Verbatim Copies.\n\n  You may convey verbatim copies of the Program's source code as you\nreceive it, in any medium, provided that you conspicuously and\nappropriately publish on each copy an appropriate copyright notice;\nkeep intact all notices stating that this License and any\nnon-permissive terms added in accord with section 7 apply to the code;\nkeep intact all notices of the absence of any warranty; and give all\nrecipients a copy of this License along with the Program.\n\n  You may charge any price or no price for each copy that you convey,\nand you may offer support or warranty protection for a fee.\n\n  5. Conveying Modified Source Versions.\n\n  You may convey a work based on the Program, or the modifications to\nproduce it from the Program, in the form of source code under the\nterms of section 4, provided that you also meet all of these conditions:\n\n    a) The work must carry prominent notices stating that you modified\n    it, and giving a relevant date.\n\n    b) The work must carry prominent notices stating that it is\n    released under this License and any conditions added under section\n    7.  This requirement modifies the requirement in section 4 to\n    \"keep intact all notices\".\n\n    c) You must license the entire work, as a whole, under this\n    License to anyone who comes into possession of a copy.  This\n    License will therefore apply, along with any applicable section 7\n    additional terms, to the whole of the work, and all its parts,\n    regardless of how they are packaged.  This License gives no\n    permission to license the work in any other way, but it does not\n    invalidate such permission if you have separately received it.\n\n    d) If the work has interactive user interfaces, each must display\n    Appropriate Legal Notices; however, if the Program has interactive\n    interfaces that do not display Appropriate Legal Notices, your\n    work need not make them do so.\n\n  A compilation of a covered work with other separate and independent\nworks, which are not by their nature extensions of the covered work,\nand which are not combined with it such as to form a larger program,\nin or on a volume of a storage or distribution medium, is called an\n\"aggregate\" if the compilation and its resulting copyright are not\nused to limit the access or legal rights of the compilation's users\nbeyond what the individual works permit.  Inclusion of a covered work\nin an aggregate does not cause this License to apply to the other\nparts of the aggregate.\n\n  6. Conveying Non-Source Forms.\n\n  You may convey a covered work in object code form under the terms\nof sections 4 and 5, provided that you also convey the\nmachine-readable Corresponding Source under the terms of this License,\nin one of these ways:\n\n    a) Convey the object code in, or embodied in, a physical product\n    (including a physical distribution medium), accompanied by the\n    Corresponding Source fixed on a durable physical medium\n    customarily used for software interchange.\n\n    b) Convey the object code in, or embodied in, a physical product\n    (including a physical distribution medium), accompanied by a\n    written offer, valid for at least three years and valid for as\n    long as you offer spare parts or customer support for that product\n    model, to give anyone who possesses the object code either (1) a\n    copy of the Corresponding Source for all the software in the\n    product that is covered by this License, on a durable physical\n    medium customarily used for software interchange, for a price no\n    more than your reasonable cost of physically performing this\n    conveying of source, or (2) access to copy the\n    Corresponding Source from a network server at no charge.\n\n    c) Convey individual copies of the object code with a copy of the\n    written offer to provide the Corresponding Source.  This\n    alternative is allowed only occasionally and noncommercially, and\n    only if you received the object code with such an offer, in accord\n    with subsection 6b.\n\n    d) Convey the object code by offering access from a designated\n    place (gratis or for a charge), and offer equivalent access to the\n    Corresponding Source in the same way through the same place at no\n    further charge.  You need not require recipients to copy the\n    Corresponding Source along with the object code.  If the place to\n    copy the object code is a network server, the Corresponding Source\n    may be on a different server (operated by you or a third party)\n    that supports equivalent copying facilities, provided you maintain\n    clear directions next to the object code saying where to find the\n    Corresponding Source.  Regardless of what server hosts the\n    Corresponding Source, you remain obligated to ensure that it is\n    available for as long as needed to satisfy these requirements.\n\n    e) Convey the object code using peer-to-peer transmission, provided\n    you inform other peers where the object code and Corresponding\n    Source of the work are being offered to the general public at no\n    charge under subsection 6d.\n\n  A separable portion of the object code, whose source code is excluded\nfrom the Corresponding Source as a System Library, need not be\nincluded in conveying the object code work.\n\n  A \"User Product\" is either (1) a \"consumer product\", which means any\ntangible personal property which is normally used for personal, family,\nor household purposes, or (2) anything designed or sold for incorporation\ninto a dwelling.  In determining whether a product is a consumer product,\ndoubtful cases shall be resolved in favor of coverage.  For a particular\nproduct received by a particular user, \"normally used\" refers to a\ntypical or common use of that class of product, regardless of the status\nof the particular user or of the way in which the particular user\nactually uses, or expects or is expected to use, the product.  A product\nis a consumer product regardless of whether the product has substantial\ncommercial, industrial or non-consumer uses, unless such uses represent\nthe only significant mode of use of the product.\n\n  \"Installation Information\" for a User Product means any methods,\nprocedures, authorization keys, or other information required to install\nand execute modified versions of a covered work in that User Product from\na modified version of its Corresponding Source.  The information must\nsuffice to ensure that the continued functioning of the modified object\ncode is in no case prevented or interfered with solely because\nmodification has been made.\n\n  If you convey an object code work under this section in, or with, or\nspecifically for use in, a User Product, and the conveying occurs as\npart of a transaction in which the right of possession and use of the\nUser Product is transferred to the recipient in perpetuity or for a\nfixed term (regardless of how the transaction is characterized), the\nCorresponding Source conveyed under this section must be accompanied\nby the Installation Information.  But this requirement does not apply\nif neither you nor any third party retains the ability to install\nmodified object code on the User Product (for example, the work has\nbeen installed in ROM).\n\n  The requirement to provide Installation Information does not include a\nrequirement to continue to provide support service, warranty, or updates\nfor a work that has been modified or installed by the recipient, or for\nthe User Product in which it has been modified or installed.  Access to a\nnetwork may be denied when the modification itself materially and\nadversely affects the operation of the network or violates the rules and\nprotocols for communication across the network.\n\n  Corresponding Source conveyed, and Installation Information provided,\nin accord with this section must be in a format that is publicly\ndocumented (and with an implementation available to the public in\nsource code form), and must require no special password or key for\nunpacking, reading or copying.\n\n  7. Additional Terms.\n\n  \"Additional permissions\" are terms that supplement the terms of this\nLicense by making exceptions from one or more of its conditions.\nAdditional permissions that are applicable to the entire Program shall\nbe treated as though they were included in this License, to the extent\nthat they are valid under applicable law.  If additional permissions\napply only to part of the Program, that part may be used separately\nunder those permissions, but the entire Program remains governed by\nthis License without regard to the additional permissions.\n\n  When you convey a copy of a covered work, you may at your option\nremove any additional permissions from that copy, or from any part of\nit.  (Additional permissions may be written to require their own\nremoval in certain cases when you modify the work.)  You may place\nadditional permissions on material, added by you to a covered work,\nfor which you have or can give appropriate copyright permission.\n\n  Notwithstanding any other provision of this License, for material you\nadd to a covered work, you may (if authorized by the copyright holders of\nthat material) supplement the terms of this License with terms:\n\n    a) Disclaiming warranty or limiting liability differently from the\n    terms of sections 15 and 16 of this License; or\n\n    b) Requiring preservation of specified reasonable legal notices or\n    author attributions in that material or in the Appropriate Legal\n    Notices displayed by works containing it; or\n\n    c) Prohibiting misrepresentation of the origin of that material, or\n    requiring that modified versions of such material be marked in\n    reasonable ways as different from the original version; or\n\n    d) Limiting the use for publicity purposes of names of licensors or\n    authors of the material; or\n\n    e) Declining to grant rights under trademark law for use of some\n    trade names, trademarks, or service marks; or\n\n    f) Requiring indemnification of licensors and authors of that\n    material by anyone who conveys the material (or modified versions of\n    it) with contractual assumptions of liability to the recipient, for\n    any liability that these contractual assumptions directly impose on\n    those licensors and authors.\n\n  All other non-permissive additional terms are considered \"further\nrestrictions\" within the meaning of section 10.  If the Program as you\nreceived it, or any part of it, contains a notice stating that it is\ngoverned by this License along with a term that is a further\nrestriction, you may remove that term.  If a license document contains\na further restriction but permits relicensing or conveying under this\nLicense, you may add to a covered work material governed by the terms\nof that license document, provided that the further restriction does\nnot survive such relicensing or conveying.\n\n  If you add terms to a covered work in accord with this section, you\nmust place, in the relevant source files, a statement of the\nadditional terms that apply to those files, or a notice indicating\nwhere to find the applicable terms.\n\n  Additional terms, permissive or non-permissive, may be stated in the\nform of a separately written license, or stated as exceptions;\nthe above requirements apply either way.\n\n  8. Termination.\n\n  You may not propagate or modify a covered work except as expressly\nprovided under this License.  Any attempt otherwise to propagate or\nmodify it is void, and will automatically terminate your rights under\nthis License (including any patent licenses granted under the third\nparagraph of section 11).\n\n  However, if you cease all violation of this License, then your\nlicense from a particular copyright holder is reinstated (a)\nprovisionally, unless and until the copyright holder explicitly and\nfinally terminates your license, and (b) permanently, if the copyright\nholder fails to notify you of the violation by some reasonable means\nprior to 60 days after the cessation.\n\n  Moreover, your license from a particular copyright holder is\nreinstated permanently if the copyright holder notifies you of the\nviolation by some reasonable means, this is the first time you have\nreceived notice of violation of this License (for any work) from that\ncopyright holder, and you cure the violation prior to 30 days after\nyour receipt of the notice.\n\n  Termination of your rights under this section does not terminate the\nlicenses of parties who have received copies or rights from you under\nthis License.  If your rights have been terminated and not permanently\nreinstated, you do not qualify to receive new licenses for the same\nmaterial under section 10.\n\n  9. Acceptance Not Required for Having Copies.\n\n  You are not required to accept this License in order to receive or\nrun a copy of the Program.  Ancillary propagation of a covered work\noccurring solely as a consequence of using peer-to-peer transmission\nto receive a copy likewise does not require acceptance.  However,\nnothing other than this License grants you permission to propagate or\nmodify any covered work.  These actions infringe copyright if you do\nnot accept this License.  Therefore, by modifying or propagating a\ncovered work, you indicate your acceptance of this License to do so.\n\n  10. Automatic Licensing of Downstream Recipients.\n\n  Each time you convey a covered work, the recipient automatically\nreceives a license from the original licensors, to run, modify and\npropagate that work, subject to this License.  You are not responsible\nfor enforcing compliance by third parties with this License.\n\n  An \"entity transaction\" is a transaction transferring control of an\norganization, or substantially all assets of one, or subdividing an\norganization, or merging organizations.  If propagation of a covered\nwork results from an entity transaction, each party to that\ntransaction who receives a copy of the work also receives whatever\nlicenses to the work the party's predecessor in interest had or could\ngive under the previous paragraph, plus a right to possession of the\nCorresponding Source of the work from the predecessor in interest, if\nthe predecessor has it or can get it with reasonable efforts.\n\n  You may not impose any further restrictions on the exercise of the\nrights granted or affirmed under this License.  For example, you may\nnot impose a license fee, royalty, or other charge for exercise of\nrights granted under this License, and you may not initiate litigation\n(including a cross-claim or counterclaim in a lawsuit) alleging that\nany patent claim is infringed by making, using, selling, offering for\nsale, or importing the Program or any portion of it.\n\n  11. Patents.\n\n  A \"contributor\" is a copyright holder who authorizes use under this\nLicense of the Program or a work on which the Program is based.  The\nwork thus licensed is called the contributor's \"contributor version\".\n\n  A contributor's \"essential patent claims\" are all patent claims\nowned or controlled by the contributor, whether already acquired or\nhereafter acquired, that would be infringed by some manner, permitted\nby this License, of making, using, or selling its contributor version,\nbut do not include claims that would be infringed only as a\nconsequence of further modification of the contributor version.  For\npurposes of this definition, \"control\" includes the right to grant\npatent sublicenses in a manner consistent with the requirements of\nthis License.\n\n  Each contributor grants you a non-exclusive, worldwide, royalty-free\npatent license under the contributor's essential patent claims, to\nmake, use, sell, offer for sale, import and otherwise run, modify and\npropagate the contents of its contributor version.\n\n  In the following three paragraphs, a \"patent license\" is any express\nagreement or commitment, however denominated, not to enforce a patent\n(such as an express permission to practice a patent or covenant not to\nsue for patent infringement).  To \"grant\" such a patent license to a\nparty means to make such an agreement or commitment not to enforce a\npatent against the party.\n\n  If you convey a covered work, knowingly relying on a patent license,\nand the Corresponding Source of the work is not available for anyone\nto copy, free of charge and under the terms of this License, through a\npublicly available network server or other readily accessible means,\nthen you must either (1) cause the Corresponding Source to be so\navailable, or (2) arrange to deprive yourself of the benefit of the\npatent license for this particular work, or (3) arrange, in a manner\nconsistent with the requirements of this License, to extend the patent\nlicense to downstream recipients.  \"Knowingly relying\" means you have\nactual knowledge that, but for the patent license, your conveying the\ncovered work in a country, or your recipient's use of the covered work\nin a country, would infringe one or more identifiable patents in that\ncountry that you have reason to believe are valid.\n\n  If, pursuant to or in connection with a single transaction or\narrangement, you convey, or propagate by procuring conveyance of, a\ncovered work, and grant a patent license to some of the parties\nreceiving the covered work authorizing them to use, propagate, modify\nor convey a specific copy of the covered work, then the patent license\nyou grant is automatically extended to all recipients of the covered\nwork and works based on it.\n\n  A patent license is \"discriminatory\" if it does not include within\nthe scope of its coverage, prohibits the exercise of, or is\nconditioned on the non-exercise of one or more of the rights that are\nspecifically granted under this License.  You may not convey a covered\nwork if you are a party to an arrangement with a third party that is\nin the business of distributing software, under which you make payment\nto the third party based on the extent of your activity of conveying\nthe work, and under which the third party grants, to any of the\nparties who would receive the covered work from you, a discriminatory\npatent license (a) in connection with copies of the covered work\nconveyed by you (or copies made from those copies), or (b) primarily\nfor and in connection with specific products or compilations that\ncontain the covered work, unless you entered into that arrangement,\nor that patent license was granted, prior to 28 March 2007.\n\n  Nothing in this License shall be construed as excluding or limiting\nany implied license or other defenses to infringement that may\notherwise be available to you under applicable patent law.\n\n  12. No Surrender of Others' Freedom.\n\n  If conditions are imposed on you (whether by court order, agreement or\notherwise) that contradict the conditions of this License, they do not\nexcuse you from the conditions of this License.  If you cannot convey a\ncovered work so as to satisfy simultaneously your obligations under this\nLicense and any other pertinent obligations, then as a consequence you may\nnot convey it at all.  For example, if you agree to terms that obligate you\nto collect a royalty for further conveying from those to whom you convey\nthe Program, the only way you could satisfy both those terms and this\nLicense would be to refrain entirely from conveying the Program.\n\n  13. Remote Network Interaction; Use with the GNU General Public License.\n\n  Notwithstanding any other provision of this License, if you modify the\nProgram, your modified version must prominently offer all users\ninteracting with it remotely through a computer network (if your version\nsupports such interaction) an opportunity to receive the Corresponding\nSource of your version by providing access to the Corresponding Source\nfrom a network server at no charge, through some standard or customary\nmeans of facilitating copying of software.  This Corresponding Source\nshall include the Corresponding Source for any work covered by version 3\nof the GNU General Public License that is incorporated pursuant to the\nfollowing paragraph.\n\n  Notwithstanding any other provision of this License, you have\npermission to link or combine any covered work with a work licensed\nunder version 3 of the GNU General Public License into a single\ncombined work, and to convey the resulting work.  The terms of this\nLicense will continue to apply to the part which is the covered work,\nbut the work with which it is combined will remain governed by version\n3 of the GNU General Public License.\n\n  14. Revised Versions of this License.\n\n  The Free Software Foundation may publish revised and/or new versions of\nthe GNU Affero General Public License from time to time.  Such new versions will\nbe similar in spirit to the present version, but may differ in detail to\naddress new problems or concerns.\n\n  Each version is given a distinguishing version number.  If the\nProgram specifies that a certain numbered version of the GNU Affero General\nPublic License \"or any later version\" applies to it, you have the\noption of following the terms and conditions either of that numbered\nversion or of any later version published by the Free Software\nFoundation.  If the Program does not specify a version number of the\nGNU Affero General Public License, you may choose any version ever published\nby the Free Software Foundation.\n\n  If the Program specifies that a proxy can decide which future\nversions of the GNU Affero General Public License can be used, that proxy's\npublic statement of acceptance of a version permanently authorizes you\nto choose that version for the Program.\n\n  Later license versions may give you additional or different\npermissions.  However, no additional obligations are imposed on any\nauthor or copyright holder as a result of your choosing to follow a\nlater version.\n\n  15. Disclaimer of Warranty.\n\n  THERE IS NO WARRANTY FOR THE PROGRAM, TO THE EXTENT PERMITTED BY\nAPPLICABLE LAW.  EXCEPT WHEN OTHERWISE STATED IN WRITING THE COPYRIGHT\nHOLDERS AND/OR OTHER PARTIES PROVIDE THE PROGRAM \"AS IS\" WITHOUT WARRANTY\nOF ANY KIND, EITHER EXPRESSED OR IMPLIED, INCLUDING, BUT NOT LIMITED TO,\nTHE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR\nPURPOSE.  THE ENTIRE RISK AS TO THE QUALITY AND PERFORMANCE OF THE PROGRAM\nIS WITH YOU.  SHOULD THE PROGRAM PROVE DEFECTIVE, YOU ASSUME THE COST OF\nALL NECESSARY SERVICING, REPAIR OR CORRECTION.\n\n  16. Limitation of Liability.\n\n  IN NO EVENT UNLESS REQUIRED BY APPLICABLE LAW OR AGREED TO IN WRITING\nWILL ANY COPYRIGHT HOLDER, OR ANY OTHER PARTY WHO MODIFIES AND/OR CONVEYS\nTHE PROGRAM AS PERMITTED ABOVE, BE LIABLE TO YOU FOR DAMAGES, INCLUDING ANY\nGENERAL, SPECIAL, INCIDENTAL OR CONSEQUENTIAL DAMAGES ARISING OUT OF THE\nUSE OR INABILITY TO USE THE PROGRAM (INCLUDING BUT NOT LIMITED TO LOSS OF\nDATA OR DATA BEING RENDERED INACCURATE OR LOSSES SUSTAINED BY YOU OR THIRD\nPARTIES OR A FAILURE OF THE PROGRAM TO OPERATE WITH ANY OTHER PROGRAMS),\nEVEN IF SUCH HOLDER OR OTHER PARTY HAS BEEN ADVISED OF THE POSSIBILITY OF\nSUCH DAMAGES.\n\n  17. Interpretation of Sections 15 and 16.\n\n  If the disclaimer of warranty and limitation of liability provided\nabove cannot be given local legal effect according to their terms,\nreviewing courts shall apply local law that most closely approximates\nan absolute waiver of all civil liability in connection with the\nProgram, unless a warranty or assumption of liability accompanies a\ncopy of the Program in return for a fee.\n\n                     END OF TERMS AND CONDITIONS\n\n            How to Apply These Terms to Your New Programs\n\n  If you develop a new program, and you want it to be of the greatest\npossible use to the public, the best way to achieve this is to make it\nfree software which everyone can redistribute and change under these terms.\n\n  To do so, attach the following notices to the program.  It is safest\nto attach them to the start of each source file to most effectively\nstate the exclusion of warranty; and each file should have at least\nthe \"copyright\" line and a pointer to where the full notice is found.\n\n    <one line to give the program's name and a brief idea of what it does.>\n    Copyright (C) <year>  <name of author>\n\n    This program is free software: you can redistribute it and/or modify\n    it under the terms of the GNU Affero General Public License as published by\n    the Free Software Foundation, either version 3 of the License, or\n    (at your option) any later version.\n\n    This program is distributed in the hope that it will be useful,\n    but WITHOUT ANY WARRANTY; without even the implied warranty of\n    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the\n    GNU Affero General Public License for more details.\n\n    You should have received a copy of the GNU Affero General Public License\n    along with this program.  If not, see <https://www.gnu.org/licenses/>.\n\nAlso add information on how to contact you by electronic and paper mail.\n\n  If your software can interact with users remotely through a computer\nnetwork, you should also make sure that it provides a way for users to\nget its source.  For example, if your program is a web application, its\ninterface could display a \"Source\" link that leads users to an archive\nof the code.  There are many ways you could offer source, and different\nsolutions will be better for different programs; see section 13 for the\nspecific requirements.\n\n  You should also get your employer (if you work as a programmer) or school,\nif any, to sign a \"copyright disclaimer\" for the program, if necessary.\nFor more information on this, and how to apply and follow the GNU AGPL, see\n<https://www.gnu.org/licenses/>.\n\n\nOriginal component notice:\n\nMIT License\n\nCopyright (c) 2026 Winspace contributors\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software and associated documentation files (the \"Software\"), to deal\nin the Software without restriction, including without limitation the rights\nto use, copy, modify, merge, publish, distribute, sublicense, and/or sell\ncopies of the Software, and to permit persons to whom the Software is\nfurnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all\ncopies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\nIMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\nFITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\nAUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\nOUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\nSOFTWARE.\n");}
function renderSettingsPage(page){
  page.replaceChildren();page.classList.add('settings-page');
  const nav=elem('nav','settings-navigation');nav.setAttribute('aria-label','Settings sections');
  nav.append(elem('h1','','Settings'),elem('p','settings-subtitle','Your explorer, your way.'));
  for(const [key,label,ico]of [['appearance','Appearance & layout','sun'],['search','Search & indexing','search'],['default','Default file explorer','folderline']]){
    nav.append(button(label,()=>{$('settings-'+key)?.scrollIntoView({block:'start',behavior:'smooth'});},'settings-nav-link',ico));
  }
  nav.append(button('Back to files',()=>{closeTab(state.activeId);},'settings-back','back'));
  const body=elem('div','settings-body');page.append(nav,body);const sectionName=state.settingsSection;
    body.className='settings-body';
    const appearanceSection=elem('section','settings-section');appearanceSection.id='settings-appearance';body.append(appearanceSection);
    const appearance=elem('div','settings-line');const label=elem('div');label.append(elem('strong','','Appearance'),elem('p','','The app, menus, and network sign-in use the same theme.'));const select=elem('select');select.setAttribute('aria-label','Theme');for(const [v,n]of [['system','Use system'],['light','Light'],['dark','Dark']]){const o=elem('option','',n);o.value=v;select.append(o);}select.value=state.theme;select.onchange=()=>applyTheme(select.value);appearance.append(label,select);appearanceSection.append(elem('h2','','Appearance & layout'),appearance);
    textSizeControls(appearanceSection);
    menuPreferenceControls(appearanceSection);appearanceSection.append(button('Reset sidebar and column widths',resetLayout,'secondary','refresh'));
    const cacheSection=elem('section','settings-section');cacheSection.id='settings-search';body.append(cacheSection);
    const heading=elem('div','settings-section-title');heading.append(icon('search'),elem('h3','','Search cache'),button('Refresh all',async()=>{await call('cacheRefresh',{});await refreshCacheStatus();},'secondary','refresh'));cacheSection.append(heading);
    cacheSection.append(elem('p','settings-help','Check a folder to index the names and paths of its files and subfolders. SMB folders work too. File contents are never cached. Open a protected share and sign in before indexing it.'));
    const list=elem('div','settings-cache-list');list.id='settings-cache-list';cacheSection.append(list);
    const add=elem('div','cache-add');const address=elem('input');address.type='text';address.placeholder='Add a folder: /home/you/Projects or \\\\nas\\share';address.setAttribute('aria-label','Folder to cache');const addButton=button('Add',async()=>{try{const r=await call('normalise',{value:address.value,base:state.settingsOrigin||state.env.home});await setCache(r.uri,true);address.value='';}catch(e){toast(e.message);}},'secondary','plus');add.append(address,addButton);cacheSection.append(add);
    const auto=elem('label','checkbox-row');const autoCheck=elem('input');autoCheck.type='checkbox';autoCheck.checked=state.env?.preferences?.autoIndex!==false;autoCheck.onchange=()=>{state.env.preferences.autoIndex=autoCheck.checked;fire('preferences',{autoIndex:autoCheck.checked});};auto.append(autoCheck,document.createTextNode('Watch enabled local folders for changes while OpenXplorer is open'));cacheSection.append(auto);
    const network=elem('div','settings-line');const networkText=elem('div');networkText.append(elem('strong','','Network / fallback checks'),elem('p','','SMB and unwatched folders use incremental directory checks, not push notifications. Large trees take longer than one interval.'));
    const interval=elem('select');interval.setAttribute('aria-label','Network check interval');for(const [value,label]of [[30,'30 seconds'],[60,'1 minute'],[300,'5 minutes']]){const option=elem('option','',label);option.value=value;interval.append(option);}interval.value=state.env.preferences.networkInterval||60;interval.onchange=()=>{state.env.preferences.networkInterval=Number(interval.value);fire('preferences',{networkInterval:Number(interval.value)});};network.append(networkText,interval);cacheSection.append(network);
    cacheSection.append(elem('p','settings-help','Local changes update the index after a short debounce. Watching uses up to 8,192 directories; timed checks cover any remaining ones. Disk roots skip system/temporary folders, nested mounts and symlinks. Select each mounted volume separately. Initial scans are limited to 1 million entries.'));
    const privacy=elem('div','settings-privacy');privacy.append(icon('info',14),elem('span','','Cached paths are stored locally and can be searched while a share is offline. Uncheck a root to stop caching and remove that root’s names. Other overlapping roots may still contain them. Hidden folders and symbolic links are skipped.'));cacheSection.append(privacy);
    const integration=elem('section','integration-card settings-section');integration.id='settings-default';const top=elem('div');top.append(icon('folderline',23));const text=elem('div');text.append(elem('strong','','Default file explorer'),elem('p','','Open local folders and SMB links in OpenXplorer. System file-picker dialogs are unchanged.'));top.append(text);integration.append(top);
    const status=elem('div','default-status','Checking the current default…');status.id='default-status';const controls=elem('div','integration-actions');const make=button('Make OpenXplorer default',()=>changeDefault('desktopDefault'),'primary');make.id='make-default';const restore=button('Restore previous',()=>changeDefault('desktopRestore'),'secondary');restore.id='restore-default';controls.append(make,restore);integration.append(status,controls);if(!native)integration.append(elem('p','settings-help','Browser preview: these controls simulate the setting; your real defaults are not changed.'));body.append(integration);
    setTimeout(()=>{renderSettingsCache();void updateDefaultStatus();if(sectionName)body.querySelector('#settings-'+sectionName)?.scrollIntoView({block:'start'});},0);

  const sizes=elem('section','settings-section');sizes.id='settings-sizes';sizes.append(elem('h2','','Folder sizes'),elem('p','settings-help','Right-click a folder → Calculate folder size. Scans run on demand, outside the browsing worker pool. Results are logical file bytes, not compressed ZFS space or snapshot usage. They are kept only for this window session. Recalculate to pick up later changes.'),elem('p','settings-help','Scans include hidden files, but skip links, nested filesystem mounts and snapshot collections. Each folder is limited to 1 million entries or 5 minutes between I/O calls. Inaccessible or excluded entries produce a partial total. Cancel stops the active scan and any queued folders.'));
  body.append(sizes);appendV07Settings(nav,body);
  const legalSection=elem('section','settings-section');legalSection.id='settings-license';legalSection.append(elem('h2','','OpenXplorer · License & source'),elem('p','settings-help','Copyright (c) 2026 OpenXplorer contributors. AGPL-3.0-only. No warranty. Original component notices are preserved.'),button('Read license & source information',showLicense,'secondary','info'));body.append(legalSection);
}
function renderSettingsCache(){
  const list=$('settings-cache-list');if(!list)return;list.replaceChildren();
  const roots=state.cache?.roots||[],places=[];const add=(uri,label)=>{uri=realLocation(uri);if(uri&&!['pc:','network:','settings:'].includes(uri)&&!deviceLocation(uri)&&!isSmbServer(uri)&&!places.some(p=>sameLocation(p.uri,uri)))places.push({uri,label:label||titleFor(uri)});};
  add(state.settingsOrigin||active()?.uri,titleFor(state.settingsOrigin||active()?.uri||''));for(const r of roots)add(r.uri,r.label);add(state.env?.home,'Home');for(const q of state.env?.quick||[])add(q.uri,q.label);for(const s of state.env?.shares||[])add(s.uri,s.label);add('file:///','Local Disk');for(const m of state.env?.mounts||[])if(m.mounted&&!deviceLocation(m.uri))add(m.uri,m.label);
  for(const p of places){const r=roots.find(r=>sameLocation(r.uri,p.uri));const row=elem('div','cache-setting-row');const label=elem('label');const cb=elem('input');cb.type='checkbox';cb.checked=!!r?.enabled;cb.setAttribute('aria-label','Cache '+p.label);cb.onchange=async()=>{cb.disabled=true;await setCache(p.uri,cb.checked,p.label);};label.append(cb,folderIcon(24));const text=elem('div','cache-folder-text');text.append(elem('strong','',p.label),elem('span','cache-folder-path',displayUri(p.uri)));label.append(text);row.append(label);const meta=elem('div','cache-row-meta');const status=r?.enabled?`${r.status} · ${Number(r.count||0).toLocaleString()} names · ${r.update_mode||'Snapshot'}`:'Not cached';const statusText=elem('span','cache-row-status',status);statusText.title=r?.watch_error||r?.error|| (r?.updated?'Last full refresh: '+new Date(r.updated*1000).toLocaleString():'Not yet scanned');meta.append(statusText);if(r?.enabled){const scan=button('',async()=>{await call(r.status==='Indexing'?'cacheStop':'cacheRefresh',{uri:p.uri});await refreshCacheStatus();},'cache-small',r.status==='Indexing'?'cancel':'refresh');scan.title=r.status==='Indexing'?'Stop indexing':'Refresh cache';scan.setAttribute('aria-label',scan.title+' '+p.label);meta.append(scan);const clear=button('',async()=>{await call('cacheClear',{uri:p.uri});await refreshCacheStatus();},'cache-small','trash');clear.title='Clear cached names only';clear.setAttribute('aria-label','Clear cache for '+p.label);meta.append(clear);}row.append(meta);list.append(row);}
}
async function updateDefaultStatus(){try{const d=await call('desktopStatus');renderDefaultStatus(d);}catch(e){if($('default-status'))$('default-status').textContent=e.message;}}
function renderDefaultStatus(d){
  if(!$('default-status'))return;
  const names=[['inode/directory','Folders'],['x-scheme-handler/smb','SMB links'],['application/zip','ZIP files']];
  const current=d.current||{};const container=$('default-status');container.replaceChildren();
  for(const [kind,label]of names){const row=elem('div','handler-status-row');row.dataset.kind=kind;const value=current[kind]||'Not set';row.append(elem('strong','',label),elem('span','',value==='io.winspace.Development.desktop'?'OpenXplorer':value));container.append(row);}
  $('make-default').disabled=false; // Reapply after another app changes a route.
  $('restore-default').disabled=!d.canRestore;
  if($('zip-status'))$('zip-status').textContent=d.zipDefault?'ZIP opening: OpenXplorer. This is separate from folder defaults.':'ZIP opening uses '+(current['application/zip']||'the desktop choice')+'. Opening a download is not Show in folder.';
  if($('zip-default'))$('zip-default').disabled=!!d.zipDefault;
  if($('zip-restore'))$('zip-restore').disabled=!d.canRestoreZip;
  const route=$('reveal-status');if(route)route.textContent=d.revealOwned?'Show in folder: OpenXplorer owns FileManager1. Browser portal routing is a separate check.':d.revealEnabled?'Show in folder: enabled, waiting'+(d.revealOwner?' for '+d.revealOwner:' for the current file manager')+'. Close other file managers or log out and back in.':'Show in folder: not enabled. Folder associations alone do not control every browser route.';
}
async function changeDefault(method){
  const make=$('make-default'),restore=$('restore-default');if(make)make.disabled=true;if(restore)restore.disabled=true;
  try{const r=await call(method,{reveal:$('include-reveal')?.checked===true,zip:$('include-zip')?.checked===true});renderDefaultStatus(r);
    if(r.integrationError)toast('File handlers updated, but Show in folder setup failed: '+r.integrationError);
    else toast(native?(method==='desktopDefault'?'Requested associations updated. Review each status below.':'Previous recorded handlers restored.'):'Preview only. No system defaults were changed.');
  }catch(e){toast(e.message);await updateDefaultStatus();}
}
async function changeZipDefault(method){
  try{renderDefaultStatus(await call(method));toast(native?(method==='zipDefault'?'ZIP files now open in OpenXplorer.':'Previous ZIP handlers restored.'):'Preview only. ZIP associations were not changed.');}
  catch(e){toast(e.message);await updateDefaultStatus();}
}


async function signOut(uri){
  if(state.operation){toast('Finish the current file operation before signing out.');return;}
  const host=new URL(uri).hostname;
  const ok=await showModal('Sign out of '+host+'?','This disconnects all SMB mounts for this server in your desktop session, including other applications. Close files on this server first.',body=>{
    const row=elem('label','checkbox-row'),forget=elem('input');forget.type='checkbox';forget.checked=true;row.append(forget,document.createTextNode('Forget saved credentials for this server'));body.append(row);
    const cacheRow=elem('label','checkbox-row'),clearCache=elem('input');clearCache.type='checkbox';clearCache.checked=false;cacheRow.append(clearCache,document.createTextNode('Also clear cached filenames for this server'));body.append(cacheRow,elem('div','modal-note','Pinned shortcuts remain. With saved credentials removed, opening a share will ask you to sign in again. This does not delete files on the server. Hostname aliases may have separate saved credentials.'));return{forget,clearCache};
  },[{label:'Cancel',cancel:true},{label:'Sign out',className:'primary',fn:f=>({forget:f.forget.checked,clearCache:f.clearCache.checked})}]);
  if(!ok)return;
  state.signedOutHosts.add(host);for(const t of state.tabs){try{if(new URL(t.uri).hostname===host&&t.busy)fire('cancel',{token:t.loadToken});}catch{}}
  try{await call('signOut',{uri,...ok});for(const t of state.tabs){try{if(new URL(t.uri).hostname===host){t.loaded=false;t.entries=[];}}catch{}}await navigate('network:');await refreshEnvironment();await refreshCacheStatus();toast(ok.forget?'Signed out. Matching saved credentials were cleared or none were present.':'Disconnected. Saved credentials are retained.');}
  catch(e){await refreshEnvironment();await showMessage('Sign-out did not fully finish',e.message);}
}

function renderNetwork(l){
  l.replaceChildren(elem('h1','','Network'),elem('p','subtitle','Find shared storage on your local network, or enter an address.'));
  const banner=elem('div','network-banner');banner.append(icon('network',38));const words=elem('div');words.append(elem('div','','Computers & network storage'),elem('p','',state.discovery.busy?'Listening for advertised SMB servers…':'Discover devices without scanning their files.'));banner.append(words,button(state.discovery.busy?'Stop':'Discover servers',state.discovery.busy?cancelDiscovery:discoverNetwork,'primary'));l.append(banner);
  const manual=elem('div','network-manual');const address=elem('input');address.type='text';address.placeholder='\\\\server or \\\\archive-nas';address.setAttribute('aria-label','SMB server address');const open=async()=>{try{const r=await call('normalise',{value:address.value});if(!r.uri.startsWith('smb:'))throw Error('Enter an SMB server or share.');navigate(r.uri);}catch(e){toast(e.message);}};address.addEventListener('keydown',e=>{if(e.key==='Enter')void open();});manual.append(address,button('Open address',open,'secondary'),button('Map location',connectDialog,'secondary','plus'));l.append(manual);
  const title=elem('div','section-title');title.append(icon('desktop'),document.createTextNode('Discovered servers'),elem('span','network-count',String(state.discovery.servers.length)));l.append(title);
  const found=elem('div','drive-grid');for(const s of state.discovery.servers){const card=button('',()=>navigate(s.uri),'drive-card discovered-server');bindMiddleOpen(card,()=>s.uri);card.append(icon('server',40));const d=elem('div','drive-info');d.append(elem('div','card-name',s.label),elem('div','card-sub',displayUri(s.uri)),elem('span','network-protocol','SMB · '+(native?'Discovered':'Sample device')));card.append(d);found.append(card);}l.append(found);
  if(!state.discovery.servers.length)l.append(elem('div','notice',state.discovery.busy?'Discovery can take a few seconds.':state.discovery.error||'No advertised SMB servers found yet. Discover again or enter an address above.'));
  l.append(elem('p','discovery-note','Discovery depends on devices advertising themselves and on local firewall/network settings. It does not guarantee a list of every host.'));
  const saved=elem('div','section-title');saved.append(icon('pin'),document.createTextNode('Connected & saved locations'));l.append(saved);const grid=elem('div','drive-grid');for(const s of networkLocations()){const card=button('',()=>navigate(s.uri),'drive-card');bindMiddleOpen(card,()=>s.uri);card.append(icon('network',34));const d=elem('div','drive-info');d.append(elem('div','card-name',s.label),elem('div','card-sub',displayUri(s.uri)));card.append(d);card.addEventListener('contextmenu',e=>{e.preventDefault();networkLocationMenu(e.clientX,e.clientY,s);});grid.append(card);}l.append(grid);
  if(!native){const demo=elem('div','network-preview-note');demo.append(icon('info'),elem('span','','Interactive preview — no real network access.'),button('Preview sign-in',authPreview,'secondary'));l.append(demo);}
}
async function discoverNetwork(){
  if(state.discovery.busy)return;state.discovery.started=true;state.discovery.busy=true;state.discovery.error='';state.discovery.servers=[];const key=++state.discovery.generation;if(active()?.uri==='network:')renderNetwork($('landing'));
  try{for(let attempt=0;attempt<(native?3:1);attempt++){const token='discovery-'+(++seq);state.discovery.token=token;const r=await call('discover',{token});if(key!==state.discovery.generation)return;const map=new Map([...state.discovery.servers,...r.servers].map(s=>[s.uri,s]));state.discovery.servers=[...map.values()];state.discovery.error=(r.warnings||[]).join('\n');if(active()?.uri==='network:')renderNetwork($('landing'));if(native&&attempt<2)await new Promise(r=>setTimeout(r,2500));if(key!==state.discovery.generation)return;}}
  catch(e){if(key===state.discovery.generation)state.discovery.error=e.message;}
  finally{if(key===state.discovery.generation){state.discovery.busy=false;if(active()?.uri==='network:')renderNetwork($('landing'));}}
}
function cancelDiscovery(){state.discovery.generation++;state.discovery.busy=false;if(state.discovery.token)fire('cancel',{token:state.discovery.token});if(active()?.uri==='network:')renderNetwork($('landing'));}

// Pointer-based internal link dragging avoids WebKitGTK's native HTML5 drag
// bridge. A drop can ONLY pin/reorder; it cannot copy, move or delete a file.
function pointerPinDown(event,node,getEntries){
  if(event.button!==0||activeAuth||!$('modal-layer').hidden||state.pinBusy||state.drag||state.outgoingFile)return;
  state.pointerPending={node,getEntries,x:event.clientX,y:event.clientY,pointerId:event.pointerId};
}
function pointerPinMove(event){
  const p=state.pointerPending;if(!p||event.pointerId!==p.pointerId)return;
  if(!state.drag){if(Math.hypot(event.clientX-p.x,event.clientY-p.y)<7)return;const items=p.getEntries();if(!items.length||items.length>200||items.some(e=>!e.isDir)){state.pointerPending=null;state.suppressClickUntil=performance.now()+400;toast(native?'File dragging requires the current desktop application. Restart OpenXplorer after updating.':'This preview can pin sample folders. Dragging files into other apps requires the installed desktop application.');return;}closeMenu();state.drag={items:items.map(e=>({...e})),before:null,over:false,pointer:true};document.body.classList.add('pin-dragging');p.node.classList.add('drag-source');try{p.node.setPointerCapture(event.pointerId);}catch{}}
  event.preventDefault();const q=$('quick-access');if(!q)return;const r=q.getBoundingClientRect(),sb=$('sidebar').getBoundingClientRect();const over=event.clientX>=sb.left&&event.clientX<=sb.right&&event.clientY>=Math.max(sb.top,r.top-16)&&event.clientY<=Math.min(sb.bottom,r.bottom+22);state.drag.over=over;
  clearDropFeedback();const badge=$('pin-drag-badge');badge.replaceChildren(icon(over?'pin':'folderline'),document.createTextNode(over?'Pin to Quick access':state.drag.items.length>1?`${state.drag.items.length} folders`:state.drag.items[0].name));badge.hidden=false;badge.style.left=Math.min(event.clientX+18,innerWidth-250)+'px';badge.style.top=Math.min(event.clientY+18,innerHeight-55)+'px';
  if(over){q.classList.add('pin-drop-active');const rows=[...q.querySelectorAll('.side-entry')],before=rows.find(row=>event.clientY<row.getBoundingClientRect().top+row.offsetHeight/2);state.drag.before=before?.dataset.uri||null;if(before)before.classList.add('drop-before');else q.querySelector('.quick-drop-tail')?.classList.add('drop-end');if(event.clientY<sb.top+35)$('sidebar').scrollTop-=12;else if(event.clientY>sb.bottom-35)$('sidebar').scrollTop+=12;}
}
function pointerPinUp(event){
  const p=state.pointerPending;if(!p||event.pointerId!==p.pointerId)return;state.pointerPending=null;
  try{if(p.node.hasPointerCapture(event.pointerId))p.node.releasePointerCapture(event.pointerId);}catch{}
  if(!state.drag)return;event.preventDefault();const d=state.drag;state.suppressClickUntil=performance.now()+350;finishPinDrag();if(event.type==='pointerup'&&d.over)void pinEntries(d.items,d.before);
}


function setup(){
  // Observe visibility changes without coupling drag availability to every
  // dialog implementation. This also clears stale native geometry for auth.
  const fileLayoutObserver=new MutationObserver(scheduleFileDragLayout);
  for(const id of ['modal-layer','auth-layer','menu'])fileLayoutObserver.observe($(id),{attributes:true,attributeFilter:['hidden','class']});
  $('sidebar').addEventListener('scroll',scheduleFileDragLayout,{passive:true});
  $('tabs').addEventListener('scroll',()=>{if(native&&state.env?.nativeTabDrag)publishTabDragLayout();},{passive:true});setButton('windows-button','desktop');$('windows-button').onclick=windowsMenu;setupSidebarResize();$('breadcrumbs').addEventListener('wheel',e=>{const c=$('breadcrumbs');if(c.scrollWidth>c.clientWidth){e.preventDefault();c.scrollLeft+=e.deltaX||e.deltaY;}},{passive:false});
  for(const[id,ico]of Object.entries({newtab:'plus',minimize:'minus',maximize:'maximize','close-window':'close',back:'back',forward:'forward',up:'up',refresh:'refresh','address-edit':'down',cut:'cut',copy:'copy',paste:'paste',rename:'rename','copy-path':'share',trash:'trash',more:'more','status-list':'list','status-grid':'grid'}))setButton(id,ico);
  setButton('new','plus','New',true);setButton('sort','sort','Sort',true);setButton('view','grid','View',true);setButton('details-toggle','details','Details');setButton('connect-sidebar','plus','Map network location');setButton('check-updates','refresh','Check for updates');$('check-updates').onclick=updatesDialog;$('search-icon').append(icon('search'));$('transfer-icon').append(icon('copy'));
  $('newtab').onclick=()=>addTab();$('minimize').onclick=()=>native?fire('window',{action:'minimize'}):toast('Window controls work in the desktop application.');$('maximize').onclick=()=>{if(native)fire('window',{action:'maximize'});else document.body.style.padding=document.body.style.padding==='0px'?'34px':'0px';};$('close-window').onclick=askClose;
  $('back').onclick=()=>goHistory(-1);$('forward').onclick=()=>goHistory(1);$('up').onclick=()=>{const p=parentUri(active().uri);if(p)navigate(p);};$('refresh').onclick=()=>{refreshEnvironment();load(active(),false);};
  $('address').addEventListener('click',e=>{if(e.target===$('address')||e.target===$('breadcrumbs')||e.target.closest('#address-edit'))editAddress();});$('address-input').addEventListener('keydown',e=>{if(e.key==='Enter'){e.preventDefault();submitAddress();}if(e.key==='Escape')finishAddress();});$('address-input').addEventListener('blur',finishAddress);
  $('search').addEventListener('input',queueSearch);
  $('file-scroll').addEventListener('scroll',()=>{$('column-head').scrollLeft=$('file-scroll').scrollLeft;requestAnimationFrame(renderRows);});
  $('file-scroll').addEventListener('click',e=>{if(e.target===$('file-canvas')||e.target===$('file-scroll')){clearSelection();$('main').focus({preventScroll:true});}});$('file-scroll').addEventListener('contextmenu',e=>{if(e.target===$('file-scroll')||e.target===$('file-canvas')){e.preventDefault();backgroundMenu(e.clientX,e.clientY);}});
  $('cut').onclick=()=>copySelection('move');$('copy').onclick=()=>copySelection('copy');$('paste').onclick=paste;$('rename').onclick=rename;$('trash').onclick=trash;$('copy-path').onclick=copyPath;
  $('new').onclick=()=>openNewMenu();
  $('sort').onclick=()=>menuBelow('sort',[...['name','modified','type','size'].map(s=>({label:{name:'Name',modified:'Date modified',type:'Type',size:'Size'}[s],icon:state.sort===s?'check':'sort',fn:()=>{state.sort=s;state.filterCache=null;renderColumns();renderRows();}})),'-',{label:state.descending?'Descending':'Ascending',icon:state.descending?'down':'up',fn:()=>{state.descending=!state.descending;state.filterCache=null;renderColumns();renderRows();}}]);
  $('view').onclick=()=>menuBelow('view',[{label:'Details',icon:state.view==='details'?'check':'list',fn:()=>changeView('details')},{label:'Large icons',icon:state.view==='grid'?'check':'grid',fn:()=>changeView('grid')},'-',{label:'Show hidden files',icon:state.showHidden?'check':'eye',fn:toggleHidden},{label:'Details pane',icon:state.details?'check':'details',fn:toggleDetails},'-',{label:'Larger text',icon:'plus',shortcut:'Ctrl++',fn:()=>changeTextSize('increase')},{label:'Smaller text',icon:'minus',shortcut:'Ctrl+−',fn:()=>changeTextSize('decrease')},{label:'Reset text size',icon:'refresh',shortcut:'Ctrl+0',fn:()=>changeTextSize('reset')}]);
  $('more').onclick=()=>menuBelow('more',[{label:'New window',icon:'plus',shortcut:'Ctrl+N',fn:()=>call('newWindow',{uri:active().uri.startsWith('file:')||active().uri.startsWith('smb:')?active().uri:state.env.home})},{label:'Settings',icon:'settings',fn:settingsDialog},{label:'Default file explorer…',icon:'folderline',fn:()=>settingsDialog('default')},...cacheMenuItems(active().uri),{label:'Map network location',icon:'network',fn:connectDialog},{label:'Pin current folder',icon:'pin',fn:pinCurrent,disabled:['home:','pc:','network:','settings:'].includes(active().uri)},'-',{label:'Light appearance',icon:state.theme==='light'?'check':'sun',fn:()=>applyTheme('light')},{label:'Dark appearance',icon:state.theme==='dark'?'check':'moon',fn:()=>applyTheme('dark')},{label:'Use system appearance',icon:state.theme==='system'?'check':'desktop',fn:()=>applyTheme('system')},{label:'Show hidden files',icon:state.showHidden?'check':'eye',fn:toggleHidden},'-',{label:'License & source',icon:'code',fn:showLicense},{label:'About this build',icon:'info',fn:()=>showMessage('OpenXplorer 1.1.0','An independent Windows 11–inspired file manager for Zorin.\n\n'+(native?'Desktop: WebKitGTK + GIO/GVfs.':'Offline preview: sample data only.')+'\n\nStable release. Replacing existing files requires confirmation; there is no permanent-delete fallback. Cached filename/path search is opt-in. Thumbnails, undo, and cross-filesystem cut/move are not implemented. ZIP browsing is read-only; local cache changes use inotify. Network changes use incremental polling.')}]);
  $('theme-toggle').onclick=appearanceMenu;setButton('settings-button','settings');$('settings-button').onclick=settingsDialog;
  $('details-toggle').onclick=toggleDetails;$('status-list').onclick=()=>changeView('details');$('status-grid').onclick=()=>changeView('grid');$('connect-sidebar').onclick=connectDialog;$('transfer-cancel').onclick=()=>{if(state.operation){fire('cancel',{token:state.operation});$('transfer-label').textContent='Cancelling…';}};
  if(!native&&window.matchMedia){window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change',()=>{if(state.theme==='system')applyTheme('system',false);});}
  document.addEventListener('pointerdown',e=>{if(!$('menu').contains(e.target)&&!e.target.closest('#sort,#view,#more,#new,#theme-toggle'))closeMenu();});
  document.addEventListener('dragover',e=>{e.preventDefault();if(!e.target.closest('#quick-access')){if(e.dataTransfer)e.dataTransfer.dropEffect='none';clearDropFeedback();}});
  document.addEventListener('drop',e=>{e.preventDefault();if(state.drag){finishPinDrag();toast('Drop folders in the Quick access section to pin them.');}});
  document.addEventListener('pointerdown',resetTypeSelect,true);
  document.addEventListener('focusin',e=>{if(!typeSelectTarget(e.target))resetTypeSelect();});
  document.addEventListener('keydown',textSizeKeys,true);
  document.addEventListener('keydown',authKeys,true);
  document.addEventListener('keydown',onKey);
  document.addEventListener('pointermove',pointerPinMove,{passive:false});
  document.addEventListener('pointerup',pointerPinUp);document.addEventListener('pointercancel',pointerPinUp);
  window.addEventListener('blur',()=>{resetTypeSelect();finishPinDrag();});
  window.addEventListener('focus',()=>{void refreshClipboard();void refreshCacheStatus();});
  window.addEventListener('storage',e=>{if(e.key==='winspace-demo-clipboard')void refreshClipboard();});
  setInterval(()=>{if(state.ready&&document.visibilityState==='visible')void refreshCacheStatus();},5000);
  for(const name of ['click','dblclick'])document.addEventListener(name,e=>{if(state.outgoingFile||performance.now()<state.suppressClickUntil){e.preventDefault();e.stopImmediatePropagation();}},true);
  document.addEventListener('mousedown',e=>{if(e.button===3||e.button===4){e.preventDefault();if(!activeAuth&&$('modal-layer').hidden)goHistory(e.button===3?-1:1);}},true);
  document.addEventListener('mouseup',e=>{if(e.button===3||e.button===4)e.preventDefault();},true);
  document.addEventListener('auxclick',e=>{if(e.button===3||e.button===4)e.preventDefault();},true);window.addEventListener('resize',()=>{applyLayout();positionTabDialog();renderRows();updateChrome();});if(window.ResizeObserver)new ResizeObserver(()=>{renderRows();updateChrome();}).observe($('main'));
}
function onKey(e){
  // Never interpret an IME's provisional key events as file actions.
  if(e.defaultPrevented||e.isComposing||e.keyCode===229)return;
  if(activeAuth)return;
  if(state.updateInstalling){if(e.key==='Tab')$('update-status')?.focus();e.preventDefault();return;}
  if((e.ctrlKey||e.metaKey)&&e.key==='Tab'&&state.modalOwner){e.preventDefault();const i=state.tabs.findIndex(t=>t.id===state.activeId);switchTab(state.tabs[(i+(e.shiftKey?-1:1)+state.tabs.length)%state.tabs.length].id);return;}
  if(state.ready&&handleTypeSelect(e))return;
  if(!['Shift','CapsLock'].includes(e.key))resetTypeSelect();
  if((e.ctrlKey||e.metaKey)&&e.key===','){e.preventDefault();void settingsDialog();return;}
  if(e.key==='Escape'&&state.drag){finishPinDrag();e.preventDefault();return;}
  if(!state.ready)return;
  if(!$('modal-layer').hidden){if(e.key==='Escape'&&!state.operation){closeModal();e.preventDefault();}if(e.key==='Tab'){const f=[...$('modal').querySelectorAll('input,button,select')].filter(x=>!x.disabled);const i=f.indexOf(document.activeElement);if(e.shiftKey&&i===0){f.at(-1)?.focus();e.preventDefault();}else if(!e.shiftKey&&i===f.length-1){f[0]?.focus();e.preventDefault();}}if(e.key==='Enter'&&e.target.tagName==='INPUT'){$('modal').querySelector('.primary')?.click();e.preventDefault();}return;}
  if(!$('menu').hidden){const buttons=[...$('menu').querySelectorAll('button:not(:disabled)')];let i=buttons.indexOf(document.activeElement);if(['ArrowDown','ArrowUp'].includes(e.key)){buttons[(i+(e.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length]?.focus();e.preventDefault();return;}if(e.key==='Escape'){closeMenu();$('main').focus();e.preventDefault();return;}}
  if(e.altKey&&e.key==='Enter'){e.preventDefault();propertiesDialog(selected()[0]||menuEntry());return;}
  if(e.key==='ContextMenu'||(e.shiftKey&&e.key==='F10')){e.preventDefault();const r=$('file-scroll').getBoundingClientRect();if(selected().length)entryMenu(r.left+40,r.top+40,selected()[0],'win10');else backgroundMenu(r.left+40,r.top+40);return;}
  const key=e.key.toLowerCase(),ctrl=e.ctrlKey||e.metaKey;
  if((ctrl&&key==='l')||(e.altKey&&key==='d')){editAddress();e.preventDefault();return;}
  if(ctrl&&key==='f'){$('search').focus();$('search').select();e.preventDefault();return;}
  if(e.key==='F5'||(ctrl&&key==='r')){if(state.query){fire('cacheRefresh',{uri:cacheRootsFor(active().uri)[0]?.uri});void runSearch();}else load(active(),false);e.preventDefault();return;}
  const input=/INPUT|TEXTAREA|SELECT/.test(e.target.tagName)||e.target.isContentEditable;if(input)return;
  if(ctrl&&key==='h'){toggleHidden();e.preventDefault();return;}
  if(ctrl&&!e.shiftKey&&key==='n'){e.preventDefault();fire('newWindow',{uri:active().uri.startsWith('file:')||active().uri.startsWith('smb:')?active().uri:state.env.home});return;}
  if(ctrl&&key==='t'){addTab();e.preventDefault();return;}
  if(ctrl&&key==='w'){closeTab(state.activeId);e.preventDefault();return;}
  if(ctrl&&key==='tab'){const i=state.tabs.findIndex(t=>t.id===state.activeId);switchTab(state.tabs[(i+(e.shiftKey?-1:1)+state.tabs.length)%state.tabs.length].id);e.preventDefault();return;}
  if(e.altKey&&e.key==='ArrowLeft'){goHistory(-1);e.preventDefault();return;}if(e.altKey&&e.key==='ArrowRight'){goHistory(1);e.preventDefault();return;}if(e.altKey&&e.key==='ArrowUp'){$('up').click();e.preventDefault();return;}
  if(e.key==='F5'){load(active(),false);e.preventDefault();return;}
  if(active()?.uri==='settings:'||e.target.closest('[role=separator],#breadcrumbs button'))return;
  if(ctrl&&key==='a'){state.selection=new Set(filtered().map(x=>x.uri));renderRows();renderDetails();updateToolbar();updateStatus();e.preventDefault();return;}
  if(ctrl&&key==='c'){copySelection('copy');e.preventDefault();return;}if(ctrl&&key==='x'){copySelection('move');e.preventDefault();return;}if(ctrl&&key==='v'){paste();e.preventDefault();return;}
  if(ctrl&&e.shiftKey&&key==='n'){if(!$('new').disabled)newItem('folder');e.preventDefault();return;}
  if(e.key==='Delete'){trash();e.preventDefault();return;}if(e.key==='F2'){rename();e.preventDefault();return;}if(e.key==='Enter'&&typeSelectTarget(e.target)){const s=selected();if(s.length===1)openEntry(s[0]);e.preventDefault();return;}
  if(e.key==='Escape'){clearSelection();return;}
  if(['ArrowDown','ArrowUp','Home','End'].includes(e.key)){const arr=filtered();if(!arr.length)return;const current=Math.max(0,state.anchor);let next=e.key==='Home'?0:e.key==='End'?arr.length-1:Math.min(arr.length-1,Math.max(0,current+(e.key==='ArrowDown'?1:-1)));selectEntry(arr[next],next,e);revealEntry(next);e.preventDefault();}
}

/* The preview transport is intentionally in-memory. Nothing below reads files
   from the browser or makes a network request. Its mutations disappear on reload. */
const demo = (()=>{
  const entries=new Map(),root='file:///home/demo',nas='smb://studio-nas/Projects',media='smb://studio-nas/Media';
  const now=1788609600;let dt=0;
  function add(parent,name,isDir=false,size=0,type=null){const uri=uriChild(parent,name);const ext=name.split('.').pop().toLowerCase();const types={pdf:'PDF document',docx:'Word document',xlsx:'Excel worksheet',pptx:'PowerPoint presentation',png:'PNG image',jpg:'JPEG image',zip:'Compressed folder',md:'Markdown document',txt:'Text document',csv:'CSV document',mp4:'MP4 video'};const e={uri,name,isDir,size:isDir?null:size,type:type||(isDir?'File folder':types[ext]||'File'),modified:now-(++dt)*14400,hidden:name.startsWith('.')};entries.set(uri,e);return uri;}
  for(const d of ['Desktop','Downloads','Documents','Pictures','Music','Videos'])add(root,d,true);
  entries.set(root,{uri:root,name:'demo',isDir:true,size:null,type:'File folder',modified:now});
  entries.set('file:///home',{uri:'file:///home',name:'home',isDir:true,size:null,type:'File folder',modified:now});
  const docs=root+'/Documents';for(const d of ['Client projects','Invoices','Personal','Work'])add(docs,d,true);
  add(docs,'Brand guidelines.pdf',false,2516582);add(docs,'Meeting notes.docx',false,38912);add(docs,'Project budget.xlsx',false,97280);add(docs,'Q3 presentation.pptx',false,5840568);add(docs,'Read me.txt',false,1638);add(docs,'Research notes.md',false,5219);add(docs,'Site mockup.png',false,1740800);add(docs,'Summer planning.pdf',false,913408);add(docs,'Website assets.zip',false,14889779);add(docs,'.draft-notes.txt',false,432);
  add(docs+'/Client%20projects','Northwind',true);add(docs+'/Client%20projects','Project brief.docx',false,68542);add(docs+'/Invoices','Invoice-2026-09.pdf',false,81920);add(docs+'/Work','Launch checklist.txt',false,3280);
  add(root+'/Downloads','Zorin-OS-guide.pdf',false,3420160);add(root+'/Downloads','Assets',true);add(root+'/Pictures','Weekend',true);add(root+'/Pictures','Mountains.jpg',false,4521984);add(root+'/Desktop','Workspace',true);
  for(const d of ['Brand assets','Design','Deliverables','Reference'])add(nas,d,true);add(nas,'Project overview.docx',false,72704);add(nas,'Roadmap.xlsx',false,133120);add(nas,'Team handbook.pdf',false,1205862);add(nas,'Welcome.txt',false,912);add(nas+'/Design','Explorations',true);add(nas+'/Design','Dashboard.png',false,1835008);for(const d of ['Movies','Music','Photos'])add(media,d,true);
  entries.set('file:///media/demo/Archive',{uri:'file:///media/demo/Archive',name:'2 TB Volume',isDir:true,type:'Local volume'});add('file:///media/demo/Archive','Projects',true);add('file:///media/demo/Archive','Backup notes.txt',false,322);
  const quick=['Desktop','Downloads','Documents','Pictures','Music','Videos'].map((label,i)=>({label,uri:root+'/'+label,icon:label.toLowerCase(),color:['#3b8ec7','#138266','#4a94d1','#9a79cb','#c66b9c','#b48540'][i]}));
  const shares=[{label:'Studio NAS (Z:)',uri:nas,connected:true},{label:'Media (M:)',uri:media,connected:true}];let prefs={view:'details',details:true,theme:'dark',showHidden:false,autoIndex:true,contextMenu:'win10',networkInterval:60,textSize:100};const cancelled=new Set();
  // This is sample data, NOT a connection to the user's server.
  const server='smb://studio-nas/';
  entries.set(server,{uri:server,name:'studio-nas',isDir:true,kind:'directory',size:null,type:'Network server',modified:0});
  for(const share of ['Projects','Media','work','Backups','Shared documents','Shipping','scripts']){
    const uri=uriChild(server,share);entries.set(uri,{uri,name:share,isDir:true,isVirtual:true,canOperate:false,kind:'mountable',targetUri:uri,size:null,type:'Network share',modified:0});
  }
  for(const folder of ['Design','Invoices','Projects','Reference'])add('smb://studio-nas/work',folder,true);
  add('smb://studio-nas/work','Project notes.docx',false,32896);add('smb://studio-nas/work','Budget.xlsx',false,84992);
  // Entirely fictional sample hierarchy. No user names, addresses or captures.
  // These locations are served only by the simulated storage adapter.
  const sampleServer='smb://archive-nas/';
  entries.set(sampleServer,{uri:sampleServer,name:'archive-nas',isDir:true,kind:'directory',size:null,type:'Network server',modified:0});
  for(const name of ['Shared','downloads','Training media','work']){const uri=uriChild(sampleServer,name);entries.set(uri,{uri,name,isDir:true,isVirtual:true,canOperate:false,kind:'mountable',targetUri:uri,size:null,type:'Network share',modified:0});}
  const sampleShare='smb://archive-nas/Shared';
  for(const name of ['Launch planning','2026 Reports','Client archive','Incoming','Receipts'])add(sampleShare,name,true);
  add(sampleShare,'Design assets.zip',false,4299161);add(sampleShare,'Design brief.pdf',false,223400);add(sampleShare,'Product walkthrough.mp4',false,12233112);add(sampleShare,'Archive.mp4',true);add(sampleShare,'Project summary.pdf',false,225184);add(sampleShare,'Download notes.txt',false,2013);
  const launch=uriChild(sampleShare,'Launch planning');
  add(launch,'Launch year-end summary.xlsx',false,89472);add(launch,'Launch financial report.pdf',false,503214);add(launch,'Receipts',true);add(uriChild(launch,'Receipts'),'Launch December invoice.pdf',false,112849);
  add(sampleShare+'/2026%20Reports','Launch draft.pdf',false,102911);
  shares.unshift({uri:sampleShare,label:'Shared library',connected:true});
  let store={};try{store=JSON.parse(localStorage.getItem('openxplorer-demo-091')||'{}');}catch(_){}
  const demoCaches=new Map(),demoSnapshots=new Map();let demoDefault=false;
  const folderKeys=['DESKTOP','DOWNLOAD','DOCUMENTS','PICTURES','MUSIC','VIDEOS'];
  const knownFolders=['Desktop','Downloads','Documents','Pictures','Music','Videos'].map((label,i)=>({key:folderKeys[i],label,icon:label.toLowerCase(),uri:root+'/'+label,path:'/home/demo/'+label,defaultPath:'/home/demo/'+label,previousPath:null}));
  const stableMount={source:'//archive-nas/Shared',path:'/mnt/winspace/u1000s8d65114a64',fstype:'cifs'};
  const versionRoot=sampleShare+'/.snapshot';const snapshotRoots=[versionRoot];
  const historical=new Map();
  for(const label of ['2026-09-05_180000','2026-09-03_090000','2026-09-01_120000']){
    const versionUri=versionRoot+'/'+label;entries.set(versionUri,{uri:versionUri,name:label,isDir:true,type:'Snapshot folder',readOnly:true});
    for(const e of [...entries.values()].filter(e=>e.uri.startsWith(sampleShare+'/')&&!e.uri.includes('/.snapshot/'))){const old=versionUri+e.uri.slice(sampleShare.length);entries.set(old,{...e,uri:old,readOnly:true});}
    historical.set(label,versionUri);
  }
  const demoTemplates=[['text','Text document','New document.txt'],['empty','Empty file','New file'],['markdown','Markdown document','New document.md'],['csv','CSV file','New spreadsheet.csv'],['json','JSON file','New file.json'],['html','HTML document','New page.html'],['user:Blank letter.docx','Blank letter.docx','New letter.docx'],['user:Budget.xlsx','Budget.xlsx','New budget.xlsx']].map(([id,name,suggestedName])=>({id,name,suggestedName,builtin:!id.startsWith('user:')}));

  function sampleIndex(uri){const r=demoCaches.get(uri);if(!r||!r.enabled)return;const snapshot=[...entries.values()].filter(e=>e.uri.startsWith(uri.replace(/\/$/,'')+'/')&&!e.hidden&&!e.isVirtual&&!e.uri.includes('/.snapshot/')).map(e=>({...e,parentUri:parentUri(e.uri),cached:true,path:displayUri(e.uri),cacheStatus:'Ready'}));demoSnapshots.set(uri,snapshot);r.status='Ready';r.count=snapshot.length;r.updated=Date.now()/1000;}
  for(const uri of [docs,sampleShare]){demoCaches.set(uri,{uri,label:baseName(uri),enabled:1,status:'Ready',count:0});sampleIndex(uri);}
  function cacheSnapshot(){return{roots:[...demoCaches.values()].map(r=>({...r,update_mode:r.uri.startsWith('smb:')?'Incremental network checks (demo)':'Live local events (demo)',watch_count:r.uri.startsWith('file:')?3:0})),count:[...demoCaches.values()].filter(r=>r.enabled).reduce((n,r)=>n+r.count,0),engine:'Simulated index (native build uses SQLite)',metadataOnly:true};}

  if(['win10','win11'].includes(store.preferences?.contextMenu))prefs.contextMenu=store.preferences.contextMenu;
  if(['light','dark','system'].includes(store.preferences?.theme))prefs.theme=store.preferences.theme;
  if(store.hiddenQuick)for(let i=quick.length-1;i>=0;i--)if(store.hiddenQuick.includes(quick[i].uri))quick.splice(i,1);
  for(const pin of (Array.isArray(store.pins)?store.pins:[]).slice(0,200)){
    try{const uri=normaliseAddress(pin.uri);if(!quick.some(p=>sameLocation(p.uri,uri)))quick.push({uri,label:String(pin.label||baseName(uri)).slice(0,120)});}catch(_){}
  }
  if(Array.isArray(store.order)){const rank=new Map(store.order.map((u,i)=>[u,i]));quick.sort((a,b)=>(rank.get(a.uri)??1000)-(rank.get(b.uri)??1000));}
  if(Number.isFinite(store.preferences?.sidebarWidth)&&store.preferences.sidebarWidth>=140&&store.preferences.sidebarWidth<=560)prefs.sidebarWidth=store.preferences.sidebarWidth;
  if(store.preferences?.columnWidths&&typeof store.preferences.columnWidths==='object'){prefs.columnWidths={};for(const [key,range]of Object.entries({name:[140,1600],modified:[100,1000],parentUri:[140,1600],type:[80,1000],size:[70,600]})){const v=store.preferences.columnWidths[key];if(Number.isFinite(v)&&v>=range[0]&&v<=range[1])prefs.columnWidths[key]=Math.round(v);}}
  prefs.textSize=window.OpenXplorerTextSize.normalize(store.preferences?.textSize);
  function savePreview(){try{localStorage.setItem('openxplorer-demo-091',JSON.stringify({...store,preferences:{theme:prefs.theme,autoIndex:prefs.autoIndex,contextMenu:prefs.contextMenu,sidebarWidth:prefs.sidebarWidth,columnWidths:prefs.columnWidths,networkInterval:prefs.networkInterval,textSize:prefs.textSize},pins:quick.filter(p=>!p.icon),order:quick.map(p=>p.uri)}));}catch(_){} }
  function immediateChildren(uri){const clean=uri.replace(/\/$/,'');return [...entries.values()].filter(e=>parentUri(e.uri)?.replace(/\/$/,'')===clean);}
  function ensureDir(uri){if(uri==='file:///'||uri==='file:///home'||uri===root||uri===nas||uri===media||shares.some(s=>s.uri===uri))return;if(!entries.get(uri)?.isDir)throw Error('This location does not exist in the preview. Try Documents or the sample NAS.');}
  function exists(uri){return entries.has(uri);}
  return{async call(method,a){await new Promise(r=>setTimeout(r,method==='list'?100:5));switch(method){
    case'updateCheck':return{currentVersion:UI_RELEASE,version:UI_RELEASE,available:false,notes:'',releaseUrl:'',canInstall:false};
    case'updateInstall':case'updateRestart':throw Error('Updates are available only in the installed desktop application.');
    case'authReply':{a.password='';return true;}
    case'cacheStatus':return cacheSnapshot();
    case'cacheSet':{const uri=normaliseAddress(a.uri);if(isSmbServer(uri))throw Error('Open a share first. Cache a folder, not a server.');ensureDir(uri);demoCaches.set(uri,{uri,label:a.label||baseName(uri),enabled:a.enabled?1:0,status:a.enabled?'Ready':'Disabled',count:0});if(a.enabled)sampleIndex(uri);else demoSnapshots.delete(uri);return cacheSnapshot();}

    case'folderSize':{ensureDir(a.uri);let bytes=0,files=0,folders=0;for(const e of entries.values()){if(e.uri.startsWith(a.uri.replace(/\/$/,'')+'/')){if(e.isDir)folders++;else{files++;bytes+=e.size||0;}}}const result={uri:a.uri,bytes,files,folders,entries:files+folders,skipped:0,errors:0,status:'complete',metric:'logical file bytes',updated:Date.now()/1000};window.__nativeEvent('folderSizeProgress',{...result,token:a.token});return result;}
    case'properties':{const e=entries.get(a.uri)||{uri:a.uri,name:baseName(a.uri),isDir:true,type:'File folder',modified:now};return{...e,parentUri:parentUri(a.uri),created:now-20*86400,accessed:now,canRead:true,canWrite:!readonlyLocation(a.uri),canExecute:!!e.isDir,owner:'demo',group:'demo',mode:e.isDir?'0o755':'0o644',defaultApp:e.isDir?null:e.name.endsWith('.pdf')?'Document Viewer':'LibreOffice Writer',contentType:e.name?.endsWith('.pdf')?'application/pdf':'application/octet-stream'};}
    case'applications':return{contentType:'application/octet-stream',apps:[{id:'writer.desktop',name:'LibreOffice Writer',default:true,recommended:true,available:true},{id:'gedit.desktop',name:'Text Editor',recommended:true,available:true},...(a.allApps?[{id:'code.desktop',name:'Visual Studio Code',available:true},{id:'viewer.desktop',name:'Document Viewer',available:true}]:[])]};
    case'openTerminal':{
      const uri=normaliseAddress(a.uri),entry=entries.get(uri);
      if(!/^(file|smb):/.test(uri)||isSmbServer(uri)||readonlyLocation(uri))throw Error('Open a regular folder or mounted share first.');
      if(!exists(uri))throw Error('The sample location is no longer here.');
      const target=entry?.isDir===false?uri.slice(0,uri.lastIndexOf('/')):uri;
      return{opened:false,preview:true,terminal:'Sample terminal',uri:target,path:displayUri(target),localShell:true};}
    case'openWith':if(!['writer.desktop','gedit.desktop','code.desktop','viewer.desktop'].includes(a.appId))throw Error('Unknown sample application.');return{launched:false,preview:true};
    case'templates':return{templates:demoTemplates,directory:'/home/demo/Templates'};
    case'createTemplate':{ensureDir(a.uri);validateName(a.name);if(!demoTemplates.some(t=>t.id===a.template))throw Error('Template not available.');if(exists(uriChild(a.uri,a.name)))throw Error('An item with that name already exists. Nothing was overwritten.');return{uri:add(a.uri,a.name,false,a.template==='json'?3:0)};}
    case'folderLocations':return{folders:knownFolders.map(f=>({...f})),mounts:[stableMount]};
    case'locationCheck':{const uri=normaliseAddress(a.value);let path=displayUri(uri);const f=knownFolders.find(f=>f.key===a.key);if(!f)throw Error('Unknown standard folder.');if(uri.startsWith('smb:')){if(!uri.startsWith(sampleShare))throw Error('No persistent mount for this preview share.');path=stableMount.path+decodeURIComponent(uri.slice(sampleShare.length));}if(path.startsWith('/run/')||path==='/home/demo'||path==='/')throw Error('Use a dedicated, persistent folder.');return{key:a.key,path,uri:'file://'+path,previous:f.path,network:path.startsWith(stableMount.path)};}
    case'locationApply':{if(!a.confirmed)throw Error('Confirmation is required.');const f=knownFolders.find(f=>f.key===a.key);if(!f)throw Error('Unknown standard folder.');const previous=f.path,old=f.uri;f.previousPath=previous;f.path=a.value;f.uri='file://'+a.value;const q=quick.find(p=>p.uri===old&&p.label===f.label);if(q)q.uri=f.uri;entries.set(f.uri,{uri:f.uri,name:f.label,isDir:true,type:'File folder',modified:now});return{path:f.path,uri:f.uri,previous,changed:true,filesMoved:false};}
    case'mountPlan':{const uri=normaliseAddress(a.value);if(!uri.startsWith('smb:')||new URL(uri).pathname.split('/').filter(Boolean).length<1)throw Error('Enter a server and share.');const u=new URL(uri),share='//'+u.host+'/'+decodeURIComponent(u.pathname.split('/')[1]);return{command:'sudo /usr/bin/openxplorer-mount-share --share '+JSON.stringify(share),mountpoint:stableMount.path,targetPath:stableMount.path,removeCommand:'sudo /usr/bin/openxplorer-mount-share --share '+JSON.stringify(share)+' --remove'};}
    case'previousVersions':{const versions=[];if((a.uri===sampleShare||a.uri.startsWith(sampleShare+'/'))&&!a.uri.includes('/.snapshot/'))for(const [label,rootUri]of historical){const uri=rootUri+a.uri.slice(sampleShare.length);const e=entries.get(uri);if(e)versions.push({...e,label,snapshotRoot:rootUri,source:versionRoot});}return{versions,sources:versions.length?[versionRoot]:[],warnings:[],provider:'Simulated snapshots',message:'No snapshots in this preview folder. Try a folder in Shared library on the sample server.',protocolEnumeration:false};}
    case'snapshotSource':{const root=normaliseAddress(a.snapshots);if(!a.remove&&!snapshotRoots.includes(root))snapshotRoots.push(root);return[];}
    case'activateItem':{const entry=entries.get(a.uri)||entries.get(a.uri.replace(/\/$/,''));if(!entry)throw Error('The preview has no item at this address.');return{action:entry.isDir?'directory':entry.name.toLowerCase().endsWith('.zip')?'archive':'opened',uri:entry.uri,entry:{...entry}};}
    case'clipboardSet':{const value={...a,token:'demo-'+Date.now()};localStorage.setItem('winspace-demo-clipboard',JSON.stringify(value));return value;}
    case'clipboardGet':return JSON.parse(localStorage.getItem('winspace-demo-clipboard')||'null');
    case'clipboardConsume':{let value=JSON.parse(localStorage.getItem('winspace-demo-clipboard')||'null');if(value?.token===a.token){value.uris=value.uris.filter(u=>!a.done.includes(u));if(!value.uris.length)value=null;localStorage.setItem('winspace-demo-clipboard',JSON.stringify(value));}return value;}
    case'detachTab':return{ready:false,preview:true};
    case'handoffReady':case'windowMetadata':return true;
    case'windows':return[{id:1,title:titleFor(active()?.uri||root)+' — OpenXplorer',tabs:state.tabs.map(t=>titleFor(t.uri)),active:true}];
    case'focusWindow':return true;
    case'revealEnable':store.reveal=true;return{revealEnabled:true,revealOwned:true};
    case'revealDisable':store.reveal=false;return{revealEnabled:false,revealOwned:false};
    case'zipDefault':store.zipDefault=true;return demo.call('desktopStatus',{});
    case'zipRestore':store.zipDefault=false;return demo.call('desktopStatus',{});
    case'moveTabToWindow':return{pending:false,preview:true};
    case'tabTransferReady':return{committed:false,preview:true};
    case'revealTest':return{tested:true};
    case'braveStatus':return{running:false,profiles:[{id:'Brave-Browser:Default',name:'Personal',flavor:'Brave-Browser',downloadPath:'/home/demo/Downloads'}],sandboxed:[]};
    case'braveSync':if(!a.confirmed)throw Error('Confirmation required.');return{updated:a.profiles,errors:[],path:a.path};
    case'braveRestore':if(!a.confirmed)throw Error('Confirmation required.');return{restored:['download','savefile']};
    case'quit':toast('Preview only — no desktop windows were closed.');return true;
    case'newWindow':toast('Desktop: opens a separate window. For this preview, open the HTML file in another tab.');return true;
    case'archiveList':{const rows=a.prefix?[{name:'Statement.pdf',member:a.prefix+'Statement.pdf',isDir:false,kind:'file',size:223400,canOpen:true}]:[{name:'Documents',member:'Documents/',isDir:true,kind:'directory',size:null},{name:'Read me.txt',member:'Read me.txt',isDir:false,kind:'file',size:512,canOpen:true}];return{entries:rows,prefix:a.prefix||'',contentsExtracted:false,readOnly:true};}
    case'archiveOpenMember':return{action:'opened',temporary:true};
    case'archiveInspect':return{uri:a.uri,files:2,folders:1,bytes:223912,entries:3};
    case'archiveExtract':{ensureDir(a.target);validateName(a.name);const uri=uriChild(a.target,a.name);if(exists(uri))throw Error('The destination already exists. Choose a new folder name.');
      window.__nativeEvent('transfer',{token:a.token,label:'Extracting sample files…',fraction:.4});await new Promise(r=>setTimeout(r,220));
      if(cancelled.has(a.token)){cancelled.delete(a.token);throw Error('Extraction cancelled. No output folder was created.');}
      add(a.target,a.name,true);add(uri,'Documents',true);add(uri+'/Documents','Statement.pdf',false,223400);add(uri,'Read me.txt',false,512);
      return{uri,name:a.name,source:a.uri,target:a.target,files:2,folders:1,bytes:223912,simulated:true};}

    case'cacheRefresh':{for(const [uri,r]of demoCaches)if(r.enabled&&(!a.uri||sameLocation(uri,a.uri)))sampleIndex(uri);return cacheSnapshot();}
    case'cacheClear':{demoSnapshots.delete(a.uri);const r=demoCaches.get(a.uri);if(r){r.count=0;r.status='Not indexed';r.updated=0;}return cacheSnapshot();}
    case'cacheRemove':{demoSnapshots.delete(a.uri);demoCaches.delete(a.uri);return cacheSnapshot();}
    case'cacheStop':return cacheSnapshot();
    case'search':{const terms=a.query.toLocaleLowerCase().trim().split(/\s+/);const all=new Map();for(const [uri,list]of demoSnapshots){if(!demoCaches.get(uri)?.enabled)continue;for(const e of list){const text=(e.name+' '+displayUri(e.parentUri)).toLocaleLowerCase();if((!a.scope||sameLocation(e.uri,a.scope)||e.uri.startsWith(a.scope.replace(/\/$/,'')+'/'))&&terms.every(t=>text.includes(t)))all.set(e.uri,{...e});}}const matches=[...all.values()];return{entries:matches.slice(0,a.limit||500),truncated:matches.length>(a.limit||500),source:'cache',elapsedMs:0};}
    case'discover':return{servers:[{uri:sampleServer,label:'archive-nas',host:'archive-nas'},{uri:server,label:'Studio NAS',host:'studio-nas'}],warnings:[]};
    case'signOut':{const host=new URL(a.uri).hostname;window.__nativeEvent('serverSigningOut',{host});for(const share of shares)if(new URL(share.uri).hostname===host)share.connected=false;if(a.clearCache)for(const [uri,r]of demoCaches)if(new URL(uri).hostname===host){demoSnapshots.delete(uri);r.count=0;r.status='Not indexed';}return{host,disconnected:1,credentialsRemoved:!!a.forget};}
    case'desktopDefault':if(a.zip)store.zipDefault=true;store.reveal=!!a.reveal;demoDefault=true;return{isDefault:true,allDefault:true,revealEnabled:!!store.reveal,revealOwned:!!store.reveal,canRestore:true,current:{'inode/directory':'io.winspace.Development.desktop'}};
    case'desktopRestore':store.reveal=false;demoDefault=false;return{isDefault:false,allDefault:false,revealEnabled:false,revealOwned:false,canRestore:false,current:{'inode/directory':'org.gnome.Nautilus.desktop'}};
    case'desktopStatus':return{zipDefault:!!store.zipDefault,canRestoreZip:!!store.zipDefault,isDefault:demoDefault,allDefault:demoDefault,revealEnabled:!!store.reveal,revealOwned:!!store.reveal,canRestore:demoDefault,current:{'application/zip':store.zipDefault?'io.winspace.Development.desktop':'org.gnome.FileRoller.desktop','x-scheme-handler/smb':demoDefault?'io.winspace.Development.desktop':'org.gnome.Nautilus.desktop','inode/directory':demoDefault?'io.winspace.Development.desktop':'org.gnome.Nautilus.desktop'}};
    case'environment':return{knownFolders:knownFolders.map(f=>({...f})),stableMounts:[stableMount],snapshotRoots:[...snapshotRoots],home:root,quick:[...quick],shares:[...shares],mounts:[{label:'2 TB Volume',uri:'file:///media/demo/Archive',mounted:true,total:2000000000000,free:1100000000000}],recent:[...entries.values()].filter(e=>!e.isDir&&e.uri.startsWith(docs)).slice(0,4),preferences:{...prefs},systemDark:!!window.matchMedia?.('(prefers-color-scheme: dark)').matches};
    case'transferConflicts':ensureDir(a.target);return{conflicts:a.uris.filter(uri=>exists(uriChild(a.target,baseName(uri))))};
    case'normalise':return{uri:normaliseAddress(a.value,a.base)};
    case'list':ensureDir(a.uri);return{uri:a.uri,entries:immediateChildren(a.uri).filter(e=>a.showHidden||!e.hidden).map(e=>({...e}))};
    case'preferences':prefs={...prefs,...a};savePreview();return true;
    case'open':if(!exists(a.uri))throw Error('The sample file is no longer here.');return true;
    case'clipboardText':if(navigator.clipboard?.writeText)try{await navigator.clipboard.writeText(a.text);}catch{throw Error('Browser clipboard unavailable.');}else throw Error('Browser clipboard unavailable.');return true;
    case'create':ensureDir(a.uri);validateName(a.name);if(exists(uriChild(a.uri,a.name)))throw Error('An item with that name already exists.');add(a.uri,a.name,a.kind==='folder',0);return true;
    case'rename':{validateName(a.name);const src=entries.get(a.uri);if(!src)throw Error('Item no longer exists.');const dst=uriChild(parentUri(a.uri),a.name);if(exists(dst))throw Error('An item with that name already exists.');const all=[...entries.values()].filter(e=>e.uri===a.uri||e.uri.startsWith(a.uri+'/'));for(const e of all){entries.delete(e.uri);const newUri=dst+e.uri.slice(a.uri.length);entries.set(newUri,{...e,uri:newUri,name:e.uri===a.uri?a.name:e.name});}return true;}
    case'pin':{
      if(!Array.isArray(a.items)||a.items.length<1||a.items.length>200)throw Error('Drag 1–200 folders.');
      const clean=[];
      for(const item of a.items){const uri=normaliseAddress(item.uri).replace(/\/$/,'');const existing=quick.find(p=>sameLocation(p.uri,uri));const entry=entries.get(uri)||entries.get(uri+'/');if(!existing&&!entry?.isDir)throw Error('Only folders and network shares can be pinned.');const target=entry?.targetUri||uri;if(!clean.some(p=>sameLocation(p.uri,target)))clean.push({uri:target,label:String(item.label||baseName(target)),...(existing?.icon?{icon:existing.icon,color:existing.color}:{})});}
      const moved=new Set(clean.map(p=>p.uri));
      if(!moved.has(a.before)){const kept=quick.filter(p=>![...moved].some(u=>sameLocation(u,p.uri)));let pos=kept.findIndex(p=>sameLocation(p.uri,a.before));if(pos<0)pos=kept.length;kept.splice(pos,0,...clean);quick.splice(0,quick.length,...kept);}
      store.hiddenQuick=(store.hiddenQuick||[]).filter(u=>!moved.has(u));savePreview();return{pins:clean};
    }
    case'bookmark':{const list=a.kind==='share'?shares:quick;const i=list.findIndex(p=>p.uri===a.uri);if(a.action==='remove'){if(i>=0)list.splice(i,1);if(a.kind==='pin')store.hiddenQuick=[...(store.hiddenQuick||[]),a.uri];}else if(i<0)list.push({uri:a.uri,label:a.label||baseName(a.uri)});savePreview();return true;}
    case'connect':{let uri=normaliseAddress(a.address,'network:');if(!uri.startsWith('smb://')||new URL(uri).pathname.split('/').filter(Boolean).length<1)throw Error('Enter a share, for example \\\\nas\\Projects.');uri=uri.replace(/\/$/,'');if(!shares.some(s=>s.uri===uri)){const originals=immediateChildren(nas);for(const e of originals)entries.set(uri+e.uri.slice(nas.length),{...e,uri:uri+e.uri.slice(nas.length)});if(a.remember)shares.push({uri,label:a.label||baseName(uri),connected:true});else entries.set(uri,{uri,name:baseName(uri),isDir:true,type:'File folder',modified:now});}return{uri};}
    case'trashSupport':return{canTrash:!String(a.uri||'').startsWith('smb://')};
    case'unmount':{const s=shares.find(s=>s.uri===a.uri);if(s)s.connected=false;return true;}
    case'mountVolume':return{uri:root};
    case'cancel':cancelled.add(a.token);return true;
    case'operate':{const done=[],errors=[],skipped=[];for(let i=0;i<a.uris.length;i++){if(cancelled.has(a.token))break;const uri=a.uris[i],src=entries.get(uri);if(!src){errors.push(baseName(uri)+': source no longer exists.');continue;}
      if(a.mode==='trash'||a.mode==='delete'){for(const key of [...entries.keys()])if(key===uri||key.startsWith(uri+'/'))entries.delete(key);done.push(uri);}
      else{ensureDir(a.target);if(a.target===uri||a.target.startsWith(uri+'/')){errors.push(src.name+': cannot place a folder inside itself.');continue;}let name=src.name,dst=uriChild(a.target,name);if(exists(dst)){if(a.policy==='skip'){skipped.push(uri);continue;}if(a.policy==='replace'&&entries.get(dst)?.isDir!==src.isDir){errors.push(src.name+': a file and folder have the same name.');continue;}if(a.policy==='keep-both'){let count=2;const dot=!src.isDir&&name.lastIndexOf('.')>0?name.lastIndexOf('.'):-1;const stem=dot>0?name.slice(0,dot):name,ext=dot>0?name.slice(dot):'';while(exists(dst)){name=stem+' (copy '+(count++)+')'+ext;dst=uriChild(a.target,name);}}}
      const all=[...entries.values()].filter(e=>e.uri===uri||e.uri.startsWith(uri+'/'));for(const e of all){const dest=dst+e.uri.slice(uri.length);entries.set(dest,{...e,uri:dest,name:e.uri===uri?name:e.name});}if(a.mode==='move')for(const e of all)entries.delete(e.uri);done.push(uri);}
      window.__nativeEvent('transfer',{token:a.token,label:(a.mode==='copy'?'Copying ':a.mode==='move'?'Moving ':a.mode==='delete'?'Deleting ':'Trashing ')+src.name,fraction:(i+1)/a.uris.length});await new Promise(r=>setTimeout(r,120));}return{done,errors,skipped,cancelled:cancelled.has(a.token)};}
    case'window':case'chrome':case'uiReady':return true;
    default:throw Error('This action is not available in the preview.');
  }}};
})();

async function start(){
  setup();applyTheme(state.theme,false);
  try{
    state.env=await call('environment');if(!native)state.env.editors=[{id:'code.desktop',name:'Visual Studio Code'}];if(state.env.warning)toast(state.env.warning);
    if(native&&state.env.version!==UI_RELEASE){
      state.hostMismatch=true;
      throw Error(`The interface is ${UI_RELEASE}, but the running application is ${state.env.version||'an older version'}. Finish active operations, then run openxplorer --restart. Closing one window is not enough when the background service is running.`);
    }
    const p=state.env.preferences||{};state.details=p.details!==false;state.view=p.view==='grid'?'grid':'details';state.showHidden=!!p.showHidden;
    applyTheme(p.theme||'system',false);applyLayout();state.ready=true;
    await refreshCacheStatus();
    addTab(state.env.startUri||state.env.home);
    let notified=false;
    const ready=()=>{if(notified)return;notified=true;renderRows();updateChrome();fire('uiReady',{theme:state.theme});};
    requestAnimationFrame(()=>requestAnimationFrame(ready));
    // GTK rendering trouble must not leave the native loading overlay up just
    // because rAF is delayed; the host issues a bounded redraw as well.
    setTimeout(ready,350);
  }catch(e){
    $('folder-view').hidden=true;$('details').hidden=true;$('empty-state').hidden=false;
    const box=$('empty-state');box.replaceChildren(icon('info'),elem('h3','','Could not start OpenXplorer'),elem('p','',e.message));
    box.append(button(state.hostMismatch?'Quit old process safely':'Retry',()=>state.hostMismatch?fire('quit'):window.location.reload()));
    if(native)fire('uiReady',{failed:true});
  }
}
/* 0.4: compact context menus, properties, XDG locations, templates, versions. */
function readonlyLocation(uri) {
  if (!uri) return false;
  let parts=[]; try { parts=decodeURIComponent(locationParts(uri).pathname).split('/'); } catch {}
  return parts.some(p=>['.snapshot','.snapshots','#snapshot'].includes(p)||p.startsWith('@GMT-')) ||
    parts.some((p,i)=>p==='.zfs'&&parts[i+1]==='snapshot') ||
    (state.env?.snapshotRoots||[]).some(root=>sameLocation(uri,root)||uri.startsWith(root.replace(/\/$/,'')+'/'));
}
function snapshotFor(tab){
  if(!tab?.uri)return null;
  if(tab.snapshot?.root&&window.OpenXplorerSnapshots.within(tab.uri,tab.snapshot.root))return tab.snapshot;
  return window.OpenXplorerSnapshots.location(tab.uri,state.env?.snapshotRoots||[]);
}
function renderSnapshotBanner(tab){
  const banner=$('snapshot-banner'),info=snapshotFor(tab);if(!banner)return;
  banner.hidden=!info;if(!info){banner.replaceChildren();return;}
  const metadata=window.OpenXplorerSnapshots.describe({label:info.label});
  const label=elem('strong','','Previous version');const date=elem('span','',metadata.iso?metadata.date+(metadata.time?' · '+metadata.time:''):info.label);
  date.title=metadata.explanation;banner.replaceChildren(icon('clock',16),label,date,elem('span','snapshot-safety','Read-only in OpenXplorer · Restore a copy to edit'));
}
function contextStyle(){return state.env?.preferences?.contextMenu==='win11'?'win11':'win10';}
function openMenu(x,y,items,options={}) {
  resetTypeSelect();
  const menu=$('menu'),style=options.style||contextStyle();
  menu.replaceChildren(); menu.className='menu '+style;
  menu.setAttribute('aria-label',style==='win10'?'Windows 10 style menu':'Windows 11 style menu');
  const addItem=it=>{
    if(it==='-'){menu.append(elem('div','menu-divider'));return;}
    const b=button('',async()=>{closeMenu();await it.fn?.();});
    b.setAttribute('role','menuitem');b.setAttribute('aria-label',it.label);b.disabled=!!it.disabled;
    b.append(icon(it.icon||'info'),elem('span','menu-label',it.label));
    if(it.shortcut)b.append(elem('span','shortcut',it.shortcut));
    b.title=it.label;menu.append(b);
  };
  if(options.strip?.length&&style==='win11'){
    const strip=elem('div','context-strip');strip.setAttribute('role','group');strip.setAttribute('aria-label','File actions');
    for(const it of options.strip){const b=button('',async()=>{closeMenu();await it.fn?.();},'',it.icon);b.title=it.label;b.setAttribute('aria-label',it.label);b.disabled=!!it.disabled;strip.append(b);}
    menu.append(strip,elem('div','menu-divider'));
  }
  for(const item of items)addItem(item);
  // Explicit width and max-width avoid content-driven expansion and stale
  // left coordinates making a context menu as wide as the available window.
  menu.style.left='0px';menu.style.top='0px';menu.hidden=false;
  menu.style.left=Math.max(6,Math.min(x,innerWidth-menu.offsetWidth-6))+'px';
  menu.style.top=Math.max(6,Math.min(y,innerHeight-menu.offsetHeight-6))+'px';
  menu.querySelector('button:not(:disabled)')?.focus();
}
function terminalMenuItem(entry,oneOnly=false){
  const uri=entry?.targetUri||entry?.uri;
  const allowed=!!uri&&/^(file|smb):/.test(uri)&&!isSmbServer(uri)&&!entry?.archiveMember&&!entry?.archiveUri&&!entry?.readOnly&&!readonlyLocation(uri)&&(!oneOnly||state.selection.size<=1);
  return {label:entry?.isDir===false?'Open containing folder in Terminal':'Open in Terminal',icon:'terminal',disabled:!allowed,
    fn:async()=>{const result=await call('openTerminal',{uri});toast(native?`Opened ${result.terminal} in ${result.path}`:'Preview only: Terminal would open here. No program was launched.');}};
}
function menuEntry(e){return e||{uri:active().uri,name:titleFor(active().uri),isDir:true};}
// 1.0.0: intentional extraction is separate from read-only ZIP browsing.
function zipOutputName(entry){return (entry.name||'Archive.zip').replace(/\.zip$/i,'').replace(/[ .]+$/,'')||'Extracted files';}
async function extractDialog(entry){
  if(state.operation){toast('Finish the current file operation before extracting.');return;}
  const owner=active()?.id,source=entry.uri;
  const sourceParent=parentUri(source);
  const defaultParent=writableLocation(sourceParent)?sourceParent:(state.env.quick.find(p=>p.icon==='downloads')?.uri||state.env.home);
  const inspectToken='zip-check-'+(++seq);let disposed=false,summary;
  const answer=await showModal('Extract compressed folder','The ZIP is kept unchanged. Files are unpacked into a new folder; existing files are never replaced.',body=>{
    const heading=elem('div','extract-source');heading.append(zipFolderIcon(40),elem('strong','',entry.name));body.append(heading);
    const target=textField(body,'Destination folder',displayUri(defaultParent),'/home/you/Downloads or \\\\nas\\share');target.id='extract-parent';target.previousElementSibling.htmlFor=target.id;
    const name=textField(body,'New folder name',zipOutputName(entry));name.id='extract-name';name.previousElementSibling.htmlFor=name.id;
    const output=elem('div','extract-target');output.setAttribute('role','status');const update=()=>{output.textContent='Extract into: '+target.value.replace(/[\\/]+$/,'')+(target.value.startsWith('\\')?'\\':'/')+name.value;};target.oninput=update;name.oninput=update;update();body.append(output);
    summary=elem('p','extract-summary','Checking archive contents…');summary.setAttribute('role','status');body.append(summary);
    const label=elem('label','checkbox-row');const show=elem('input');show.type='checkbox';show.checked=true;show.id='extract-open';label.append(show,document.createTextNode('Show extracted files when finished'));body.append(label);
    body.append(elem('p','hint','For SMB, open and sign in to the source and destination shares first. Password-protected ZIPs need an external archive manager.'));
    const data={target,name,show,ready:false,onCancel:()=>{disposed=true;fire('cancel',{token:inspectToken});}};
    void call('archiveInspect',{uri:source,token:inspectToken}).then(r=>{if(disposed||!summary.isConnected)return;if(!r||!['files','folders','bytes','entries'].every(k=>Number.isSafeInteger(r[k])&&r[k]>=0))throw Error('The ZIP check returned an incomplete response. Restart OpenXplorer after upgrading, then try again.');data.ready=true;summary.textContent=`${r.files.toLocaleString()} ${r.files===1?'file':'files'} · ${r.folders.toLocaleString()} ${r.folders===1?'folder':'folders'} · ${prettyBytes(r.bytes)} unpacked`;}).catch(e=>{data.ready=false;if(!disposed&&summary.isConnected){summary.textContent=e.message;summary.classList.add('error');}});
    return data;
  },[{label:'Open in archive manager',fn:async f=>{f.onCancel();await call('open',{uri:source});return{external:true};}},{label:'Cancel',cancel:true},{label:'Extract',className:'primary',fn:async f=>{
    if(!f.ready)throw Error('Wait for the ZIP check to finish. Unsupported archives need an external archive manager.');
    const name=validateName(f.name.value);const normalized=await call('normalise',{value:f.target.value,base:defaultParent});
    if(!writableLocation(normalized.uri))throw Error('Choose a writable folder outside Previous versions, not a server listing.');
    return{target:normalized.uri,name,show:f.show.checked};
  }}]);
  disposed=true;if(!answer||answer.external)return;
  const token='extract-'+(++seq);state.operation=token;updateToolbar();$('transfer').hidden=false;updateTransfer({token,label:'Preparing extraction…',fraction:0});
  let result=null,failure=null;
  try{result=await call('archiveExtract',{uri:source,target:answer.target,name:answer.name,token});}
  catch(e){failure=e;}
  finally{state.operation=null;$('transfer').hidden=true;updateToolbar();}
  if(failure){await showMessage('Extraction stopped',failure.message+'\n\nThe ZIP is unchanged. Existing files were not overwritten.');return;}
  toast(result.simulated?'Preview only — sample files extracted. Nothing was written to your computer.':`Extracted ${result.files} files into ${result.name}.`);
  const origin=state.tabs.find(t=>t.id===owner);
  if(answer.show){if(origin&&state.activeId===owner)await navigate(result.uri);else addTab(result.uri);}
  else{for(const tab of state.tabs)if(sameLocation(tab.uri,answer.target))await load(tab,false);}
}

function textMetrics(){return window.OpenXplorerTextSize.metrics(state.env?.preferences?.textSize);}
function applyTextSize(value,render=true){
  value=window.OpenXplorerTextSize.normalize(value);if(state.env)state.env.preferences.textSize=value;
  const metrics=window.OpenXplorerTextSize.metrics(value);document.documentElement.style.setProperty('--text-scale',String(metrics.scale));document.documentElement.dataset.textSize=String(value);
  const select=$('text-size-select');if(select)select.value=String(value);
  if(render&&state.ready){renderRows();positionTabDialog();updateChrome();}
}
let textSaveQueue=Promise.resolve();
function changeTextSize(action){
  if(!state.ready)return;const api=window.OpenXplorerTextSize,current=api.normalize(state.env.preferences.textSize);
  const value=typeof action==='number'?api.normalize(action):action==='reset'?100:api.step(current,action==='increase'?1:-1);
  applyTextSize(value);toast('Text size: '+value+'%');
  // Serialize saves so rapidly repeated keys cannot persist an earlier size last.
  textSaveQueue=textSaveQueue.then(()=>call('preferences',{textSize:value})).catch(e=>toast('Text size changed for this window, but could not be saved: '+e.message));
  return textSaveQueue;
}
function textSizeKeys(e){const action=window.OpenXplorerTextSize.action(e);if(!action||!state.ready||e.defaultPrevented)return;e.preventDefault();e.stopImmediatePropagation();resetTypeSelect();void changeTextSize(action);}
function textSizeControls(parent){
  const row=elem('div','settings-line');row.id='text-size-line';const text=elem('div');text.append(elem('strong','','Text size'),elem('p','','Ctrl + makes text larger, Ctrl − smaller, and Ctrl 0 resets it. Saved for all windows; desktop scaling is unchanged.'));
  const select=elem('select');select.id='text-size-select';select.setAttribute('aria-label','Text size');for(const value of window.OpenXplorerTextSize.levels){const option=elem('option','',value+'%'+(value===100?' (default)':''));option.value=value;select.append(option);}select.value=window.OpenXplorerTextSize.normalize(state.env.preferences.textSize);select.onchange=()=>changeTextSize(Number(select.value));row.append(text,select);parent.append(row);
}

function entryMenu(x,y,e,forceStyle) {
  const one=state.selection.size<=1,mutable=canOperate(e)&&!readonlyLocation(e.uri);
  const copyOK=selected().length>0&&selected().every(e=>canOperate(e));
  const basic=[{label:'Open',icon:'folderline',fn:()=>openEntry(e),shortcut:'Enter',disabled:!one||(!e.isDir&&readonlyLocation(e.uri))}];
  if(isZipEntry(e))basic.push({label:'Extract all…',icon:'zip',fn:()=>extractDialog(e),disabled:!one||!!state.operation});
  basic.push(terminalMenuItem(e,true));
  basic.push({label:e.isDir?'Open folder with…':'Open with…',icon:'grid',fn:()=>openWithDialog(e),disabled:!one||readonlyLocation(e.uri)});
  for(const editor of uniqueEditors(state.env.editors||[]))basic.push({label:'Open in '+editor.name,icon:'documents',fn:()=>call('openWith',{uri:e.uri,appId:editor.id}).then(r=>toast(r.warning||'Opened with '+editor.name)),disabled:!one||readonlyLocation(e.uri)});
  if(e.isDir)basic.push({label:'Open in new tab',icon:'plus',fn:()=>addTab(e.targetUri||e.uri),disabled:!one},{label:'Pin to Quick access',icon:'pin',fn:()=>pinEntry(e),disabled:!one});
  if(state.query)basic.push({label:'Open file location',icon:'folderline',fn:()=>openContainingFolder(e),disabled:!one});
  const edits=[{label:'Cut',icon:'cut',fn:()=>copySelection('move'),disabled:!copyOK||!mutable,shortcut:'Ctrl+X'},
    {label:'Copy',icon:'copy',fn:()=>copySelection('copy'),disabled:!copyOK,shortcut:'Ctrl+C'},
    {label:'Paste',icon:'paste',fn:paste,disabled:!state.clipboard||!writableLocation(active().uri),shortcut:'Ctrl+V'},
    {label:'Rename',icon:'rename',fn:rename,disabled:!one||!mutable,shortcut:'F2'},
    {label:deleteLabel(e.uri),icon:'trash',fn:trash,disabled:!mutable||!copyOK,shortcut:'Delete'}];
  const common=[{label:'Copy path',icon:'link',fn:copyPath,disabled:!one},'-',
    ...(e.isDir&&!isSmbServer(e.uri)?[{label:'Calculate folder size',icon:'drive',fn:()=>scanFolderSizes(selected().filter(v=>v.isDir)),disabled:!!state.sizeRun}]:[]),
    {label:'Previous versions',icon:'clock',fn:()=>propertiesDialog(e,'versions'),disabled:!one},
    {label:'Properties',icon:'info',fn:()=>propertiesDialog(e),disabled:!one,shortcut:'Alt+Enter'}];
  const style=forceStyle||contextStyle();
  if(style==='win11')openMenu(x,y,[...basic,...common,'-',{label:'Show more options',icon:'more',shortcut:'Shift+F10',fn:()=>entryMenu(x,y,e,'win10')}],{style,strip:edits});
  else openMenu(x,y,[...basic,'-',...edits.slice(0,3),'-',...edits.slice(3),...common.slice(0,1),
    ...(e.isDir?cacheMenuItems(e.targetUri||e.uri,e.name):[]),
    ...(e.uri.startsWith('smb:')?[{label:'Sign out of server…',icon:'eject',fn:()=>signOut(e.uri)}]:[]),...common.slice(1)],{style});
}
function backgroundMenu(x,y){
  const e=menuEntry();
  openMenu(x,y,[{label:'New…',icon:'plus',fn:()=>openNewMenu(x,y),disabled:!writableLocation(e.uri)},
    {label:'Paste',icon:'paste',fn:paste,disabled:!state.clipboard||!writableLocation(e.uri),shortcut:'Ctrl+V'},
    {label:'Refresh',icon:'refresh',fn:()=>load(active(),false),shortcut:'F5'},terminalMenuItem(e),'-',
    {label:'Pin this folder',icon:'pin',fn:pinCurrent},...cacheMenuItems(e.uri),
    {label:'Calculate folder sizes',icon:'drive',fn:()=>scanFolderSizes(filtered().filter(v=>v.isDir)),disabled:!!state.sizeRun},'-',
    {label:'Previous versions',icon:'clock',fn:()=>propertiesDialog(e,'versions')},
    {label:'Properties',icon:'info',fn:()=>propertiesDialog(e),shortcut:'Alt+Enter'}]);
}
function sidebarMenu(x,y,uri,label,share){
  const e={uri,name:label,isDir:true};
  openMenu(x,y,[{label:'Open',icon:'folderline',fn:()=>navigate(uri)},
    {label:'Open in new tab',icon:'plus',fn:()=>addTab(uri)},terminalMenuItem(e),{label:'Open folder with…',icon:'grid',fn:()=>openWithDialog(e)},...cacheMenuItems(uri,label),
    ...(uri?.startsWith('smb:')?[{label:'Sign out of server…',icon:'eject',fn:()=>signOut(uri)}]:[]),'-',
    {label:share?'Remove saved location':'Unpin from Quick access',icon:'pin',fn:()=>removeBookmark(uri,share)},'-',
    {label:'Previous versions',icon:'clock',fn:()=>propertiesDialog(e,'versions')},
    {label:'Properties',icon:'info',fn:()=>propertiesDialog(e)}]);
}
function openNewMenu(x,y){
  const items=[{label:'Folder',icon:'folderline',fn:()=>newItem('folder'),shortcut:'Ctrl+Shift+N'},
    {label:'Text document',icon:'documents',fn:()=>newTemplateDialog('text')},
    {label:'File…',icon:'documents',fn:()=>newTemplateDialog('empty')},'-',
    {label:'Markdown document',icon:'documents',fn:()=>newTemplateDialog('markdown')},
    {label:'CSV file',icon:'list',fn:()=>newTemplateDialog('csv')},
    {label:'JSON file',icon:'documents',fn:()=>newTemplateDialog('json')},
    {label:'HTML document',icon:'documents',fn:()=>newTemplateDialog('html')},'-',
    {label:'From template…',icon:'copy',fn:()=>newTemplateDialog(null)}];
  if(x===undefined){const r=$('new').getBoundingClientRect();x=r.left;y=r.bottom+4;}
  openMenu(x,y,items);
}
async function newTemplateDialog(initial){
  const uri=active().uri;if(!writableLocation(uri))return;
  let data;try{data=await call('templates');}catch(e){toast(e.message);return;}
  const initialTemplate=data.templates.find(t=>t.id===initial)||data.templates[0];
  const result=await showModal(initial==='empty'?'New file':'New from template',initial==='empty'?'Create an empty file with any filename and extension.':'Create a new copy without changing the template.',body=>{
    const name=textField(body,'File name',initialTemplate.suggestedName);name.id='new-file-name';name.previousElementSibling.htmlFor=name.id;
    const label=elem('label','field-label','File type / template');const select=elem('select','template-select');select.id='new-file-template';label.htmlFor=select.id;
    for(const t of data.templates){const o=elem('option','',t.name+(t.builtin?'':' · Your template'));o.value=t.id;select.append(o);}select.value=initialTemplate.id;
    select.onchange=()=>{name.value=data.templates.find(t=>t.id===select.value).suggestedName;};body.append(label,select);
    body.append(elem('div','modal-note','An empty .docx, .xlsx, .pdf, or .odt file is not a valid document. For those formats, place a real starter document in your Templates folder and choose it here. Templates are copied, never executed.'));
    body.append(elem('p','template-path','Templates folder: '+data.directory));
    return {name,select};
  },[{label:'Cancel',cancel:true},{label:'Create',className:'primary',fn:async(f,b)=>{b.disabled=true;try{await call('createTemplate',{uri,name:validateName(f.name.value),template:f.select.value});return true;}finally{b.disabled=false;}}}]);
  if(result)load(active(),false);
}
async function openWithDialog(entry){
  let apps=[],chosen=null,list,search,all,remember,message,open;
  const render=()=>{
    if(!list?.isConnected)return;list.replaceChildren();
    const visible=apps.filter(a=>a.name.toLocaleLowerCase().includes(search.value.toLocaleLowerCase()));
    for(const a of visible){const row=button('',()=>{chosen=a.id;render();});row.className='app-choice'+(a.id===chosen?' chosen':'');row.disabled=!a.available;row.setAttribute('role','option');row.setAttribute('aria-selected',String(a.id===chosen));row.append(icon('grid',25));const text=elem('span');text.append(elem('strong','',a.name),elem('small','',a.default?'Current default':!a.available?'Requires a local mount':a.recommended?'Recommended':'Installed application'));row.append(text);list.append(row);}
    if(!visible.length)list.append(elem('div','apps-empty','No matching installed applications.'));
    if(open)open.disabled=!chosen;
  };
  const fetch=async()=>{message.textContent='Finding installed applications…';try{const data=await call('applications',{uri:entry.uri,allApps:all.checked});if(!list.isConnected)return;apps=data.apps;chosen=apps.find(a=>a.default&&a.available)?.id||apps.find(a=>a.available)?.id||null;message.textContent=native?'Choose an installed application.':'Preview applications only; nothing will launch.';render();}catch(e){message.textContent=e.message;}};
  const promise=showModal('Open with',entry.name,body=>{
    search=textField(body,'Find an application','','Search installed applications');search.id='app-filter';search.previousElementSibling.htmlFor=search.id;search.oninput=render;
    list=elem('div','app-choice-list');list.setAttribute('role','listbox');list.setAttribute('aria-label','Installed applications');body.append(list);
    const line=elem('label','checkbox-row');all=elem('input');all.type='checkbox';all.checked=!!entry.isDir;all.onchange=fetch;line.append(all,document.createTextNode('Show all installed applications'));body.append(line);
    const def=elem('label','checkbox-row');remember=elem('input');remember.type='checkbox';remember.checked=false;remember.id='open-with-default';remember.disabled=!!entry.isDir;def.hidden=!!entry.isDir;def.append(remember,document.createTextNode('Always use this app for this file type'));body.append(def);
    message=elem('p','apps-message','Loading…');body.append(message);setTimeout(fetch,0);
  },[{label:'Cancel',cancel:true},{label:'Open',className:'primary',fn:async(_,b)=>{if(!chosen)return false;b.disabled=true;try{const r=await call('openWith',{uri:entry.uri,appId:chosen,makeDefault:remember.checked});toast(r.warning||(!native?'Preview only. No application launched.':'Opened with the selected application.'));return true;}finally{b.disabled=false;}}}]);
  $('modal').classList.add('open-with-modal');open=$('modal').querySelector('.primary');open.disabled=true;return promise;
}
function findKnownFolder(e){
  const all=state.env?.knownFolders||[];
  return all.find(f=>e.folderKey===f.key)||all.find(f=>sameLocation(f.uri,e.uri)&&f.label===e.name)||all.find(f=>sameLocation(f.uri,e.uri));
}
function propertyRow(grid,label,value){
  grid.append(elem('dt','',label),elem('dd','',value===null||value===undefined||value===''?'Not provided':String(value)));
}
function timestamp(value){return value?new Date(value*1000).toLocaleString():'Not provided';}
async function propertiesDialog(entry,initial='general'){
  entry=menuEntry(entry);let current={...entry},known=findKnownFolder(entry),tabs,panels={},info,permission,versionLoaded=false,alive=true;
  const requestedUri=entry.targetUri||entry.uri;
  const promise=showModal((known?.label||entry.name||baseName(entry.uri))+' Properties','',body=>{
    body.className='properties-body';tabs=elem('div','properties-tabs');tabs.setAttribute('role','tablist');body.append(tabs);
    const names=[['general','General'],...(known?[['location','Location']]:[]),['permissions','Permissions'],['versions','Previous versions']];
    for(const [key,label] of names){const b=button(label,()=>selectTab(key),'property-tab');b.id='prop-tab-'+key;b.dataset.propertyKey=key;b.setAttribute('role','tab');b.setAttribute('aria-controls','prop-'+key);tabs.append(b);const panel=elem('section','properties-panel');panel.id='prop-'+key;panel.setAttribute('role','tabpanel');panel.setAttribute('aria-labelledby',b.id);body.append(panel);panels[key]=panel;}
    info=panels.general;info.append(elem('p','quiet','Reading file properties…'));permission=panels.permissions;
    if(known)renderLocationPanel(panels.location,known);
    setTimeout(()=>selectTab(names.some(n=>n[0]===initial)?initial:'general'),0);
    return {onCancel:finish};
  },[{label:'Close',className:'primary',fn:()=>{finish();return true;}}]);
  function finish(){alive=false;if(panels.versions?.dataset.token)fire('cancel',{token:panels.versions.dataset.token});}
  $('modal').classList.add('properties-modal');attachTabDialog();
  function selectTab(key){
    if(!alive||!info.isConnected)return;
    $('modal').classList.toggle('versions-modal',key==='versions');
    for(const name in panels){panels[name].hidden=name!==key;const b=tabs.querySelector('[data-property-key="'+name+'"]');b.classList.toggle('active',name===key);b.setAttribute('aria-selected',String(name===key));}
    if(key==='versions'&&!versionLoaded){versionLoaded=true;renderVersionsPanel(panels.versions,current);}
  }
  try{
    const p=await call('properties',{uri:requestedUri});if(!alive||!info.isConnected)return promise;current={...current,...p};
    info.replaceChildren();const header=elem('div','property-file');header.append(fileIcon(current,48),elem('strong','',current.name));info.append(header);
    const grid=elem('dl','property-grid');propertyRow(grid,'Type',p.type);propertyRow(grid,'Location',displayUri(p.parentUri||p.uri));propertyRow(grid,'Full path',displayUri(p.uri));
    propertyRow(grid,'Size',p.isDir?(folderSizeText(p.uri)||'Not scanned'):prettyBytes(p.size));if(p.isDir){grid.lastChild.dataset.sizeValue=p.uri;}
    if(!p.isDir)propertyRow(grid,'Opens with',p.defaultApp||'No default application');
    propertyRow(grid,'Created',timestamp(p.created));propertyRow(grid,'Modified',timestamp(p.modified));propertyRow(grid,'Accessed',timestamp(p.accessed));info.append(grid);
    if(!p.isDir&&!readonlyLocation(p.uri))info.append(button('Change app…',()=>{closeModal();openWithDialog(current);},'secondary','grid'));
    const copy=button('Copy full path',()=>call('clipboardText',{text:displayUri(p.uri)}).then(()=>toast('Full path copied.')),'secondary','copy');info.append(copy);
    if(p.isDir&&!isSmbServer(p.uri)){info.append(button('Calculate folder size',()=>scanFolderSizes([p]),'secondary','drive'),elem('p','size-explanation','Logical file bytes, measured on demand. Skips links, nested mounts and snapshot collections. The result may be partial; it is not ZFS compressed or snapshot usage.'));}
    permission.replaceChildren();const pg=elem('dl','property-grid');propertyRow(pg,'Owner',p.owner);propertyRow(pg,'Group',p.group);propertyRow(pg,'POSIX mode',p.mode);
    for(const [name,key]of [['Readable','canRead'],['Writable','canWrite'],['Executable','canExecute']])propertyRow(pg,name,p[key]===true?'Yes':p[key]===false?'No':'Not reported by this backend');permission.append(pg,elem('div','modal-note','These are the permissions reported by Linux/GIO. This page does not edit Windows ACLs, take ownership, or change server permissions.'));
  }catch(e){if(info.isConnected){info.replaceChildren(elem('div','modal-note',e.message));permission.replaceChildren(elem('p','','Metadata could not be read.'));}}
  return promise;
}
function renderLocationPanel(panel,known){
  panel.append(elem('p','location-intro','Choose where '+known.label+' is stored. Applications that honor Linux’s standard-folder settings will use this location.'));
  const input=textField(panel,'Folder location',known.path);input.id='folder-location';input.previousElementSibling.htmlFor=input.id;let validated=null;
  const status=elem('div','location-status');status.setAttribute('role','status');
  const mounts=state.env?.stableMounts||[];
  if(mounts.length){const select=elem('select','mount-picker');select.setAttribute('aria-label','Mounted network drives');select.append(elem('option','','Choose a mounted network drive…'));for(const m of mounts){const o=elem('option','',m.source+' → '+m.path);o.value=m.path;select.append(o);}select.onchange=()=>{if(select.selectedIndex>0){input.value=select.value;validated=null;syncApply();status.textContent='Check this destination before applying.';}};panel.append(select);}
  const controls=elem('div','location-controls');const check=button('Check location',async()=>{check.disabled=true;validated=null;syncApply();const requested=input.value;status.textContent='Checking the folder and write access…';try{const result=await call('locationCheck',{key:known.key,value:requested});if(input.value!==requested){status.textContent='The destination changed. Check it again.';return;}validated=result;input.value=validated.path;status.textContent=(validated.network?'Mounted network folder':'Local folder')+' · '+validated.path;status.className='location-status valid';}catch(e){status.textContent=e.message;status.className='location-status error';}finally{check.disabled=false;syncApply();}} ,'secondary','check');check.id='check-location';
  controls.append(check,button('Restore default',()=>{input.value=known.defaultPath;validated=null;syncApply();status.textContent='Check this folder, then Apply. Existing files stay where they are.';},'secondary'));
  if(known.previousPath)controls.append(button('Use previous',()=>{input.value=known.previousPath;validated=null;syncApply();status.textContent='Check the previous folder, then Apply.';},'secondary'));
  panel.append(controls,status);
  input.oninput=()=>{validated=null;syncApply();status.textContent='Check the changed destination before applying.';};
  const notice=elem('div','location-warning');notice.append(icon('info',18),elem('span','','Existing files will NOT be moved. Your browser may have its own download setting—set it to the same Linux path. A network folder is unavailable while its server is offline.'));panel.append(notice);
  const syncBrave=elem('input');syncBrave.type='checkbox';syncBrave.id='sync-brave-location';if(known.key==='DOWNLOAD'){const row=elem('label','checkbox-row');row.append(syncBrave,document.createTextNode('Also update Brave’s download directory (choose profiles after Apply).'));panel.append(row);}
  const confirmation=elem('label','checkbox-row');const consent=elem('input');consent.type='checkbox';consent.id='confirm-location';confirmation.append(consent,document.createTextNode('Change the system folder location; leave existing files in place.'));panel.append(confirmation);
  const apply=button('Apply location',async()=>{
    if(!consent.checked){status.textContent='Confirm the change using the checkbox first.';return;}
    if(!validated||validated.path!==input.value){status.textContent='Check the destination first.';return;}
    applying=true;syncApply();
    try{const r=await call('locationApply',{key:known.key,value:input.value,confirmed:true});await refreshEnvironment();known.previousPath=r.previous;known.path=r.path;known.uri=r.uri;consent.checked=false;validated=null;status.textContent=native?'Location updated. Configuration backed up. No files moved.':'Preview only: simulated location updated. Your Linux settings were not changed.';status.className='location-status valid';if(syncBrave.checked){setTimeout(()=>void braveDialog(r.path),100);}}
    catch(e){status.textContent=e.message;status.className='location-status error';}finally{applying=false;syncApply();}
  },'primary','check');apply.id='apply-location';panel.append(apply);let applying=false;const syncApply=()=>{apply.disabled=applying||!consent.checked||!validated||validated.path!==input.value;};consent.onchange=syncApply;syncApply();
  const setup=elem('details','network-mount-assistant');setup.append(elem('summary','','Set up network mount (SMB)'));const content=elem('div');setup.append(content);panel.append(setup);
  content.append(elem('p','','An SMB bookmark is not a permanent Linux path. This assistant prepares a command for an on-demand CIFS mount; it does not run it. Administrator approval is required in your terminal.'));
  const address=textField(content,'Network folder','','\\\\archive-nas\\Shared');address.id='mount-share-address';address.previousElementSibling.htmlFor=address.id;
  const result=elem('div','mount-plan');content.append(button('Prepare setup command',async()=>{try{const plan=await call('mountPlan',{value:address.value});result.replaceChildren(elem('p','','1. Install cifs-utils if needed. 2. Review and run the command in a terminal. 3. Return here and use the Linux path below.'));
    const command=elem('textarea','setup-command');command.readOnly=true;command.value=plan.command;command.setAttribute('aria-label','Mount setup command');result.append(command,button('Copy command',()=>call('clipboardText',{text:plan.command}).then(()=>toast('Setup command copied. Review it before running.')),'secondary','copy'));
    result.append(elem('p','mount-target','Linux folder: '+plan.targetPath),button('Use this path',()=>{input.value=plan.targetPath;validated=null;syncApply();status.textContent='After mounting, click Check location.';},'secondary'));
    result.append(elem('div','modal-note','The helper creates two systemd units and a root-only plaintext SMB credential file. It uses SMB 3.0, prompts before changes, does not edit fstab, and refuses to overwrite existing configuration. This system mount is separate from your GVfs/keyring session; “Sign out of server” does not remove it.'));
    const remove=elem('details');remove.append(elem('summary','','Removal command (after restoring folder locations)'),elem('pre','',plan.removeCommand));result.append(remove);
  }catch(e){result.textContent=e.message;}},'secondary','network'),result);
}
async function renderVersionsPanel(panel,entry){
  if(panel.dataset.token)fire('cancel',{token:panel.dataset.token});
  panel.replaceChildren(elem('div','versions-intro','Browse real snapshot or backup folders exposed by your server. This build does not enumerate Windows SMB shadow-copy protocol responses or create snapshots.'));
  const toolbar=elem('div','versions-toolbar');toolbar.append(button('Refresh',()=>renderVersionsPanel(panel,entry),'secondary','refresh'),button('Snapshot source…',()=>renderSnapshotSource(panel,entry),'secondary','settings'));panel.append(toolbar);
  const list=elem('div','versions-list');panel.append(list);list.textContent='Checking readable snapshot folders…';
  const token='versions-'+(++seq);panel.dataset.token=token;
  try{
    const data=await call('previousVersions',{uri:entry.uri,isDir:!!entry.isDir,token});if(!list.isConnected)return;
    list.replaceChildren();list.setAttribute('role','list');list.setAttribute('aria-label','Previous versions');
    if(data.versions.length){
      const head=elem('div','version-columns');head.setAttribute('aria-hidden','true');
      head.append(elem('span','','Snapshot'),elem('span','','Date & time'),elem('span','version-action-heading','Actions'));list.append(head);
      const note=elem('p','version-date-note','Dates are read from snapshot names. Times stay as written; UTC is marked when supplied.');panel.insertBefore(note,list);
    }
    if(!data.versions.length)list.append(icon('clock',30),elem('h3','','No accessible previous versions'),elem('p','',data.message));
    for(const version of data.versions){
      const row=elem('div','version-row');row.setAttribute('role','listitem');
      const identity=elem('div','version-identity');identity.append(fileIcon(version,23));
      const text=elem('div','version-text');text.title=version.label+'\n'+displayUri(version.source);
      text.append(elem('strong','',version.label),elem('small','',displayUri(version.source)));identity.append(text);row.append(identity);
      const metadata=window.OpenXplorerSnapshots.describe(version);
      const date=elem('div','version-date');date.title=metadata.explanation;
      const stamp=elem('time','version-stamp');if(metadata.iso)stamp.setAttribute('datetime',metadata.iso);
      stamp.append(elem('span','version-calendar-date',metadata.date));
      if(metadata.time)stamp.append(elem('span','version-time',metadata.time));
      date.append(stamp);row.append(date);
      const actions=elem('div','version-actions');
      if(version.isDir)actions.append(button('Browse',()=>{const tab=addTab(version.uri);tab.snapshot={root:version.snapshotRoot||version.uri,label:version.label};renderTabs();renderSnapshotBanner(tab);},'secondary','folderline'));
      actions.append(button('Restore a copy…',()=>restoreVersion(version),'secondary','copy'));row.append(actions);list.append(row);
    }
    if(data.truncated)panel.append(elem('p','quiet','Showing at most 100 versions. Configure a narrower snapshot folder to see more.'));
    if(data.warnings?.length){const details=elem('details','snapshot-warnings');details.append(elem('summary','','Availability details'),elem('pre','',data.warnings.join('\n')));panel.append(details);}
    panel.append(elem('div','modal-note','Snapshots must already exist and be readable through your NAS. Built-in layouts include .snapshot, #snapshot, .zfs/snapshot, and Snapper .snapshots/<id>/snapshot. Restore a copy never overwrites the live item. A cached search result is not a previous version.'));
  }catch(e){if(list.isConnected)list.textContent=e.message;}
}
function renderSnapshotSource(panel,entry){
  if(panel.dataset.token)fire('cancel',{token:panel.dataset.token});
  panel.replaceChildren(elem('p','','Map the current live folder to a directory containing dated snapshots. Each snapshot must contain the same relative paths. These may also be existing backup folders; OpenXplorer does not certify them as immutable.'));
  let live=entry.isDir?entry.uri:parentUri(entry.uri);
  try{const u=new URL(entry.uri);if(u.protocol==='smb:')live='smb://'+u.host+'/'+u.pathname.split('/').filter(Boolean)[0];}catch{}
  const root=textField(panel,'Live folder',displayUri(live));const snapshots=textField(panel,'Snapshot collection folder',displayUri(live).replace(/[\\/]$/,'')+(live.startsWith('smb:')?'\\.snapshot':'/.snapshot'));
  const layout=elem('select');layout.setAttribute('aria-label','Snapshot layout');for(const [value,text]of [['direct','snapshot-name / relative path'],['snapper','snapshot-id / snapshot / relative path']]){const o=elem('option','',text);o.value=value;layout.append(o);}panel.append(layout);
  const error=elem('div','modal-error');panel.append(error);
  const controls=elem('div','snapshot-source-actions');controls.append(button('Back',()=>renderVersionsPanel(panel,entry),'secondary'),button('Save source',async()=>{try{await call('snapshotSource',{live:root.value,snapshots:snapshots.value,layout:layout.value});await refreshEnvironment();renderVersionsPanel(panel,entry);}catch(e){error.textContent=e.message;}},'primary'),button('Remove mapping',async()=>{try{await call('snapshotSource',{live:root.value,snapshots:snapshots.value,remove:true});await refreshEnvironment();renderVersionsPanel(panel,entry);}catch(e){error.textContent=e.message;}},'secondary'));panel.append(controls);
}
async function restoreVersion(version){
  const result=await showModal('Restore a copy',version.label+' · '+version.name,body=>{
    body.append(elem('div','modal-note','Copies this version into a destination you choose. Existing names are kept; the restored item receives a copy name if needed. The live original and snapshot are not replaced.'));
    return {target:textField(body,'Destination folder',displayUri(state.env.home))};
  },[{label:'Cancel',cancel:true},{label:'Copy version',className:'primary',fn:async(f)=>{const r=await call('normalise',{value:f.target.value});if(readonlyLocation(r.uri))throw Error('Choose a folder outside the snapshot collection.');return r.uri;}}]);
  if(result)await runOperation('copy',{uris:[version.uri],target:result,policy:'keep-both'});
}
function menuPreferenceControls(body){
  const line=elem('div','settings-line');const label=elem('div');label.append(elem('strong','','Right-click menu'),elem('p','','Windows 10 is compact and shows familiar text commands.'));
  const select=elem('select');select.id='context-menu-style';select.setAttribute('aria-label','Right-click menu style');for(const [value,name]of [['win10','Windows 10 · Classic (default)'],['win11','Windows 11 · Compact actions']]){const o=elem('option','',name);o.value=value;select.append(o);}select.value=contextStyle();select.onchange=()=>{state.env.preferences.contextMenu=select.value;fire('preferences',{contextMenu:select.value});};line.append(label,select);body.append(line);
}


function driveMenu(x,y,uri,label,volume){
  if(volume){openMenu(x,y,[{label:'Mount volume',icon:'drive',fn:()=>mountVolume(volume)}]);return;}
  const entry={uri,name:label,isDir:true};
  openMenu(x,y,[{label:'Open',icon:'drive',fn:()=>navigate(uri)},{label:'Open in new window',icon:'plus',fn:()=>call('newWindow',{uri})},...cacheMenuItems(uri,label),'-',{label:'Properties',icon:'info',fn:()=>propertiesDialog(entry)}]);
}
async function archiveDialog(entry){
  let prefix='',list,pathLabel,error,requestId=0;
  const render=async(next='')=>{
    const generation=++requestId;prefix=next;pathLabel.textContent=displayUri(entry.uri)+(prefix?' › '+prefix:'');list.replaceChildren(elem('p','quiet','Reading ZIP directory…'));error.textContent='';
    try{const result=await call('archiveList',{uri:entry.uri,prefix});if(generation!==requestId||!list.isConnected)return;list.replaceChildren();
      for(const item of result.entries){const row=button('',()=>{},'archive-row');row.setAttribute('aria-label',item.name);row.append(fileIcon(item,25),elem('span','archive-name',item.name),elem('span','archive-size',item.isDir?'Folder':prettyBytes(item.size)));
        row.addEventListener('dblclick',()=>{if(item.isDir)void render(item.member);else if(item.canOpen)void openMember(item);else error.textContent='Use an archive manager for encrypted, large or unsupported members.';});
        row.addEventListener('keydown',ev=>{if(ev.key==='Enter'){ev.preventDefault();if(item.isDir)void render(item.member);else if(item.canOpen)void openMember(item);}});list.append(row);}
      if(!result.entries.length)list.append(elem('p','quiet','This archive folder is empty.'));
      if(result.skippedUnsafe)error.textContent=result.skippedUnsafe+' unsafe names or links are hidden.';
      if(result.truncated)error.textContent+=' Showing the first 5,000 entries.';
    }catch(e){if(list.isConnected){list.replaceChildren();error.textContent=e.message;}}
  };
  const openMember=async(item)=>{error.textContent='Opening a read-only temporary copy…';try{await call('archiveOpenMember',{uri:entry.uri,member:item.member});error.textContent=native?'Opened a temporary copy. Changes are not saved back to the ZIP.':'Preview only. No content was extracted or opened.';}catch(e){error.textContent=e.message;}};
  const promise=showModal(entry.name+' — Compressed folder','Browse without extracting the entire archive.',body=>{
    const toolbar=elem('div','archive-toolbar');toolbar.append(button('Up',()=>{const parts=prefix.replace(/\/$/,'').split('/').filter(Boolean);parts.pop();void render(parts.length?parts.join('/')+'/':'');},'secondary','up'));
    pathLabel=elem('div','archive-path');toolbar.append(pathLabel);body.append(toolbar);
    body.append(elem('p','settings-help','Read-only ZIP. Double-click a folder to browse it. Opening a supported file creates only that file’s temporary copy; it does not update the ZIP.'));
    list=elem('div','archive-list');body.append(list);error=elem('div','archive-error');error.setAttribute('role','status');body.append(error);setTimeout(()=>render(''),0);
  },[{label:'Extract all…',fn:()=>{closeModal();void extractDialog(entry);return false;}},{label:'Open in archive manager',fn:async()=>{await call('open',{uri:entry.uri});return true;}},{label:'Close',className:'primary',fn:()=>true}]);
  $('modal').classList.add('archive-modal');return promise;
}
// ---- 0.6: workspace layout, segmented paths, tab-owned Properties, size scans ----
function breadcrumbSegments(uri){
  if(['home:','pc:','network:','settings:'].includes(uri))return[{label:titleFor(uri),uri}];
  try{
    const u=locationParts(uri),parts=u.pathname.split('/').filter(Boolean),result=[];
    let acc;
    if(u.protocol==='smb:'){acc='smb://'+u.host;result.push({label:u.host,uri:acc+'/'});}
    else if(u.protocol==='file:'){acc='file://';result.push({label:'/',uri:'file:///'});}
    else if(deviceLocation(uri)){acc=u.protocol+'//'+u.host;result.push({label:deviceMountName(uri),uri:acc+'/'});}
    else return[{label:uri,uri}];
    for(const part of parts){acc+='/'+part;result.push({label:decodeURIComponent(part),uri:acc});}
    return result;
  }catch{return[{label:uri,uri}];}
}
function networkLocation(uri){
  if(!uri)return false;if(uri.startsWith('smb:'))return true;
  if(!uri.startsWith('file:'))return false;
  try{const path=decodeURIComponent(new URL(uri).pathname).replace(/\/$/,'')||'/';
    return(state.env?.stableMounts||[]).some(m=>['cifs','smb3'].includes(m.fstype||'cifs')&&m.path&&(path===m.path||path.startsWith(m.path.replace(/\/$/,'')+'/')));
  }catch{return false;}
}
function uniqueEditors(editors){
  const seen=new Set();return editors.filter(e=>{
    const name=String(e.name||'').trim().toLocaleLowerCase();
    if(!e.id||!name||seen.has(name)||/url[-_]?handler/i.test(e.id))return false;
    seen.add(name);return true;
  });
}
function saveLayout(values){
  if(!state.env)return;Object.assign(state.env.preferences,values);fire('preferences',values);
}
function resetLayout(){
  saveLayout({sidebarWidth:210,columnWidths:{}});applyLayout();renderColumns();renderRows();
}
function sidebarLimit(){
  const width=document.querySelector('.workspace').clientWidth;
  const details=state.details&&getComputedStyle($('details')).display!=='none'?$('details').offsetWidth:0;
  return Math.max(140,Math.min(560,width-details-300));
}
function applyLayout(){
  const pref=state.env?.preferences||{};
  applyTextSize(pref.textSize,false);
  const width=Math.max(140,Math.min(sidebarLimit(),Number(pref.sidebarWidth)||210));
  $('app').style.setProperty('--sidebar-width',width+'px');
  const handle=$('sidebar-resizer');handle?.setAttribute('aria-valuemin','140');handle?.setAttribute('aria-valuemax',String(sidebarLimit()));handle?.setAttribute('aria-valuenow',String(Math.round(width)));
  applyColumnLayout();
}
const columnDefaults={name:260,modified:152,parentUri:330,type:135,size:90};
const columnLimits={name:[140,1600],modified:[100,1000],parentUri:[140,1600],type:[80,1000],size:[70,600]};
function columnFields(){return['name',state.query?'parentUri':'modified','type','size'];}
function applyColumnLayout(){
  const widths=state.env?.preferences?.columnWidths||{},fields=columnFields();
  const values=fields.map(k=>Math.max(columnLimits[k][0],Math.min(columnLimits[k][1],Number(widths[k])||columnDefaults[k])));
  const total=values.reduce((a,b)=>a+b,0)+28;
  // Name expands only until the user resizes it. Then all widths are exact;
  // overflow is horizontal scrolling, never hiding columns or losing alignment.
  const parts=values.map((w,i)=>i===0&&!widths.name?`minmax(${w}px,1fr)`:w+'px');
  $('main').style.setProperty('--file-columns',parts.join(' '));
  $('main').style.setProperty('--columns-min',total+'px');
  $('file-canvas').style.minWidth=state.view==='grid'?'0':total+'px';
  $('column-head').scrollLeft=$('file-scroll').scrollLeft;
}
function setupSidebarResize(){
  const handle=$('sidebar-resizer');handle.title='Drag to resize sidebar · double-click to reset';
  let drag=null;
  handle.addEventListener('pointerdown',e=>{
    if(e.button!==0)return;e.preventDefault();e.stopPropagation();resetTypeSelect();
    drag={x:e.clientX,width:document.querySelector('.sidebar').offsetWidth};handle.setPointerCapture(e.pointerId);document.body.classList.add('resizing');
  });
  handle.addEventListener('pointermove',e=>{
    if(!drag)return;const width=Math.round(Math.max(140,Math.min(sidebarLimit(),drag.width+e.clientX-drag.x)));
    state.env.preferences.sidebarWidth=width;applyLayout();renderRows();
  });
  const end=()=>{if(!drag)return;drag=null;document.body.classList.remove('resizing');saveLayout({sidebarWidth:state.env.preferences.sidebarWidth||210});};
  handle.addEventListener('pointerup',end);handle.addEventListener('pointercancel',end);handle.addEventListener('lostpointercapture',end);
  handle.addEventListener('dblclick',()=>{saveLayout({sidebarWidth:210});applyLayout();renderRows();});
  handle.addEventListener('keydown',e=>{
    let width=Number(state.env.preferences.sidebarWidth)||210;
    if(e.key==='ArrowLeft')width-=e.shiftKey?40:10;else if(e.key==='ArrowRight')width+=e.shiftKey?40:10;else if(e.key==='Home')width=210;else return;
    e.preventDefault();e.stopPropagation();saveLayout({sidebarWidth:Math.max(140,Math.min(sidebarLimit(),width))});applyLayout();renderRows();
  });
}
function setColumnWidth(field,width){
  const [min,max]=columnLimits[field];
  state.env.preferences.columnWidths={...(state.env.preferences.columnWidths||{}),[field]:Math.round(Math.max(min,Math.min(max,width)))};
  applyColumnLayout();
}
function fitColumn(field){
  const ctx=document.createElement('canvas').getContext('2d');ctx.font=(12*textMetrics().scale)+'px '+getComputedStyle($('main')).fontFamily;
  const rows=filtered().slice(0,2000);let width=columnLimits[field][0];
  for(const e of rows){let text=field==='modified'?dateText(e.modified):field==='parentUri'?displayUri(e.parentUri||parentUri(e.uri)):field==='size'?(e.isDir?folderSizeText(e.uri):prettyBytes(e.size)):String(e[field]||'');width=Math.max(width,ctx.measureText(text).width+(field==='name'?64:30));}
  setColumnWidth(field,width);saveLayout({columnWidths:state.env.preferences.columnWidths});renderRows();
}
function renderColumns(){
  const head=$('column-head');head.hidden=state.view==='grid';head.replaceChildren();applyColumnLayout();
  const labels={name:'Name',modified:'Date modified',parentUri:'Folder path',type:'Type',size:'Size'};
  for(const field of columnFields()){
    const cell=elem('div','column');cell.dataset.field=field;cell.setAttribute('role','columnheader');cell.setAttribute('aria-sort',state.sort===field?(state.descending?'descending':'ascending'):'none');
    const label=button(labels[field],()=>{if(state.sort===field)state.descending=!state.descending;else{state.sort=field;state.descending=false;}state.filterCache=null;renderColumns();renderRows();},'column-label');
    if(state.sort===field){const arrow=icon('down');if(!state.descending)arrow.style.transform='rotate(180deg)';label.append(arrow);}
    const handle=elem('span','column-resizer');handle.tabIndex=0;handle.setAttribute('role','separator');handle.setAttribute('aria-orientation','vertical');handle.setAttribute('aria-label','Resize '+labels[field]+' column');
    handle.title='Drag to resize · double-click to fit loaded items (up to 2,000)';
    handle.setAttribute('aria-valuemin',columnLimits[field][0]);handle.setAttribute('aria-valuemax',columnLimits[field][1]);handle.setAttribute('aria-valuenow',(state.env?.preferences?.columnWidths||{})[field]||columnDefaults[field]);
    let drag=null;
    handle.addEventListener('pointerdown',e=>{if(e.button!==0)return;e.preventDefault();e.stopPropagation();drag={x:e.clientX,width:cell.getBoundingClientRect().width};handle.setPointerCapture(e.pointerId);document.body.classList.add('resizing');});
    handle.addEventListener('pointermove',e=>{if(!drag)return;setColumnWidth(field,drag.width+e.clientX-drag.x);handle.setAttribute('aria-valuenow',state.env.preferences.columnWidths[field]);});
    const end=()=>{if(!drag)return;drag=null;document.body.classList.remove('resizing');saveLayout({columnWidths:state.env.preferences.columnWidths||{}});renderRows();};
    handle.addEventListener('pointerup',end);handle.addEventListener('pointercancel',end);handle.addEventListener('lostpointercapture',end);
    handle.addEventListener('click',e=>e.stopPropagation());handle.addEventListener('dblclick',e=>{e.preventDefault();e.stopPropagation();fitColumn(field);});
    handle.addEventListener('keydown',e=>{if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();e.stopPropagation();setColumnWidth(field,cell.getBoundingClientRect().width+(e.key==='ArrowLeft'?-10:10));saveLayout({columnWidths:state.env.preferences.columnWidths});handle.setAttribute('aria-valuenow',state.env.preferences.columnWidths[field]);}else if(e.key==='Home'){e.preventDefault();e.stopPropagation();fitColumn(field);}});
    cell.append(label,handle);head.append(cell);
  }
}
function positionTabDialog(){
  if(!state.modalOwner)return;const app=$('app').getBoundingClientRect(),title=document.querySelector('.titlebar').getBoundingClientRect();
  const layer=$('modal-layer');layer.style.setProperty('--dialog-top',title.bottom+'px');layer.style.setProperty('--dialog-left',app.left+'px');layer.style.setProperty('--dialog-right',(innerWidth-app.right)+'px');layer.style.setProperty('--dialog-bottom',(innerHeight-app.bottom)+'px');
}
function attachTabDialog(){$('modal').setAttribute('aria-modal','false');state.modalOwner=state.activeId;$('modal-layer').classList.add('tab-scoped');positionTabDialog();}
function suspendTabDialog(){
  if(!state.modalOwner)return;const tab=state.tabs.find(t=>t.id===state.modalOwner);if(!tab)return;
  const box=$('modal');tab.savedDialog={box,resolve:state.modalResolve,cancel:state.modalCancel,focus:box.contains(document.activeElement)?document.activeElement:null};
  box.id='modal-'+tab.id;box.hidden=true;$('dialog-vault').append(box);
  const empty=elem('section','modal');empty.id='modal';empty.setAttribute('role','dialog');empty.setAttribute('aria-modal','true');empty.setAttribute('aria-labelledby','modal-title');$('modal-layer').append(empty);
  state.modalResolve=null;state.modalCancel=null;state.modalOwner=null;$('modal-layer').hidden=true;$('modal-layer').classList.remove('tab-scoped');
}
function restoreTabDialog(){
  const tab=active(),saved=tab?.savedDialog;if(!saved)return;
  $('modal').remove();saved.box.id='modal';saved.box.hidden=false;$('modal-layer').append(saved.box);$('modal-layer').hidden=false;
  state.modalResolve=saved.resolve;state.modalCancel=saved.cancel;tab.savedDialog=null;attachTabDialog();
  requestAnimationFrame(()=>{if(state.modalOwner===tab.id)(saved.focus||saved.box.querySelector('button'))?.focus({preventScroll:true});});
}
function discardTabDialog(tab){
  if(!tab)return;if(state.modalOwner===tab.id)closeModal();
  if(tab.savedDialog){const saved=tab.savedDialog;tab.savedDialog=null;saved.cancel?.();saved.resolve?.(null);saved.box.remove();}
}
function sizeKey(uri){return uri.replace(/\/$/,'');}
function itemSize(entry){return entry.isDir?(state.folderSizes.get(sizeKey(entry.uri))?.bytes||0):(entry.size||0);}
function folderSizeText(uri){
  const result=state.folderSizes.get(sizeKey(uri));if(!result)return'';
  if(result.status==='scanning')return 'Scanning…';
  if(result.status==='error')return 'Unavailable';
  return(result.status==='complete'?'':'≥ ')+prettyBytes(result.bytes);
}
function receiveFolderSize(data){
  if(!state.sizeRun||data.token!==state.sizeRun.token)return;
  state.folderSizes.set(sizeKey(data.uri),data);
  const text=`${state.sizeRun.index+1}/${state.sizeRun.total} · ${baseName(data.uri)} · ${prettyBytes(data.bytes)} · ${Number(data.files||0).toLocaleString()} files`;
  $('size-scan-label').textContent=(state.sizeRun.cancelled?'Cancelling… ':'Scanning folder size · ')+text;
  updateSizeLabels();
}
function updateSizeLabels(){
  for(const el of document.querySelectorAll('[data-size-value]')){const result=state.folderSizes.get(sizeKey(el.dataset.sizeValue));el.textContent=folderSizeText(el.dataset.sizeValue)||'Not scanned';el.title=result?`${result.status} · ${result.files||0} files · ${result.skipped||0} skipped · ${result.errors||0} unreadable. ${result.reason||''} ${result.updated?new Date(result.updated*1000).toLocaleString():''}`:'';}
}
function stopSizeScan(){
  if(state.sizeRun){state.sizeRun.cancelled=true;fire('cancel',{token:state.sizeRun.token});$('size-scan-label').textContent='Cancelling size scan…';$('size-scan-stop').disabled=true;}
  else $('size-scan').hidden=true;
}
async function scanFolderSizes(entries){
  if(state.sizeRun){toast('Cancel or finish the current folder-size scan first.');return;}
  const unique=new Map();for(const entry of entries||[]){const uri=entry.targetUri||entry.uri;if(entry.isDir&&!isSmbServer(uri)&&/^(file|smb):/.test(uri))unique.set(sizeKey(uri),{...entry,uri});}
  const folders=[...unique.values()];if(!folders.length){toast('Select a folder or share to calculate its size.');return;}
  const run={token:null,cancelled:false,index:0,total:folders.length};state.sizeRun=run;
  $('size-scan').hidden=false;$('size-scan-stop').textContent='Cancel scan';$('size-scan-stop').disabled=false;$('size-scan-stop').onclick=stopSizeScan;
  let done=0,partial=0;
  try{
    for(let i=0;i<folders.length;i++){
      if(run.cancelled)break;run.index=i;run.token='size-'+(++seq);const entry=folders[i];
      receiveFolderSize({uri:entry.uri,token:run.token,bytes:0,files:0,folders:0,status:'scanning'});renderRows();
      try{const result=await call('folderSize',{uri:entry.uri,token:run.token});state.folderSizes.set(sizeKey(entry.uri),result);if(result.status==='complete')done++;else partial++;}
      catch(e){state.folderSizes.set(sizeKey(entry.uri),{uri:entry.uri,status:'error',bytes:0,reason:e.message});partial++;}
      state.filterCache=null;renderRows();renderDetails();updateSizeLabels();
    }
  }finally{
    state.sizeRun=null;const hint=run.cancelled?'Size scan cancelled':`Size scan finished · ${done} complete${partial?' · '+partial+' partial/unavailable':''}`;
    $('size-scan-label').textContent=hint+' · Logical bytes; recalculate after changes';$('size-scan-stop').textContent='Dismiss';$('size-scan-stop').disabled=false;
  }
}

// 0.7: connected-network entries, window management and searchable settings.
function networkLocations(){
  if(state.env.networkLocations)return state.env.networkLocations;
  const map=new Map();
  for(const s of state.env.shares||[])map.set(s.uri.replace(/\/$/,''),{...s,saved:true,kind:'share'});
  for(const m of state.env.mounts||[])if(m.mounted&&m.uri?.startsWith('smb:')){const key=m.uri.replace(/\/$/,'');map.set(key,{...m,...map.get(key),connected:true,kind:'share'});}
  for(const s of state.sessionNetwork||[]){const key=s.uri.replace(/\/$/,'');if(!map.has(key))map.set(key,s);}
  return [...map.values()];
}
function rememberPreviewNetwork(uri){
  if(native||!uri.startsWith('smb:'))return;
  const u=new URL(uri),first=u.pathname.split('/').filter(Boolean)[0];
  const root=u.origin==='null'?'smb://'+u.host+(first?'/'+first:'/'):uri;
  state.sessionNetwork=state.sessionNetwork||[];
  if(!state.sessionNetwork.some(s=>sameLocation(s.uri,root)))state.sessionNetwork.push({uri:root,label:first?decodeURIComponent(first):u.host,connected:true,saved:false,kind:first?'share':'server'});
  renderSidebar();
}
function networkLocationMenu(x,y,s){
  const items=[{label:'Open',icon:'folderline',fn:()=>navigate(s.uri)},
    {label:'Open in new tab',icon:'plus',fn:()=>addTab(s.uri)},
    {label:'Open in new window',icon:'share',fn:()=>call('newWindow',{uri:s.uri})},terminalMenuItem({uri:s.uri,isDir:true})];
  if(s.uri.startsWith('smb:')&&!isSmbServer(s.uri))items.push(s.saved?
    {label:'Remove saved location',icon:'pin',fn:()=>removeBookmark(s.uri,true)}:
    {label:'Keep in Network',icon:'pin',fn:async()=>{await call('bookmark',{action:'add',kind:'share',uri:s.uri,label:s.label});await refreshEnvironment();}});
  if(s.uri.startsWith('smb:'))items.push({label:'Sign out of server…',icon:'eject',fn:()=>signOut(s.uri)});
  if(s.kind==='mount')items.push({label:'Properties',icon:'info',fn:()=>propertiesDialog({uri:s.uri,name:s.label,isDir:true})});
  openMenu(x,y,items);
}
function prepareTabTransfer(t){
  return {uri:t.uri,history:t.history.slice(-200),index:Math.max(0,t.index-Math.max(0,t.history.length-200)),
    scroll:t.id===state.activeId?$('file-scroll').scrollTop:t.scroll||0,
    selection:t.id===state.activeId?[...state.selection]:[],view:state.view,sort:state.sort,
    descending:state.descending,settingsSection:state.settingsSection||null};
}
async function detachTab(id){
  const t=state.tabs.find(t=>t.id===id);if(!t||state.detaching)return;
  if(activeAuth||state.operation||t.savedDialog||state.modalOwner===id){toast('Close this tab’s dialog and finish its file operation before moving it. The tab was kept.');return;}
  state.detaching=true;
  try{
    const snapshot=prepareTabTransfer(t);state.lastDetachedTab=snapshot;
    const result=await call('detachTab',{tab:snapshot});
    if(!result.ready){toast('Preview only — the desktop application creates the separate window. Your tab was kept.');return;}
    if(state.tabs.length===1){if(native)fire('window',{action:'close'});else addTab();}
    if(state.tabs.length>1)closeTab(id);
  }catch(e){toast(e.message+' The original tab was kept.');}
  finally{state.detaching=false;}
}
async function restoreTransferredTab(value){
  try{
    const t=active();state.view=value.view==='grid'?'grid':'details';state.sort=value.sort||'name';state.descending=!!value.descending;state.settingsSection=value.settingsSection;
    await navigate(value.uri,false);t.history=value.history;t.index=value.index;t.scroll=value.scroll||0;
    state.selection=new Set(value.selection||[]);$('file-scroll').scrollTop=t.scroll;renderRows();renderNavigation();updateToolbar();
    if(t.uri==='settings:')renderContent();
    await call('handoffReady');
  }catch(e){toast('Could not restore the moved tab: '+e.message);}
}
function setupTabDrag(node,t){
  let drag=null;
  node.addEventListener('contextmenu',e=>{e.preventDefault();openMenu(e.clientX,e.clientY,[
    {label:'Move tab to new window',icon:'share',fn:()=>detachTab(t.id)},
    {label:'Move tab to window…',icon:'desktop',fn:()=>moveTabMenu(t.id)},
    {label:'Duplicate tab',icon:'copy',fn:()=>addTab(t.uri)},
    {label:'Open windows…',icon:'desktop',fn:windowsMenu},'-',
    {label:'Close tab',icon:'close',fn:()=>closeTab(t.id)}]);});
  if(native&&state.env?.nativeTabDrag)return; // Real GTK pointer grab, not DOM pointer capture.
  node.addEventListener('pointerdown',e=>{
    if(e.button!==0||e.target.closest('.tab-close')||activeAuth||state.detaching)return;
    drag={id:e.pointerId,x:e.clientX,y:e.clientY,moving:false};
    try{node.setPointerCapture(e.pointerId);}catch{}
  });
  node.addEventListener('pointermove',e=>{
    if(!drag||drag.id!==e.pointerId)return;
    if(!drag.moving&&Math.hypot(e.clientX-drag.x,e.clientY-drag.y)<10)return;
    drag.moving=true;e.preventDefault();node.classList.add('tab-drag-source');
    const bar=document.querySelector('.titlebar').getBoundingClientRect();
    drag.detach=e.clientY>bar.bottom+45||e.clientY<bar.top-25||e.clientX<bar.left-20||e.clientX>bar.right+20;
    const hint=$('tab-drag-hint');hint.hidden=false;hint.textContent=drag.detach?'Release to move tab to a new window':'Drag below the tab strip to create a new window';
    hint.style.left=Math.max(8,Math.min(e.clientX+15,innerWidth-335))+'px';hint.style.top=Math.max(8,Math.min(e.clientY+20,innerHeight-60))+'px';
  });
  const end=e=>{
    if(!drag||drag.id!==e.pointerId)return;const last=drag;drag=null;
    try{if(node.hasPointerCapture(e.pointerId))node.releasePointerCapture(e.pointerId);}catch{}
    $('tab-drag-hint').hidden=true;node.classList.remove('tab-drag-source');
    if(last.moving){state.tabClickSuppress=performance.now()+400;e.preventDefault();if(e.type==='pointerup'&&last.detach)void detachTab(t.id);}
  };
  node.addEventListener('pointerup',end);node.addEventListener('pointercancel',end);
}
function publishTabDragLayout(){
  const bar=document.querySelector('.titlebar').getBoundingClientRect();
  const end=$('windows-button').getBoundingClientRect().left;
  fire('tabDragLayout',{width:innerWidth,height:bar.bottom,end:Math.max(0,end),tabs:[...$('tabs').children].map(n=>{const r=n.getBoundingClientRect(),c=n.querySelector('.tab-close').getBoundingClientRect();return{id:n.dataset.tabId,left:r.left,right:r.right,close:c.left};})});
}
function tabCanMove(id){
  const t=state.tabs.find(t=>t.id===id);
  if(!t||state.detaching||state.outgoingTab||activeAuth||state.operation||t.savedDialog||state.modalOwner===id){toast('Close this tab’s dialog and finish file operations before moving it.');return null;}
  return t;
}
async function beginNativeTabDrag(id){
  const t=tabCanMove(id);if(!t)return;
  state.outgoingTab=id;state.tabClickSuppress=performance.now()+700;
  try{await call('beginTabDrag',{tabId:id,tab:prepareTabTransfer(t)});}
  catch(e){state.outgoingTab=null;toast(e.message);}
}
async function moveTabMenu(id){
  const t=tabCanMove(id);if(!t)return;const windows=await call('windows');
  const candidates=windows.filter(w=>w.id!==state.env.windowId&&(native||!w.active));
  const r=[...$('tabs').children].find(n=>n.dataset.tabId===id)?.getBoundingClientRect()||{left:20,bottom:50};
  openMenu(Math.min(r.left,innerWidth-300),r.bottom+5,[
    ...(candidates.length?candidates.map(w=>({label:w.title||'OpenXplorer window',icon:'desktop',disabled:w.ready===false,fn:()=>moveTabToWindow(id,w.id)})):[{label:'No other OpenXplorer windows',disabled:true}]),
    '-',{label:'Move tab to new window',icon:'share',fn:()=>detachTab(id)}]);
}
async function moveTabToWindow(id,windowId){
  const t=tabCanMove(id);if(!t)return;state.outgoingTab=id;
  try{const result=await call('moveTabToWindow',{tabId:id,tab:prepareTabTransfer(t),windowId});
    if(!result.pending){state.outgoingTab=null;toast('Preview only — moving between windows requires the desktop app.');}
  }catch(e){state.outgoingTab=null;toast(e.message+' The original tab was kept.');}
}
function reorderTab(id,beforeId){
  if(id===beforeId)return;const i=state.tabs.findIndex(t=>t.id===id);if(i<0)return;
  const [t]=state.tabs.splice(i,1);const dest=state.tabs.findIndex(t=>t.id===beforeId);state.tabs.splice(dest<0?state.tabs.length:dest,0,t);renderTabs();
}
function showTabDropHint(data){
  const bar=$('tabs');bar.classList.toggle('tab-drop-active',!!data.show);
  bar.querySelectorAll('.tab-insert-before').forEach(n=>n.classList.remove('tab-insert-before'));
  if(data.show&&data.beforeId)[...bar.children].find(n=>n.dataset.tabId===data.beforeId)?.classList.add('tab-insert-before');
}
function finishTabTransfer(data){
  if(state.outgoingTab===data.tabId)state.outgoingTab=null;
  showTabDropHint({show:false});state.tabClickSuppress=performance.now()+400;
  if(!data.committed){if(data.message)toast(data.message);return;}
  if(!state.tabs.some(t=>t.id===data.tabId))return;
  if(state.tabs.length===1&&native)fire('window',{action:'close'});else closeTab(data.tabId);
}
const incomingTabs=new Map();
function settleIncomingTab(data){
  const id=incomingTabs.get(data.token);incomingTabs.delete(data.token);
  if(id&&!data.committed&&state.tabs.some(t=>t.id===id))closeTab(id);
}
async function receiveTransferredTab(data){
  if(activeAuth||state.operation||!$('modal-layer').hidden||incomingTabs.size){await call('tabTransferReady',{token:data.token,accepted:false});return;}
  let t;
  try{
    const value=data.tab;state.view=value.view==='grid'?'grid':'details';state.sort=value.sort||'name';state.descending=!!value.descending;state.settingsSection=value.settingsSection;
    t=addTab(value.uri);incomingTabs.set(data.token,t.id);reorderTab(t.id,data.beforeId);
    t.history=value.history;t.index=value.index;t.scroll=value.scroll||0;
    // State is restored immediately; directory loading (including SMB) is allowed
    // to continue after acknowledgement. We transfer a tab, never file contents.
    t.pendingRestore={selection:value.selection||[],scroll:t.scroll};
    applyPendingTabRestore(t);renderNavigation();
    const result=await call('tabTransferReady',{token:data.token,accepted:true});
    if(!result.committed)settleIncomingTab({token:data.token,committed:false});
  }catch(e){
    try{await call('tabTransferReady',{token:data.token,accepted:false});}catch{}
    settleIncomingTab({token:data.token,committed:false});toast('Tab move failed. The original was kept. '+e.message);
  }
}
function applyPendingTabRestore(t){
  if(!t.pendingRestore||t.busy||t!==active())return;
  const value=t.pendingRestore;delete t.pendingRestore;state.selection=new Set(value.selection);state.filterCache=null;renderRows();$('file-scroll').scrollTop=value.scroll;updateToolbar();updateStatus();
}
async function windowsMenu(){
  const data=await call('windows');
  const b=$('windows-button').getBoundingClientRect();
  openMenu(Math.min(b.left,innerWidth-300),b.bottom+5,[
    ...data.map(w=>({label:w.title||'OpenXplorer',icon:w.active?'check':'desktop',fn:()=>call('focusWindow',{id:w.id})})),
    '-',{label:'New window',icon:'plus',shortcut:'Ctrl+N',fn:()=>call('newWindow',{uri:state.env.home})},
    {label:'Quit OpenXplorer',icon:'close',fn:()=>call('quit')}]);
}
async function handleFileManagerRequest(data){
  const method=data.method,uris=data.uris||[];if(!uris.length)return;
  // Never reinterpret a file URI as a directory. ShowItems means select in parent.
  if(method==='ShowFolders'){for(const uri of uris)addTab(uri);return;}
  if(method==='ShowItemProperties'){const uri=uris[0];addTab(parentUri(uri)||state.env.home);void propertiesDialog({uri,name:baseName(uri)});return;}
  const groups=new Map();for(const uri of uris){const parent=parentUri(uri)||uri;if(!groups.has(parent))groups.set(parent,[]);groups.get(parent).push(uri);}
  for(const [parent,items]of groups){let t=state.tabs.find(t=>sameLocation(t.uri,parent));if(t)switchTab(t.id);else t=addTab(parent);await load(t);if(t!==active())continue;
    state.selection=new Set(items);const rows=filtered(),index=rows.findIndex(e=>items.some(u=>sameLocation(u,e.uri)));
    if(index>=0)revealEntry(index);syncRowSelection();renderDetails();updateToolbar();updateStatus();}
}
function settingsSearch(query,scroll=false){
  state.settingsQuery=query;const words=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const results=$('settings-search-results');if(!results)return;results.replaceChildren();
  const body=document.querySelector('.settings-page>.settings-body');if(!body)return;
  for(const el of body.querySelectorAll('.settings-match,.settings-target'))el.classList.remove('settings-match','settings-target');
  if(!words.length){results.hidden=true;return;}results.hidden=false;
  const candidates=[...body.querySelectorAll('[data-settings-label]')];const matched=candidates.filter(el=>words.every(w=>(el.dataset.settingsLabel+' '+(el.dataset.searchTerms||'')+' '+el.textContent).toLocaleLowerCase().includes(w)));
  const seen=new Set();const unique=matched.filter(el=>{const label=el.dataset.settingsLabel;if(seen.has(label))return false;seen.add(label);return true;});
  results.append(elem('div','settings-result-count',unique.length?unique.length+' matching setting'+(unique.length===1?'':'s'):'No matching settings'));
  for(const el of unique){el.classList.add('settings-match');const item=button(el.dataset.settingsLabel,()=>{for(const n of body.querySelectorAll('.settings-target'))n.classList.remove('settings-target');el.classList.add('settings-target');el.scrollIntoView({block:'center',behavior:'smooth'});},'settings-search-result','chevron');results.append(item);}
  if(scroll&&unique.length){unique[0].classList.add('settings-target');unique[0].scrollIntoView({block:'center',behavior:'smooth'});}
}
function appendV07Settings(nav,body){
  const searchWrap=elem('div','settings-search-wrap');searchWrap.append(icon('search',16));
  const input=elem('input');input.type='search';input.id='settings-search-input';input.placeholder='Search settings';input.setAttribute('aria-label','Search settings');input.value=state.settingsQuery||'';searchWrap.append(input);
  const results=elem('div','settings-search-results');results.id='settings-search-results';results.setAttribute('aria-live','polite');results.hidden=true;
  nav.insertBefore(searchWrap,nav.querySelector('.settings-nav-link'));nav.insertBefore(results,nav.querySelector('.settings-nav-link'));
  input.oninput=()=>settingsSearch(input.value,true);input.onkeydown=e=>{if(e.key==='Enter'){e.preventDefault();results.querySelector('button')?.click();}if(e.key==='Escape'){input.value='';settingsSearch('');}};
  const settingsWindowLink=button('Windows & tabs',()=>{$('settings-windows').scrollIntoView({block:'start'});},'settings-nav-link','desktop');nav.insertBefore(settingsWindowLink,nav.querySelector('.settings-back'));
  nav.insertBefore(button('Brave & downloads',()=>{$('settings-brave').scrollIntoView({block:'start'});},'settings-nav-link','downloads'),nav.querySelector('.settings-back'));
  const def=$('settings-default');
  const revealOption=elem('label','checkbox-row');const reveal=elem('input');reveal.type='checkbox';reveal.checked=true;reveal.id='include-reveal';revealOption.append(reveal,document.createTextNode('Include Brave / other apps’ Show in folder integration (per-user, starts at login)'));def.append(revealOption);
  const zipOption=elem('label','checkbox-row');const zip=elem('input');zip.type='checkbox';zip.id='include-zip';zipOption.append(zip,document.createTextNode('Also open ZIP files in OpenXplorer (changes the archive association)'));def.append(zipOption);
  const zipStatus=elem('p','settings-help');zipStatus.id='zip-status';def.append(zipStatus);
  const zipControls=elem('div','integration-actions');const zipMake=button('Use OpenXplorer for ZIPs',()=>changeZipDefault('zipDefault'),'secondary');zipMake.id='zip-default';const zipRestore=button('Restore ZIP handler',()=>changeZipDefault('zipRestore'),'secondary');zipRestore.id='zip-restore';zipControls.append(zipMake,zipRestore,button('Refresh status',updateDefaultStatus,'secondary','refresh'));def.append(zipControls);
  const route=elem('p','settings-help');route.id='reveal-status';def.append(route);
  const routeControls=elem('div','integration-actions');routeControls.append(button('Enable Show in folder',async()=>{await call('revealEnable');await updateDefaultStatus();},'secondary'),button('Test Show in folder',async()=>{try{await call('revealTest');toast(native?'Test request sent through FileManager1.':'Simulated test; desktop settings were not changed.');}catch(e){toast(e.message);}},'secondary'),button('Disable Show in folder',async()=>{await call('revealDisable');await updateDefaultStatus();},'secondary'));def.append(routeControls);
  const guide=elem('details','zorin-guide');guide.append(elem('summary','','Zorin + Brave setup and troubleshooting'));
  for(const text of [
    '1. Click Make OpenXplorer default with Show in folder checked. Both folder/SMB handlers and the optional FileManager1 service are configured for your account.',
    '2. Close other file-manager windows. If Show in folder says waiting, log out of Zorin and log back in. OpenXplorer does not terminate Files or Dolphin.',
    '3. Clicking a ZIP filename in Brave opens its ZIP handler, not your folder handler. Use OpenXplorer for ZIPs above to change that association. Restart Brave after changing it.',
    '4. Downloads → Show in folder is a different action. Test Show in folder checks FileManager1, not Brave or its portal. The status must show OpenXplorer as the owner, not just enabled.',
    '4. Flatpak/Snap Brave or a remembered portal choice may still use another handler. In a chooser, select OpenXplorer. Do not disable your desktop portal: file-picker dialogs remain system dialogs.',
    '5. Pin the installed OpenXplorer folder icon to your Zorin panel. Right-click it → Open windows… lists existing windows; New window creates another. Super+E is a separate system keyboard shortcut.',
    'Restore previous removes OpenXplorer’s unmodified per-user reveal/autostart files and restores recorded file handlers. No system packages are removed.'
  ])guide.append(elem('p','settings-help',text));def.append(guide);
  const windows=elem('section','settings-section');windows.id='settings-windows';windows.append(elem('h2','','Windows & tabs'),elem('p','settings-help','Drag a tab onto another OpenXplorer window’s tab strip to merge it, or outside a window to detach it. Right-click a tab → Move tab to window… lets you pick an existing window without dragging. The original is kept until the destination accepts it. Close this tab’s dialogs and finish file operations first. Drag selected files or folders into another app to open or attach them. Drop files on a writable folder to copy them, or drop folders in Quick access to pin them. File drops never remove the source. ZIP contents must be extracted first; some apps need a mounted network path.'),button('Open windows…',windowsMenu,'secondary','desktop'),button('New window',()=>call('newWindow',{uri:state.env.home}),'secondary','plus'));body.append(windows);
  const brave=elem('section','settings-section');brave.id='settings-brave';brave.append(elem('h2','','Brave & downloads'),elem('p','settings-help','Use the Linux Downloads location in selected native Brave profiles. Fully quit Brave first, including background processes. OpenXplorer backs up Preferences and changes only the download and Save as directories. This is a one-time sync, not a managed browser policy.'),button('Use Linux Downloads in Brave…',()=>braveDialog(),'primary','downloads'),elem('p','settings-help','Flatpak/Snap, custom profiles, or managed browsers: open brave://settings/downloads and choose the same mounted Linux directory manually. SMB bookmarks are not persistent download paths.'));body.append(brave);
  const label=(selector,name,terms)=>{const el=body.querySelector(selector);if(el){el.dataset.settingsLabel=name;el.dataset.searchTerms=terms;}};
  label('#settings-appearance .settings-line','Theme & appearance','dark light system color');
  label('#context-menu-style','Right-click menu style','windows 10 11 classic context menu');
  label('#text-size-line','Text size','zoom font larger smaller ctrl + minus reset accessibility');
  label('#settings-appearance>.secondary','Reset layout widths','sidebar resize columns reset');
  label('#settings-search .cache-add','Add a custom folder to the cache','directory path add local disk smb nas search indexing');
  label('#settings-search>.checkbox-row','Watch folders for live changes','inotify automatic file updates');
  label('#settings-search .settings-line','Network refresh interval','smb polling nas seconds minute');
  label('#settings-cache-list','Folders selected for search indexing','cache entire drive 2tb local disk');
  label('#settings-default','Default file explorer & Show in folder','brave zorin nautilus dolphin default reveal filemanager1 zip archive download');
  label('#settings-brave','Brave download location','downloads sync save as browser');
  label('#settings-windows','Windows & tabs','taskbar panel detach drag separate window');
  label('#settings-sizes','Calculate folder sizes','zfs logical bytes snapshots');
  setTimeout(()=>settingsSearch(state.settingsQuery||''),0);
}
async function braveDialog(destination=null){
  destination=destination||state.env.knownFolders?.folders?.find(f=>f.key==='DOWNLOAD')?.path||state.env.knownFolders?.find?.(f=>f.key==='DOWNLOAD')?.path||displayUri(state.env.quick.find(q=>q.label==='Downloads')?.uri||state.env.home);
  let profiles=[],choices=[],status,consent,applyButton;
  return showModal('Use this Downloads folder in Brave','One-time sync. Quit Brave before applying. Your browser’s other settings are left alone.',body=>{
    body.classList.add('brave-dialog');body.append(elem('div','brave-destination',destination));
    status=elem('p','settings-help','Checking native Brave profiles…');body.append(status);
    const list=elem('div','brave-profile-list');body.append(list);
    const confirm=elem('label','checkbox-row');consent=elem('input');consent.type='checkbox';consent.id='confirm-brave';confirm.append(consent,document.createTextNode('Back up and update the selected Brave profiles.'));body.append(confirm);
    const load=async()=>{try{const d=await call('braveStatus');if(!list.isConnected)return;profiles=d.profiles;choices=[];list.replaceChildren();for(const p of profiles){const row=elem('label','brave-profile');const cb=elem('input');cb.type='checkbox';cb.checked=true;cb.dataset.profileId=p.id;choices.push(cb);const text=elem('div');text.append(elem('strong','',p.name+' · '+p.flavor),elem('span','',p.downloadPath||'Uses browser default'));row.append(cb,text);list.append(row);}status.textContent=d.running?'Brave is running. Quit it completely, then Recheck.':profiles.length?'Brave is closed. Select the profiles to update.':'No supported native profiles found. Set brave://settings/downloads manually.';if(d.sandboxed?.length)status.textContent+=' '+d.sandboxed.join(' / ')+' installations need manual browser settings.';}
      catch(e){status.textContent=e.message;}};
    body.append(button('Recheck profiles',load,'secondary','refresh'));body.append(elem('p','settings-help','Only detected native profiles are changed. Passwords and account data are not shown here. Backups remain private in your OpenXplorer configuration directory.'));void load();
    return {};
  },[{label:'Cancel',cancel:true},{label:'Restore previous',fn:async()=>{const selected=choices.filter(c=>c.checked);if(!consent.checked||selected.length!==1){status.textContent='Select one profile and confirm to restore its previous download setting.';return false;}await call('braveRestore',{profile:selected[0].dataset.profileId,confirmed:true});toast('Previous Brave download setting restored.');return true;}},{label:'Apply to Brave',className:'primary',fn:async(_f,b)=>{
    if(!consent.checked){status.textContent='Confirm the change using the checkbox.';return false;}const ids=choices.filter(c=>c.checked).map(c=>c.dataset.profileId);if(!ids.length){status.textContent='Select at least one profile.';return false;}
    b.disabled=true;try{const result=await call('braveSync',{profiles:ids,path:destination,confirmed:true});if(result.errors?.length){status.textContent=result.updated.length+' updated. '+result.errors.map(e=>e.message).join(' ');return false;}toast(native?'Brave Downloads updated for '+result.updated.length+' profile(s).':'Preview only — Brave settings were not changed.');return true;}catch(e){status.textContent=e.message;return false;}finally{b.disabled=false;}
  }}]);
}

window.OpenXplorer={moveTabMenu,moveTabToWindow,receiveTransferredTab,settleIncomingTab,finishTabTransfer,reorderTab,renderDefaultStatus,changeZipDefault,extractDialog,changeTextSize,applyTextSize,isZipEntry,zipFolderIcon,prepareTabTransfer,detachTab,restoreTransferredTab,settingsSearch,windowsMenu,handleFileManagerRequest,networkLocations,braveDialog,previewTransport:native?null:demo,snapshotFor,renderTabs,renderSidebar,closeModal,icon,folderIcon,fileIcon,breadcrumbSegments,deviceLocation,displayUri,networkLocation,uniqueEditors,applyLayout,resetLayout,switchTab,closeTab,scanFolderSizes,stopSizeScan,state,navigate,addTab,refreshEnvironment,call,normaliseAddress,validateName,applyTheme,settingsDialog,authPreview,receiveAuth,dismissAuth,runSearch,refreshCacheStatus,discoverNetwork,signOut,goHistory,propertiesDialog,openWithDialog,entryMenu,newTemplateDialog,openNewMenu,readonlyLocation,openEntry,archiveDialog,copySelection,paste,refreshClipboard};
start();
})();
