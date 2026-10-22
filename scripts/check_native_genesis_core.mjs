// Compare the native source snapshot against the actual Playground WASM and
// its renderFrames implementation. No output device, Tauri app or server opens.
import assert from 'node:assert/strict';
import {mkdtempSync, readFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {spawnSync} from 'node:child_process';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const source = resolve(process.argv[2] ?? join(repo, '../..'));
const directory = mkdtempSync(join(tmpdir(), 'tetorica-native-genesis-'));
try {
  const native = join(repo, 'src-tauri/native-genesis');
  const executable = join(directory, 'render');
  const nuked = join(repo, 'src-tauri/native-nuked/snapshot/third_party/nuked-opn2');
  const provider = join(directory, 'nuked.o');
  const symbols = ['Reset', 'SetChipType', 'Clock', 'Write', 'SetTestPin', 'ReadTestPin', 'ReadIRQPin', 'Read'];
  const c = spawnSync(process.env.CC ?? 'cc', ['-std=c11', '-O2', '-fwrapv', ...symbols.map(name => `-DOPN2_${name}=tetorica_nuked_OPN2_${name}`), '-I', nuked, '-c', join(nuked, 'ym3438.c'), '-o', provider], {encoding: 'utf8'});
  assert.equal(c.status, 0, c.stderr);
  const files = ['src/native_genesis_fixture.cpp', 'src/native_genesis_core.cpp', 'src/native_genesis_nuked.cpp',
    ...['ym2612_wasm.cpp', 'segapsg_wasm.cpp', 'ymfm_opn.cpp', 'ymfm_adpcm.cpp', 'ymfm_ssg.cpp', 'segapsg.cpp'].map(name => `native-genesis/${name}`)];
  const compiled = spawnSync(process.env.CXX ?? 'c++', ['-std=c++14', '-O2', '-I', native, '-I', nuked,
    ...files.map(file => join(repo, 'src-tauri', file)), provider, '-o', executable], {encoding: 'utf8'});
  assert.equal(compiled.status, 0, compiled.stderr);
  const [fmFactory, psgFactory, fmApi, psgApi] = await Promise.all([
    import(pathToFileURL(join(source, 'docs/generated/ym2612_wasm.js'))),
    import(pathToFileURL(join(source, 'docs/generated/segapsg_wasm.js'))),
    import(pathToFileURL(join(source, 'web/ym2612.js'))),
    import(pathToFileURL(join(source, 'web/segapsg.js'))),
  ]);
  const worklet = readFileSync(join(source, 'web/ym2612-worklet.js'), 'utf8');
  const begin = worklet.indexOf('  renderFrames(leftOut, rightOut, offset, frames) {');
  const end = worklet.indexOf('\n  flushMixerMeters(', begin);
  assert(begin >= 0 && end > begin, 'Use the actual Worklet render method');
  const method = worklet.slice(begin, end).trim().replace(/^renderFrames/, 'function');
  function patch(port = 0, channel = 0) {
    const commands = [];
    const write = (reg, value) => commands.push({kind: 'w', frame: 0, port, reg, value});
    for (const slot of [0, 4, 8, 12]) {
      write(0x30 + slot + channel, 1); write(0x40 + slot + channel, slot === 12 ? 20 : 127);
      write(0x50 + slot + channel, 31); write(0x60 + slot + channel, 0);
      write(0x70 + slot + channel, 0); write(0x80 + slot + channel, 15);
    }
    write(0xb0 + channel, 7); write(0xb4 + channel, 0xc0);
    write(0xa4 + channel, 0x22); write(0xa0 + channel, 0x84);
    commands.push({kind: 'w', frame: 0, port: 0, reg: 0x28, value: 0xf0 | channel | (port ? 4 : 0)});
    return commands;
  }
  const frames = 8192;
  for (const rate of [44100, 48000, 96000]) for (const scenario of ['fm', 'mixed-reset']) {
    const commands = patch(scenario === 'fm' ? 0 : 1, scenario === 'fm' ? 0 : 2);
    if (scenario === 'mixed-reset') commands.push(
      {kind: 'p', frame: 1111, value: 0x8e}, {kind: 'p', frame: 1111, value: 0x0f},
      {kind: 'p', frame: 1111, value: 0x94}, {kind: 's', frame: 4500, value: 0x10},
      {kind: 'r', frame: 6000}, {kind: 'p', frame: 6010, value: 0xe4}, {kind: 'p', frame: 6010, value: 0xf5});
    commands.sort((a, b) => a.frame - b.frame);
    const input = commands.map(c => `${c.kind} ${c.frame}${c.kind === 'w' ? ` ${c.port} ${c.reg} ${c.value}` : c.kind === 'r' ? '' : ` ${c.value}`}`).join('\n') + '\n';
    const result = spawnSync(executable, [String(rate), String(frames)], {input, maxBuffer: frames * 8 + 4096});
    assert.equal(result.status, 0, result.stderr.toString());
    assert.equal(result.stdout.length, frames * 8);
    const ym = await fmApi.createYm2612(fmFactory.default, {wasmBinary: readFileSync(join(source, 'docs/generated/ym2612_wasm.wasm'))});
    const psg = await psgApi.SegaPSG.create({moduleFactory: psgFactory.default,
      moduleOptions: {wasmBinary: readFileSync(join(source, 'docs/generated/segapsg_wasm.wasm'))},
      sampleRate: ym.sampleRate(), clock: 3579545});
    try {
      const idle = ym.generateStereoView(1);
      const state = {ym2612: ym, psg, resampleRemainder: 0, psgRemainder: 0,
        lastLeft: 0, lastRight: 0, psgLeft: 0, psgRight: 0, idleLeft: idle.left[0], idleRight: idle.right[0],
        mixRemaining: {ym2612: 0, segapsg: 0}, mixCurrent: {ym2612: [1, 1], segapsg: [1, 1]},
        mixMeterEnabled: {ym2612: false, segapsg: false}};
      ym.reset();
      const render = new Function('sampleRate', 'YM_GAIN', 'PSG_GAIN', 'MIX_SOURCES', 'clampSample', `return ${method}`)(
        rate, .9, .35, ['ym2612', 'segapsg'], value => Math.max(-1, Math.min(1, value)));
      const left = new Float32Array(frames), right = new Float32Array(frames);
      let offset = 0;
      for (const command of commands) {
        render.call(state, left, right, offset, command.frame - offset); offset = command.frame;
        if (command.kind === 'w') ym.writeRegister(command.reg, command.value, command.port);
        else if (command.kind === 'p') psg.write(command.value);
        else if (command.kind === 's') psg.setStereo(command.value);
        else { ym.reset(); psg.reset(); state.resampleRemainder = state.psgRemainder = 0;
          state.lastLeft = state.lastRight = state.psgLeft = state.psgRight = 0; }
      }
      render.call(state, left, right, offset, frames - offset);
      let difference = 0, peak = 0;
      for (let i = 0; i < frames; i++) for (let channel = 0; channel < 2; channel++) {
        const value = result.stdout.readFloatLE((i * 2 + channel) * 4);
        assert(Number.isFinite(value)); peak = Math.max(peak, Math.abs(value));
        difference = Math.max(difference, Math.abs(value - (channel ? right : left)[i]));
      }
      assert(peak > .01, 'The real chips must produce a signal');
      assert(difference < 2e-7, `${scenario}/${rate}: maximum native/WASM difference ${difference}`);
      if (scenario === 'fm') {
        const crossings = [];
        for (let i = 1025; i < frames; i++) if (left[i - 1] <= 0 && left[i] > 0) crossings.push(i);
        const measured = rate * (crossings.length - 1) / (crossings.at(-1) - crossings[0]);
        const expected = 0x284 * 7670454 * 2 ** (4 - 1) / (144 * 2 ** 20);
        assert(Math.abs(measured / expected - 1) < .01, `FM pitch ${measured} versus ${expected} Hz`);
      }
      console.log(`${scenario} ${rate} Hz: native/WASM PCM max difference ${difference}`);
    } finally { ym.dispose(); psg.dispose(); }
  }
} finally { rmSync(directory, {recursive: true, force: true}); }
