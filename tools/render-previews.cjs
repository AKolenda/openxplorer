#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/** Offline PRESENTATIONAL renderer. Not React, Next.js, a dependency resolver,
 * or a production-build substitute. Transpiles the same pure TSX components
 * to standalone HTML so design review works without registry access. */
const fs=require('fs'),path=require('path'),vm=require('vm'),cp=require('child_process');
let ts;try{ts=require(path.resolve(__dirname,'../apps/web/node_modules/typescript'));}catch{try{ts=require('typescript');}catch{try{ts=require(path.join(cp.execSync('npm root -g',{encoding:'utf8'}).trim(),'typescript'));}catch{throw Error('Install TypeScript, or run pnpm install before pnpm designs.');}}}
require('./prepare-web.cjs');
const root=path.resolve(__dirname,'..'),web=path.join(root,'apps/web'),out=path.join(root,'designs');fs.mkdirSync(out,{recursive:true});fs.mkdirSync(path.join(root,'test-results'),{recursive:true});
const Fragment=Symbol('Fragment'),runtime={Fragment,jsx:(type,props)=>({type,props:props||{}}),jsxs:(type,props)=>({type,props:props||{}})};
const cache={};let checked=0;
function load(filename){
 if(filename.endsWith('.json'))return JSON.parse(fs.readFileSync(filename,'utf8'));
 if(cache[filename])return cache[filename].exports;
 const module={exports:{}};cache[filename]=module;
 const source=fs.readFileSync(filename,'utf8');
 const result=ts.transpileModule(source,{fileName:filename,reportDiagnostics:true,compilerOptions:{jsx:ts.JsxEmit.ReactJSX,module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,esModuleInterop:true,resolveJsonModule:true}});
 const errors=(result.diagnostics||[]).filter(d=>d.category===ts.DiagnosticCategory.Error);if(errors.length)throw Error(errors.map(d=>ts.flattenDiagnosticMessageText(d.messageText,'\n')).join('\n'));checked++;
 const req=id=>{if(id==='react/jsx-runtime')return runtime;if(id==='react')return{Fragment};if(!id.startsWith('.'))throw Error('Unsupported render-time dependency: '+id);let p=path.resolve(path.dirname(filename),id);if(!fs.existsSync(p))p=['.tsx','.ts','.json'].map(ext=>p+ext).find(fs.existsSync);if(!p)throw Error('Missing '+id);return load(p);};
 new vm.Script('(function(require,module,exports){'+result.outputText+'\n})',{filename}).runInThisContext()(req,module,module.exports);return module.exports;
}
const escape=s=>String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
const voids=new Set(['area','base','br','col','embed','hr','img','input','link','meta','param','source','track','wbr']);
const bools=new Set(['hidden','disabled','checked','multiple','selected','autoFocus','required','readOnly','open','download']);
const attrs={className:'class',htmlFor:'for',tabIndex:'tabindex',autoComplete:'autocomplete',strokeWidth:'stroke-width',strokeLinecap:'stroke-linecap',strokeLinejoin:'stroke-linejoin',fillRule:'fill-rule',clipRule:'clip-rule'};
function render(node){
 if(node==null||typeof node==='boolean')return'';if(Array.isArray(node))return node.map(render).join('');if(typeof node==='string'||typeof node==='number')return escape(node);
 const{type,props}=node;if(type===Fragment)return render(props.children);if(typeof type==='function')return render(type(props));
 let attributes='';for(const[k,v]of Object.entries(props)){if(v==null||k==='children'||k==='key'||k==='ref'||k==='dangerouslySetInnerHTML'||k.startsWith('on'))continue;if(k==='style'){const style=Object.entries(v).map(([key,value])=>key.replace(/[A-Z]/g,c=>'-'+c.toLowerCase())+':'+value).join(';');attributes+=' style="'+escape(style)+'"';continue;}if(bools.has(k)&&typeof v==='boolean'){if(v)attributes+=' '+(attrs[k]||k).toLowerCase();continue;}attributes+=' '+(attrs[k]||k)+'="'+escape(v)+'"';}
 const content=props.dangerouslySetInnerHTML?String(props.dangerouslySetInnerHTML.__html):render(props.children);
 return'<'+type+attributes+'>'+(!voids.has(type)?content+'</'+type+'>':'');
}
const C=load(path.join(web,'components/site.tsx'));
const css=fs.readFileSync(path.join(web,'public/assets/site.css'),'utf8'),js=fs.readFileSync(path.join(web,'public/assets/site.js'),'utf8');
const docs=JSON.parse(fs.readFileSync(path.join(web,'lib/docs.json'),'utf8'));
const routes={'/':'index.html','/source/':'source.html','/concepts/':'concepts.html','/docs/':'docs-introduction.html'};
for(const v of ['windows','zorin','vercel'])routes['/concepts/'+v+'/']='openxplorer-'+v+'.html';
for(const d of docs)routes['/docs/'+d.slug+'/']='docs-'+d.slug+'.html';
function rewrite(html){return html.replace(/(href|src)="([^\"]+)"/g,(all,attr,url)=>{const [base,hash]=url.split('#');if(routes[base])return attr+'="'+routes[base]+(hash?'#'+hash:'')+'"';if(base.startsWith('/'))return attr+'="'+url.slice(1)+'"';return all;});}
function page(name,component,title){
 let markup=rewrite(render(component));
 // Standalone HTML remains reviewable without a server: actual preview in
 // a script-only sandboxed srcdoc; screenshots are the captured PNG bytes.
 const app=fs.readFileSync(path.join(web,'public/app-preview.html'),'utf8');
 markup=markup.replace(/(<iframe[^>]*?) src="[^"]*"/g,(_,prefix)=>prefix+' srcdoc="'+escape(app)+'"');
 markup=markup.replace(/(<img[^>]*?) src="(assets\/screenshots\/[^"]+)"/g,(_,prefix,asset)=>{
   const f=path.join(web,'public',asset);return prefix+' src="data:image/png;base64,'+fs.readFileSync(f).toString('base64')+'"';
 });const html='<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="description" content="OpenXplorer — Windows File Explorer-inspired files for Linux."><title>'+escape(title)+' | OpenXplorer</title><style>'+css+'</style></head><body>'+markup+'<script>window.__OX_OFFLINE__=true;'+js.replace(/<\/script/gi,'<\\/script')+'</script></body></html>';
 fs.writeFileSync(path.join(out,name),html);
}
page('index.html',runtime.jsx(C.Home,{vibe:'zorin'}),'Familiar files. Open possibilities.');
page('concepts.html',runtime.jsx(C.Concepts,{}),'Three design directions');
for(const v of ['windows','zorin','vercel'])page('openxplorer-'+v+'.html',runtime.jsx(C.Home,{vibe:v,lab:true}),'Design lab · '+v);
for(const d of docs)page('docs-'+d.slug+'.html',runtime.jsx(C.DocPage,{slug:d.slug}),d.title);
page('source.html',runtime.jsx(C.SourcePage,{}),'Source code');
fs.copyFileSync(path.join(root,'LICENSE'),path.join(out,'LICENSE.txt'));
fs.writeFileSync(path.join(root,'test-results/presentation-transpile.json'),JSON.stringify({checkedComponentModules:checked,standalonePages:fs.readdirSync(out).filter(p=>p.endsWith('.html')).length,nextBuild:false,scope:'TypeScript transpileModule syntax checks and offline presentational rendering; not React or Next runtime validation'},null,2));
console.log('Rendered standalone designs + documentation to designs/ (not a Next.js production build).');
