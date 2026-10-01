const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../desktop/desktop-interface.js'), 'utf8');

function boot(url, devOrigin = null, subframe = false) {
  const window = {};
  window.top = subframe ? {} : window;
  const listeners = [];
  const document = {readyState: 'loading', addEventListener: (...args) => listeners.push(args)};
  vm.runInNewContext(source.replace('__DESKTOP_DEV_ORIGIN__', JSON.stringify(devOrigin)), {
    window, location: new URL(url), document,
  });
  return {window, listeners};
}

test('packaged macOS and Windows URLs initialize the menu', () => {
  for (const url of ['tauri://localhost/index.html', 'http://tauri.localhost/index.html']) {
    const result = boot(url);
    assert.equal(result.listeners[0][0], 'DOMContentLoaded');
  }
});
test('configured Tauri development server initializes the same menu', () => {
  for (const origin of ['http://localhost:1430', 'http://127.0.0.1:1430']) {
    const result = boot(origin + '/index.html', origin);
    assert.equal(result.listeners[0][0], 'DOMContentLoaded');
  }
});
test('other localhost ports, remote pages and subframes do not receive the bridge', () => {
  for (const [url, dev, frame] of [
    ['http://localhost:1430', null, false],
    ['http://localhost:1431', 'http://localhost:1430', false],
    ['https://example.com', 'http://localhost:1430', false],
    ['tauri://localhost/index.html', null, true],
  ]) {
    const result = boot(url, dev, frame);
    assert.equal(result.window.__tetoricaDesktop, undefined);
    assert.equal(result.listeners.length, 0);
  }
});
