// SPDX-License-Identifier: AGPL-3.0-only
import {Icon} from './icons';
// Template children live in HTMLTemplateElement.content, outside React's child
// hydration traversal. Keep this fixed, script-only sandbox markup opaque so
// hydration preserves it for the desktop-only loader in interactions.js.
const DEMO_FRAME_HTML='<iframe name="openxplorer-website-demo" title="Interactive OpenXplorer demo with simulated files" src="/app-preview.html?embed=1&amp;theme=light&amp;scene=home" sandbox="allow-scripts" loading="lazy" referrerpolicy="no-referrer"></iframe>';
export function Screenshot({name,alt,caption,className='',priority=false}:{name:string;alt:string;caption?:string;className?:string;priority?:boolean}){
 return <figure className={'product-shot '+className}><img src={'/assets/screenshots/'+name+'.png'} alt={alt} loading={priority?'eager':'lazy'} fetchPriority={priority?'high':undefined} decoding="async" width={1440} height={900}/>{caption&&<figcaption>{caption}</figcaption>}</figure>;
}
export function ProductDemo({compact=false}:{compact?:boolean}){
 return <div className={'live-product desktop-demo-only '+(compact?'compact-demo':'')} data-product-demo="">
  <div className="demo-heading"><div><strong>Try the OpenXplorer interface.</strong><span>The desktop app’s controls with fictional sample files. File access and dragging into other apps require the desktop build.</span></div><a href="/app-preview.html?embed=1&amp;theme=light" target="_blank" rel="noreferrer">Open full-size preview <Icon name="share" size={16}/></a></div>
  <div className="demo-controls"><button className="demo-play" data-demo-command="play"><Icon name="play" size={17}/> Watch: open NAS &amp; pin a folder</button><button data-demo-command="stop" hidden>Stop tour</button><div className="demo-scenes"><button data-demo-command="scene" data-demo-value="home">Local files</button><button data-demo-command="scene" data-demo-value="nas">Sample NAS</button><button data-demo-command="scene" data-demo-value="snapshots">Previous versions</button></div><button data-demo-command="theme" data-demo-value="dark" aria-label="Switch preview to dark mode"><Icon name="moon" size={16}/><span>Dark</span></button><button data-demo-command="reset" aria-label="Reset sample preview"><Icon name="refresh" size={16}/></button></div>
  <div className="demo-viewport"><template data-demo-template="" dangerouslySetInnerHTML={{__html:DEMO_FRAME_HTML}}/></div>
  <p className="demo-status" data-demo-status="" role="status">Double-click a folder, try a right-click, or drag a folder to the sidebar. No access to your files, NAS or credentials.</p>
 </div>;
}
export function BentoFeatures(){
 return <section id="features" className="section product-features"><div className="section-heading"><h2>Less looking around.<br/>More getting things done.</h2><p>Local folders and the NAS.<br/>Finally speaking the same language.</p></div>
  <div className="product-bento">
   <article className="bento-network"><div className="bento-copy"><Icon name="network" size={27}/><h3>Your NAS belongs here.</h3><p>Native SMB access through Linux’s GIO/GVfs. Open a share with a Windows-style path, sign in, and browse it alongside your local files.</p><code className="unc-example">{'\\\\studio-nas\\Projects'}</code><a href="/docs/network-shares/">Connect your network storage</a></div><Screenshot name="network-path" alt="Actual OpenXplorer breadcrumb buttons for studio-nas, Projects and Design"/></article>
   <article className="bento-pins"><div className="bento-copy"><Icon name="pin" size={24}/><h3>Drag it where you need it.</h3><p>Drag selected files into compatible editors and attachment fields. Drop onto a folder to copy, or into Quick access to pin a folder. Sources stay in place.</p><a href="/docs/interface/#file-drag-drop">File dragging &amp; compatibility</a><a href="#demo" data-play-tour="" className="desktop-demo-link">Watch folder pinning</a></div><Screenshot name="pinned-sidebar" alt="Actual OpenXplorer sidebar with Design pinned and a green network indicator"/></article>
   <article className="bento-search"><div className="bento-copy"><Icon name="search" size={24}/><h3>Find the file. Keep the path.</h3><p>Opt selected folders into the filename cache. Search locally or on a share, then open the actual location.</p><a href="/docs/search-indexing/">How cached search works</a></div><Screenshot name="cached-search" alt="Real cached search results with filenames and full SMB folder paths"/></article>
   <article className="bento-versions"><div className="bento-copy"><Icon name="clock" size={24}/><h3>The right moment, clearly marked.</h3><p>Browse exposed NAS snapshots with dates beside each version. Historical tabs carry a Previous version badge so you know where you are.</p><a href="/docs/interface/#snapshots">Explore previous versions</a></div><Screenshot name="previous-versions" alt="Previous versions of the fictional Launch planning folder, with compact dates and separate actions"/></article>
  </div>
  <p className="product-disclosure">Screenshots use the real interface with fictional files. Native file dragging is available in the desktop build; the browser preview simulates folder pinning. Network files may need an existing local mount for apps that only accept local files.</p>
 </section>;
}
