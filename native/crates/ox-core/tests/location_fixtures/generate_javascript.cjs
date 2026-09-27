// SPDX-License-Identifier: AGPL-3.0-only
// Runs the display helpers from desktop/ui/app.js over the parity URIs.
const fs = require('fs');
const [appPath, outPath] = process.argv.slice(2);
const lines = fs.readFileSync(appPath, 'utf8').split('\n');
const range = (from, to) => lines.slice(from - 1, to).join('\n');
const source = [range(122, 132), range(508, 510), range(512, 512), range(1228, 1234), range(1591, 1610)].join('\n');

const state = {
  env: {
    home: 'file:///home/test',
    mounts: [
      { label: 'Sample Phone', uri: 'mtp://[usb:001,010]/', mounted: true },
      { label: 'Unplugged', uri: 'mtp://[usb:009,009]/', mounted: false },
      { label: '', uri: 'afc://blank/', mounted: true },
    ],
    snapshotRoots: ['file:///srv/snaps', 'smb://nas/backup/'],
    stableMounts: [
      { path: '/mnt/nas', fstype: 'cifs' },
      { path: '/mnt/other', fstype: 'nfs' },
      { path: '/mnt/s3/', fstype: 'smb3' },
      { path: '/mnt/plain' },
      { path: '', fstype: 'cifs' },
    ],
  },
};
const api = new Function('state', source + '\nreturn {prettyBytes,dateText,baseName,parentUri,displayUri,titleFor,sameLocation,writableLocation,isSmbShareRoot,readonlyLocation,breadcrumbSegments,networkLocation,deviceRoot};')(state);

const uris = [
  'file:///', 'file:///home/test', 'file:///home/test/', 'file:///home/test/Documents', 'file:///tmp/a%20b/%23c',
  'file:///tmp/%E2%82%AC', 'file:///tmp/bad%zz', 'file:///tmp/100%25', 'file:///tmp/%C3', 'smb://nas/', 'smb://nas',
  'smb://nas/share', 'smb://nas/share/', 'smb://nas:1445/share/one/two/three/four/a%20b/%23snapshot',
  'smb://archive-nas/Shared/Launch%20planning', 'smb://NAS/Share', 'smb://nas/%zz', 'smb://[fe80::1]/share/x',
  'mtp://[usb:001,010]/', 'mtp://[usb:001,010]/Internal%20storage', 'mtp://[usb:001,010]/Internal%20storage/DCIM',
  'mtp://[usb:001,010]/Internal%20storage/DCIM/', 'mtp://[usb:009,009]/x', 'MTP://[usb:001,010]/DCIM',
  'gphoto2://[usb:001,002]/', 'afc://abc/', 'afc://abc', 'afc://blank/Photos', 'mtp://[usb:001,010]/bad%zz',
  'file:///srv/snaps', 'file:///srv/snaps/', 'file:///srv/snaps/x', 'file:///srv/snapshots', 'file:///data/.snapshot/x',
  'file:///data/.snapshots', 'smb://nas/s/@GMT-2024.01.01-00.00.00/x', 'file:///p/.zfs/snapshot/a', 'file:///p/.zfs/x',
  'file:///p/.zfs', 'smb://nas/backup', 'smb://nas/backup/x', 'smb://nas/backupx', 'file:///mnt/nas',
  'file:///mnt/nas/', 'file:///mnt/nas/folder', 'file:///mnt/nas-other/folder', 'file:///mnt/other/x', 'file:///mnt/s3/x',
  'file:///mnt/s3', 'file:///mnt/plain/x', 'file:///tmp/%23snapshot', 'file:///tmp/a%2Fb', 'file:///tmp/%40GMT-x',
  'smb://nas:445/', 'file:///a/b/c/d', 'smb://nas//', 'file:///tmp/x%2F', '',
];
const pairs = [
  ['file:///a', 'file:///a/'], ['file:///a//', 'file:///a'], ['file:///a/', 'file:///a/'], ['smb://nas', 'smb://nas/'],
  ['file:///', 'file://'], ['a', 'b'], ['', '/'], ['file:///A', 'file:///a'],
];
const sizes = [0, 1, 912, 1023, 1024, 1280, 1536, 72704, 102400, 102912, 102913, 133120, 1048063, 1048064, 1048576,
  1101004, 104805376, 1073741824, 1099511627776, 1125899906842624, 2 ** 53, 5000, 10240, 10291, 10292, 1075, 1127,
  1178, 1229, 1331, 1382, 1433, 1485, 1587, 2355, 2406, 2457, 1638, 1690, 1741, 1792, 1843, 1894, 1946, 1997,
  1049, 1101, 1152, 1203, 1254, 1306, 1357, 1408, 1459, 1510, 1561, 1613, 1664, 1715, 1766, 1818, 1869, 1920, 1971, 2022];

const result = {
  uris: uris.map((uri) => ({
    uri,
    baseName: api.baseName(uri),
    parentUri: api.parentUri(uri),
    displayUri: api.displayUri(uri),
    titleFor: api.titleFor(uri),
    writable: api.writableLocation(uri),
    shareRoot: api.isSmbShareRoot(uri),
    readonly: api.readonlyLocation(uri),
    crumbs: api.breadcrumbSegments(uri),
    network: api.networkLocation(uri),
    deviceRoot: api.deviceRoot(uri),
  })),
  pairs: pairs.map(([a, b]) => [a, b, api.sameLocation(a, b)]),
  sizes: sizes.map((n) => [n, api.prettyBytes(n)]),
};
fs.writeFileSync(outPath, JSON.stringify(result, null, 0));
