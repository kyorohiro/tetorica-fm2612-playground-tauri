const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../desktop/desktop-interface.js'), 'utf8');
class Element {
  constructor() { this.attributes = {}; this.listeners = {}; }
  addEventListener(name, fn) { (this.listeners[name] ??= []).push(fn); }
  setAttribute(name, value) { this.attributes[name] = value; }
  getAttribute(name) { return this.attributes[name]; }
  showModal() { this.open = true; }
  close() { this.open = false; this.listeners.close?.forEach(fn => fn()); }
  append(...items) { this.children = items; }
  remove() { this.removed = true; }
  select() {}
  focus() { this.focused = true; }
  click() { if (!this.disabled) return Promise.all(this.listeners.click.map(fn => fn({target: this, preventDefault(){}, stopImmediatePropagation(){}}))); }
}
const settle = () => new Promise(resolve => setImmediate(resolve));
test('closing waits for autosave; a failed save allows cancellation', async () => {
  let finish, dialog;
  const commands=[];
  const window={__TAURI_INTERNALS__:{invoke:async command=>commands.push(command)},
    __tetoricaAutosaveFlush:()=>new Promise(resolve=>finish=resolve)};
  window.top=window;
  const document={readyState:'loading',addEventListener(){},createElement:()=>new Element(),body:{append:element=>dialog=element}};
  vm.runInNewContext(source.replace('__DESKTOP_DEV_ORIGIN__','null'),{window,document,location:new URL('tauri://localhost')});
  const closing=window.__tetoricaCloseRequested();
  assert.deepEqual(commands,[]);
  finish();await closing;assert.deepEqual(commands,['window_close']);
  window.__tetoricaAutosaveFlush=async()=>{throw Error('disk full');};
  const failed=window.__tetoricaCloseRequested();await settle();
  assert.match(dialog.children[0].textContent,/disk full/);
  await dialog.children[1].click();await failed;
  assert.deepEqual(commands,['window_close']);
});
test('single click toggles despite delayed native state; failures recover inside existing Menu', async () => {
  const menu = new Element();
  const children = [];
  menu.querySelector = () => ({append: (...items) => children.push(...items)});
  const document = {readyState:'complete', getElementById: id => id === 'mainMenu' ? menu : null,
    createElement: () => new Element(), body:{append(){}}};
  let actual = false, fail = false;
  const requests = [];
  const window = {__TAURI_INTERNALS__: {invoke: async (cmd, args) => {
    if (cmd === 'mcp_status') return {enabled:false,url:'http://127.0.0.1:39127/mcp',token:'test'};
    if (cmd === 'window_top_get') return actual;
    requests.push(args.enabled);
    if (fail) throw Error('native failure');
    return args.enabled;
  }}};
  window.top = window;
  vm.runInNewContext(source.replace('__DESKTOP_DEV_ORIGIN__','null'), {window, document, location:new URL('tauri://localhost')});
  menu.open = true;
  menu.listeners.toggle.forEach(fn => fn()); await settle();
  await children[0].click();
  assert.equal(children[0].textContent, 'Always on Top: On');
  await children[0].click();
  assert.equal(children[0].textContent, 'Always on Top: Off');
  assert.deepEqual(requests,[true,false]);
  actual = true;
  menu.open = true;
  menu.listeners.toggle.forEach(fn => fn()); await settle();
  assert.equal(children[0].getAttribute('aria-pressed'),'true');
  fail = true;
  await children[0].click();
  assert.match(children[1].textContent,/native failure/);
  assert.equal(children[0].getAttribute('aria-pressed'),'true');
  assert.equal(children[0].disabled,false);
  assert.equal(children.length, 5);
  assert.equal(children[0].id, 'desktop-window-top');
});

test('MCP retains configuration across toggles and explicitly regenerates only while off', async () => {
  const menu = new Element(), children = [];
  menu.querySelector = () => ({append: (...items) => children.push(...items)});
  const elements = [];
  const document = {readyState:'complete', getElementById:id => id === 'mainMenu' ? menu : null,
    createElement:tag => { const el = new Element(); el.tag = tag; elements.push(el); return el; },
    body:{append(el){
      if (el.getAttribute('aria-label') === 'Confirm action') setImmediate(() => el.children[confirmed ? 2 : 1].click());
    }}};
  let token = 'original', enabled = false, generation = 'session-1', confirmed = false;
  const calls = [];
  const window = {confirm:() => confirmed, __TAURI_INTERNALS__:{invoke:async (cmd,args) => {
    calls.push(cmd);
    if (cmd === 'mcp_set') { enabled = args.enabled; if (enabled) generation += '-new'; }
    if (cmd === 'mcp_regenerate_token') { assert.equal(enabled,false); token = 'replacement'; }
    return {enabled,token,generation,url:'http://127.0.0.1:39127/mcp'};
  }}};
  window.top = window;
  vm.runInNewContext(source.replace('__DESKTOP_DEV_ORIGIN__','null'), {window,document,location:new URL('tauri://localhost')});
  await settle();
  const mcp = children[2], rotate = children[4], config = elements.find(el => el.tag === 'textarea');
  await mcp.click();
  assert.equal(rotate.disabled,true);
  assert.equal(window.__tetoricaMcpGeneration,generation);
  assert.match(config.value,/original/);
  await mcp.click();
  assert.equal(rotate.disabled,false);
  await rotate.click();
  assert.equal(calls.includes('mcp_regenerate_token'),false);
  confirmed = true;
  await rotate.click();
  assert.match(config.value,/replacement/);
  assert.equal(elements.find(el => el.tag === 'dialog').open,true);
});

test('offline reset reloads Tauri without browser cache APIs and clears only app caches when available', async () => {
  for (const available of [false,true]) {
    const menu = new Element(), reset = new Element(), children = [];
    menu.querySelector = () => ({append:(...items) => children.push(...items)});
    const document = {readyState:'complete',
      getElementById:id => id === 'mainMenu' ? menu : id === 'resetOfflineCacheButton' ? reset : null,
      createElement:() => new Element(), body:{append(el){
        if (el.getAttribute('aria-label') === 'Confirm action') setImmediate(() => el.children[confirmed ? 2 : 1].click());
      }}};
    const actions = [];
    let confirmed = false, fail = false;
    const window = {confirm:() => confirmed, __TAURI_INTERNALS__:{invoke:async cmd => {
      if (cmd === 'window_reload') { if (fail) throw Error('reload failed'); actions.push('reload'); }
      return {enabled:false,token:'test',generation:'session'};
    }}};
    if (available) {
      window.caches = {keys:async () => ['hello-ymfm-docs-v1','other-app'],
        delete:async key => actions.push(key)};
      window.navigator = {serviceWorker:{getRegistrations:async () => [{unregister:async () => actions.push('unregister')}]}};
    }
    window.top = window;
    vm.runInNewContext(source.replace('__DESKTOP_DEV_ORIGIN__','null'), {window,document,location:new URL('tauri://localhost')});
    await settle();
    await reset.click();
    assert.deepEqual(actions,[]);
    confirmed = true;
    await reset.click();
    assert.deepEqual(actions,available ? ['hello-ymfm-docs-v1','unregister','reload'] : ['reload']);
    reset.disabled = false;
    fail = true;
    await reset.click();
    assert.equal(reset.disabled,false);
    assert.match(children[1].textContent,/reload failed/);
  }
});
