#!/usr/bin/env node
// Runs only the offline stability test. Never runs ignored physical-device tests.
import {spawnSync} from 'node:child_process';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {dirname, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const option = name => {
  const inline = args.find(value => value.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = args.indexOf(name);
  return index < 0 ? undefined : args[index + 1];
};
if (args.includes('--help')) {
  console.log('Usage: node scripts/check_native_stability.mjs [--out=report.json] [--long] [--seconds=30] [--rounds=3] [--compare=previous.json] [--release]');
  console.log('Default: 23 modes, 2 constructor/drop rounds, two 0.125-second runs per round, offline and no audio devices.');
  console.log('--long defaults to 30 simulated seconds per run and 3 rounds. Timings are diagnostic; no absolute speed threshold.');
  process.exit(0);
}
const out = resolve(option('--out') ?? resolve(repository, 'native-validation/stability.json'));
await mkdir(dirname(out), {recursive: true});
let report = {schemaVersion: 1, passed: false, deviceOpened: false, modes: []};
const start = Date.now();
try {
  const long = args.includes('--long');
  const seconds = Number(option('--seconds') ?? (long ? 30 : 0.125));
  const rounds = Number(option('--rounds') ?? (long ? 3 : 2));
  if (!Number.isFinite(seconds) || seconds < 0.09 || seconds > 3600) throw new Error('--seconds must be between 0.09 and 3600');
  if (!Number.isInteger(rounds) || rounds < 1 || rounds > 1000) throw new Error('--rounds must be an integer between 1 and 1000');
  const cargoArgs = ['test', '--locked', '--offline', '--manifest-path', 'src-tauri/Cargo.toml'];
  if (args.includes('--release')) cargoArgs.push('--release');
  cargoArgs.push('native_stability_', '--', '--test-threads=1', '--nocapture');
  await writeFile(out, JSON.stringify({...report, runner: {state: 'running', command: ['cargo', ...cargoArgs]}}, null, 2) + '\n');
  const result = spawnSync('cargo', cargoArgs, {
    cwd: repository,
    env: {...process.env, TETORICA_NATIVE_STABILITY_PROFILE: long ? 'long' : 'bounded',
      TETORICA_NATIVE_STABILITY_SECONDS: String(seconds), TETORICA_NATIVE_STABILITY_ROUNDS: String(rounds),
      TETORICA_NATIVE_STABILITY_REPORT: out},
    stdio: 'inherit',
  });
  report = JSON.parse(await readFile(out, 'utf8'));
  report.runner = {state: 'complete', platform: process.platform, arch: process.arch,
    command: ['cargo', ...cargoArgs], exitCode: result.status, signal: result.signal,
    wallSeconds: (Date.now() - start) / 1000, ...(result.error ? {error: result.error.message} : {})};
  report.passed = result.status === 0 && report.passed === true && report.modes.length === 23;
  if (option('--compare')) {
    const baseline = JSON.parse(await readFile(resolve(option('--compare')), 'utf8'));
    report.comparison = {baseline: resolve(option('--compare')),
      comparableConfiguration: baseline.profile === report.profile && baseline.sampleRate === report.sampleRate &&
        baseline.secondsPerRun === report.secondsPerRun && baseline.rounds === report.rounds && baseline.timingBuild === report.timingBuild,
      note: 'Timing ratios are diagnostic. Host load, compiler, architecture, and build settings can change results.',
      modes: report.modes.filter(mode => mode.passed).map(mode => {
        const previous = baseline.modes?.find(item => item.mode === mode.mode && item.passed);
        return {mode: mode.mode, realtimeFactor: mode.realtimeFactor,
          baselineRealtimeFactor: previous?.realtimeFactor ?? null,
          realtimeFactorRatio: previous?.realtimeFactor > 0 ? mode.realtimeFactor / previous.realtimeFactor : null};
      })};
  }
} catch (error) {
  report.passed = false;
  report.runner = {...report.runner, state: 'failed', error: String(error?.stack ?? error), wallSeconds: (Date.now() - start) / 1000};
}
await writeFile(out, JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify({passed: report.passed, modes: report.modes?.length ?? 0, deviceOpened: false, report: out}));
if (!report.passed) process.exitCode = 1;
