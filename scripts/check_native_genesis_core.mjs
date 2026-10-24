// Compare the application's pinned SDK sources against the actual Playground WASM and
// its renderFrames implementation. No output device, Tauri app or server opens.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {spawn, spawnSync} from 'node:child_process';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const options = {}; let sourceArgument;
for (let i = 2; i < process.argv.length; i++) {
  const argument = process.argv[i];
  if (argument === '--help') {
    console.log('Usage: node scripts/check_native_genesis_core.mjs [hello_ymfm-path] [--seconds=60] [--rate=48000] [--out=report.json]');
    console.log('Without --seconds the original six short PCM checks run. Continuous mode accepts 1–80 seconds and streams PCM without retaining a full recording.');
    process.exit(0);
  }
  if (argument.startsWith('--')) {
    const [key, inline] = argument.slice(2).split(/=(.*)/s, 2);
    assert(['seconds', 'rate', 'out'].includes(key), `Unknown option --${key}`);
    const value = inline ?? process.argv[++i];
    assert(value && !value.startsWith('--'), `--${key} needs a value`); options[key] = value;
  } else { assert(sourceArgument === undefined, 'Only one parent checkout path is accepted'); sourceArgument = argument; }
}
const source = resolve(sourceArgument ?? join(repo, '../..'));
const longMode = options.seconds !== undefined;
const seconds = Number(options.seconds);
if (longMode) assert(Number.isFinite(seconds) && seconds >= 1 && seconds <= 80, '--seconds must be between 1 and 80');
const rates = options.rate === undefined ? [44100, 48000, 96000] : [Number(options.rate)];
assert(rates.every(rate => [44100, 48000, 96000].includes(rate)), '--rate must be 44100, 48000 or 96000');
const report = {schemaVersion: 1, deviceOpened: false, continuous: longMode, rates, cases: [], maximumDifference: 0,
  timingNotes: 'Timing is diagnostic, with no speed threshold. Native render time measures 128-frame render blocks including command application and timing instrumentation, excluding stdout writes/backpressure; setup includes command parsing and construction. WASM render time measures the actual Worklet renderFrames method in blocks of at most 128 output frames. WASM module initialization and command writes are measured separately. Compiler, process wall, pipe transport and JS comparison overhead are reported separately; native and JS run concurrently and wall times are not additive. These are wall-clock measurements, not OS CPU accounting. PCM is streamed through bounded chunks and two 128-frame reference buffers; full recordings are never retained in continuous mode. No listening or output device is involved.'};
const started = performance.now();
const directory = mkdtempSync(join(tmpdir(), 'tetorica-native-genesis-'));
try {
  const native = join(repo, 'src-tauri/native-genesis');
  const sdk = 'vendor/tetorica-soundchip-native';
  const sdkRoot = join(repo, 'src-tauri', sdk);
  const executable = join(directory, 'render');
  const nuked = join(repo, 'src-tauri/native-nuked/snapshot/third_party/nuked-opn2');
  const provider = join(directory, 'nuked.o');
  const symbols = ['Reset', 'SetChipType', 'Clock', 'Write', 'SetTestPin', 'ReadTestPin', 'ReadIRQPin', 'Read'];
  const compileStart = performance.now();
  const c = spawnSync(process.env.CC ?? 'cc', ['-std=c11', '-O2', '-fwrapv', ...symbols.map(name => `-DOPN2_${name}=tetorica_nuked_OPN2_${name}`), '-I', nuked, '-c', join(nuked, 'ym3438.c'), '-o', provider], {encoding: 'utf8'});
  assert.equal(c.status, 0, c.stderr);
  // Match build.rs: the app retains Genesis gain/DC/mixing and PSG, while its
  // primary YM2612 adapter and once-compiled OPN/SSG/ADPCM provider use the SDK.
  const files = ['src/native_genesis_fixture.cpp', 'src/native_genesis_core.cpp', 'src/native_genesis_nuked.cpp',
    `${sdk}/wasm/ym2612_wasm.cpp`, `${sdk}/native/src/ymfm_opn_provider.cpp`,
    `${sdk}/vendor/ymfm/ymfm_adpcm.cpp`, `${sdk}/vendor/ymfm/ymfm_ssg.cpp`,
    'native-genesis/segapsg_wasm.cpp', 'native-genesis/segapsg.cpp'];
  report.nativeSources = files.map(file => ({file: `src-tauri/${file}`,
    sha256: createHash('sha256').update(readFileSync(join(repo, 'src-tauri', file))).digest('hex')}));
  const compiled = spawnSync(process.env.CXX ?? 'c++', ['-std=c++14', '-O2',
    '-I', join(sdkRoot, 'vendor/ymfm'), '-I', join(sdkRoot, 'wasm'), '-I', native, '-I', nuked,
    ...files.map(file => join(repo, 'src-tauri', file)), provider, '-o', executable], {encoding: 'utf8'});
  assert.equal(compiled.status, 0, compiled.stderr);
  report.compilerMilliseconds = performance.now() - compileStart;
  const [fmFactory, psgFactory, fmApi, psgApi] = await Promise.all([
    import(pathToFileURL(join(source, 'docs/generated/ym2612_wasm.js'))),
    import(pathToFileURL(join(source, 'docs/generated/segapsg_wasm.js'))),
    import(pathToFileURL(join(source, longMode ? 'docs/js/ym2612.js' : 'web/ym2612.js'))),
    import(pathToFileURL(join(source, longMode ? 'docs/js/segapsg.js' : 'web/segapsg.js'))),
  ]);
  const workletPath = longMode ? 'docs/js/ym2612-worklet.js' : 'web/ym2612-worklet.js';
  const worklet = readFileSync(join(source, workletPath), 'utf8');
  report.reference = {worklet: workletPath, workletSha256: createHash('sha256').update(worklet).digest('hex'),
    fmWasmSha256: createHash('sha256').update(readFileSync(join(source, 'docs/generated/ym2612_wasm.wasm'))).digest('hex'),
    psgWasmSha256: createHash('sha256').update(readFileSync(join(source, 'docs/generated/segapsg_wasm.wasm'))).digest('hex')};
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
  if (longMode) {
    for (const rate of rates) for (const mixed of [false, true]) {
      const frames = Math.round(rate * seconds), port = mixed ? 1 : 0, channel = mixed ? 2 : 0;
      const key = channel | (port ? 4 : 0), commands = patch(port, channel);
      let seed = 0x2612;
      const notes = [52, 55, 57, 59, 62, 64, 67, 69, 71, 74];
      for (let index = 0; Math.round(index * .25 * rate) < frames; index++) {
        const frame = Math.round(index * .25 * rate);
        seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
        const midi = notes[seed % notes.length], hz = 440 * 2 ** ((midi - 69) / 12);
        let block = 0, fnum;
        for (; block < 7; block++) { fnum = Math.round(hz * 144 * 2 ** 20 / 7670454 / 2 ** (block - 1)); if (fnum < 2048) break; }
        const w = (reg, value) => commands.push({kind: 'w', frame, port: reg === 0x28 ? 0 : port, reg, value});
        w(0x28, key); w(0xa4 + channel, (block << 3) | (fnum >> 8)); w(0xa0 + channel, fnum & 255); w(0x28, 0xf0 | key);
        if (mixed) {
          const period = Math.max(1, Math.min(1023, Math.round(3579545 / (32 * hz))));
          for (const value of [0x80 | (period & 15), period >> 4, 0x9a]) commands.push({kind: 'p', frame, value});
        }
        const off = Math.round((index * .25 + .2) * rate);
        if (off < frames) {
          commands.push({kind: 'w', frame: off, port: 0, reg: 0x28, value: key});
          if (mixed) commands.push({kind: 'p', frame: off, value: 0x9f});
        }
      }
      commands.sort((a, b) => a.frame - b.frame);
      assert(commands.every(command => command.kind !== 'r'), 'Continuous comparison must never reset the running chips');
      const input = commands.map(c => `${c.kind} ${c.frame}${c.kind === 'w' ? ` ${c.port} ${c.reg} ${c.value}` : ` ${c.value}`}`).join('\n') + '\n';
      const initStart = performance.now();
      const ym = await fmApi.createYm2612(fmFactory.default, {wasmBinary: readFileSync(join(source, 'docs/generated/ym2612_wasm.wasm'))});
      const psg = await psgApi.SegaPSG.create({moduleFactory: psgFactory.default,
        moduleOptions: {wasmBinary: readFileSync(join(source, 'docs/generated/segapsg_wasm.wasm'))}, sampleRate: ym.sampleRate(), clock: 3579545});
      try {
        const idle = ym.generateStereoView(1);
        const state = {ym2612: ym, psg, resampleRemainder: 0, psgRemainder: 0,
          lastLeft: 0, lastRight: 0, psgLeft: 0, psgRight: 0, idleLeft: idle.left[0], idleRight: idle.right[0],
          mixRemaining: {ym2612: 0, segapsg: 0}, mixCurrent: {ym2612: [1, 1], segapsg: [1, 1]},
          mixMeterEnabled: {ym2612: false, segapsg: false}};
        ym.reset(); ym.reserveStereoFrames(Math.ceil(128 * ym.sampleRate() / rate)); psg.reserveStereoFrames(Math.ceil(128 * ym.sampleRate() / rate));
        const render = new Function('sampleRate', 'YM_GAIN', 'PSG_GAIN', 'MIX_SOURCES', 'clampSample', `return ${method}`)(
          rate, .9, .35, ['ym2612', 'segapsg'], value => Math.max(-1, Math.min(1, value)));
        const wasmInitialiseMilliseconds = performance.now() - initStart;
        const processStart = performance.now(), child = spawn(executable, [String(rate), String(frames), '--timing'], {stdio: ['pipe', 'pipe', 'pipe']});
        let errors = ''; child.stderr.setEncoding('utf8'); child.stderr.on('data', chunk => { errors = (errors + chunk).slice(-65536); });
        const completion = new Promise((resolve, reject) => { child.once('error', reject); child.once('close', (code, signal) => resolve({code, signal})); });
        completion.catch(() => {}); child.stdin.on('error', () => {}); child.stdin.end(input);
        const left = new Float32Array(128), right = new Float32Array(128);
        let frame = 0, next = 0, carry = Buffer.alloc(0), difference = 0, peak = 0, wasmRenderMilliseconds = 0, wasmCommandMilliseconds = 0, jsComparisonMilliseconds = 0, maximumNativeChunkBytes = 0;
        try {
          for await (const chunk of child.stdout) {
            const data = carry.length ? Buffer.concat([carry, chunk]) : chunk;
            maximumNativeChunkBytes = Math.max(maximumNativeChunkBytes, data.length);
            const available = Math.floor(data.length / 8); let offset = 0;
            while (offset < available) {
              assert(frame < frames, 'Native fixture emitted excess PCM');
              const commandStart = performance.now();
              while (next < commands.length && commands[next].frame === frame) {
                const command = commands[next++];
                if (command.kind === 'w') ym.writeRegister(command.reg, command.value, command.port); else psg.write(command.value);
              }
              wasmCommandMilliseconds += performance.now() - commandStart;
              const count = Math.min(128, available - offset, frames - frame, next < commands.length ? commands[next].frame - frame : Infinity);
              assert(count > 0, 'The shared event timeline must advance');
              const renderStart = performance.now(); render.call(state, left, right, 0, count); wasmRenderMilliseconds += performance.now() - renderStart;
              const comparisonStart = performance.now();
              for (let i = 0; i < count; i++) for (let c = 0; c < 2; c++) {
                const value = data.readFloatLE(((offset + i) * 2 + c) * 4);
                assert(Number.isFinite(value)); peak = Math.max(peak, Math.abs(value)); difference = Math.max(difference, Math.abs(value - (c ? right : left)[i]));
              }
              jsComparisonMilliseconds += performance.now() - comparisonStart;
              frame += count; offset += count;
            }
            carry = Buffer.from(data.subarray(available * 8));
          }
          const result = await completion;
          assert.equal(result.code, 0, `Native fixture failed (${result.signal}): ${errors}`);
          assert.equal(carry.length, 0); assert.equal(frame, frames); assert.equal(next, commands.length);
        } catch (error) { child.kill(); await completion.catch(() => {}); throw error; }
        const nativeProcessWallMilliseconds = performance.now() - processStart;
        const native = JSON.parse(errors.trim().split(/\r?\n/).at(-1)); assert.equal(native.frames, frames);
        assert(peak > .01, 'Continuous comparison must contain a real signal');
        assert(difference < 2e-7, `${mixed ? 'continuous-fm-and-psg' : 'continuous-fm'}/${rate}: maximum native/WASM difference ${difference}`);
        const entry = {name: mixed ? 'continuous-fm-and-psg' : 'continuous-fm', sampleRate: rate, frames, seconds: frames / rate,
          events: commands.length, seed: '0x2612', resetsDuringSegment: 0, maximumDifference: difference, peak,
          nativeRenderMilliseconds: native.nativeRenderMilliseconds, nativeSetupMilliseconds: native.nativeSetupMilliseconds,
          nativeWriteMilliseconds: native.nativeWriteMilliseconds, nativeStreamMilliseconds: native.nativeStreamMilliseconds,
          nativeProcessWallMilliseconds, wasmInitialiseMilliseconds, wasmRenderMilliseconds, wasmCommandMilliseconds, jsComparisonMilliseconds,
          commandStreamSha256: createHash('sha256').update(input).digest('hex'),
          nativeRenderRealtimeFactor: frames / rate * 1000 / native.nativeRenderMilliseconds,
          wasmRenderRealtimeFactor: frames / rate * 1000 / wasmRenderMilliseconds,
          streamedPcmBytes: frames * 8, maximumNativeChunkBytes, referencePcmBufferBytes: left.byteLength + right.byteLength};
        report.cases.push(entry); report.maximumDifference = Math.max(report.maximumDifference, difference);
        console.log(`${entry.name} ${rate} Hz / ${entry.seconds}s: native/WASM PCM max difference ${difference}; native render ${native.nativeRenderMilliseconds.toFixed(1)} ms, WASM render ${wasmRenderMilliseconds.toFixed(1)} ms`);
      } finally { ym.dispose(); psg.dispose(); }
    }
  } else {
  const frames = 8192;
  for (const rate of rates) for (const scenario of ['fm', 'mixed-reset']) {
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
      report.cases.push({name: scenario, sampleRate: rate, frames, seconds: frames / rate, maximumDifference: difference, peak}); report.maximumDifference = Math.max(report.maximumDifference, difference);
      console.log(`${scenario} ${rate} Hz: native/WASM PCM max difference ${difference}`);
    } finally { ym.dispose(); psg.dispose(); }
  }
  }
  report.totalStereoFrames = report.cases.reduce((sum, value) => sum + value.frames, 0);
  report.totalSimulatedSeconds = report.cases.reduce((sum, value) => sum + value.seconds, 0);
  report.status = 'passed';
} catch (error) { report.status = 'failed'; report.error = String(error?.stack ?? error); throw error; }
finally {
  report.wallMilliseconds = performance.now() - started;
  if (options.out) { const out = resolve(options.out); mkdirSync(dirname(out), {recursive: true}); writeFileSync(out, JSON.stringify(report, null, 2) + '\n'); }
  rmSync(directory, {recursive: true, force: true});
}
