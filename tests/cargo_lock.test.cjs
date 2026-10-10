const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

test('Cargo.lock records the current application version for locked CI builds', () => {
  const root = path.resolve(__dirname, '..');
  const manifest = fs.readFileSync(path.join(root, 'src-tauri/Cargo.toml'), 'utf8');
  const lock = fs.readFileSync(path.join(root, 'src-tauri/Cargo.lock'), 'utf8');
  const application = manifest.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
  assert.ok(application, 'Cargo.toml must contain a package table');
  const field = (table, name) => table.match(new RegExp(`^${name}\\s*=\\s*"([^"]+)"`, 'm'))?.[1];
  const name = field(application, 'name');
  const version = field(application, 'version');
  assert.ok(name && version, 'The application must have a name and version');
  const locked = lock.split(/^\[\[package\]\]\s*$/m).find(table => field(table, 'name') === name);
  assert.ok(locked, 'Cargo.lock must contain the application package');
  assert.equal(field(locked, 'version'), version,
    'Cargo.toml and Cargo.lock differ. Run cargo check to update Cargo.lock, then commit it with the release version change.');
});
