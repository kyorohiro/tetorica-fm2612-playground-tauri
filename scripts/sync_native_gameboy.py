#!/usr/bin/env python3
"""Pin the existing browser Game Boy core for the Native backend (DSP unchanged)."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['third_party/mame-gameboy/gameboy_apu.cpp',
         'third_party/mame-gameboy/gameboy_apu.h',
         'third_party/mame-gameboy/LICENSE',
         'third_party/mame-gameboy/README.md', 'wasm/gameboy_apu_wasm.cpp']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def adapt(name, data):
    if name.endswith('/gameboy_apu.h'):
        old = b'class GameboyApu {\n'
        if data.count(old) != 1:
            raise SystemExit('Game Boy class declaration changed')
        return data.replace(old, old + b'    friend struct NativeGameboyAccess; // Native state observation / gate-only Stop; DSP unchanged.\n')
    return data

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[1]
    destination = repository / 'src-tauri/native-gameboy'
    manifest = destination / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (destination / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('Game Boy pinned snapshot changed: ' + name)
            if args.source and adapt(name, (args.source / name).read_bytes()) != data:
                raise SystemExit('Game Boy snapshot differs from browser core: ' + name)
        if (destination / 'LICENSE').read_bytes() != (destination / 'snapshot/third_party/mame-gameboy/LICENSE').read_bytes():
            raise SystemExit('Game Boy license alias changed')
        print(f'Native Game Boy source verified ({len(info["files"])} files)')
        return
    if not args.source:
        parser.error('--source /path/to/hello_ymfm is required when synchronizing')
    files = {}
    for name in FILES:
        original = (args.source / name).read_bytes()
        data = adapt(name, original)
        relative = 'snapshot/' + name
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        files[name] = {'snapshot': relative, 'sha256': digest(data), 'canonicalSha256': digest(original)}
        if name.endswith('/LICENSE'):
            (destination / 'LICENSE').write_bytes(data)
    manifest.write_text(json.dumps({
        'source': 'hello_ymfm third_party/mame-gameboy (current browser WASM core)',
        'license': 'BSD-3-Clause',
        'mameCppRevision': '0e6df89e98f10a05af5fd4df32572982e1709c81',
        'mameHeaderRevision': 'fca00cace9e40ca70d82bbcc6dbd82167325e12b',
        'adaptation': 'Header adds one friend accessor for applied state / gate-only Stop; DSP .cpp unchanged',
        'files': files}, indent=2) + '\n')
    print(f'Native Game Boy synchronized ({len(files)} files)')

if __name__ == '__main__':
    main()
