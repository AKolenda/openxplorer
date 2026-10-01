#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-only
/* No registry dependencies. Pictures come from the real native app, never a mockup. */
const fs=require('fs'),path=require('path'),cp=require('child_process');
const root=path.resolve(__dirname,'..'),web=path.join(root,'apps/web'),pub=path.join(web,'public');
const python=process.env.PYTHON||'python3';
// Website hosting carries no release binaries; visitors are directed to the
// public GitHub repository instead.
for(const directory of [path.join(pub,'downloads'),path.join(web,'out','downloads'),path.join(root,'designs','downloads')])fs.rmSync(directory,{recursive:true,force:true});
cp.execFileSync(python,[path.join(root,'tools/sync-docs.py')],{stdio:'inherit'});
const docs=JSON.parse(fs.readFileSync(path.join(web,'lib/docs.json'),'utf8'));
const markdown=JSON.parse(fs.readFileSync(path.join(pub,'assets/markdown.json'),'utf8'));
const code='/* SPDX-License-Identifier: AGPL-3.0-only */\nwindow.__OX_DOCS__='+JSON.stringify(docs)+';\nwindow.__OX_MARKDOWN__='+JSON.stringify(markdown)+';\n'+fs.readFileSync(path.join(pub,'assets/interactions.js'),'utf8');
fs.writeFileSync(path.join(pub,'assets/site.js'),code);
// The tour runs in a script-only sandbox, which cannot fetch scenes.json, so
// its data is loaded as a script. tools/capture-native-tour.py writes scenes.json.
const tour=JSON.parse(fs.readFileSync(path.join(pub,'tour/scenes.json'),'utf8'));
for(const scene of tour.scenes)for(const image of Object.values(scene.images))if(!fs.existsSync(path.join(pub,'tour',image)))throw new Error('The tour picture '+image+' is missing; run tools/capture-native-tour.py.');
fs.writeFileSync(path.join(pub,'tour/scenes.js'),'/* SPDX-License-Identifier: AGPL-3.0-only */\n/* Generated from scenes.json by tools/prepare-web.cjs. */\nwindow.OPENXPLORER_TOUR='+JSON.stringify(tour)+';\n');
fs.copyFileSync(path.join(root,'LICENSE'),path.join(pub,'LICENSE.txt'));
fs.mkdirSync(path.join(root,'designs'),{recursive:true});
for(const name of ['assets','docs-markdown','tour'])fs.cpSync(path.join(pub,name),path.join(root,'designs',name),{recursive:true});
console.log('Prepared the native app tour, Markdown, docs search and license.');
