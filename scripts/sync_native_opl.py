#!/usr/bin/env python3
"""Pin DOS OPL Browser wrappers and verify the once-compiled shared ymfm core."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['wasm/ym3526_wasm.cpp', 'wasm/ym3812_wasm.cpp', 'wasm/ymf262_wasm.cpp',
         'web/ym3526.js', 'web/ym3812.js', 'web/ymf262.js', 'web/opl_synth.js',
         'web/playground_opl_worklet.js', 'web/playground_opl_inspector.js',
         'web/playground_chip_clock.js', 'wasm/ymfm_wasm_interface.h']
SHARED = ['src/ymfm_opl.cpp', 'src/ymfm_opl.h', 'src/ymfm_pcm.cpp', 'src/ymfm_pcm.h']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-opl'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('DOS OPL pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('DOS OPL differs from Browser reference: ' + name)
        for name, expected in info['sharedOplProvider'].items():
            if digest((base / 'native-master-system/snapshot' / name).read_bytes()) != expected:
                raise SystemExit('DOS OPL shared provider differs: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != expected:
                raise SystemExit('DOS OPL shared Browser source differs: ' + name)
        if (target / 'snapshot/wasm/ymfm_wasm_interface.h').read_bytes() != (base / 'native-genesis/ymfm_wasm_interface.h').read_bytes():
            raise SystemExit('DOS OPL timer interface differs from the compiled common interface')
        print(f'Native DOS OPL snapshot verified ({len(info["files"])} files, {len(info["sharedOplProvider"])} shared sources)')
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
    manifest.write_text(json.dumps({'source': 'hello_ymfm Browser YM3526/YM3812/YMF262 and shared ymfm OPL/PCM',
        'license': 'BSD-3-Clause (ymfm Aaron Giles)',
        'adaptation': 'Synthesis sources unchanged and compiled exactly once by native-master-system. Only the selected DOS OPL core is constructed. Native mirrors the shipped OPL Worklet source-rate average/hold resampler, all four OPL3 output buses folded A+C/B+D into stereo, authoring register shadow, forced channel mute and TL restoration, deferred manual retrigger, clock and source phase preservation, and silent Stop with retained configuration. Y8950/MSX Audio is not enabled.',
        'files': files, 'sharedOplProvider': shared}, indent=2) + '\n')
    print(f'Native DOS OPL synchronized ({len(files)} files, {len(shared)} shared sources)')

if __name__ == '__main__':
    main()
