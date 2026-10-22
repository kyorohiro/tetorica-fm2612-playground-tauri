#!/usr/bin/env python3
"""Pin Game Gear Browser controls and verify the shared Sega PSG synthesis core."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['wasm/segapsg_wasm.cpp', 'web/segapsg.js', 'web/segapsg_api.js',
         'web/playground_segapsg.js', 'web/playground_segapsg_worklet.js',
         'web/playground_chip_inspector_state.js', 'web/playground_chip_clock.js']
SHARED = ['src/segapsg.h', 'src/segapsg.cpp', 'src/chip_state.h']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-game-gear'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('Game Gear pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('Game Gear differs from Browser reference: ' + name)
        for name, expected in info['sharedPsgProvider'].items():
            if digest((base / 'native-genesis' / Path(name).name).read_bytes()) != expected:
                raise SystemExit('Game Gear shared PSG provider differs: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != expected:
                raise SystemExit('Game Gear shared Browser PSG source differs: ' + name)
        print(f'Native Game Gear snapshot verified ({len(info["files"])} files, {len(info["sharedPsgProvider"])} shared sources)')
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
    manifest.write_text(json.dumps({'source': 'hello_ymfm Browser Game Gear Sega PSG',
        'license': 'Project Sega PSG, same existing Native Genesis provider',
        'adaptation': 'Synthesis sources unchanged and compiled exactly once by native-genesis. NativePsgCore is shared with Master System; Game Gear constructs only one standalone PSG. Stereo register low nibble routes channels to right and high nibble to left. Fixed creation clock, source phase and ChipClockResampler average/hold are preserved across clock edits. Stop retains stereo mask, clock, tone periods, noise mode and remembered attenuation without resetting LFSR. Reset restores stereo 0xff while retaining the selected clock and external source mute.',
        'files': files, 'sharedPsgProvider': shared}, indent=2) + '\n')
    print(f'Native Game Gear synchronized ({len(files)} files, {len(shared)} shared sources)')

if __name__ == '__main__':
    main()
