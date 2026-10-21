#!/usr/bin/env python3
"""Pin the complete modified LGPL SNES-SPC source and reference resampler."""
import argparse
import hashlib
import json
from pathlib import Path


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[1]
    destination = repository / 'src-tauri/native-snes'
    manifest_path = destination / 'sources.json'
    if args.check:
        manifest = json.loads(manifest_path.read_text())
        for name, entry in manifest['files'].items():
            data = (destination / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('Native SNES snapshot changed: ' + name)
            origin = repository if entry.get('origin') == 'application' else args.source
            if origin and data != (origin / name).read_bytes():
                raise SystemExit('Native SNES snapshot differs from source: ' + name)
        if (destination / 'LICENSE').read_bytes() != (destination / 'snapshot/third_party/snes-spc/LICENSE').read_bytes():
            raise SystemExit('Native SNES LGPL license alias differs')
        print(f"Native SNES verified ({len(manifest['files'])} exact source files)")
        return
    if not args.source:
        parser.error('--source /path/to/hello_ymfm is required when synchronizing')
    source = args.source.resolve()
    names = sorted(str(path.relative_to(source)) for path in (source / 'third_party/snes-spc').rglob('*') if path.is_file())
    names += ['web/spc_resampler.js', 'web/spc.js', 'wasm/spc_wasm.cpp', 'scripts/build_spc_wasm.sh']
    files = {}
    for name in names:
        data = (source / name).read_bytes()
        relative = 'snapshot/' + name
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        files[name] = {'snapshot': relative, 'sha256': digest(data)}
    for name in ['src-tauri/src/native_genesis_snes.cpp', 'src-tauri/src/native_genesis_snes.h',
                 'src-tauri/src/native_genesis_snes_fixture.cpp']:
        data = (repository / name).read_bytes()
        relative = 'snapshot/application/' + name
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        files[name] = {'snapshot': relative, 'sha256': digest(data), 'origin': 'application'}
    (destination / 'LICENSE').write_bytes((source / 'third_party/snes-spc/LICENSE').read_bytes())
    manifest_path.write_text(json.dumps({'source': 'hello_ymfm/third_party/snes-spc',
        'revision': 'ec8ee2bbe30451614c1d02a83f7af1c97d497d45', 'license': 'LGPL-2.1-or-later',
        'adaptation': 'Accurate SPC_DSP, local output mute/RAM wrap/read tracking changes retained verbatim; no SPC700 CPU or ROM.',
        'files': files}, indent=2) + '\n')
    print(f'Native SNES synchronized ({len(files)} exact source files)')


if __name__ == '__main__':
    main()
