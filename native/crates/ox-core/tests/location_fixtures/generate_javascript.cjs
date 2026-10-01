// SPDX-License-Identifier: AGPL-3.0-only
// Prints what the display helpers in v2.0.0:desktop/ui/app.js answer for the
// location parity inputs.
//
// Usage, from the repository root:
//
//   node native/crates/ox-core/tests/location_fixtures/generate_javascript.cjs "$desktop/ui/app.js"
//
// where $desktop is the Python app's sources (python3 native/tools/python_app.py).
//
// The JSON document on standard output is javascript.json.
// location_javascript.rs compares the Rust port with it, and
// location_fixture_drift.rs runs this script again to prove that
// javascript.json still matches app.js.
//
// The helpers are found by name, together with every app.js function they
// mention, and run in their own V8 context. app.js as a whole cannot run
// here: it needs a browser document. The helpers catch their own errors and
// fall back to a default answer, so a helper that reads anything this script
// does not provide (`document`, a function it failed to extract) would
// silently capture a wrong answer. The context therefore records every such
// name, and the capture fails if there is one.
'use strict';

const fs = require('fs');
const vm = require('vm');

// The helpers whose answers javascript.json records.
const HELPERS = [
  'prettyBytes', 'baseName', 'parentUri', 'displayUri', 'titleFor', 'sameLocation', 'writableLocation',
  'isSmbShareRoot', 'readonlyLocation', 'breadcrumbSegments', 'networkLocation', 'deviceRoot',
];

// The part of app.js's `state` the helpers read: a home folder, mounted and
// unmounted devices, snapshot folders and stable network mounts.
const ENVIRONMENT = {
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
};

const URIS = [
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

const PAIRS = [
  ['file:///a', 'file:///a/'], ['file:///a//', 'file:///a'], ['file:///a/', 'file:///a/'], ['smb://nas', 'smb://nas/'],
  ['file:///', 'file://'], ['a', 'b'], ['', '/'], ['file:///A', 'file:///a'],
];

// Around every rounding boundary of prettyBytes, and its largest units.
const SIZES = [
  0, 1, 912, 1023, 1024, 1280, 1536, 72704, 102400, 102912, 102913, 133120, 1048063, 1048064, 1048576,
  1101004, 104805376, 1073741824, 1099511627776, 1125899906842624, 2 ** 53, 5000, 10240, 10291, 10292, 1075, 1127,
  1178, 1229, 1331, 1382, 1433, 1485, 1587, 2355, 2406, 2457, 1638, 1690, 1741, 1792, 1843, 1894, 1946, 1997,
  1049, 1101, 1152, 1203, 1254, 1306, 1357, 1408, 1459, 1510, 1561, 1613, 1664, 1715, 1766, 1818, 1869, 1920, 1971, 2022,
];

// A function declaration's name, with an optional `async` before it.
const DECLARATION = /(?:\basync\s+)?\bfunction\s*\*?\s*([A-Za-z_$][\w$]*)\s*\(/g;
const IDENTIFIER = /[A-Za-z_$][\w$]*/g;

// Where each function declared in `source` starts, by name.
function declarationStarts(source) {
  const starts = new Map();
  for (const match of source.matchAll(DECLARATION)) {
    const name = match[1];
    starts.set(name, [...(starts.get(name) || []), match.index]);
  }
  return starts;
}

// The whole declaration that starts at `start`. It ends at the first `}` at
// which the text read so far compiles: a `}` inside a string, template,
// regular expression or comment leaves that text unterminated, and an inner
// block's `}` leaves the function body open.
function declarationAt(source, start) {
  for (let end = source.indexOf('}', start); end !== -1; end = source.indexOf('}', end + 1)) {
    const candidate = source.slice(start, end + 1);
    if (compiles(candidate)) return candidate;
  }
  throw new Error(`No complete function declaration at offset ${start}.`);
}

// True when `code` parses as a strict-mode script. Nothing is run.
function compiles(code) {
  try {
    new vm.Script(`'use strict';\n${code}`);
    return true;
  } catch {
    return false;
  }
}

// The declarations of `names` and of every app.js function they mention,
// directly or through each other.
function extractWithDependencies(source, names) {
  const starts = declarationStarts(source);
  const extracted = new Map();
  const pending = [...names];
  while (pending.length) {
    const name = pending.pop();
    if (extracted.has(name)) continue;
    const places = starts.get(name) || [];
    if (places.length !== 1) throw new Error(`app.js declares ${name} ${places.length} times; expected once.`);
    const declaration = declarationAt(source, places[0]);
    extracted.set(name, declaration);
    const mentioned = declaration.match(IDENTIFIER).filter((word) => starts.has(word));
    pending.push(...mentioned);
  }
  return [...extracted.values()].join('\n');
}

// Runs `code` in a new V8 context whose only globals are `provided` and the
// ECMAScript built-ins. Returns the completion value and a set that collects
// the name of every other global the code reads.
function runIsolated(code, provided) {
  const builtIns = new Set(vm.runInNewContext('Object.getOwnPropertyNames(globalThis)'));
  const unresolved = new Set();
  const globals = new Proxy(provided, {
    get(target, name) {
      if (name in target) return target[name];
      if (builtIns.has(name)) return globalThis[name];
      if (typeof name === 'string') unresolved.add(name);
      return undefined;
    },
  });
  const value = vm.runInContext(`'use strict';\n${code}`, vm.createContext(globals));
  return { value, unresolved };
}

// Every table in javascript.json: the helpers' answers for the inputs above.
function capture(api) {
  return {
    environment: ENVIRONMENT,
    uris: URIS.map((uri) => ({
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
    pairs: PAIRS.map(([first, second]) => ({ first, second, same: api.sameLocation(first, second) })),
    sizes: SIZES.map((bytes) => ({ bytes, text: api.prettyBytes(bytes) })),
  };
}

// Extracts the helpers from the app.js named on the command line, runs them
// and prints javascript.json. Fails if a helper read a global it lacked.
function main() {
  const [appPath] = process.argv.slice(2);
  const helpers = extractWithDependencies(fs.readFileSync(appPath, 'utf8'), HELPERS);
  const code = `${helpers}\n({${HELPERS.join(', ')}});`;
  const state = { env: ENVIRONMENT };
  const { value: api, unresolved } = runIsolated(code, { state, URL });
  const document = capture(api);
  if (unresolved.size) {
    throw new Error(`The helpers read globals this script does not provide: ${[...unresolved].join(', ')}.`);
  }
  process.stdout.write(`${JSON.stringify(document, null, 2)}\n`);
}

main();
