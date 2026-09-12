#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/* No registry dependencies. Assets come from the real desktop UI, never a mockup. */
const fs=require('fs'),path=require('path'),cp=require('child_process');
const root=path.resolve(__dirname,'..'),web=path.join(root,'apps/web'),pub=path.join(web,'public');
const python=process.env.PYTHON||'python3';
// Website hosting carries no release binaries; visitors are directed to the
// public GitHub repository instead.
for(const directory of [path.join(pub,'downloads'),path.join(web,'out','downloads'),path.join(root,'designs','downloads')])fs.rmSync(directory,{recursive:true,force:true});
cp.execFileSync(python,[path.join(root,'desktop/tools/build_preview.py')],{stdio:'inherit'});
cp.execFileSync(python,[path.join(root,'tools/sync-docs.py')],{stdio:'inherit'});
const docs=JSON.parse(fs.readFileSync(path.join(web,'lib/docs.json'),'utf8'));
const markdown=JSON.parse(fs.readFileSync(path.join(pub,'assets/markdown.json'),'utf8'));
const code='/* SPDX-License-Identifier: AGPL-3.0-only */\nwindow.__OX_DOCS__='+JSON.stringify(docs)+';\nwindow.__OX_MARKDOWN__='+JSON.stringify(markdown)+';\n'+fs.readFileSync(path.join(pub,'assets/interactions.js'),'utf8');
fs.writeFileSync(path.join(pub,'assets/site.js'),code);
fs.copyFileSync(path.join(root,'LICENSE'),path.join(pub,'LICENSE.txt'));
fs.copyFileSync(path.join(root,'desktop/ui/winspace.svg'),path.join(pub,'assets/folder.svg'));
fs.copyFileSync(path.join(root,'desktop/preview.html'),path.join(pub,'app-preview.html'));
fs.mkdirSync(path.join(root,'designs'),{recursive:true});
for(const name of ['assets','docs-markdown'])fs.cpSync(path.join(pub,name),path.join(root,'designs',name),{recursive:true});
fs.copyFileSync(path.join(pub,'app-preview.html'),path.join(root,'designs/app-preview.html'));
console.log('Prepared actual app preview, Markdown, docs search, license and icons.');
