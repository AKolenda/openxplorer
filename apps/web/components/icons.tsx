// SPDX-License-Identifier: AGPL-3.0-only
import type { CSSProperties } from 'react';
export function Icon({name = 'folder',size = 20,className = '',style}:{name?:string;size?:number;className?:string;style?:CSSProperties}) {
 const paths: Record<string, string[]> = {
  play:['M8 4v16l13-8z'], moon:['M20 15A9 9 0 0 1 9 4a9 9 0 1 0 11 11Z'], refresh:['M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1'], clock:['M12 7v6l4 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0'], pin:['m16 2 6 6-3 1-4 5v4l-5-5-6 8 4-10-3-3h4l6-4z'], share:['M14 3h7v7m0-7-11 11M10 5H3v16h16v-7'], arrow:['M5 12h14m-5-5 5 5-5 5'], back:['M19 12H5m5-5-5 5 5 5'],
  chevron:['m9 5 7 7-7 7'], down:['m6 9 6 6 6-6'],
  download:['M12 3v12m-5-5 5 5 5-5','M4 16v4h16v-4'],
  code:['m8 6-6 6 6 6m8-12 6 6-6 6m-3-15-2 18'],
  search:['M21 21l-5-5','M18 10a8 8 0 1 0-16 0 8 8 0 0 0 16 0'],
  terminal:['m5 7 5 5-5 5m8 0h6'],
  network:['M8 2h8v6H8zM2 16h6v6H2zM16 16h6v6h-6zM12 8v5M5 16v-3h14v3'],
  server:['M3 3h18v7H3zM3 14h18v7H3zM6 6h1m-1 11h1'],
  book:['M3 3h6a3 3 0 0 1 3 3v15a4 4 0 0 0-4-2H3zm18 0h-6a3 3 0 0 0-3 3v15a4 4 0 0 1 4-2h5z'],
  copy:['M8 8h13v13H8zM16 8V3H3v13h5'],
  check:['m4 12 5 5L20 6'],
  home:['m3 10 9-7 9 7v11h-6v-7H9v7H3z'],
  file:['M5 2h9l5 5v15H5zM14 2v6h5M8 12h8m-8 4h6'],
  grid:['M3 3h6v6H3zM15 3h6v6h-6zM3 15h6v6H3zM15 15h6v6h-6z'],
  lock:['M5 10h14v11H5zM8 10V6a4 4 0 0 1 8 0v4m-4 4v3'],
  globe:['M21 12a9 9 0 1 0-18 0 9 9 0 0 0 18 0M3 12h18M12 3c5 5 5 13 0 18-5-5-5-13 0-18'],
  star:['m12 2 3 6 7 1-5 5 1 7-6-3-6 3 1-7-5-5 7-1z'],
  menu:['M4 6h16M4 12h16M4 18h16'], close:['m6 6 12 12M6 18 18 6'],
  disk:['M5 3h14l3 13v5H2v-5zM2 16h20m-4 2h1'],
  sun:['M12 2v2m0 16v2M2 12h2m16 0h2M5 5l2 2m10 10 2 2M5 19l2-2M17 7l2-2','M16 12a4 4 0 1 0-8 0 4 4 0 0 0 8 0'],
  info:['M12 11v6m0-10v.01','M22 12a10 10 0 1 0-20 0 10 10 0 0 0 20 0'],
  branch:['M6 3v13a4 4 0 0 0 8 0v-5m-8-4h12M4 3a2 2 0 1 0 4 0 2 2 0 0 0-4 0M16 7a2 2 0 1 0 4 0 2 2 0 0 0-4 0'],
  speed:['M3 18a10 10 0 1 1 18 0m-9-5 5-5M12 13v5'],
  folder:['M2 5h8l2 3h10v12H2z'],
 };
 return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className={className} style={style} aria-hidden="true">{(paths[name]||paths.folder).map((d,i)=><path key={i} d={d}/>)}</svg>;
}
export function Folder({network=false,className=''}:{network?:boolean;className?:string}) {
 return <span className={'folder-mark '+className}><svg viewBox="0 0 28 25" aria-hidden="true"><path d="M1 6a2 2 0 0 1 2-2h7l3 3h12a2 2 0 0 1 2 2v13H1z" fill="#D7991C"/><path d="M2 9h24v3H2z" fill="#FFF0A7"/><path d="M1 12a2 2 0 0 1 2-2h22a2 2 0 0 1 2 2l-1 9a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2z" fill="#FBC546"/><path d="M3 11h22" stroke="#FFE99A" strokeWidth="1.5"/></svg>{network&&<i/>}</span>;
}
