const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../desktop/desktop-interface.js'), 'utf8');
class Element {
  constructor() { this.attributes = {}; this.listeners = {}; }
  addEventListener(name, fn) { this.listeners[name] = fn; }
  setAttribute(name, value) { this.attributes[name] = value; }
  getAttribute(name) { return this.attributes[name]; }
  showModal() { this.open = true; }
  close() { this.open = false; this.listeners.close(); }
  focus() { this.focused = true; }
  click() { if (!this.disabled) return this.listeners.click({target: this}); }
}
const settle = () => new Promise(resolve => setImmediate(resolve));
test('single click toggles despite delayed native state; failures recover inside existing Menu', async () => {
  const menu = new Element();
  const children = [];
  menu.querySelector = () => ({append: (...items) => children.push(...items)});
  const document = {readyState:'complete', getElementById: id => id === 'mainMenu' ? menu : null,
    createElement: () => new Element()};
  let actual = false, fail = false;
  const requests = [];
  const window = {__TAURI_INTERNALS__: {invoke: async (cmd, args) => {
    if (cmd === 'window_top_get') return actual;
    requests.push(args.enabled);
    if (fail) throw Error('native failure');
    return args.enabled;
  }}};
  window.top = window;
  vm.runInNewContext(source.replace('__DESKTOP_DEV_ORIGIN__','null'), {window, document, location:new URL('tauri://localhost')});
  menu.open = true;
  menu.listeners.toggle(); await settle();
  await children[0].click();
  assert.equal(children[0].textContent, 'Always on Top: On');
  await children[0].click();
  assert.equal(children[0].textContent, 'Always on Top: Off');
  assert.deepEqual(requests,[true,false]);
  actual = true;
  menu.open = true;
  menu.listeners.toggle(); await settle();
  assert.equal(children[0].getAttribute('aria-pressed'),'true');
  fail = true;
  await children[0].click();
  assert.match(children[1].textContent,/native failure/);
  assert.equal(children[0].getAttribute('aria-pressed'),'true');
  assert.equal(children[0].disabled,false);
  assert.equal(children.length, 2);
  assert.equal(children[0].id, 'desktop-window-top');
});
