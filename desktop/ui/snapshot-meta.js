// SPDX-License-Identifier: AGPL-3.0-only
/* Display metadata, not a ZFS creation-time API. Never call folder mtime the snapshot date. */
(function(root,factory){const api=factory();if(typeof module==='object'&&module.exports)module.exports=api;else root.OpenXplorerSnapshots=api;})(typeof window!=='undefined'?window:globalThis,function(){
 'use strict';
 function parse(label){
  label=String(label||'');let m,parts,utc=false,precision='minute';
  if((m=label.match(/^@GMT-(\d{4})\.(\d{2})\.(\d{2})-(\d{2})\.(\d{2})\.(\d{2})(?:$|[^\d])/))){parts=m.slice(1,7).map(Number);utc=true;precision='second';}
  else if((m=label.match(/(?:^|[^\d])(\d{4})-(\d{2})-(\d{2})(?:[T_ -](\d{2})(?:[:-]?(\d{2}))(?:(?:[:-]?)(\d{2}))?)?(?:$|[^\d])/))){parts=[...m.slice(1,4).map(Number),Number(m[4]||0),Number(m[5]||0),Number(m[6]||0)];precision=m[6]?'second':m[4]?'minute':'day';}
  else return null;
  const [y,mo,d,h,mi,se]=parts;
  if(y<1970||y>9999||mo<1||mo>12||d<1||d>31||h>23||mi>59||se>59)return null;
  const check=new Date(Date.UTC(y,mo-1,d,h,mi,se));
  if(check.getUTCFullYear()!==y||check.getUTCMonth()!==mo-1||check.getUTCDate()!==d)return null;
  const pad=n=>String(n).padStart(2,'0'),date=`${y}-${pad(mo)}-${pad(d)}`,time=`${pad(h)}:${pad(mi)}`+(precision==='second'?`:${pad(se)}`:'');
  return{date,time:precision==='day'?'':time+(utc?' UTC':''),iso:date+(precision==='day'?'':'T'+time+(utc?'Z':'')),utc,precision,parts};
 }
 function describe(version,locale){
  const date=parse(version?.label);
  if(date){const [y,m,d]=date.parts;return {...date,date:new Intl.DateTimeFormat(locale,{year:'numeric',month:'short',day:'numeric',timeZone:'UTC'}).format(new Date(Date.UTC(y,m-1,d))),sourceLabel:'From snapshot name',explanation:date.utc?'Date encoded in the @GMT snapshot name, shown in UTC.':'Date encoded in the snapshot name. Timezone was not supplied by the server; no conversion has been applied.'};}
  // mtime is intentionally NOT used as a fallback; it can describe the live folder, not snapshot creation.
  return{date:'Date unavailable',time:'',iso:null,sourceLabel:'No date in name',explanation:'This snapshot has no recognized date in its name. Folder modification times do not establish snapshot creation time.'};
 }
 function within(uri,root){return uri.replace(/\/$/,'')===root.replace(/\/$/,'')||uri.startsWith(root.replace(/\/$/,'')+'/');}
 function location(uri,roots=[]){
  try{const u=new URL(uri),encoded=u.pathname.split('/'),parts=encoded.map(decodeURIComponent);
   let end=-1;
   for(let i=0;i<parts.length;i++){
    if(['.snapshot','.snapshots','#snapshot'].includes(parts[i])){end=i+1;if(!parts[end])return {root:uri,label:'Snapshot collection'};if(parts[i]==='.snapshots'&&parts[end+1]==='snapshot')end++;break;}
    if(parts[i]==='.zfs'&&parts[i+1]==='snapshot'){end=i+2;if(!parts[end])return {root:uri,label:'Snapshot collection'};break;}
    if(parts[i].startsWith('@GMT-')){end=i;break;}
   }
   if(end>=0){const name=parts[end]==='snapshot'&&parts[end-2]==='.snapshots'?parts[end-1]:parts[end];const r=new URL(uri);r.pathname=encoded.slice(0,end+1).join('/');return{root:r.href,label:name};}
   for(const r of [...roots].sort((a,b)=>b.length-a.length)){if(!within(uri,r))continue;const base=r.replace(/\/$/,'');const next=uri.slice(base.length).replace(/^\//,'').split('/')[0];return{root:next?base+'/'+next:base,label:next?decodeURIComponent(next):'Snapshot collection'};}
  }catch{}return null;
 }
 return{parse,describe,location,within};
});
