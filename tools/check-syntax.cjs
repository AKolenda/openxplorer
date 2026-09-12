#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
const fs=require('fs'),path=require('path'),cp=require('child_process');
let ts;try{ts=require(path.resolve(__dirname,'../apps/web/node_modules/typescript'));}catch{try{ts=require('typescript');}catch{ts=require(path.join(cp.execSync('npm root -g',{encoding:'utf8'}).trim(),'typescript'));}}
const root=path.resolve(__dirname,'..');const files=[];
function walk(dir){for(const entry of fs.readdirSync(dir,{withFileTypes:true})){if(['node_modules','.next','out'].includes(entry.name))continue;const p=path.join(dir,entry.name);if(entry.isDirectory())walk(p);else if(/\.tsx?$/.test(p)&&!p.endsWith('.d.ts'))files.push(p);}}
walk(path.join(root,'apps/web'));const failures=[];
for(const file of files){const r=ts.transpileModule(fs.readFileSync(file,'utf8'),{fileName:file,reportDiagnostics:true,compilerOptions:{target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.ESNext,jsx:ts.JsxEmit.ReactJSX}});for(const d of r.diagnostics||[])if(d.category===ts.DiagnosticCategory.Error)failures.push({file:path.relative(root,file),message:ts.flattenDiagnosticMessageText(d.messageText,'\n')});}
const report={files:files.map(f=>path.relative(root,f)),count:files.length,failures,scope:'Syntax transpilation only; not a dependency-aware TypeScript check or Next.js build.'};fs.writeFileSync(path.join(root,'test-results/website-syntax.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(failures.length)process.exit(1);
