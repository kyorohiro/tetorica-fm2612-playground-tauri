const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const root = path.join(__dirname, '..');
test('development uses the same asset response hook as the packaged app', () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json')));
  const config = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json')));
  assert.match(pkg.scripts.dev, /\btauri dev\b.*--no-dev-server\b/);
  assert.equal(config.build.devUrl, undefined);
});
