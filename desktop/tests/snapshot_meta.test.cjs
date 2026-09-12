// SPDX-License-Identifier: AGPL-3.0-only
const assert=require('node:assert/strict'),S=require('../ui/snapshot-meta.js');
let count=0;const check=(name,fn)=>{fn();console.log('PASS',name);count++;};
for(const [name,date,time] of [
 ['auto-2026-09-04_16-30','2026-09-04','16:30'],['auto-2026-09-04_16-30-21','2026-09-04','16:30:21'],
 ['2026-09-05_180000','2026-09-05','18:00:00'],['2026-09-05_18:00:00','2026-09-05','18:00:00'],
 ['daily-2026-09-05','2026-09-05',''],['@GMT-2026.09.05-18.00.00','2026-09-05','18:00:00 UTC'],
 ['2024-02-29_12-05','2024-02-29','12:05'],['backup_2026-12-31_23-59','2026-12-31','23:59']
])check('Parse '+name,()=>{const d=S.parse(name);assert.equal(d.date,date);assert.equal(d.time,time);});
for(const name of ['manual','1','2026-02-29_12-00','2026-09-31_00-00','2026-13-01','2026-00-01','2026-09-00','2026-09-04_24-01','2026-09-04_12-60','2026-09-04_12-01-60','@GMT-2026.13.05-18.00.00','<img onerror=alert(1)>'])check('Reject invalid / unknown '+name,()=>assert.equal(S.parse(name),null));
check('Never disguise mtime as snapshot date',()=>{const d=S.describe({label:'manual',snapshotModified:1788658800});assert.equal(d.iso,null);assert.equal(d.date,'Date unavailable');});
check('Name source made explicit',()=>assert.equal(S.describe({label:'auto-2026-09-04_16-30'}).sourceLabel,'From snapshot name'));
check('Does not invent a timezone',()=>assert.equal(S.parse('auto-2026-09-04_16-30').iso,'2026-09-04T16:30'));
check('GMT has explicit timezone',()=>assert.equal(S.parse('@GMT-2026.09.04-16.30.00').iso,'2026-09-04T16:30:00Z'));
check('Location boundary not prefix coincidence',()=>assert.equal(S.within('file:///backups-old/x','file:///backups'),false));
for(const [uri,label]of [
 ['smb://nas/share/.zfs/snapshot/auto-2026-09-04_16-30/notes','auto-2026-09-04_16-30'],
 ['smb://nas/share/.snapshot/2026-09-05_180000','2026-09-05_180000'],
 ['file:///home/.snapshots/123/snapshot/docs','123'],
 ['smb://nas/share/%23snapshot/manual/children','manual'],
 ['smb://nas/share/@GMT-2026.09.04-16.30.00/a','@GMT-2026.09.04-16.30.00'],
 ['file:///backups/.zfs/snapshot','Snapshot collection']
])check('Historical path '+uri,()=>assert.equal(S.location(uri)?.label,label));
check('Custom configured roots recognized',()=>assert.equal(S.location('smb://nas/backup/nightly/files',['smb://nas/backup']).label,'nightly'));
check('Ordinary share not misidentified',()=>assert.equal(S.location('smb://nas/share/folder.mp4'),null));
check('Encoded markers recognized',()=>assert.equal(S.location('smb://nas/share/%2Esnapshot/manual').label,'manual'));
check('Snapshot root excludes descendants',()=>assert.equal(S.location('smb://nas/share/.zfs/snapshot/nightly/child').root,'smb://nas/share/.zfs/snapshot/nightly'));
console.log(JSON.stringify({passed:true,checks:count,scope:'Date parsing and historical-location classification, no filesystem access'}));
