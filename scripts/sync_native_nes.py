#!/usr/bin/env python3
"""Synchronize the canonical C++ core; retain/check its pinned JS reference."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = [
    'docs/js/nes_apu_vendor/index.js',
    'docs/js/nes_apu_vendor/channel-square.js',
    'docs/js/nes_apu_vendor/channel-triangle.js',
    'docs/js/nes_apu_vendor/channel-noise.js',
    'docs/js/nes_apu_vendor/channel-dm.js',
    'docs/js/nes_apu_vendor/utils.js',
    'docs/js/nesapuaudioengine.js', 'docs/js/fds_audio.js',
    'web/nesapu.js', 'web/playground_chip_clock.js',
    'third_party/jsnes/README.md', 'third_party/jsnes/AUTHORS.md',
    'third_party/jsnes/LICENSE', 'third_party/fixnes-fds/README.md',
    'third_party/fixnes-fds/LICENSE',
]
ALIASES = {'third_party/jsnes/LICENSE': 'LICENSE-JSNES',
           'third_party/fixnes-fds/LICENSE': 'LICENSE-FDS'}
CORE = {'third_party/tetorica-nes/nes_core.cpp': 'src-tauri/src/native_genesis_nes.cpp',
        'third_party/tetorica-nes/nes_core.h': 'src-tauri/src/native_genesis_nes.h'}

def digest(data):
    return hashlib.sha256(data).hexdigest()

def native_source(name, data):
    if name.endswith('.cpp'):
        before = b'#include "nes_core.h"'
        if data.count(before) != 1:
            raise SystemExit('Canonical NES include changed: ' + name)
        return data.replace(before, b'#include "native_genesis_nes.h"')
    return data

def verify_reference(destination):
    manifest = json.loads((destination / 'sources.json').read_text())
    for name, entry in manifest['files'].items():
        path = destination / entry['snapshot']
        if digest(path.read_bytes()) != entry['sha256']:
            raise SystemExit('Native NES source snapshot changed: ' + name)
        if name in ALIASES and (destination / ALIASES[name]).read_bytes() != path.read_bytes():
            raise SystemExit('Native NES license alias changed: ' + ALIASES[name])
    return len(manifest['files'])

def mirror_core(repository, destination, source, check):
    manifest_path = destination / 'core.json'
    if check:
        manifest = json.loads(manifest_path.read_text())
        for name, entry in manifest['files'].items():
            mirrored = (repository / entry['mirror']).read_bytes()
            if digest(mirrored) != entry['sha256']:
                raise SystemExit('Native NES C++ mirror changed: ' + entry['mirror'])
            if source:
                canonical = (source / name).read_bytes()
                if digest(canonical) != entry['canonicalSha256'] or native_source(name, canonical) != mirrored:
                    raise SystemExit('Native NES mirror differs from canonical core: ' + name)
        return len(manifest['files'])
    files = {}
    for name, relative in CORE.items():
        canonical = (source / name).read_bytes()
        mirrored = native_source(name, canonical)
        path = repository / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(mirrored)
        files[name] = {'mirror': relative, 'canonicalSha256': digest(canonical),
                       'sha256': digest(mirrored)}
    manifest_path.write_text(json.dumps({
        'source': 'hello_ymfm/third_party/tetorica-nes',
        'adaptation': 'nes_core.h include renamed to native_genesis_nes.h; DSP unchanged',
        'files': files,
    }, indent=2) + '\n')
    return len(files)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--source', type=Path)
    parser.add_argument('--refresh-reference', action='store_true',
                        help='explicitly replace the pinned JS reference after a DSP parity review')
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[1]
    destination = repository / 'src-tauri/native-nes'
    manifest_path = destination / 'sources.json'
    if args.check and args.refresh_reference:
        parser.error('--check and --refresh-reference are mutually exclusive')
    if args.check:
        count = verify_reference(destination)
        core_count = mirror_core(repository, destination, args.source, True)
        print(f'Native NES verified ({count} pinned reference files, {core_count} C++ mirrors)')
        return
    if not args.source:
        parser.error('synchronizing requires --source /path/to/hello_ymfm')
    destination.mkdir(parents=True, exist_ok=True)
    if not args.refresh_reference:
        count = verify_reference(destination)
        core_count = mirror_core(repository, destination, args.source, False)
        print(f'Native NES C++ synchronized ({core_count} files); pinned JS reference retained ({count} files)')
        return
    files = {}
    for name in FILES:
        data = (args.source / name).read_bytes()
        relative = 'snapshot/' + name
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        files[name] = {'snapshot': relative, 'sha256': digest(data)}
        if name in ALIASES:
            (destination / ALIASES[name]).write_bytes(data)
    manifest_path.write_text(json.dumps({
        'source': 'hello_ymfm current deployed JS NES/FDS',
        'jsnes_revision': 'b8a45d088e922b6abcd87c4eb1b7237919e9d3d2',
        'fixnes_revision': '156fcaca9f4cd9c23d423737994c45cfb05d16ca',
        'licenses': ['Apache-2.0', 'MIT'], 'files': files,
    }, indent=2) + '\n')
    core_count = mirror_core(repository, destination, args.source, False)
    print(f'Native NES reference explicitly refreshed ({len(files)} files), C++ synchronized ({core_count} files)')

if __name__ == '__main__':
    main()
