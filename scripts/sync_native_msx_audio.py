#!/usr/bin/env python3
"""Pin MSX Audio Y8950 controls and verify existing shared MSX/ymfm providers."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['wasm/y8950_wasm.cpp', 'web/y8950.js', 'web/opl_synth.js',
         'web/playground_opl.js', 'web/playground_opl_worklet.js',
         'web/playground_opl_inspector.js', 'wasm/ymfm_wasm_interface.h']
SHARED = {'native-master-system/snapshot': ['src/ymfm_opl.cpp', 'src/ymfm_opl.h',
          'src/ymfm_pcm.cpp', 'src/ymfm_pcm.h'],
          'native-msx/snapshot': ['third_party/mame-ay8910/ay8910.cpp',
          'third_party/mame-ay8910/ay8910.h']}

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-msx-audio'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('MSX Audio pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('MSX Audio differs from Browser reference: ' + name)
        for name, entry in info['sharedProviders'].items():
            if digest((base / entry['native']).read_bytes()) != entry['sha256']:
                raise SystemExit('MSX Audio shared provider differs: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != entry['sha256']:
                raise SystemExit('MSX Audio shared Browser source differs: ' + name)
        if (target / 'snapshot/wasm/ymfm_wasm_interface.h').read_bytes() != (base / 'native-genesis/ymfm_wasm_interface.h').read_bytes():
            raise SystemExit('MSX Audio timer interface differs from compiled common interface')
        print(f'Native MSX Audio snapshot verified ({len(info["files"])} files, {len(info["sharedProviders"])} shared sources)')
        return
    if not args.source:
        parser.error('--source is required to synchronize')
    files = {}
    for name in FILES:
        data = (args.source / name).read_bytes()
        relative = 'snapshot/' + name
        path = target / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        files[name] = {'snapshot': relative, 'sha256': digest(data)}
    shared = {}
    for provider, names in SHARED.items():
        for name in names:
            native = provider + '/' + name
            expected = digest((args.source / name).read_bytes())
            if digest((base / native).read_bytes()) != expected:
                raise SystemExit('Synchronize shared provider first: ' + native)
            shared[name] = {'native': native, 'sha256': expected}
    manifest.write_text(json.dumps({'source': 'hello_ymfm Browser MSX Audio Y8950 plus shared MSX pair',
        'license': 'BSD-3-Clause (ymfm Aaron Giles; AY Couriersud)',
        'adaptation': 'Unmodified shared ymfm synthesizes the selected single Y8950 core (profile 3). Rust combines it with the existing independently mixed MSX YM2413/AY pair. The public OPLSynth FM/rhythm API matches the shipped Web Worklet, including source-rate average/hold, forced mute/TL restoration, delayed preview retrigger, timers, status and IRQ. No new ADPCM RAM/upload API is exposed. Empty ADPCM RAM and external I/O use zero reads/no-op writes, matching the shipped WASM sample_interface before loading memory; raw register writes and data/status reads pass through the same ymfm chip. Timer advancement remains exactly 72 master clocks per source sample.',
        'files': files, 'sharedProviders': shared}, indent=2) + '\n')
    print(f'Native MSX Audio synchronized ({len(files)} files, {len(shared)} shared sources)')

if __name__ == '__main__':
    main()
