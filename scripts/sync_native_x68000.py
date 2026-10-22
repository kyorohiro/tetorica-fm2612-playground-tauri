#!/usr/bin/env python3
"""Pin Browser OPM/OKI sources and verify the one shared Native ymfm provider."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['src/ymfm_opm.cpp', 'src/ymfm_opm.h', 'wasm/ym2151_wasm.cpp',
         'wasm/okim6258_wasm.cpp', 'web/ym2151audioengine.js', 'web/okim6258synth.js',
         'third_party/mame-okim6258/okim6258.cpp', 'third_party/mame-okim6258/okim6258.h',
         'third_party/mame-okim6258/LICENSE', 'third_party/mame-okim6258/README.md', 'LICENSE']
SHARED = ['src/ymfm.h', 'src/ymfm_fm.h', 'src/ymfm_fm.ipp', 'src/chip_state.h',
          'wasm/ymfm_wasm_interface.h']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def native_oki(data):
    old = b'{ compute_tables();reset(); }'
    new = b'{ static const bool initialized = []{compute_tables();return true;}();(void)initialized;reset(); }'
    if data.count(old) != 1:
        raise SystemExit('OKI constructor adaptation anchor changed')
    return data.replace(old, new)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-x68000'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('X68000 pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('X68000 differs from Browser reference: ' + name)
        for name, expected in info['sharedYmfm'].items():
            if digest((base / 'native-genesis' / Path(name).name).read_bytes()) != expected:
                raise SystemExit('X68000 shared Native ymfm provider differs: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != expected:
                raise SystemExit('X68000 shared source differs from Browser: ' + name)
        if (target / 'okim6258.cpp').read_bytes() != native_oki((target / 'snapshot/third_party/mame-okim6258/okim6258.cpp').read_bytes()):
            raise SystemExit('X68000 generated one-time OKI initialization differs')
        print(f'Native X68000 snapshot verified ({len(info["files"])} files, {len(info["sharedYmfm"])} shared ymfm sources)')
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
    (target / 'okim6258.cpp').write_bytes(native_oki((args.source / 'third_party/mame-okim6258/okim6258.cpp').read_bytes()))
    manifest.write_text(json.dumps({'source': 'hello_ymfm Browser YM2151 and OKIM6258',
        'license': 'BSD-3-Clause (Aaron Giles / Barry Rodewald)',
        'adaptation': 'Pinned synthesis sources. Generated okim6258.cpp initializes its shared ADPCM difference table once using thread-safe C++ static initialization; decoder unchanged. Native wrapper mirrors Browser resampling, authored TL/Stop state and Oki6258Stream byte feeding; two sample banks reserve 16 MiB each at construction.',
        'files': files, 'sharedYmfm': shared}, indent=2) + '\n')
    print(f'Native X68000 synchronized ({len(files)} files, {len(shared)} shared ymfm sources)')

if __name__ == '__main__':
    main()
