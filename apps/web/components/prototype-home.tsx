// SPDX-License-Identifier: AGPL-3.0-only
// PROTOTYPE — throwaway. Three structurally different homepages on the existing `/`
// route, switchable via `?variant=A|B|C` (and `?variant=current` for today's page).
// Question being answered: what should the homepage look like with ONE consistent
// width system, and how should the SMB story lead? Hidden in production builds.
'use client';
import {Suspense,useEffect} from 'react';
import {useSearchParams} from 'next/navigation';
import {site} from '../lib/site';
import {Header,Footer,GlobalSearch,Home} from './site';
import {Icon} from './icons';
import {Screenshot,ProductDemo} from './product';
import '../public/assets/prototype-home.css';

const VARIANTS=[
 ['current','Today’s page'],
 ['A','Ledger — one column, headings only'],
 ['B','Workbench — sticky address bar, scrolling proof'],
 ['C','Grid — dark hero, 12-column tiles'],
] as const;
type Key=typeof VARIANTS[number][0];

/* Facts about the SMB layer, taken from desktop/core.py and the docs. Keep truthful. */
const ADDRESSES=[
 ['\\\\studio-nas\\Projects','UNC path, by name'],
 ['\\\\10.0.0.1\\share','UNC path, by IP address'],
 ['//archive-nas/Shared','Forward-slash UNC'],
 ['smb://nas:1445/share','smb:// URL, custom port'],
 ['..\\Design','Relative to the share you’re in'],
];
const SMB_POINTS=[
 ['Type it the Windows way.','Backslashes, IP addresses, forward slashes or smb:// URLs. Any of them lands you in the share. No mapping a drive letter first.'],
 ['Sign in once per server.','Credentials are scoped to the server and port, then reused across its shares. Keep them in the system keyring, or only for this login session.'],
 ['Never in the address.','A username or password typed into the path is refused outright. The sign-in dialog is the only place secrets go.'],
 ['Aliases stay separate.','nas, nas.local and 10.0.0.1 are treated as different servers on purpose, so a saved password never leaks to a look-alike host.'],
 ['Breadcrumbs all the way up.','Every ancestor of \\\\server\\share\\deep\\folder is its own clickable button. Pin any level to the sidebar.'],
 ['Snapshots, with dates.','Browse exposed NAS previous versions from the same tab, with the date beside each one.'],
];

function AddressBar({value,caption}:{value:string;caption?:string}){
 return <div className="proto-address" aria-label="Example address"><Icon name="network" size={15}/><code>{value}</code><span><Icon name="arrow" size={14}/></span>{caption&&<small>{caption}</small>}</div>;
}
function ReleaseCta({center=false}:{center?:boolean}){
 return <div className={'proto-cta'+(center?' center':'')}><a className="button primary" href={site.releases}><Icon name="code" size={18}/> View releases</a><a className="button secondary desktop-demo-link" href="#demo">Try it in your browser</a><small>{site.version} · AGPL-3.0-only · Zorin OS, Ubuntu and Debian</small></div>;
}

/* ───────────────── Variant A · Ledger ─────────────────
   One column. Text sits on a 680px measure, images on the full 1152px shell.
   No cards anywhere. Features are numbered chapters with alternating shots. */
export function VariantA(){
 return <div className="site theme-zorin proto proto-a"><Header/><main id="main">
  <section className="shell a-hero"><h1>A familiar place.<br/>A fresh start.</h1><p className="lede">Windows File Explorer habits, native Linux storage. Type <code>\\nas\share</code> and you’re there.</p><ReleaseCta center/></section>
  <section className="shell a-shot"><Screenshot name="explorer-light" alt="Actual OpenXplorer interface in light mode showing Projects on a sample NAS" priority caption="The actual interface. Entirely fictional sample files."/></section>
  <section className="shell a-chapter" id="features"><div className="measure"><h2>Your NAS, addressed like you always have.</h2><p>The address bar accepts the paths you already know from Windows, plus the Linux ones. Names, IP addresses, forward or back slashes, smb:// URLs with a port. One sign-in per server, stored where you choose.</p></div><div className="a-addresses">{ADDRESSES.map(([v,c])=><AddressBar key={v} value={v} caption={c}/>)}</div><Screenshot name="network-path" className="crop wide" alt="Actual OpenXplorer breadcrumb buttons for studio-nas, Projects and Design" caption="Each breadcrumb of a share is its own button."/></section>
  <section className="shell a-chapter"><div className="measure"><h2>Drag it where you need it.</h2><p>Drag selected files into compatible editors and attachment fields. Drop onto a folder to copy, or into Quick access to pin it. Sources stay in place.</p><a className="text-link" href="/docs/interface/#file-drag-drop">File dragging &amp; compatibility <Icon name="arrow" size={16}/></a></div><Screenshot name="pinned-sidebar" className="crop tall" alt="Actual OpenXplorer sidebar with Design pinned and a green network indicator"/></section>
  <section className="shell a-chapter"><div className="measure"><h2>Find the file. Keep the path.</h2><p>Opt selected folders into a local filename cache. Search locally or on a share, then open the actual location instead of another search maze.</p><a className="text-link" href="/docs/search-indexing/">How cached search works <Icon name="arrow" size={16}/></a></div><Screenshot name="cached-search" className="crop" alt="Real cached search results with filenames and full SMB folder paths"/></section>
  <section className="shell a-chapter"><div className="measure"><h2>The right moment, clearly marked.</h2><p>Browse exposed NAS snapshots with the date beside each version. Historical tabs carry a Previous version badge so you always know where you are.</p><a className="text-link" href="/docs/interface/#snapshots">Explore previous versions <Icon name="arrow" size={16}/></a></div><Screenshot name="previous-versions" className="crop mid" alt="Previous versions of the fictional Launch planning folder, with compact dates and separate actions"/></section>
  <section id="demo" className="shell a-chapter desktop-demo-only"><div className="measure"><h2>Take the familiar route.</h2><p>Type a path. Open the NAS. Pin a folder. Try the interface before installing anything.</p></div><ProductDemo/></section>
  <section className="shell a-end"><div className="measure"><h2>Yours to use. Yours to understand.</h2><p>The desktop app, this website, the docs and the build tools are all public under AGPL-3.0-only. Nothing behind a login.</p><ReleaseCta/></div></section>
 </main><Footer/><GlobalSearch/></div>;
}

/* ───────────────── Variant B · Workbench ─────────────────
   Two columns on the same shell: a sticky left rail (pitch + address bar +
   chapter index) and a right column that scrolls through the proof. */
export function VariantB(){
 const chapters=[['smb','Any address'],['drag','Drag & pin'],['search','Search'],['versions','Previous versions'],['demo','Try it']];
 return <div className="site theme-zorin proto proto-b"><Header/><main id="main" className="shell b-layout">
  <aside className="b-rail"><p className="eyebrow">Made for your next chapter</p><h1>Your files.<br/>Your NAS.<br/>No detour.</h1><p className="lede">A Windows-Explorer-shaped file manager for Linux. Type the share the way you always have.</p><div className="b-typed"><AddressBar value={'\\\\10.0.0.1\\share'} caption="works · so does smb://, a name, or a port"/></div><ReleaseCta/><nav className="b-index" aria-label="Sections">{chapters.map(([id,label],i)=><a key={id} href={'#'+id}><span>0{i+1}</span>{label}</a>)}</nav></aside>
  <div className="b-proof">
   <Screenshot name="explorer-light" alt="Actual OpenXplorer interface in light mode showing Projects on a sample NAS" priority caption="The actual interface. Entirely fictional sample files."/>
   <section id="smb" className="b-block"><h2>Type any address. Land in the share.</h2><p>Names, IPs, backslashes, forward slashes, smb:// with a port. Credentials go in the sign-in dialog, never in the path, and are reused across a server’s shares.</p><table className="b-table"><tbody>{ADDRESSES.map(([v,c])=><tr key={v}><td><code>{v}</code></td><td>{c}</td><td><Icon name="check" size={16}/></td></tr>)}</tbody></table><ul className="b-facts">{SMB_POINTS.slice(1,4).map(([t,d])=><li key={t}><strong>{t}</strong> {d}</li>)}</ul><Screenshot name="network-path" className="crop wide" alt="Actual OpenXplorer breadcrumb buttons for studio-nas, Projects and Design"/></section>
   <section id="drag" className="b-block"><h2>Drag it where you need it.</h2><p>Files into compatible editors and attachment fields. Folders into Quick access to pin them. Sources stay in place.</p><Screenshot name="pinned-sidebar" className="crop tall" alt="Actual OpenXplorer sidebar with Design pinned and a green network indicator"/></section>
   <section id="search" className="b-block"><h2>Find the file. Keep the path.</h2><p>Opt folders into the filename cache, search a local disk or a share, open the real location.</p><Screenshot name="cached-search" className="crop" alt="Real cached search results with filenames and full SMB folder paths"/></section>
   <section id="versions" className="b-block"><h2>The right moment, clearly marked.</h2><p>Exposed NAS snapshots with a date beside each version and a badge on historical tabs.</p><Screenshot name="previous-versions" className="crop mid" alt="Previous versions of the fictional Launch planning folder, with compact dates and separate actions"/></section>
   <section id="demo" className="b-block desktop-demo-only"><h2>Take the familiar route.</h2><p>Type a path, open the NAS, pin a folder. Nothing to install.</p><ProductDemo compact/></section>
   <section className="b-block b-open"><span className="eyebrow">Open by choice</span><h2>Yours to use. Yours to understand.</h2><p>App, website, docs and tools: all public, AGPL-3.0-only.</p><a className="button secondary" href={site.repository}><Icon name="code" size={17}/> Browse the GitHub repository</a></section>
  </div>
 </main><Footer/><GlobalSearch/></div>;
}

/* ───────────────── Variant C · Grid ─────────────────
   Full-bleed dark hero band with contained text; everything below sits on a
   strict 12-column grid with one gutter. Tiles span columns, never freelance widths. */
export function VariantC(){
 return <div className="site theme-zorin proto proto-c"><Header/><main id="main">
  <section className="c-hero"><div className="shell c-hero-inner"><div><p className="eyebrow">Made for your next chapter</p><h1>The Explorer you know.<br/>The NAS you own.</h1><p className="lede">Type <code>\\ip\share</code>, <code>\\name\share</code> or <code>smb://</code>. Sign in once. Browse it next to your local files.</p><ReleaseCta/></div><Screenshot name="explorer-dark" alt="Actual OpenXplorer interface in dark mode" priority/></div></section>
  <section className="shell c-grid" id="features">
   <article className="c-tile span-12 c-smb"><div><Icon name="network" size={26}/><h2>Every way you’ve ever typed a share. All of them work.</h2><p>No drive-letter mapping, no separate connect dialog. The address bar parses UNC paths by name or IP, forward-slash paths, and smb:// URLs with a port, then signs you in with a prompt that is scoped to that server.</p></div><div className="c-chips">{ADDRESSES.map(([v,c])=><div key={v}><code>{v}</code><span>{c}</span></div>)}</div></article>
   {SMB_POINTS.slice(1).map(([t,d])=><article key={t} className="c-tile span-4 c-fact"><strong>{t}</strong><p>{d}</p></article>)}
   <article className="c-tile span-6 c-shot"><Screenshot name="network-path" className="crop wide" alt="Actual OpenXplorer breadcrumb buttons for studio-nas, Projects and Design"/><h3>Breadcrumbs for shares</h3><a href="/docs/network-shares/">Connect your network storage <Icon name="arrow" size={14}/></a></article>
   <article className="c-tile span-6 c-shot"><Screenshot name="pinned-sidebar" className="crop tall" alt="Actual OpenXplorer sidebar with Design pinned and a green network indicator"/><h3>Drag to pin. Drag to copy.</h3><a href="/docs/interface/#file-drag-drop">File dragging &amp; compatibility <Icon name="arrow" size={14}/></a></article>
   <article className="c-tile span-6 c-shot"><Screenshot name="cached-search" className="crop" alt="Real cached search results with filenames and full SMB folder paths"/><h3>Search that keeps the path</h3><a href="/docs/search-indexing/">How cached search works <Icon name="arrow" size={14}/></a></article>
   <article className="c-tile span-6 c-shot"><Screenshot name="previous-versions" className="crop mid" alt="Previous versions of the fictional Launch planning folder, with compact dates and separate actions"/><h3>Previous versions, dated</h3><a href="/docs/interface/#snapshots">Explore previous versions <Icon name="arrow" size={14}/></a></article>
  </section>
  <section id="demo" className="shell c-demo desktop-demo-only"><div className="c-demo-head"><h2>Take the familiar route.</h2><p>Type a path. Open the NAS. Pin a folder.</p></div><ProductDemo/></section>
  <section className="shell c-grid c-bottom"><div className="c-tile span-7 c-open"><span className="eyebrow">Open by choice</span><h2>Yours to use. Yours to understand.</h2><p>App, website, docs and build tools, all public under AGPL-3.0-only.</p><a className="button secondary" href={site.repository}><Icon name="code" size={17}/> Browse the GitHub repository</a></div><div className="c-tile span-5 c-get"><span className="eyebrow">Development release {site.version}</span><h2>Get OpenXplorer.</h2><p>Zorin OS and compatible Ubuntu or Debian systems.</p><a className="button primary" href={site.releases}><Icon name="code" size={18}/> View releases on GitHub</a></div></section>
 </main><Footer/><GlobalSearch/></div>;
}

/* ───────────────── Switcher ───────────────── */
function go(key:Key){const u=new URL(window.location.href);u.searchParams.set('variant',key);u.hash='';window.location.assign(u.toString());}
export function PrototypeSwitcher({current}:{current:Key}){
 const i=VARIANTS.findIndex(v=>v[0]===current),prev=VARIANTS[(i-1+VARIANTS.length)%VARIANTS.length][0],next=VARIANTS[(i+1)%VARIANTS.length][0];
 useEffect(()=>{
  const h=(e:KeyboardEvent)=>{const t=e.target as HTMLElement|null;if(t&&(t.closest('input,textarea,[contenteditable]')))return;if(e.key==='ArrowLeft')go(prev);if(e.key==='ArrowRight')go(next);};
  window.addEventListener('keydown',h);return()=>window.removeEventListener('keydown',h);
 },[prev,next]);
 if(process.env.NODE_ENV==='production')return null;
 return <div className="proto-switcher" role="group" aria-label="Prototype variants"><button onClick={()=>go(prev)} aria-label="Previous variant">←</button><span><b>{current}</b> {VARIANTS[i][1]}</span><button onClick={()=>go(next)} aria-label="Next variant">→</button></div>;
}
function Inner(){
 const q=useSearchParams().get('variant');
 const key=(VARIANTS.some(v=>v[0]===q)?q:'A') as Key;
 return <>{key==='current'&&<Home vibe={site.defaultDesign}/>}{key==='A'&&<VariantA/>}{key==='B'&&<VariantB/>}{key==='C'&&<VariantC/>}<PrototypeSwitcher current={key}/></>;
}
export function PrototypeHome(){return <Suspense fallback={<div className="site theme-zorin" style={{minHeight:'100vh'}}/>}><Inner/></Suspense>;}
