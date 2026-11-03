const test = require('node:test');
const assert = require('node:assert/strict');
const {mkdtemp, mkdir, writeFile, readFile, rm} = require('node:fs/promises');
const {tmpdir} = require('node:os');
const {join} = require('node:path');
const {pathToFileURL} = require('node:url');

test('parent source resolution reads legacy and moved trees and preserves public/upstream paths', async t => {
  const {sourcePath} = await import('../scripts/source_path.mjs');
  const temporary = await mkdtemp(join(tmpdir(), 'parent source paths '));
  t.after(() => rm(temporary, {recursive: true, force: true}));
  for (const modern of [false, true]) {
    const root = join(temporary, modern ? 'modern' : 'legacy');
    await mkdir(root);
    if (modern) await mkdir(join(root, 'tetorica_src'));
    for (const [original, moved] of Object.entries({web: 'runtime', node: 'node', cli: 'cli', native: 'native', wasm: 'wasm'})) {
      const directory = modern ? join(root, 'tetorica_src', moved) : join(root, original);
      await mkdir(directory, {recursive: true});
      await writeFile(join(directory, 'fixture.mjs'), 'export default ' + JSON.stringify(original));
      const resolved = sourcePath(root, original, 'fixture.mjs');
      assert.equal(resolved, join(directory, 'fixture.mjs'));
      assert.equal((await import(pathToFileURL(resolved))).default, original);
    }
    for (const path of ['src/upstream.cpp', 'third_party/vendor/source.cpp', 'docs/generated/core.wasm']) {
      assert.equal(sourcePath(root, path), join(root, path));
    }
  }
});

test('missing moved sources fail despite an obsolete file at the legacy path', async t => {
  const {sourcePath} = await import('../scripts/source_path.mjs');
  const root = await mkdtemp(join(tmpdir(), 'parent missing source '));
  t.after(() => rm(root, {recursive: true, force: true}));
  await mkdir(join(root, 'tetorica_src'));
  await mkdir(join(root, 'web'));
  await writeFile(join(root, 'web/obsolete.js'), 'obsolete');
  await assert.rejects(readFile(sourcePath(root, 'web/obsolete.js')), {code: 'ENOENT'});
});
