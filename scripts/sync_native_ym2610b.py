#!/usr/bin/env python3
"""Pin the browser YM2610B reference and verify its shared Native ymfm provider."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['wasm/ym2610b_wasm.cpp', 'wasm/opn_state.h', 'LICENSE']
SHARED = ['src/ymfm.h', 'src/ymfm_fm.h', 'src/ymfm_fm.ipp', 'src/ymfm_opn.h',
          'src/ymfm_opn.cpp', 'src/ymfm_adpcm.h', 'src/ymfm_adpcm.cpp',
          'src/ymfm_ssg.h', 'src/ymfm_ssg.cpp', 'src/chip_state.h',
          'wasm/ymfm_wasm_interface.h']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-ym2610b'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('YM2610B pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('YM2610B differs from browser reference: ' + name)
        for name, expected in info['sharedYmfm'].items():
            if digest((base / 'native-genesis' / Path(name).name).read_bytes()) != expected:
                raise SystemExit('YM2610B shared Native ymfm provider differs: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != expected:
                raise SystemExit('YM2610B shared source differs from browser: ' + name)
        print(f'Native YM2610B snapshot verified ({len(info["files"])} files, {len(info["sharedYmfm"])} shared ymfm sources)')
        return
    if not args.source:
        parser.error('--source /path/to/hello_ymfm is required when synchronizing')
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
            raise SystemExit('Synchronize native-genesis first; ymfm provider differs: ' + name)
    manifest.write_text(json.dumps({'source': 'hello_ymfm shared ymfm OPNB core',
        'license': 'BSD-3-Clause',
        'adaptation': 'Unmodified pinned ymfm provider, compiled once in native-genesis. native_genesis_ym2610b.cpp retains the primary dense-ROM adapter, adds independent four-FM sparse ROM with immutable control-prepared pages, mirrors the additional Worklet resampling, and provides observational metadata and host voice cancellation.',
        'files': files, 'sharedYmfm': shared}, indent=2) + '\n')
    print(f'Native YM2610B synchronized ({len(files)} files, {len(shared)} shared ymfm sources)')

if __name__ == '__main__':
    main()
