#!/usr/bin/env python3
"""Pin the shared browser/WASM 32X PWM core for direct Native compilation."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['wasm/pwm32x_core.h', 'third_party/mame-32x-pwm/LICENSE',
         'third_party/mame-32x-pwm/README.md']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    destination = Path(__file__).resolve().parents[1] / 'src-tauri/native-pwm'
    manifest = destination / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (destination / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('PWM pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('PWM snapshot differs from browser core: ' + name)
        if (destination / 'LICENSE').read_bytes() != (destination / 'snapshot/third_party/mame-32x-pwm/LICENSE').read_bytes():
            raise SystemExit('PWM license alias changed')
        print(f'Native PWM source verified ({len(info["files"])} files)')
        return
    if not args.source:
        parser.error('--source /path/to/hello_ymfm is required when synchronizing')
    files = {}
    for name in FILES:
        data = (args.source / name).read_bytes()
        relative = 'snapshot/' + name
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        files[name] = {'snapshot': relative, 'sha256': digest(data)}
        if name.endswith('/LICENSE'):
            (destination / 'LICENSE').write_bytes(data)
    manifest.write_text(json.dumps({'source': 'hello_ymfm shared wasm/pwm32x_core.h',
        'license': 'BSD-3-Clause', 'adaptation': 'None; byte-identical shared Browser/WASM/Native core',
        'files': files}, indent=2) + '\n')
    print(f'Native PWM synchronized ({len(files)} files)')

if __name__ == '__main__':
    main()
