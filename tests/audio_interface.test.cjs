const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../desktop/audio-interface.js'), 'utf8')
  .replaceAll('import.meta.url', JSON.stringify('tauri://localhost/playground.js'));

class Element {
  constructor(tag) { this.tag = tag; this.children = []; this.listeners = {}; this.attributes = {}; this.style = {}; this.value = ''; }
  append(...items) { this.children.push(...items); }
  addEventListener(name, fn) { (this.listeners[name] ??= []).push(fn); }
  setAttribute(name, value) { this.attributes[name] = value; }
  add(option) { this.children.push(option); if (this.children.length === 1) this.value = option.value; }
  replaceChildren(...items) { this.children = []; for (const item of items) this.add(item); }
  get options() { return this.children; }
  get selectedOptions() { return this.children.filter(item => item.value === this.value); }
  showModal() { this.open = true; }
  close() { this.open = false; for (const fn of this.listeners.close ?? []) fn(); }
  focus() { this.focused = true; }
  select() { this.selected = true; }
  async emit(name) {
    for (const fn of this.listeners[name] ?? []) await fn({preventDefault(){}, stopImmediatePropagation(){}});
  }
}
const settle = async () => { for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve)); };

function setup() {
  const elements = [];
  const menu = new Element('details'), summary = new Element('summary'), settings = new Element('div'), run = new Element('button');
  menu.querySelector = selector => selector === ':scope > summary' ? summary : null;
  const body = new Element('body');
  const document = {body, activeElement:{blur(){ document.blurred = true; }},
    getElementById(id) { return {mainMenu:menu, desktopSettingsMenu:settings, runButton:run}[id] ?? elements.find(el => el.id === id) ?? null; },
    createElement(tag) { const el = new Element(tag); elements.push(el); return el; }};
  const calls = [], sockets = [], logs = [], connections = [], worklets = [];
  let startError = null, stops = 0;
  const context = {sampleRate:48000, state:'running', destination:{}, audioWorklet:{async addModule(url){ worklets.push(String(url)); }}};
  const output = {connect(to){ connections.push(['connect', to]); }, disconnect(to){ connections.push(['disconnect', to]); }};
  class Socket {
    static OPEN = 1;
    constructor(url) {
      this.url = url; this.bufferedAmount = 0; this.readyState = 1; sockets.push(this);
      setImmediate(() => this.onopen?.());
    }
    close() { this.readyState = 3; this.onclose?.(); }
    send() {}
  }
  const listeners = {};
  const window = {__TAURI_INTERNALS__:{async invoke(command, data) {
    calls.push({command, ...data});
    if (data?.op === 'devices') return [{id:3, name:'Studio speakers', isDefault:true}];
    if (data?.op === 'start') {
      if (startError) throw startError;
      return {url:'ws://127.0.0.1:54321/audio', frames:128, sampleRate:48000};
    }
  }}, addEventListener(name, fn){ listeners[name] = fn; }};
  const environment = {window, document, URL, setTimeout, clearTimeout, WebSocket:Socket,
    Option:class {constructor(label, value){ this.textContent = label; this.value = value; }},
    AudioWorkletNode:class {constructor(){ this.port = {postMessage(){}}; } connect(){} disconnect(){} },
    runtime:{async ensureReady(){}, megaDrive:{audioContext:context, audio:{outputNode:output}}},
    async stopRun(){ stops++; }, logLine:line => logs.push(line), navigator:{clipboard:{async writeText(){}}}};
  const sandbox = vm.createContext(environment);
  vm.runInContext(source, sandbox);
  return {document, menu, summary, settings, body, run, calls, sockets, logs, connections, worklets, listeners,
    get stops(){return stops;}, failStart(error){startError = error;}, sandbox,
    get:id => document.getElementById(id)};
}

test('Audio output exposes one current-value menu row and keeps controls in a dialog', async () => {
  const app = setup();
  assert.equal(app.settings.children.length, 1);
  assert.equal(app.settings.children[0].id, 'desktop-audio-output');
  assert.equal(app.get('desktop-audio-value').textContent, 'WebView');
  const dialog = app.get('desktop-audio-dialog');
  assert.equal(dialog.attributes['aria-label'], 'Audio output');
  assert.equal(app.body.children.includes(dialog), true);
  assert.equal(app.settings.children.includes(app.get('desktop-audio-mode')), false);
  app.menu.open = true;
  await app.get('desktop-audio-output').emit('click');
  assert.equal(app.menu.open, false);
  assert.equal(app.document.blurred, true);
  assert.equal(dialog.open, true);
  assert.equal(app.get('desktop-audio-mode').focused, true);
  assert.equal(app.stops, 0);
  const close = dialog.children.find(el => el.textContent === 'Close');
  await close.emit('click');
  assert.equal(dialog.open, false);
  assert.equal(app.summary.focused, true);
});

test('menu value tracks applied output, device changes, and return to WebView', async () => {
  const app = setup();
  const mode = app.get('desktop-audio-mode'), device = app.get('desktop-audio-device');
  mode.value = 'audify';
  await mode.emit('change'); await settle();
  assert.equal(app.get('desktop-audio-value').textContent, 'WebView');
  device.value = '3';
  // Desktop transport retains its original constructor when user-script guards replace globals.
  app.sandbox.WebSocket = function(){ throw Error('User code networking is disabled'); };
  const apply = app.get('desktop-audio-dialog').children[1].children.find(el => el.textContent === 'Apply output');
  await apply.emit('click'); await settle();
  assert.equal(app.stops, 2);
  assert.equal(app.sockets.length, 1);
  assert.match(app.get('desktop-audio-value').textContent, /Audify.*Studio speakers/);
  assert.equal(app.calls.find(call => call.op === 'start').args.deviceId, 3);
  assert.equal(app.worklets.length, 1);
  mode.value = 'webview';
  await mode.emit('change'); await settle();
  assert.equal(app.get('desktop-audio-value').textContent, 'WebView');
  assert.equal(app.sockets[0].readyState, 3);
  assert.equal(app.stops, 3);
  assert.equal(app.run.disabled, false);
});

test('failed Audify apply keeps a truthful WebView summary and dialog error details', async () => {
  const app = setup();
  const mode = app.get('desktop-audio-mode');
  mode.value = 'audify';
  await mode.emit('change'); await settle();
  app.failStart(Error('Device unavailable\nNative backend detail'));
  const apply = app.get('desktop-audio-dialog').children[1].children.find(el => el.textContent === 'Apply output');
  await apply.emit('click'); await settle();
  assert.equal(app.get('desktop-audio-value').textContent, 'WebView');
  assert.equal(app.get('desktop-audio-error').hidden, false);
  assert.match(app.get('desktop-audio-error').value, /Native backend detail/);
  assert.equal(app.run.disabled, false);
  assert.equal(mode.disabled, false);
  assert.match(app.logs[0], /Device unavailable/);
});
