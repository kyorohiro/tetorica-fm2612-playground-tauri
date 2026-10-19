const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

test('macOS sampler recording declares its microphone purpose and audio input entitlement', () => {
  const root = path.join(__dirname, '../src-tauri');
  const {bundle} = JSON.parse(fs.readFileSync(path.join(root, 'tauri.conf.json'), 'utf8'));
  const info = fs.readFileSync(path.join(root, bundle.macOS.infoPlist), 'utf8');
  const entitlements = fs.readFileSync(path.join(root, bundle.macOS.entitlements), 'utf8');
  assert.match(info, /<key>NSMicrophoneUsageDescription<\/key>\s*<string>[^<]+<\/string>/);
  assert.match(entitlements, /<key>com\.apple\.security\.device\.audio-input<\/key>\s*<true\s*\/>/);
});
