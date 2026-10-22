#!/usr/bin/env python3
"""Pin PC-98 YM2203 Browser references and verify shared OPN/SSG providers."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['wasm/ym2203_wasm.cpp', 'wasm/opn_state.h', 'web/ym2203.js',
         'web/ym2203audioengine.js', 'web/ym2203-worklet.js',
         'web/playground_opn_worklet.js', 'web/playground_opn_inspector.js',
         'web/playground_audition_guard.js', 'wasm/ymfm_wasm_interface.h']
SHARED = ['src/ymfm.h', 'src/ymfm_fm.h', 'src/ymfm_fm.ipp', 'src/ymfm_opn.cpp',
          'src/ymfm_opn.h', 'src/ymfm_ssg.cpp', 'src/ymfm_ssg.h',
          'src/ymfm_adpcm.cpp', 'src/ymfm_adpcm.h']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-ym2203'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('YM2203 pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('YM2203 differs from Browser reference: ' + name)
        for name, expected in info['sharedProviders'].items():
            if digest((base / 'native-genesis' / Path(name).name).read_bytes()) != expected:
                raise SystemExit('YM2203 shared Native provider differs: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != expected:
                raise SystemExit('YM2203 shared Browser source differs: ' + name)
        print(f'Native YM2203 snapshot verified ({len(info["files"])} files, {len(info["sharedProviders"])} shared sources)')
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
        if digest((base / 'native-genesis' / Path(name).name).read_bytes()) != expected:
            raise SystemExit('Synchronize native-genesis first: ' + name)
    manifest.write_text(json.dumps({'source': 'hello_ymfm Browser PC-98 YM2203 / OPN',
        'license': 'BSD-3-Clause (ymfm Aaron Giles)',
        'adaptation': 'Unmodified OPN/SSG synthesis is compiled exactly once by native-genesis. The selected standalone YM2203 has three FM and three SSG voices with no Genesis/OPNA chip. FM plus all three SSG integer outputs are summed and clipped before the shipped source-rate average/hold resampler; mono is duplicated to L/R. Default master clock is 4000000 Hz and MAX fidelity output rate is clock/4. Prescalers, frequency latches/channel3 special mode, timer/status/IRQ and SSG I/O reads retain core semantics. Forced mute preserves authored TL; SSG Stop retains authored level including explicit zero, periods, shared noise/envelope and clock. Preview retrigger waits at least one real FM clock interval.',
        'files': files, 'sharedProviders': shared}, indent=2) + '\n')
    print(f'Native YM2203 synchronized ({len(files)} files, {len(shared)} shared sources)')

if __name__ == '__main__':
    main()
