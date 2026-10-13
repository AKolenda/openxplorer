// SPDX-License-Identifier: AGPL-3.0-only
// Run with node --test tests/test_web_tooling.cjs after pnpm install.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {test} = require('node:test');
const {withTranspiledWeb} = require('../tools/transpile-web.cjs');

function fixture(t, source) {
  const directory = fs.mkdtempSync(path.resolve(__dirname, '../apps/web/.transpile-fixture-'));
  t.after(() => fs.rmSync(directory, {recursive: true, force: true}));
  const file = path.join(directory, 'example.tsx');
  fs.writeFileSync(file, source);
  return file;
}

test('the pinned compiler emits JSX without claiming to check types', t => {
  const file = fixture(t, 'const label: number = "Fictional example"; export const view = <main>{label}</main>;');
  let output;
  withTranspiledWeb([file], directory => {
    output = directory;
    const emitted = path.join(directory, path.basename(path.dirname(file)), 'example.js');
    assert.match(fs.readFileSync(emitted, 'utf8'), /react\/jsx-runtime/);
  });
  assert.equal(fs.existsSync(output), false);
});

test('malformed JSX stops rendering instead of producing a successful syntax report', t => {
  const file = fixture(t, 'export const view = <main>;');
  assert.throws(() => withTranspiledWeb([file], () => assert.fail('Invalid JSX was accepted')),
    error => error.status !== 0 && error.stdout.includes('error TS'));
});

test('temporary compiler output is removed when the renderer fails', t => {
  const file = fixture(t, 'export const value = "Fictional example";');
  let output;
  assert.throws(() => withTranspiledWeb([file], directory => {
    output = directory;
    throw new Error('Fictional rendering failure');
  }), /Fictional rendering failure/);
  assert.equal(fs.existsSync(output), false);
});
