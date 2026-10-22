#!/usr/bin/env python3
"""Pin Browser Master System sources and verify the shared Native ymfm/PSG provider."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['src/ymfm_opl.cpp', 'src/ymfm_opl.h', 'src/ymfm_pcm.cpp', 'src/ymfm_pcm.h',
         'wasm/ym2413_wasm.cpp', 'wasm/segapsg_wasm.cpp', 'web/ym2413.js', 'web/ym2413audioengine.js',
         'web/playground_ym2413_worklet.js', 'web/playground_segapsg_worklet.js',
         'web/playground_chip_clock.js', 'LICENSE']
SHARED = ['src/ymfm.h', 'src/ymfm_fm.h', 'src/ymfm_fm.ipp', 'src/ymfm_adpcm.h',
          'src/ymfm_adpcm.cpp', 'src/chip_state.h', 'src/segapsg.h', 'src/segapsg.cpp',
          'wasm/ymfm_wasm_interface.h']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-master-system'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('Master System pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('Master System differs from Browser reference: ' + name)
        for name, expected in info['sharedProvider'].items():
            if digest((base / 'native-genesis' / Path(name).name).read_bytes()) != expected:
                raise SystemExit('Master System shared Native provider differs: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != expected:
                raise SystemExit('Master System shared Browser source differs: ' + name)
        print(f'Native Master System snapshot verified ({len(info["files"])} files, {len(info["sharedProvider"])} shared sources)')
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
    manifest.write_text(json.dumps({'source': 'hello_ymfm Browser Sega PSG and YM2413',
        'license': 'BSD-3-Clause (ymfm Aaron Giles; project Sega PSG)',
        'adaptation': 'Synthesis sources unchanged. Native wrapper retains the fixed PSG base-clock core and mirrors ChipClockResampler double phase/average/hold on clock edits. OPLL mirrors Worklet source-rate average/hold, authored volume masking, physical drum sharing and deferred manual retrigger. OPLL reset restores a power-on snapshot allocated at construction, matching Browser core recreation without callback allocation/free. Both outputs are mono before individual chip mixing. Common ymfm and Sega PSG sources are compiled once in native-genesis.',
        'files': files, 'sharedProvider': shared}, indent=2) + '\n')
    print(f'Native Master System synchronized ({len(files)} files, {len(shared)} shared sources)')

if __name__ == '__main__':
    main()
