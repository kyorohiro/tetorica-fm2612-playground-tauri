#!/usr/bin/env python3
"""Pin the shipped AY-3-8910 core and its Browser control/resampling reference."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['third_party/mame-ay8910/ay8910.cpp', 'third_party/mame-ay8910/ay8910.h',
         'third_party/mame-ay8910/LICENSE', 'third_party/mame-ay8910/README.md',
         'wasm/ay8910_wasm.cpp', 'web/ay8910.js', 'web/ay8910synth.js',
         'web/playground_ay8910_worklet.js', 'web/playground_ay8910_inspector.js',
         'web/playground_chip_clock.js']
SHARED = ['src/ymfm_opl.cpp', 'src/ymfm_opl.h', 'src/ymfm_pcm.cpp', 'src/ymfm_pcm.h',
          'web/playground_ym2413_worklet.js']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-msx'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('MSX pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('MSX differs from Browser reference: ' + name)
        for name, expected in info['sharedOpllProvider'].items():
            if digest((base / 'native-master-system/snapshot' / name).read_bytes()) != expected:
                raise SystemExit('MSX shared OPLL provider differs: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != expected:
                raise SystemExit('MSX shared Browser OPLL source differs: ' + name)
        print(f'Native MSX snapshot verified ({len(info["files"])} files, {len(info["sharedOpllProvider"])} shared OPLL sources)')
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
    shared = {name: digest((args.source / name).read_bytes()) for name in SHARED}
    for name, expected in shared.items():
        if digest((base / 'native-master-system/snapshot' / name).read_bytes()) != expected:
            raise SystemExit('Synchronize native-master-system first: ' + name)
    manifest.write_text(json.dumps({'source': 'hello_ymfm Browser AY-3-8910 and shared YM2413',
        'license': 'BSD-3-Clause (AY Couriersud; ymfm Aaron Giles)',
        'adaptation': 'AY synthesis sources unchanged. Native keeps its original base-clock core and mirrors ChipClockResampler double phase/average/hold across clock edits; register volume-zero gates suppress the same normalized DAC baseline as the Worklet. Stop retains separate authored levels and all pitch/mixer/noise/envelope/I/O settings while clearing active volume registers. YM2413 uses the same shared NativeOpllCore as Master System; one common provider is compiled once. Both chips produce separate mono output planes.',
        'files': files, 'sharedOpllProvider': shared}, indent=2) + '\n')
    print(f'Native MSX synchronized ({len(files)} files, {len(shared)} shared OPLL sources)')

if __name__ == '__main__':
    main()
