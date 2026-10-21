#!/usr/bin/env python3
"""Synchronize/check original JavaScript and notices used by the Native NES port."""
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

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--source', type=Path)
    args = parser.parse_args()
    destination = Path(__file__).resolve().parents[1] / 'src-tauri/native-nes'
    manifest_path = destination / 'sources.json'
    if args.check:
        manifest = json.loads(manifest_path.read_text())
        for name, entry in manifest['files'].items():
            path = destination / entry['snapshot']
            if digest(path.read_bytes()) != entry['sha256']:
                raise SystemExit('Native NES source snapshot changed: ' + name)
            if args.source and digest((args.source / name).read_bytes()) != entry['sha256']:
                raise SystemExit('Native NES snapshot differs from current JS: ' + name)
            if name in ALIASES and (destination / ALIASES[name]).read_bytes() != path.read_bytes():
                raise SystemExit('Native NES license alias changed: ' + ALIASES[name])
        print(f"Native NES source snapshot verified ({len(manifest['files'])} files)")
        return
    if not args.source:
        parser.error('synchronizing requires --source /path/to/hello_ymfm')
    destination.mkdir(parents=True, exist_ok=True)
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
    print(f'Native NES source snapshot synchronized ({len(files)} files)')

if __name__ == '__main__':
    main()
