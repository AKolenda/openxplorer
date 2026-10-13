// SPDX-License-Identifier: AGPL-3.0-only
// Use the pinned compiler's CLI: TypeScript 7 no longer provides transpileModule.
// Temporary output is only for offline presentation and syntax checks.
const fs = require('fs');
const os = require('os');
const path = require('path');
const cp = require('child_process');
const {createRequire} = require('module');

const web = path.resolve(__dirname, '../apps/web');
const webRequire = createRequire(path.join(web, 'package.json'));

function withTranspiledWeb(files, consume) {
  let manifestPath;
  try {
    manifestPath = webRequire.resolve('typescript/package.json');
  } catch {
    throw new Error('Install the pinned website compiler with pnpm install --frozen-lockfile.');
  }
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const compiler = path.resolve(path.dirname(manifestPath), manifest.bin.tsc);
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'openxplorer-transpile-'));
  try {
    cp.execFileSync(process.execPath, [compiler,
      '--ignoreConfig', '--noCheck', '--noEmitOnError',
      '--jsx', 'react-jsx', '--module', 'node16', '--moduleResolution', 'node16',
      '--target', 'es2020', '--resolveJsonModule', '--esModuleInterop',
      '--rootDir', web, '--outDir', temporary, ...files,
    ], {cwd: web, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe']});
    return consume(temporary);
  } finally {
    fs.rmSync(temporary, {recursive: true, force: true});
  }
}

module.exports = {withTranspiledWeb};
