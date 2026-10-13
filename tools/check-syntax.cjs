#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
const fs=require('fs'),path=require('path');
const {withTranspiledWeb}=require('./transpile-web.cjs');
const root=path.resolve(__dirname,'..');const files=[];
function walk(dir){for(const entry of fs.readdirSync(dir,{withFileTypes:true})){if(['node_modules','.next','out'].includes(entry.name))continue;const p=path.join(dir,entry.name);if(entry.isDirectory())walk(p);else if(/\.tsx?$/.test(p)&&!p.endsWith('.d.ts'))files.push(p);}}
walk(path.join(root,'apps/web'));const failures=[];
try{withTranspiledWeb(files,()=>{});}catch(error){failures.push({message:error.stdout?.trim()||error.message});}
fs.mkdirSync(path.join(root,'test-results'),{recursive:true});
const report={files:files.map(f=>path.relative(root,f)),count:files.length,failures,scope:'Syntax transpilation only; not a dependency-aware TypeScript check or Next.js build.'};fs.writeFileSync(path.join(root,'test-results/website-syntax.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));if(failures.length)process.exit(1);
