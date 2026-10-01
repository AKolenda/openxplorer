// SPDX-License-Identifier: AGPL-3.0-only
import {Icon} from './icons';
// Template children live in HTMLTemplateElement.content, outside React's child
// hydration traversal. Keep this fixed, script-only sandbox markup opaque so
// hydration preserves it for the desktop-only loader in interactions.js.
// The tour shows pictures of the real native app (tools/capture-native-tour.py);
// a GTK app cannot run in a web page.
const DEMO_FRAME_HTML='<iframe name="openxplorer-website-demo" title="Click-through tour of the real OpenXplorer app with sample files" src="/tour/index.html?embed=1&amp;theme=light" sandbox="allow-scripts" loading="lazy" referrerpolicy="no-referrer"></iframe>';
export function Screenshot({name,alt,caption,className='',priority=false}:{name:string;alt:string;caption?:string;className?:string;priority?:boolean}){
 return <figure className={'product-shot '+className}><img src={'/assets/screenshots/'+name+'.png'} alt={alt} loading={priority?'eager':'lazy'} fetchPriority={priority?'high':undefined} decoding="async" width={1440} height={900}/>{caption&&<figcaption>{caption}</figcaption>}</figure>;
}
export function ProductDemo({compact=false}:{compact?:boolean}){
 return <div className={'live-product desktop-demo-only '+(compact?'compact-demo':'')} data-product-demo="">
  <div className="demo-heading"><div><strong>Tour the real OpenXplorer app.</strong><span>Pictures of the native GTK 4 app with fictional sample files. Click a control in a picture to see where it leads.</span></div><a href="/tour/index.html?theme=light" target="_blank" rel="noreferrer">Open full-size tour <Icon name="share" size={16}/></a></div>
  <div className="demo-controls"><button className="demo-play" data-demo-command="play"><Icon name="play" size={17}/> Watch: a file’s menu and previous versions</button><button data-demo-command="stop" hidden>Stop tour</button><div className="demo-scenes"><button data-demo-command="scene" data-demo-value="home">Home</button><button data-demo-command="scene" data-demo-value="context-menu">Context menu</button><button data-demo-command="scene" data-demo-value="network">Network</button><button data-demo-command="scene" data-demo-value="settings">Settings</button></div><button data-demo-command="theme" data-demo-value="dark" aria-label="Switch tour to dark mode"><Icon name="moon" size={16}/><span>Dark</span></button><button data-demo-command="reset" aria-label="Restart the tour"><Icon name="refresh" size={16}/></button></div>
  <div className="demo-viewport"><template data-demo-template="" dangerouslySetInnerHTML={{__html:DEMO_FRAME_HTML}}/></div>
  <p className="demo-status" data-demo-status="" role="status">Click a control in the picture, or use the buttons above. The pictures cannot reach your files, NAS or credentials.</p>
 </div>;
}
const CURSOR=<svg viewBox="0 0 28 34"><path d="M3 2 25 18l-10 2 6 9-5 3-6-10-7 7Z"/></svg>;
/** CSS-only guided tour over a crop of the real native app's screenshot. Each
 *  click target and cursor share one coordinate system: percentages of the
 *  cropped frame. */
export function ProductTour(){
 return <figure className="product-tour" aria-labelledby="tour-caption">
  <div className="tour-board">
   <div className="tour-topline"><span>Guided tour</span><span>9 seconds</span></div>
   <div className="tour-stage">
    <div className="tour-frame"><img src="/assets/screenshots/explorer-light.png" alt="OpenXplorer showing the sample Documents folder, with Launch planning selected and pinned to Quick access in the sidebar" width={1440} height={900} fetchPriority="high"/></div>
    <span className="click-target target-path" aria-hidden="true"/>
    <span className="click-target target-folder" aria-hidden="true"/>
    <span className="click-target target-pin" aria-hidden="true"/>
    <span className="tour-cursor cursor-path" aria-hidden="true">{CURSOR}<b>Type a share path</b></span>
    <span className="tour-cursor cursor-folder" aria-hidden="true">{CURSOR}<b>Open a folder</b></span>
    <span className="tour-cursor cursor-pin" aria-hidden="true">{CURSOR}<b>Drag it to Quick access</b></span>
   </div>
  </div>
  <figcaption id="tour-caption"><span>Path</span><i/><span>Folder</span><i/><span>Pin</span></figcaption>
 </figure>;
}
